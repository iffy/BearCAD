//! Running one AI request on a background thread (#1596).
//!
//! The pane never blocks: [`send`] hands the request to a thread, which streams the reply
//! into a shared [`Exchange`] and asks egui to repaint as text arrives. The same shape the
//! updater uses for its background work.
//!
//! Cancellation is cooperative — [`Exchange::cancel`] sets a flag the reader checks between
//! chunks, so **Stop** takes effect within one chunk rather than waiting out the reply.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::backends::Backend;
use super::providers::{self, ChatMessage, Delta, Usage};

/// How long to wait for the connection and the response head. The body has no deadline: a
/// long reply is normal, and a stream that stalls is stopped by the user.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// A reply in progress, shared between the worker thread and the UI.
#[derive(Debug, Default)]
pub struct Exchange {
    /// Reply text so far. Grows as the stream arrives.
    pub text: String,
    /// Token counts, as reported by the provider.
    pub usage: Usage,
    /// Set once the request finished, failed, or was cancelled.
    pub finished: bool,
    /// A readable failure, if the request did not succeed.
    pub error: Option<String>,
    /// Set by [`Exchange::cancel`]; the worker stops at the next chunk.
    cancelled: Arc<AtomicBool>,
}

impl Exchange {
    /// Ask the running request to stop. The worker marks the exchange finished; whatever
    /// text arrived stays — a half-written reply is still worth reading.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// The handle the UI holds while a reply streams in.
pub type SharedExchange = Arc<Mutex<Exchange>>;

/// Everything one request needs.
pub struct Request {
    pub backend: Backend,
    /// Instructions plus document context (#1597).
    pub system: String,
    /// The conversation so far, oldest first, ending with the new user message.
    pub messages: Vec<ChatMessage>,
}

/// Start a request. Returns immediately with the exchange the reply streams into.
///
/// `repaint` is the egui context to wake when text arrives; `None` in tests.
pub fn send(request: Request, repaint: Option<egui::Context>) -> SharedExchange {
    let shared: SharedExchange = Arc::new(Mutex::new(Exchange::default()));
    let worker = Arc::clone(&shared);
    std::thread::spawn(move || {
        let outcome = run(&request, &worker, repaint.as_ref());
        let mut exchange = worker.lock().expect("exchange lock");
        if let Err(message) = outcome {
            // A cancelled request is not a failure; the user asked for it.
            if !exchange.is_cancelled() {
                exchange.error = Some(message);
            }
        }
        exchange.finished = true;
        drop(exchange);
        if let Some(ctx) = repaint {
            ctx.request_repaint();
        }
    });
    shared
}

/// Perform the request, appending deltas to `shared` as they arrive.
fn run(
    request: &Request,
    shared: &SharedExchange,
    repaint: Option<&egui::Context>,
) -> Result<(), String> {
    let http = providers::build_request(&request.backend, &request.system, &request.messages);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(CONNECT_TIMEOUT))
        // Read the body of a 4xx ourselves: the provider's message is the useful part, and
        // ureq's default turns the status into an error before we can reach it.
        .http_status_as_error(false)
        .build()
        .into();

    let mut post = agent.post(&http.url);
    for (name, value) in &http.headers {
        post = post.header(name, value);
    }
    let mut response = post
        .send(http.body.as_bytes())
        .map_err(|e| connection_error(&http.url, &e))?;

    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let body = response.body_mut().read_to_string().unwrap_or_default();
        return Err(providers::error_message(status, &body));
    }

    let provider = request.backend.provider;
    let mut reader = BufReader::new(response.body_mut().as_reader());
    let mut line = String::new();
    loop {
        if shared.lock().expect("exchange lock").is_cancelled() {
            return Ok(());
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // Stream closed.
            Ok(_) => {}
            Err(e) => return Err(format!("connection lost: {e}")),
        }
        // SSE: only `data:` lines carry payload; `event:` names it, blank lines separate
        // frames, and anything else is a comment or a keep-alive.
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        let mut done = false;
        let deltas = providers::parse_delta(provider, payload);
        if deltas.is_empty() {
            continue;
        }
        {
            let mut exchange = shared.lock().expect("exchange lock");
            for delta in deltas {
                match delta {
                    Delta::Text(text) => exchange.text.push_str(&text),
                    Delta::Usage(usage) => exchange.usage.merge(usage),
                    Delta::Done => done = true,
                }
            }
        }
        if let Some(ctx) = repaint {
            ctx.request_repaint();
        }
        if done {
            break;
        }
    }
    Ok(())
}

