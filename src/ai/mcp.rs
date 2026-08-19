//! A local MCP server, so an outside agent can drive the document that is open on screen
//! (#1605).
//!
//! **Off until switched on.** When running it listens on **127.0.0.1 only**, and every
//! request must carry a bearer token the app generated. A request from anywhere else, or
//! without the token, is refused before it reaches the document.
//!
//! Shape: JSON-RPC 2.0 over HTTP POST, which is MCP's streamable-HTTP transport in its
//! single-response form. `initialize` and `tools/list` are answered by the listener thread
//! on its own; `tools/call` is handed to the UI thread — the only thread allowed near the
//! document — through a channel, and the listener waits for the answer.
//!
//! The tool surface is deliberately small. `run_lua` already reaches the whole scripting
//! API (the published agent skill teaches it), so a hundred one-per-verb tools would add
//! nothing but noise in the agent's context.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

/// How long a tool call may take before the listener gives up on the UI thread. Generous:
/// a rebuild after a big edit is slow, but a wedged app should not hold a socket forever.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// The MCP protocol version this server implements.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// One tool call on its way to the UI thread.
pub struct Job {
    pub tool: String,
    pub arguments: Value,
    /// Where the answer goes. `Ok` is text for the agent; `Err` is a tool error.
    pub reply: SyncSender<Result<String, String>>,
}

/// One handled request, for the pane's activity log (#1606).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    /// Method or tool name.
    pub what: String,
    /// Whether it succeeded.
    pub ok: bool,
    /// A short detail — the error, or the first line of the result.
    pub detail: String,
}

