//! Connecting to a backend with a browser instead of a pasted key (#1624).
//!
//! Where a provider offers it, setting up a backend is one button: **Connect to
//! OpenRouter**. The app opens the provider's page, the user says yes there, and the
//! provider hands back a key of its own — the user never sees an API key, never pastes one,
//! and never has to find the console page it lives on.
//!
//! The flow is [PKCE](https://datatracker.ietf.org/doc/html/rfc7636), which exists for
//! exactly this case: a desktop app has no client secret it could keep. Instead it invents a
//! random **verifier**, sends only its SHA-256 (the **challenge**) to the provider, and
//! proves ownership by producing the verifier when it redeems the code. Someone who
//! intercepts the code cannot use it.
//!
//! Three things make this safe to run on a laptop:
//!
//! - The callback is a **loopback** listener on a port the OS picked. Nothing outside this
//!   machine can reach it, and it serves exactly one request before closing.
//! - The listener only ever yields a code; the key arrives over HTTPS from the provider, in
//!   the response to a request this process made.
//! - The key lands in [`super::backends::KeySource::OAuth`], which redacts itself in `Debug`
//!   like every other credential here.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::backends::{OAuthService, Provider};

/// How long the app waits for the user to finish in the browser before giving up. Long
/// enough to find the tab, log in, and read the permission screen.
const AUTHORIZE_TIMEOUT: Duration = Duration::from_secs(300);

/// How long the token exchange may take once the code is in hand.
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(30);

/// A PKCE verifier and the challenge derived from it.
///
/// The verifier is the secret; the challenge is what goes over the wire first. `Debug`
/// redacts the verifier — a `{:?}` of a half-finished flow must not print it.
#[derive(Clone)]
pub struct Pkce {
    verifier: String,
    challenge: String,
}

impl std::fmt::Debug for Pkce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pkce")
            .field("verifier", &"<redacted>")
            .field("challenge", &self.challenge)
            .finish()
    }
}

impl Pkce {
    /// A fresh pair, from 32 bytes of the operating system's random source.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        // A verifier that is not random is a verifier an attacker can guess. There is no
        // sensible fallback if the OS cannot produce randomness, so say so loudly.
        getrandom::fill(&mut bytes).expect("the OS random source");
        Self::from_verifier(&base64url(&bytes))
    }

    /// The pair for a known verifier — how the RFC's test vector is checked.
    pub fn from_verifier(verifier: &str) -> Self {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(verifier.as_bytes());
        Self {
            verifier: verifier.to_string(),
            challenge: base64url(&digest),
        }
    }

    /// The `code_challenge` to send with the authorization request.
    pub fn challenge(&self) -> &str {
        &self.challenge
    }
}

/// Base64url without padding — the encoding RFC 7636 specifies for both halves.
fn base64url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Where the browser is sent: the provider's authorization page, told where to come back to
/// and what challenge the app will have to answer.
pub fn authorize_url(service: &OAuthService, callback: &str, challenge: &str) -> String {
    format!(
        "{}?callback_url={}&code_challenge={}&code_challenge_method=S256",
        service.authorize_url,
        query_escape(callback),
        query_escape(challenge),
    )
}

/// Percent-encode everything that is not unreserved, so a callback URL survives being a
/// query parameter.
fn query_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The request that trades an authorization code for a key.
///
/// A POST of JSON to the provider's token endpoint, carrying the verifier that matches the
/// challenge sent earlier. No key, no client secret: the verifier is the whole proof.
pub fn token_request(
    base_url: &str,
    service: &OAuthService,
    code: &str,
    verifier: &str,
) -> super::providers::HttpRequest {
    super::providers::HttpRequest {
        url: format!("{}{}", base_url.trim_end_matches('/'), service.token_path),
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: serde_json::json!({
            "code": code,
            "code_verifier": verifier,
            "code_challenge_method": "S256",
        })
        .to_string(),
    }
}

