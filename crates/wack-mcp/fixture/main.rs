//! A deterministic MCP server for tests and manual checks, replacing version 2's
//! `mock-mcp-server.mjs`. Plain JSON-RPC, using only tokio and serde_json.
//!
//!   wack-mcp-fixture                              stdio (what a "command on your Mac" server runs)
//!   wack-mcp-fixture --http 0                     Streamable HTTP at /mcp, legacy SSE at /sse (0: any port)
//!   ... --require-auth <token>                    HTTP only: 401 unless "Authorization: Bearer <token>"
//!   ... --session                                 HTTP only: hand out an Mcp-Session-Id and require it
//!   ... --sse-replies                             HTTP only: answer POSTs with an event stream
//!   ... --redirect <url>                          HTTP only: /moved redirects there (307)
//!   ... --startup-delay <ms>                      Delay every answer until this much time has passed
//!   ... --startup-gate <file> --tools-file <file> Hold answers until the file exists; list these tools
//!   ... --ready-file <file>                       Written when tools/list is answered
//!
//! Tools: echo (text back), peek (marked read-only), whoami (the Authorization header it saw,
//! $MOCK_MCP_SECRET and its working directory), fail (an error result), and hang (never answers,
//! for timeout and Stop tests).
//!
//! The HTTP listener prints `wack-mcp-fixture: http://127.0.0.1:<port>/mcp` on its first line.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::mpsc,
};

#[derive(Clone, Default)]
struct Options {
    http: Option<u16>,
    token: Option<String>,
    session: bool,
    sse_replies: bool,
    redirect: Option<String>,
    delay: u64,
    gate: Option<String>,
    tools_file: Option<String>,
    ready_file: Option<String>,
}

fn options() -> Options {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| args.iter().position(|arg| arg == name).and_then(|index| args.get(index + 1)).cloned();
    Options {
        http: value("--http").and_then(|port| port.parse().ok()),
        token: value("--require-auth"),
        session: args.iter().any(|arg| arg == "--session"),
        sse_replies: args.iter().any(|arg| arg == "--sse-replies"),
        redirect: value("--redirect"),
        delay: value("--startup-delay").and_then(|delay| delay.parse().ok()).unwrap_or(0),
        gate: value("--startup-gate"),
        tools_file: value("--tools-file"),
        ready_file: value("--ready-file"),
    }
}

fn tools() -> Value {
    json!([
        { "name": "echo", "description": "Return the given text unchanged.",
          "inputSchema": { "type": "object", "properties": { "text": { "type": "string", "description": "Text to echo." } }, "required": ["text"] } },
        { "name": "peek", "description": "Read a fixed value without changing anything.",
          "inputSchema": { "type": "object", "properties": {} }, "annotations": { "readOnlyHint": true } },
        { "name": "whoami", "description": "Report the Authorization header, the MOCK_MCP_SECRET variable and the working directory the server sees.",
          "inputSchema": { "type": "object", "properties": {} }, "annotations": { "readOnlyHint": true } },
        { "name": "fail", "description": "Always return an error result.", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "hang", "description": "Never answer.", "inputSchema": { "type": "object", "properties": {} } },
    ])
}

struct Server {
    options: Options,
    started: Instant,
}