/// A running server. Dropping it stops the listener.
///
/// `Debug` prints the port but never the token: a `{:?}` of app state reaches logs.
pub struct Server {
    port: u16,
    /// Shared with the listener thread so a new token takes effect without rebinding the
    /// socket — restarting to change a token would race the old listener for the port.
    token: Arc<Mutex<String>>,
    /// Tool calls waiting for the UI thread.
    jobs: Receiver<Job>,
    shutdown: Arc<AtomicBool>,
    log: Arc<Mutex<Vec<LogEntry>>>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpServer")
            .field("port", &self.port)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Server {
    /// Start listening on loopback. `port` of 0 picks a free one.
    pub fn start(port: u16, token: String) -> Result<Self, String> {
        // 127.0.0.1, never 0.0.0.0: this is a local tool, not a network service.
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        // A listener that was just stopped can take a moment to let go of the port (its
        // thread notices the flag on its next poll), so a restart on the same port retries
        // briefly rather than failing on a race the user did not cause.
        let deadline = Instant::now() + Duration::from_millis(600);
        let listener = loop {
            match TcpListener::bind(addr) {
                Ok(listener) => break listener,
                Err(e) if Instant::now() < deadline => {
                    let _ = e;
                    std::thread::sleep(Duration::from_millis(30));
                }
                Err(e) => return Err(format!("cannot listen on 127.0.0.1:{port}: {e}")),
            }
        };
        let port = listener
            .local_addr()
            .map_err(|e| format!("cannot read the port: {e}"))?
            .port();
        // A short accept timeout so the thread notices `shutdown` without a connection.
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("cannot configure the listener: {e}"))?;

        let (jobs_tx, jobs) = std::sync::mpsc::channel::<Job>();
        let shutdown = Arc::new(AtomicBool::new(false));
        let log: Arc<Mutex<Vec<LogEntry>>> = Arc::new(Mutex::new(Vec::new()));

        let token = Arc::new(Mutex::new(token));
        let thread_token = Arc::clone(&token);
        let thread_shutdown = Arc::clone(&shutdown);
        let thread_log = Arc::clone(&log);
        std::thread::spawn(move || {
            while !thread_shutdown.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, peer)) => {
                        // Belt and braces: the bind is loopback-only, and a non-loopback
                        // peer is refused anyway.
                        if !peer.ip().is_loopback() {
                            continue;
                        }
                        // The listener is non-blocking so it can notice `shutdown` without
                        // a connection; an accepted socket inherits that on some platforms,
                        // which turns a client that has not finished sending into a
                        // "malformed request". Put this one back into blocking mode.
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_nodelay(true);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                        let token = thread_token.lock().map(|t| t.clone()).unwrap_or_default();
                        handle_connection(stream, &token, &jobs_tx, &thread_log);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self { port, token, jobs, shutdown, log })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The bearer token clients must send. Not returned by status queries — it goes to the
    /// clipboard and to a client config, nowhere else (#1606).
    pub fn token(&self) -> String {
        self.token.lock().map(|t| t.clone()).unwrap_or_default()
    }

    /// Require a different token from now on. Takes effect on the next request; the socket
    /// is left alone.
    pub fn set_token(&self, token: String) {
        if let Ok(mut current) = self.token.lock() {
            *current = token;
        }
    }

    /// The URL a client connects to.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    /// Tool calls waiting to be run against the document. Called from the frame loop.
    pub fn next_job(&self) -> Option<Job> {
        self.jobs.try_recv().ok()
    }

    /// The most recent requests, oldest first.
    pub fn log(&self) -> Vec<LogEntry> {
        self.log.lock().map(|l| l.clone()).unwrap_or_default()
    }

    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

/// A random-enough token for a loopback service: time and address entropy, hex encoded.
/// Not a secret against someone already running code as this user — it stops other local
/// programs from stumbling into the port.
pub fn generate_token() -> String {
    use std::hash::{BuildHasher, Hasher, RandomState};
    let mut out = String::with_capacity(32);
    for _ in 0..2 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u128(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        hasher.write_usize(&out as *const String as usize);
        out.push_str(&format!("{:016x}", hasher.finish()));
    }
    out
}

/// Read one HTTP request, answer it, close.
fn handle_connection(
    mut stream: TcpStream,
    token: &str,
    jobs: &std::sync::mpsc::Sender<Job>,
    log: &Arc<Mutex<Vec<LogEntry>>>,
) {
    let Some(request) = read_http_request(&mut stream) else {
        let _ = write_http(&mut stream, 400, "text/plain", "bad request");
        return;
    };
    if !request.authorized(token) {
        record(log, "auth", false, "rejected a request without the token");
        let _ = write_http(&mut stream, 401, "application/json", &json!({
            "jsonrpc": "2.0",
            "error": { "code": -32001, "message": "unauthorized: send the bearer token from BearCAD's AI pane" }
        }).to_string());
        return;
    }
    let Ok(message) = serde_json::from_str::<Value>(&request.body) else {
        let _ = write_http(&mut stream, 400, "application/json", &json!({
            "jsonrpc": "2.0",
            "error": { "code": -32700, "message": "parse error" }
        }).to_string());
        return;
    };

    match handle_message(&message, jobs, log) {
        // A notification gets no body: MCP clients accept 202 for those.
        None => {
            let _ = write_http(&mut stream, 202, "text/plain", "");
        }
        Some(response) => {
            let _ = write_http(&mut stream, 200, "application/json", &response.to_string());
        }
    }
}

/// Turn one JSON-RPC message into its response, or `None` for a notification.
fn handle_message(
    message: &Value,
    jobs: &std::sync::mpsc::Sender<Job>,
    log: &Arc<Mutex<Vec<LogEntry>>>,
) -> Option<Value> {
    let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
    let id = message.get("id").cloned();
    // No id means a notification: acknowledged, never answered.
    if id.is_none() {
        record(log, method, true, "notification");
        return None;
    }
    let id = id.unwrap_or(Value::Null);

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "bearcad", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "BearCAD is a parametric CAD app. `run_lua` drives its whole \
                             scripting API; `document_lua` reads the open document back as \
                             the script that recreates it. Lengths are millimetres."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let tool = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            match call_tool(&tool, arguments, jobs) {
                Ok(text) => {
                    record(log, &tool, true, first_line(&text));
                    Ok(json!({
                        "content": [{ "type": "text", "text": text }],
                        "isError": false
                    }))
                }
                Err(error) => {
                    record(log, &tool, false, &error);
                    // A failed tool is a *result*, not a protocol error: the agent needs to
                    // read what went wrong and try something else.
                    Ok(json!({
                        "content": [{ "type": "text", "text": error }],
                        "isError": true
                    }))
                }
            }
        }
        other => Err(format!("unknown method '{other}'")),
    };

    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(message) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": message }
        }),
    })
}