/// The key out of a token response. Providers name the field differently — OpenRouter says
/// `key`, the OAuth spec says `access_token` — so both are accepted.
pub fn parse_token(body: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| "the provider's reply was not JSON".to_string())?;
    for field in ["key", "access_token"] {
        if let Some(key) = value.get(field).and_then(|v| v.as_str()) {
            if !key.trim().is_empty() {
                return Ok(key.to_string());
            }
        }
    }
    // An error body is worth repeating verbatim; it is the provider explaining itself.
    let message = value
        .get("error")
        .and_then(|e| e.get("message").or(Some(e)))
        .and_then(|m| m.as_str())
        .unwrap_or("the provider returned no key");
    Err(message.to_string())
}

/// How a connection attempt is going. Held behind a mutex, read by the pane each frame.
#[derive(Clone)]
pub enum Connect {
    /// The browser is open and the user has not answered yet.
    Waiting,
    /// Done: this is the key to store. Taken once, by the frame loop.
    Connected(String),
    Failed(String),
}

impl std::fmt::Debug for Connect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Waiting => write!(f, "Waiting"),
            Self::Connected(_) => write!(f, "Connected(<redacted>)"),
            Self::Failed(e) => write!(f, "Failed({e})"),
        }
    }
}

/// One connection attempt, running on its own thread.
///
/// Created by [`start`], which returns as soon as the loopback listener is bound — the URL
/// is available immediately so the caller can open a browser at it.
pub struct Flow {
    /// The backend this flow will connect when it finishes.
    pub backend: String,
    /// The page to open in the browser.
    pub authorize_url: String,
    /// Set once the caller has actually opened it, so the frame loop opens it exactly once.
    pub opened: bool,
    state: Arc<Mutex<Connect>>,
    cancelled: Arc<AtomicBool>,
    /// The loopback port, so cancelling can wake the listener by connecting to it.
    port: u16,
}

impl std::fmt::Debug for Flow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Flow")
            .field("backend", &self.backend)
            .field("state", &self.state.lock().map(|s| format!("{s:?}")).ok())
            .finish()
    }
}

impl Flow {
    /// How the attempt is going right now.
    pub fn state(&self) -> Connect {
        self.state
            .lock()
            .map(|s| s.clone())
            .unwrap_or(Connect::Failed("the connection thread died".to_string()))
    }

    /// Stop waiting. The listener wakes, sees the flag, and the thread ends.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        // The accept loop polls, so this is belt and braces — but it makes the thread exit
        // now rather than up to a poll interval later.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

impl Drop for Flow {
    fn drop(&mut self) {
        // A flow the user walked away from must not leave a listener bound.
        self.cancel();
    }
}

/// Begin connecting `backend`.
///
/// Binds the loopback callback, starts the waiting thread, and returns the page to open.
/// Nothing has reached the network yet when this returns: opening the browser is the
/// caller's move, so a test can drive the whole flow without one.
pub fn start(
    backend_id: &str,
    provider: Provider,
    base_url: &str,
    repaint: Option<egui::Context>,
) -> Result<Flow, String> {
    let service = provider
        .oauth()
        .ok_or_else(|| format!("{} has no connect flow — paste an API key", provider.label()))?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("could not open a callback port: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .port();
    listener
        .set_nonblocking(true)
        .map_err(|e| e.to_string())?;

    let pkce = Pkce::generate();
    let callback = format!("http://127.0.0.1:{port}/callback");
    let authorize_url = authorize_url(&service, &callback, pkce.challenge());

    let state = Arc::new(Mutex::new(Connect::Waiting));
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_state = Arc::clone(&state);
    let worker_cancelled = Arc::clone(&cancelled);
    let base_url = base_url.trim_end_matches('/').to_string();
    std::thread::spawn(move || {
        let outcome = run(&listener, &service, &base_url, &pkce, &worker_cancelled);
        // A cancelled flow is not a failure; the pane has already forgotten it.
        if !worker_cancelled.load(Ordering::Relaxed) {
            if let Ok(mut slot) = worker_state.lock() {
                *slot = match outcome {
                    Ok(key) => Connect::Connected(key),
                    Err(message) => Connect::Failed(message),
                };
            }
            if let Some(ctx) = repaint {
                ctx.request_repaint();
            }
        }
    });

    Ok(Flow {
        backend: backend_id.to_string(),
        authorize_url,
        opened: false,
        state,
        cancelled,
        port,
    })
}

/// Wait for the browser's callback, then trade the code for a key.
fn run(
    listener: &TcpListener,
    service: &OAuthService,
    base_url: &str,
    pkce: &Pkce,
    cancelled: &AtomicBool,
) -> Result<String, String> {
    let code = wait_for_code(listener, cancelled)?;
    exchange(base_url, service, &code, &pkce.verifier)
}

/// Accept connections until one carries the callback. Anything else (a stray probe, a
/// favicon request) is answered and ignored.
fn wait_for_code(listener: &TcpListener, cancelled: &AtomicBool) -> Result<String, String> {
    let deadline = Instant::now() + AUTHORIZE_TIMEOUT;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err("cancelled".to_string());
        }
        if Instant::now() > deadline {
            return Err("gave up waiting for the browser".to_string());
        }
        match listener.accept() {
            Ok((stream, _)) => match handle_callback(stream) {
                Some(result) => return result,
                // Not the callback — keep listening.
                None => continue,
            },
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("callback listener failed: {e}")),
        }
    }
}