impl Server {
    async fn ready(&self) {
        let delay = Duration::from_millis(self.options.delay).saturating_sub(self.started.elapsed());
        tokio::time::sleep(delay).await;
        if let Some(gate) = &self.options.gate {
            while tokio::fs::metadata(gate).await.is_err() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }

    /// A JSON-RPC answer, `None` for a notification, or never (hang).
    async fn handle(&self, message: Value, authorization: Option<String>) -> Option<Value> {
        self.ready().await;
        let id = message.get("id").cloned().filter(|id| !id.is_null())?;
        let method = message["method"].as_str().unwrap_or_default();
        let params = &message["params"];
        let result = |value: Value| Some(json!({ "jsonrpc": "2.0", "id": id, "result": value }));
        match method {
            "initialize" => result(json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "wack-mcp-fixture", "version": "1.0.0" },
            })),
            "ping" => result(json!({})),
            "tools/list" => {
                if let Some(file) = &self.options.ready_file {
                    let _ = tokio::fs::write(file, "ready").await;
                }
                let listed = match &self.options.tools_file {
                    Some(file) => serde_json::from_str(&tokio::fs::read_to_string(file).await.unwrap_or_default()).unwrap_or(Value::Null),
                    None => tools(),
                };
                // A tools file that is not a list breaks tools/list's shape on purpose.
                result(if listed.is_array() { json!({ "tools": listed }) } else { listed })
            }
            "tools/call" => {
                let input = &params["arguments"];
                match params["name"].as_str().unwrap_or_default() {
                    "echo" => result(json!({ "content": [{ "type": "text", "text": input["text"].as_str().unwrap_or_default() }] })),
                    "peek" => result(json!({ "content": [{ "type": "text", "text": "peeked" }] })),
                    "whoami" => {
                        let report = json!({
                            "authorization": authorization,
                            "secret": std::env::var("MOCK_MCP_SECRET").ok(),
                            "cwd": std::env::current_dir().ok().map(|path| path.display().to_string()),
                        });
                        result(json!({ "content": [{ "type": "text", "text": report.to_string() }] }))
                    }
                    "fail" => result(json!({ "content": [{ "type": "text", "text": "The mock tool failed on purpose." }], "isError": true })),
                    "hang" => std::future::pending().await,
                    name => Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": format!("Unknown tool: {name}") } })),
                }
            }
            _ => Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("Method not found: {method}") } })),
        }
    }
}

#[tokio::main]
async fn main() {
    let options = options();
    let server = Arc::new(Server { options: options.clone(), started: Instant::now() });
    match options.http {
        None => stdio(server).await,
        Some(port) => http(server, port).await,
    }
}

async fn stdio(server: Arc<Server>) {
    let (sender, mut receiver) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(reply) = receiver.recv().await {
            let _ = stdout.write_all(format!("{reply}\n").as_bytes()).await;
            let _ = stdout.flush().await;
        }
    });
    eprintln!("wack-mcp-fixture ready on stdio");
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        let (server, sender) = (server.clone(), sender.clone());
        tokio::spawn(async move {
            if let Some(reply) = server.handle(message, None).await {
                let _ = sender.send(reply);
            }
        });
    }
}