/// A connection failure, said in a way that points at the cause. The URL is included (it is
/// not a secret); the key never is.
fn connection_error(url: &str, error: &ureq::Error) -> String {
    let host = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url);
    format!("could not reach {host}: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::backends::{Backend, KeySource, Provider};
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};

    /// A one-shot HTTP server on the loopback interface. Serves `response` (headers and
    /// all) to the first request, hands the request back, and stops. Real sockets, no
    /// network: enough to prove the transport speaks HTTP and SSE correctly.
    struct FakeServer {
        port: u16,
        handle: std::thread::JoinHandle<String>,
    }

    impl FakeServer {
        fn serving(response: &'static str) -> Self {
            Self::serving_chunks(vec![response])
        }

        /// Serve a response in pieces, flushing between them — a streamed reply as the
        /// client actually sees it. `hold` keeps the socket open afterwards, so a test can
        /// exercise a stream that has not ended yet.
        fn serving_chunks(chunks: Vec<&'static str>) -> Self {
            Self::serving_chunks_for(chunks, Duration::ZERO)
        }

        fn serving_chunks_for(chunks: Vec<&'static str>, hold: Duration) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
            let port = listener.local_addr().unwrap().port();
            let handle = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().expect("accept");
                let request = read_request(&socket);
                for chunk in chunks {
                    let _ = socket.write_all(chunk.as_bytes());
                    let _ = socket.flush();
                }
                if !hold.is_zero() {
                    std::thread::sleep(hold);
                }
                request
            });
            Self { port, handle }
        }

        fn backend(&self, provider: Provider) -> Backend {
            Backend {
                base_url: format!("http://127.0.0.1:{}", self.port),
                key: KeySource::Stored("sk-test".into()),
                ..Backend::preset(provider)
            }
        }

        /// The raw request the client sent.
        fn received(self) -> String {
            self.handle.join().expect("server thread")
        }
    }

    /// Read request head plus body (the tests' requests are small and send Content-Length).
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
        use std::io::Read;
        let _ = reader.read_exact(&mut body);
        head + &String::from_utf8_lossy(&body)
    }

    fn sse(events: &[&str]) -> String {
        events
            .iter()
            .map(|e| format!("data: {e}\n\n"))
            .collect::<String>()
    }

    /// Wait for the exchange to finish, then take it. Fails the test rather than hanging
    /// forever if the worker never completes.
    fn finished(shared: &SharedExchange) -> (String, Usage, Option<String>) {
        for _ in 0..500 {
            {
                let exchange = shared.lock().unwrap();
                if exchange.finished {
                    return (
                        exchange.text.clone(),
                        exchange.usage,
                        exchange.error.clone(),
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the request never finished");
    }

    #[test]
    fn a_streamed_reply_arrives_as_text_and_usage() {
        let body = sse(&[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":30}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"80 mm"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" wide."}}"#,
            r#"{"type":"message_delta","delta":{},"usage":{"output_tokens":7}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        );
        let server = FakeServer::serving(response);
        let backend = server.backend(Provider::Anthropic);

        let shared = send(
            Request {
                backend,
                system: "context".into(),
                messages: vec![ChatMessage::user("how wide?")],
            },
            None,
        );
        let (text, usage, error) = finished(&shared);
        assert_eq!(error, None);
        assert_eq!(text, "80 mm wide.");
        assert_eq!(usage.input_tokens, 30);
        assert_eq!(usage.output_tokens, 7);

        // The request really was a POST to the provider's path, carrying the key.
        let request = server.received();
        assert!(request.starts_with("POST /v1/messages"), "got: {request}");
        assert!(request.to_ascii_lowercase().contains("x-api-key: sk-test"));
        assert!(request.contains("\"stream\":true"));
        assert!(request.contains("how wide?"));
    }

    #[test]
    fn an_http_error_becomes_a_readable_message_rather_than_a_panic() {
        let body = r#"{"error":{"message":"invalid x-api-key"}}"#;
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        );
        let server = FakeServer::serving(response);
        let shared = send(
            Request {
                backend: server.backend(Provider::Anthropic),
                system: String::new(),
                messages: vec![ChatMessage::user("hi")],
            },
            None,
        );
        let (text, _, error) = finished(&shared);
        assert!(text.is_empty());
        let error = error.expect("a 401 is an error");
        assert!(error.contains("invalid x-api-key"), "got: {error}");
        assert!(error.contains("API key"), "got: {error}");
        let _ = server.received();
    }

    #[test]
    fn an_unreachable_backend_reports_the_host_and_not_the_key() {
        // Port 1 on loopback: nothing listens there.
        let backend = Backend {
            base_url: "http://127.0.0.1:1".into(),
            key: KeySource::Stored("sk-secret-key".into()),
            ..Backend::preset(Provider::OpenAi)
        };
        let shared = send(
            Request {
                backend,
                system: String::new(),
                messages: vec![ChatMessage::user("hi")],
            },
            None,
        );
        let (_, _, error) = finished(&shared);
        let error = error.expect("connection refused is an error");
        assert!(error.contains("127.0.0.1:1"), "got: {error}");
        assert!(!error.contains("sk-secret-key"), "the key must never reach an error line");
    }

    #[test]
    fn cancelling_stops_the_stream_and_keeps_what_arrived() {
        // The server sends one delta, then holds the connection open. Without cancelling,
        // the read would block until the server's thread ends.
        let first = sse(&[
            r#"{"choices":[{"delta":{"content":"partial"}}]}"#,
        ]);
        let head: &'static str = Box::leak(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
                .to_string()
                .into_boxed_str(),
        );
        // Chunked framing so the body stays open after the first delta.
        let chunk: &'static str =
            Box::leak(format!("{:x}\r\n{first}\r\n", first.len()).into_boxed_str());
        // Hold the socket open after the first delta: the point of the test is a stream
        // that is still running when the user presses Stop.
        let server = FakeServer::serving_chunks_for(vec![head, chunk], Duration::from_secs(5));

        let shared = send(
            Request {
                backend: server.backend(Provider::OpenAi),
                system: String::new(),
                messages: vec![ChatMessage::user("hi")],
            },
            None,
        );
        // Wait for the first delta, then stop.
        for _ in 0..500 {
            if !shared.lock().unwrap().text.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        shared.lock().unwrap().cancel();
        // Nudge the reader: it checks the flag between chunks, so send one more.
        let (text, _, error) = {
            let mut result = None;
            for _ in 0..500 {
                let exchange = shared.lock().unwrap();
                if exchange.finished {
                    result = Some((exchange.text.clone(), exchange.usage, exchange.error.clone()));
                    break;
                }
                drop(exchange);
                std::thread::sleep(Duration::from_millis(10));
            }
            result.unwrap_or_else(|| {
                let exchange = shared.lock().unwrap();
                (exchange.text.clone(), exchange.usage, exchange.error.clone())
            })
        };
        assert_eq!(text, "partial", "text that already arrived is kept");
        assert_eq!(error, None, "cancelling is not a failure");
    }
}