/// Read one request off the callback socket and answer it in the browser.
///
/// `None` means "that was not the callback"; `Some` carries the code, or the error the
/// provider redirected back with.
fn handle_callback(mut stream: TcpStream) -> Option<Result<String, String>> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).ok()? == 0 {
        return None;
    }
    // "GET /callback?code=… HTTP/1.1"
    let target = line.split_whitespace().nth(1)?;
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut error = None;
    let mut description = None;
    for pair in query.split('&') {
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        match name {
            "code" => code = Some(percent_decode(value)),
            // The description is the sentence a person can read; the code is the fallback.
            "error_description" => description = Some(percent_decode(value)),
            "error" => error = Some(percent_decode(value)),
            _ => {}
        }
    }
    let result = match (code, description.or(error)) {
        (Some(code), _) if !code.trim().is_empty() => Ok(code),
        (_, Some(error)) => Err(error),
        _ => {
            // A request to some other path: not an answer either way.
            let _ = respond(&mut stream, "Nothing to see here.");
            return None;
        }
    };
    let _ = respond(
        &mut stream,
        match &result {
            Ok(_) => "BearCAD is connected. You can close this tab.",
            Err(_) => "BearCAD was not connected. You can close this tab.",
        },
    );
    Some(result)
}

/// The page the browser lands on when the provider redirects back.
fn respond(stream: &mut TcpStream, message: &str) -> std::io::Result<()> {
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>BearCAD</title>\
         <body style=\"font:16px system-ui;padding:3rem\">{message}</body>"
    );
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&value[i + 1..i + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Redeem the code. The only network call this module makes, and the only place the key
/// exists before it reaches the backend's configuration.
fn exchange(
    base_url: &str,
    service: &OAuthService,
    code: &str,
    verifier: &str,
) -> Result<String, String> {
    let request = token_request(base_url, service, code, verifier);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(EXCHANGE_TIMEOUT))
        .timeout_recv_response(Some(EXCHANGE_TIMEOUT))
        // The provider's own message on a 4xx is the useful part.
        .http_status_as_error(false)
        .build()
        .into();
    let mut post = agent.post(&request.url);
    for (name, value) in &request.headers {
        post = post.header(name, value);
    }
    let mut response = post
        .send(request.body.as_bytes())
        .map_err(|e| format!("could not reach {}: {e}", host_of(&request.url)))?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(super::providers::error_message(status, &body));
    }
    parse_token(&body)
}