/// Hand a tool call to the UI thread and wait for its answer.
fn call_tool(
    tool: &str,
    arguments: Value,
    jobs: &std::sync::mpsc::Sender<Job>,
) -> Result<String, String> {
    if !TOOLS.iter().any(|t| t.name == tool) {
        return Err(format!(
            "unknown tool '{tool}' — call tools/list for what is available"
        ));
    }
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    jobs.send(Job {
        tool: tool.to_string(),
        arguments,
        reply,
    })
    .map_err(|_| "BearCAD is not accepting requests".to_string())?;
    match answer.recv_timeout(CALL_TIMEOUT) {
        Ok(result) => result,
        Err(_) => Err("BearCAD did not answer in time".to_string()),
    }
}

/// One MCP tool.
struct Tool {
    name: &'static str,
    description: &'static str,
    /// JSON schema for the arguments, as a literal.
    schema: fn() -> Value,
}

/// The tool surface. Small on purpose: `run_lua` reaches the whole scripting API, and the
/// published skill teaches that API, so per-verb tools would only crowd the agent's context.
static TOOLS: &[Tool] = &[
    Tool {
        name: "document_summary",
        description: "What is open right now: each document's name, units, and how much \
                      geometry it holds. Start here.",
        schema: || json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    },
    Tool {
        name: "document_lua",
        description: "The active document as the Lua script that recreates it — the most \
                      complete way to read what is there.",
        schema: || json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    },
    Tool {
        name: "run_lua",
        description: "Run BearCAD Lua against the active document and return what it did. \
                      This is the whole scripting API: bearcad.rect, bearcad.extrude, \
                      bearcad.parameter, bearcad.get, and so on. Lengths are millimetres. \
                      Changes are undoable.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Lua source to run." }
                },
                "required": ["source"],
                "additionalProperties": false
            })
        },
    },
    Tool {
        name: "undo",
        description: "Undo the last change to the active document.",
        schema: || json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    },
    Tool {
        name: "screenshot",
        description: "Render the viewport to a PNG file and return its path — how to see \
                      what the model actually looks like.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Where to write the PNG. Defaults to a temporary file." },
                    "view": { "type": "string", "description": "Optional standard view first: front, back, left, right, top, bottom, iso." }
                },
                "additionalProperties": false
            })
        },
    },
];

fn tool_definitions() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": (tool.schema)()
            })
        })
        .collect()
}

/// The names of every tool, for tests and the pane.
pub fn tool_names() -> Vec<&'static str> {
    TOOLS.iter().map(|t| t.name).collect()
}