struct Request {
    method: String,
    path: String,
    query: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn read_request(stream: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    stream.read_line(&mut line).await.ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let (path, query) = target.split_once('?').map_or((target.clone(), String::new()), |(path, query)| (path.to_string(), query.to_string()));
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        stream.read_line(&mut header).await.ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let length: usize = headers.get("content-length").and_then(|length| length.parse().ok()).unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await.ok()?;
    Some(Request { method, path, query, headers, body })
}

async fn respond(stream: &mut BufReader<TcpStream>, status: &str, headers: &[(&str, String)], body: &str) {
    let mut text = format!("HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n", body.len());
    for (name, value) in headers {
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    text.push_str("\r\n");
    text.push_str(body);
    let _ = stream.get_mut().write_all(text.as_bytes()).await;
    let _ = stream.get_mut().shutdown().await;
}

type Streams = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<String>>>>;

async fn http(server: Arc<Server>, port: u16) {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    println!("wack-mcp-fixture: http://127.0.0.1:{port}/mcp");
    let streams: Streams = Arc::new(Mutex::new(HashMap::new()));
    let sessions = Arc::new(AtomicU64::new(0));
    while let Ok((stream, _)) = listener.accept().await {
        let (server, streams, sessions) = (server.clone(), streams.clone(), sessions.clone());
        tokio::spawn(async move {
            let mut stream = BufReader::new(stream);
            let Some(request) = read_request(&mut stream).await else { return };
            serve(&server, request, stream, &streams, &sessions).await;
        });
    }
}

async fn serve(server: &Server, request: Request, mut stream: BufReader<TcpStream>, streams: &Streams, sessions: &AtomicU64) {
    let options = &server.options;
    let authorization = request.headers.get("authorization").cloned();
    if let Some(token) = &options.token
        && authorization.as_deref() != Some(format!("Bearer {token}").as_str())
    {
        return respond(&mut stream, "401 Unauthorized", &[("content-type", "application/json".into())], "{\"error\":\"unauthorized\"}").await;
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/moved") => match &options.redirect {
            Some(target) => respond(&mut stream, "307 Temporary Redirect", &[("location", target.clone())], "").await,
            None => respond(&mut stream, "307 Temporary Redirect", &[("location", "/mcp".into())], "").await,
        },
        ("POST", "/mcp") => {
            let Ok(message) = serde_json::from_slice::<Value>(&request.body) else {
                return respond(&mut stream, "400 Bad Request", &[], "").await;
            };
            let initialize = message["method"] == "initialize";
            let mut headers: Vec<(&str, String)> = Vec::new();
            if options.session {
                if initialize {
                    headers.push(("mcp-session-id", format!("session-{}", sessions.fetch_add(1, Ordering::SeqCst) + 1)));
                } else if !request.headers.contains_key("mcp-session-id") {
                    return respond(&mut stream, "400 Bad Request", &[], "").await;
                } else if request.headers.get("mcp-session-id").map(String::as_str) == Some("session-expired") {
                    return respond(&mut stream, "404 Not Found", &[], "").await;
                }
            }
            let Some(reply) = server.handle(message, authorization).await else {
                return respond(&mut stream, "202 Accepted", &headers, "").await;
            };
            if options.sse_replies {
                headers.push(("content-type", "text/event-stream".into()));
                let body = format!("event: message\ndata: {reply}\n\n");
                respond(&mut stream, "200 OK", &headers, &body).await;
            } else {
                headers.push(("content-type", "application/json".into()));
                respond(&mut stream, "200 OK", &headers, &reply.to_string()).await;
            }
        }
        ("GET", "/sse") => {
            let id = format!("s{}", sessions.fetch_add(1, Ordering::SeqCst) + 1);
            let (sender, mut receiver) = mpsc::unbounded_channel::<String>();
            streams.lock().unwrap().insert(id.clone(), sender);
            let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: keep-alive\r\n\r\n";
            let socket = stream.get_mut();
            if socket.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            let endpoint = match &options.redirect {
                Some(elsewhere) => format!("{elsewhere}?sessionId={id}"),
                None => format!("/messages?sessionId={id}"),
            };
            let _ = socket.write_all(format!("event: endpoint\ndata: {endpoint}\n\n").as_bytes()).await;
            while let Some(event) = receiver.recv().await {
                if socket.write_all(event.as_bytes()).await.is_err() {
                    break;
                }
            }
            streams.lock().unwrap().remove(&id);
        }
        ("POST", "/messages") => {
            let id = request.query.split('&').find_map(|pair| pair.strip_prefix("sessionId=")).unwrap_or_default().to_string();
            let Some(sender) = streams.lock().unwrap().get(&id).cloned() else {
                return respond(&mut stream, "404 Not Found", &[], "").await;
            };
            let Ok(message) = serde_json::from_slice::<Value>(&request.body) else {
                return respond(&mut stream, "400 Bad Request", &[], "").await;
            };
            respond(&mut stream, "202 Accepted", &[], "").await;
            if let Some(reply) = server.handle(message, authorization).await {
                let _ = sender.send(format!("event: message\ndata: {reply}\n\n"));
            }
        }
        _ => respond(&mut stream, "404 Not Found", &[], "").await,
    }
}