fn host_of(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// A stand-in for the provider's token endpoint: one request, one canned reply, and the
    /// request body handed back so a test can check what was proved to it.
    struct FakeProvider {
        port: u16,
        handle: std::thread::JoinHandle<String>,
    }

    impl FakeProvider {
        fn replying(status: &'static str, body: &'static str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().unwrap().port();
            let handle = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().expect("accept");
                let request = read_request(&socket);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes());
                let _ = socket.flush();
                request
            });
            Self { port, handle }
        }

        fn base_url(&self) -> String {
            format!("http://127.0.0.1:{}", self.port)
        }

        fn received(self) -> String {
            self.handle.join().expect("provider thread")
        }
    }

    fn read_request(socket: &TcpStream) -> String {
        let mut reader = BufReader::new(socket);
        let mut head = String::new();
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap_or(0);
            }
            let blank = line.trim().is_empty();
            head.push_str(&line);
            if blank {
                break;
            }
        }
        let mut body = vec![0u8; length];
        let _ = reader.read_exact(&mut body);
        head + &String::from_utf8_lossy(&body)
    }

    /// Play the browser: fetch the callback URL the flow is listening on, the way the
    /// provider's redirect would.
    fn visit(url: &str) -> String {
        let rest = url.split("://").nth(1).expect("an http url");
        let (host, path) = rest.split_once('/').expect("a path");
        let mut stream = TcpStream::connect(host).expect("connect to the callback");
        write!(stream, "GET /{path} HTTP/1.1\r\nHost: {host}\r\n\r\n").expect("send");
        let mut body = String::new();
        let _ = stream.read_to_string(&mut body);
        body
    }

    /// The callback URL out of an authorize URL, decoded.
    fn callback_of(authorize_url: &str) -> String {
        let query = authorize_url.split_once('?').expect("a query").1;
        for pair in query.split('&') {
            if let Some(value) = pair.strip_prefix("callback_url=") {
                return percent_decode(value);
            }
        }
        panic!("no callback_url in {authorize_url}");
    }

    fn state_within(flow: &Flow, limit: Duration) -> Connect {
        let deadline = Instant::now() + limit;
        loop {
            match flow.state() {
                Connect::Waiting if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                other => return other,
            }
        }
    }

    #[test]
    fn the_challenge_is_the_sha256_of_the_verifier_in_base64url() {
        // RFC 7636 appendix B, the worked example every implementation is checked against.
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(pkce.challenge(), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        // No padding, and nothing that would need escaping in a URL.
        assert!(!pkce.challenge().contains('='));
        assert!(!pkce.challenge().contains('+') && !pkce.challenge().contains('/'));
    }

    #[test]
    fn a_generated_verifier_is_long_random_and_never_printed() {
        let one = Pkce::generate();
        let two = Pkce::generate();
        assert_ne!(one.challenge(), two.challenge(), "each attempt is its own secret");
        assert!(one.verifier.len() >= 43, "RFC 7636 wants at least 43 characters");
        assert_eq!(Pkce::from_verifier(&one.verifier).challenge(), one.challenge());
        let debugged = format!("{one:?}");
        assert!(!debugged.contains(&one.verifier), "Debug leaked the verifier: {debugged}");
    }

    #[test]
    fn the_authorize_url_carries_the_callback_and_the_challenge() {
        let service = Provider::OpenRouter.oauth().unwrap();
        let url = authorize_url(&service, "http://127.0.0.1:5123/callback", "CHALLENGE");
        assert!(url.starts_with("https://openrouter.ai/auth?"), "got {url}");
        assert!(url.contains("code_challenge=CHALLENGE"));
        assert!(url.contains("code_challenge_method=S256"));
        // The callback is escaped, so its own `:` and `/` cannot end the parameter.
        assert!(url.contains("callback_url=http%3A%2F%2F127.0.0.1%3A5123%2Fcallback"), "got {url}");
        assert_eq!(callback_of(&url), "http://127.0.0.1:5123/callback");
    }

    #[test]
    fn the_exchange_proves_the_verifier_and_reads_the_key_back() {
        let service = Provider::OpenRouter.oauth().unwrap();
        let request = token_request("https://openrouter.ai/api/v1/", &service, "CODE", "VERIFIER");
        assert_eq!(request.url, "https://openrouter.ai/api/v1/auth/keys");
        let body: serde_json::Value = serde_json::from_str(&request.body).unwrap();
        assert_eq!(body["code"], "CODE");
        assert_eq!(body["code_verifier"], "VERIFIER");
        assert_eq!(body["code_challenge_method"], "S256");

        assert_eq!(parse_token(r#"{"key":"sk-or-v1-abc"}"#).unwrap(), "sk-or-v1-abc");
        assert_eq!(parse_token(r#"{"access_token":"tok"}"#).unwrap(), "tok");
        let error = parse_token(r#"{"error":{"message":"code already used"}}"#).unwrap_err();
        assert_eq!(error, "code already used");
        assert!(parse_token("not json").is_err());
    }

    #[test]
    fn connecting_end_to_end_turns_a_browser_visit_into_a_key() {
        // A local stand-in for the provider: no live service, no real credentials.
        let provider = FakeProvider::replying("200 OK", r#"{"key":"sk-or-v1-issued"}"#);
        let flow = start(
            "openrouter",
            Provider::OpenRouter,
            &provider.base_url(),
            None,
        )
        .expect("start the flow");
        assert!(matches!(flow.state(), Connect::Waiting), "nothing until the user answers");

        // The browser comes back with a code, exactly as the redirect would.
        let page = visit(&format!("{}?code=THE-CODE", callback_of(&flow.authorize_url)));
        assert!(page.contains("connected"), "the browser gets a page back: {page}");

        match state_within(&flow, Duration::from_secs(10)) {
            Connect::Connected(key) => assert_eq!(key, "sk-or-v1-issued"),
            other => panic!("expected a key, got {other:?}"),
        }

        // What the app proved to the provider: the code, and the verifier behind the
        // challenge it sent to the browser.
        let request = provider.received();
        assert!(request.starts_with("POST /auth/keys"), "got: {request}");
        assert!(request.contains("THE-CODE"));
        let body = request.split("\r\n\r\n").nth(1).expect("a body");
        let body: serde_json::Value = serde_json::from_str(body).expect("json body");
        let verifier = body["code_verifier"].as_str().expect("a verifier");
        let expected = Pkce::from_verifier(verifier);
        assert!(
            flow.authorize_url.contains(&format!("code_challenge={}", expected.challenge())),
            "the verifier must match the challenge the browser was sent"
        );
    }

    #[test]
    fn a_refused_authorization_fails_the_flow_rather_than_hanging() {
        let provider = FakeProvider::replying("200 OK", r#"{"key":"never-issued"}"#);
        let flow = start("openrouter", Provider::OpenRouter, &provider.base_url(), None)
            .expect("start the flow");
        visit(&format!(
            "{}?error=access_denied&error_description=You%20said%20no",
            callback_of(&flow.authorize_url)
        ));
        match state_within(&flow, Duration::from_secs(10)) {
            Connect::Failed(message) => assert_eq!(message, "You said no"),
            other => panic!("expected a failure, got {other:?}"),
        }
        drop(provider); // Never contacted: there was no code to redeem.
    }

    #[test]
    fn a_provider_that_refuses_the_code_says_why() {
        let provider = FakeProvider::replying(
            "400 Bad Request",
            r#"{"error":{"message":"code_verifier does not match"}}"#,
        );
        let flow = start("openrouter", Provider::OpenRouter, &provider.base_url(), None)
            .expect("start the flow");
        visit(&format!("{}?code=STALE", callback_of(&flow.authorize_url)));
        match state_within(&flow, Duration::from_secs(10)) {
            Connect::Failed(message) => {
                assert!(message.contains("code_verifier does not match"), "got {message}")
            }
            other => panic!("expected a failure, got {other:?}"),
        }
        let _ = provider.received();
    }

    #[test]
    fn a_provider_without_a_flow_cannot_be_connected() {
        // #1624 falls back to a pasted key rather than pretending every provider has PKCE.
        let error = start("claude", Provider::Anthropic, "https://api.anthropic.com", None)
            .expect_err("Anthropic has no connect flow");
        assert!(error.contains("paste an API key"), "got {error}");
    }

    #[test]
    fn cancelling_stops_waiting_and_gives_the_port_back() {
        let flow = start("openrouter", Provider::OpenRouter, "http://127.0.0.1:1", None)
            .expect("start the flow");
        let callback = callback_of(&flow.authorize_url);
        flow.cancel();
        drop(flow);
        // The listener is gone, so the callback address refuses connections again.
        let address = callback
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap()
            .to_string();
        for _ in 0..100 {
            if TcpStream::connect(&address).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the callback listener is still bound after cancelling");
    }
}