fn record(log: &Arc<Mutex<Vec<LogEntry>>>, what: &str, ok: bool, detail: &str) {
    let Ok(mut log) = log.lock() else { return };
    // A bounded log: this is an activity indicator, not an audit trail.
    if log.len() >= 100 {
        log.remove(0);
    }
    log.push(LogEntry {
        what: what.to_string(),
        ok,
        detail: detail.chars().take(160).collect(),
    });
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// A parsed HTTP request: the headers we care about, and the body.
struct HttpRequest {
    authorization: Option<String>,
    body: String,
}

impl HttpRequest {
    fn authorized(&self, token: &str) -> bool {
        match &self.authorization {
            Some(value) => {
                let given = value.trim();
                let given = given.strip_prefix("Bearer ").unwrap_or(given);
                // Constant-ish comparison: length first, then every byte.
                given.len() == token.len()
                    && given
                        .bytes()
                        .zip(token.bytes())
                        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                        == 0
            }
            None => false,
        }
    }
}

fn read_http_request(stream: &mut TcpStream) -> Option<HttpRequest> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    if !line.starts_with("POST") {
        return None;
    }
    let mut length = 0usize;
    let mut authorization = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 {
            break;
        }
        if header.trim().is_empty() {
            break;
        }
        let lower = header.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        } else if lower.starts_with("authorization:") {
            authorization = header
                .split_once(':')
                .map(|(_, value)| value.trim().to_string());
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).ok()?;
    Some(HttpRequest {
        authorization,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn write_http(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Send one JSON-RPC message to the server and return the parsed response (or `None`
    /// for a notification's empty body).
    fn post(port: u16, token: Option<&str>, message: Value) -> (u16, Option<Value>) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        let body = message.to_string();
        let auth = match token {
            Some(token) => format!("Authorization: Bearer {token}\r\n"),
            None => String::new(),
        };
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .expect("write");
        stream.flush().unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read");
        let status: u16 = response
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .expect("status line");
        let body = response.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
        (status, serde_json::from_str(body).ok())
    }

    /// Post `message` from another thread while this one plays the frame loop: draining
    /// jobs and answering them the way the app does. `Server` stays where it belongs — one
    /// thread, the one that owns the document.
    fn post_while_serving(server: &Server, message: Value) -> (u16, Option<Value>) {
        let port = server.port();
        let token = server.token();
        let client = std::thread::spawn(move || post(port, Some(&token), message));

        // Answer whatever arrives until the client has its response.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !client.is_finished() && Instant::now() < deadline {
            match server.next_job() {
                Some(job) => {
                    let answer = match job.tool.as_str() {
                        "run_lua" => Ok(format!(
                            "ran: {}",
                            job.arguments.get("source").and_then(Value::as_str).unwrap_or("")
                        )),
                        "document_summary" => Ok("Untitled: 1 body".to_string()),
                        "undo" => Err("nothing to undo".to_string()),
                        other => Ok(format!("did {other}")),
                    };
                    let _ = job.reply.send(answer);
                }
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        client.join().expect("client thread")
    }

    #[test]
    fn the_server_listens_on_loopback_only() {
        let server = Server::start(0, "token".into()).expect("start");
        assert!(server.port() > 0);
        assert!(server.url().starts_with("http://127.0.0.1:"));
        // Nothing is reachable on an external interface: the bind was loopback, so binding
        // the same port on 0.0.0.0 still succeeds.
        let external = TcpListener::bind((Ipv4Addr::UNSPECIFIED, server.port()));
        assert!(
            external.is_ok(),
            "the server should not have taken the port on every interface"
        );
    }

    #[test]
    fn a_request_without_the_token_is_refused_before_it_reaches_the_document() {
        let server = Server::start(0, "the-real-token".into()).expect("start");
        let (status, body) = post(
            server.port(),
            None,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        );
        assert_eq!(status, 401);
        assert!(body.expect("a body").to_string().contains("unauthorized"));

        let (status, _) = post(
            server.port(),
            Some("guessed"),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        );
        assert_eq!(status, 401, "a wrong token is no better than none");

        // No job ever reached the queue.
        assert!(server.next_job().is_none());
    }

    #[test]
    fn initialize_and_tools_list_answer_without_the_app() {
        let server = Server::start(0, generate_token().to_string()).expect("start");
        let token = server.token();

        let (status, body) = post(
            server.port(),
            Some(&token),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        );
        assert_eq!(status, 200);
        let body = body.expect("a body");
        assert_eq!(body["jsonrpc"], "2.0");
        assert_eq!(body["id"], 1);
        assert_eq!(body["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(body["result"]["serverInfo"]["name"], "bearcad");
        assert!(body["result"]["capabilities"]["tools"].is_object());

        let (_, body) = post(
            server.port(),
            Some(&token),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        );
        let tools = body.expect("a body")["result"]["tools"].clone();
        let names: Vec<String> = tools
            .as_array()
            .expect("tools array")
            .iter()
            .map(|t| t["name"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(names.contains(&"run_lua".to_string()));
        assert!(names.contains(&"document_lua".to_string()));
        // Every tool carries a schema an MCP client can validate against.
        for tool in tools.as_array().unwrap() {
            assert!(tool["description"].as_str().is_some_and(|d| d.len() > 20));
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn a_new_token_takes_effect_without_dropping_the_socket() {
        let server = Server::start(0, "first-token".into()).expect("start");
        let port = server.port();
        server.set_token("second-token".into());
        assert_eq!(server.port(), port, "the socket is left alone");

        let (status, _) = post(port, Some("first-token"), json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }));
        assert_eq!(status, 401, "the old token stops working");
        let (status, _) = post(port, Some("second-token"), json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }));
        assert_eq!(status, 200, "the new one works");
    }

    #[test]
    fn a_tool_call_reaches_the_app_and_its_answer_comes_back() {
        let server = Server::start(0, generate_token()).expect("start");
        let (status, body) = post_while_serving(
            &server,
            json!({
                "jsonrpc": "2.0", "id": 7, "method": "tools/call",
                "params": { "name": "run_lua", "arguments": { "source": "bearcad.new()" } }
            }),
        );
        assert_eq!(status, 200);
        let body = body.expect("a body");
        assert_eq!(body["id"], 7);
        assert_eq!(body["result"]["isError"], false);
        assert_eq!(body["result"]["content"][0]["text"], "ran: bearcad.new()");
    }

    #[test]
    fn a_failing_tool_is_a_result_the_agent_can_read_not_a_protocol_error() {
        let server = Server::start(0, generate_token()).expect("start");
        let (status, body) = post_while_serving(
            &server,
            json!({
                "jsonrpc": "2.0", "id": 8, "method": "tools/call",
                "params": { "name": "undo", "arguments": {} }
            }),
        );
        assert_eq!(status, 200, "still a successful JSON-RPC exchange");
        let body = body.expect("a body");
        assert!(body.get("error").is_none(), "not a protocol error");
        assert_eq!(body["result"]["isError"], true);
        assert_eq!(body["result"]["content"][0]["text"], "nothing to undo");
    }

    #[test]
    fn an_unknown_tool_is_refused_without_bothering_the_app() {
        let server = Server::start(0, generate_token()).expect("start");
        let token = server.token();
        let (_, body) = post(
            server.port(),
            Some(&token),
            json!({
                "jsonrpc": "2.0", "id": 9, "method": "tools/call",
                "params": { "name": "delete_everything", "arguments": {} }
            }),
        );
        let body = body.expect("a body");
        assert_eq!(body["result"]["isError"], true);
        assert!(body["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown tool"));
        assert!(server.next_job().is_none(), "it never reached the document");
    }

    #[test]
    fn an_unknown_method_is_a_protocol_error() {
        let server = Server::start(0, generate_token()).expect("start");
        let token = server.token();
        let (_, body) = post(
            server.port(),
            Some(&token),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/list" }),
        );
        let body = body.expect("a body");
        assert_eq!(body["error"]["code"], -32601);
    }

    #[test]
    fn a_notification_is_accepted_with_no_answer() {
        let server = Server::start(0, generate_token()).expect("start");
        let token = server.token();
        let (status, body) = post(
            server.port(),
            Some(&token),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        );
        assert_eq!(status, 202);
        assert!(body.is_none(), "notifications get no response body");
    }

    #[test]
    fn requests_show_up_in_the_activity_log() {
        let server = Server::start(0, generate_token()).expect("start");
        post_while_serving(
            &server,
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "document_summary", "arguments": {} }
            }),
        );
        post(server.port(), None, json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));

        let log = server.log();
        assert!(
            log.iter().any(|e| e.what == "document_summary" && e.ok),
            "a handled call is logged: {log:?}"
        );
        assert!(
            log.iter().any(|e| e.what == "auth" && !e.ok),
            "a refused request is logged too: {log:?}"
        );
    }

    #[test]
    fn generated_tokens_differ_and_are_long_enough_to_be_worth_having() {
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b);
        assert!(a.len() >= 32, "got {}", a.len());
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
