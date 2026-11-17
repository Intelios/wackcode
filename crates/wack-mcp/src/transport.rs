//! The three ways to reach an MCP server, under one JSON-RPC peer.
//!
//! - **stdio**: the server is a child process in its own process group, spoken to in
//!   newline-delimited JSON. It gets the user's login environment (`shell_env`) without provider
//!   keys (`SECRET_ENV`), plus its own variables. Its stderr is kept, the last 4,000 characters,
//!   for error messages only. Closing it sends SIGTERM to the group and SIGKILL two seconds later;
//!   dropping it without closing kills the group at once, so no server outlives its connection.
//! - **Streamable HTTP**: every message is a POST; a request's answer comes back as JSON or as an
//!   event stream. The session id and protocol version the server gives are sent back on every
//!   request. A 404 on a session means it expired: the connection closes and the next message
//!   reconnects.
//! - **Legacy SSE**: a GET stream whose first `endpoint` event names where to POST. That endpoint
//!   must be on the configured origin.
//!
//! Over HTTP the user's headers (often a token) go to the configured origin only: redirects are
//! followed by hand, at most five, and only within that origin; no proxy, no cookies.

use std::{
    collections::HashMap,
    path::Path,
    process::Stdio as ProcessStdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use futures::StreamExt;
use reqwest::{
    Response, StatusCode,
    header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, LOCATION},
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{oneshot, watch},
    task::AbortHandle,
};
use url::Url;
use wack_llm::sse::SseParser;

use crate::spec::{ServerSpec, Transport};

const STDERR_TAIL_CHARS: usize = 4_000;
const MAX_REDIRECTS: usize = 5;
/// One JSON reply larger than this is refused rather than held in memory.
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const USER_AGENT: &str = "WackCode-mcp/1.0";
const SESSION_HEADER: &str = "mcp-session-id";
const VERSION_HEADER: &str = "mcp-protocol-version";

/// Why a request did not get its answer.
#[derive(Debug)]
pub(crate) enum Failure {
    Timeout,
    Stopped,
    /// The connection ended; the reason, when one is known.
    Closed(Option<String>),
    Rpc { code: i64, message: String },
    Http(u16),
    SessionExpired,
    Spawn(std::io::Error),
    /// A sentence already meant for the user.
    Said(String),
}

// -------------------------------------------------------------------------------------------------
// The JSON-RPC peer: requests waiting for answers, and what the server sends unasked.

type Answer = Result<Value, Failure>;

pub(crate) struct Peer {
    next: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<Answer>>>,
    closed: watch::Sender<Option<String>>,
    tools_changed: AtomicBool,
}

impl Peer {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { next: AtomicU64::new(0), pending: Mutex::new(HashMap::new()), closed: watch::channel(None).0, tools_changed: AtomicBool::new(false) })
    }

    pub fn register(&self) -> (u64, oneshot::Receiver<Answer>) {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, sender);
        (id, receiver)
    }

    pub fn forget(&self, id: u64) {
        self.pending.lock().unwrap().remove(&id);
    }

    pub fn fail(&self, id: u64, failure: Failure) {
        if let Some(sender) = self.pending.lock().unwrap().remove(&id) {
            let _ = sender.send(Err(failure));
        }
    }

    pub fn is_pending(&self, id: u64) -> bool {
        self.pending.lock().unwrap().contains_key(&id)
    }

    /// Handles what the server sent. Returns the replies owed to it (to a `ping`, or a refusal).
    pub fn receive(&self, message: Value) -> Vec<Value> {
        if let Value::Array(messages) = message {
            return messages.into_iter().flat_map(|message| self.receive(message)).collect();
        }
        let id = message.get("id").filter(|id| !id.is_null()).cloned();
        if let Some(method) = message.get("method").and_then(Value::as_str) {
            return match id {
                Some(id) if method == "ping" => vec![json!({ "jsonrpc": "2.0", "id": id, "result": {} })],
                Some(id) => vec![json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("WackCode does not support {method}.") } })],
                None => {
                    if method == "notifications/tools/list_changed" {
                        self.tools_changed.store(true, Ordering::SeqCst);
                    }
                    Vec::new()
                }
            };
        }
        let Some(id) = id.and_then(|id| id.as_u64().or_else(|| id.as_str().and_then(|text| text.parse().ok()))) else { return Vec::new() };
        let Some(sender) = self.pending.lock().unwrap().remove(&id) else { return Vec::new() };
        let answer = match message.get("error") {
            Some(error) => Err(Failure::Rpc {
                code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: error.get("message").and_then(Value::as_str).unwrap_or("The server reported an error.").to_string(),
            }),
            None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = sender.send(answer);
        Vec::new()
    }

    /// Ends the connection: every waiting request fails with `reason`. Only the first call counts.
    pub fn close(&self, reason: String) {
        let first = self.closed.send_if_modified(|closed| {
            if closed.is_some() {
                return false;
            }
            *closed = Some(reason.clone());
            true
        });
        if first {
            for (_, sender) in self.pending.lock().unwrap().drain() {
                let _ = sender.send(Err(Failure::Closed(Some(reason.clone()))));
            }
        }
    }

    pub fn closed(&self) -> Option<String> {
        self.closed.borrow().clone()
    }

    pub async fn wait_closed(&self) -> String {
        let mut closed = self.closed.subscribe();
        match closed.wait_for(Option::is_some).await {
            Ok(reason) => reason.clone().unwrap_or_default(),
            Err(_) => String::new(),
        }
    }

    pub fn take_tools_changed(&self) -> bool {
        self.tools_changed.swap(false, Ordering::SeqCst)
    }
}

// -------------------------------------------------------------------------------------------------
// The link to the server.

pub(crate) enum Link {
    Stdio(StdioLink),
    Http(HttpLink),
    Sse(SseLink),
}

impl Link {
    /// Starts or reaches the server. Nothing is said to it yet.
    pub async fn open(spec: &ServerSpec, cwd: &Path, peer: &Arc<Peer>, groups: &Groups) -> Result<Arc<Link>, Failure> {
        match spec.transport {
            Transport::Stdio => StdioLink::open(spec, cwd, peer, groups).await,
            Transport::Http => HttpLink::open(spec, peer),
            Transport::Sse => SseLink::open(spec, peer).await,
        }
    }

    /// Sends one message. For a request (`id`), its answer reaches the peer, here or later.
    pub async fn send(link: &Arc<Link>, peer: &Arc<Peer>, message: &Value, id: Option<u64>) -> Result<(), Failure> {
        match &**link {
            Link::Stdio(stdio) => stdio.write(message).await,
            Link::Http(_) => HttpLink::send(link, peer, message, id).await,
            Link::Sse(sse) => sse.post(message).await,
        }
    }

    /// Ends the connection without waiting: a stdio server is asked to stop (SIGTERM), then
    /// killed two seconds later.
    pub fn close(&self) {
        match self {
            Link::Stdio(stdio) => stdio.close(true),
            Link::Http(http) => http.close(),
            Link::Sse(sse) => abort(&sse.tasks),
        }
    }

    /// Lets go without signalling: the caller is signalling the process group itself.
    pub fn abandon(&self) {
        match self {
            Link::Stdio(stdio) => stdio.close(false),
            Link::Http(http) => abort(&http.tasks),
            Link::Sse(sse) => abort(&sse.tasks),
        }
    }

    pub fn pgid(&self) -> Option<i32> {
        match self {
            Link::Stdio(stdio) => Some(stdio.pgid),
            _ => None,
        }
    }

    /// The last line the server wrote to stderr (stdio only).
    pub fn stderr_line(&self) -> Option<String> {
        let Link::Stdio(stdio) = self else { return None };
        let tail = stdio.stderr.lock().unwrap();
        tail.trim().lines().last().map(|line| line.trim().chars().take(500).collect()).filter(|line: &String| !line.is_empty())
    }

    pub fn set_version(&self, version: &str) {
        if let Link::Http(http) = self {
            *http.version.lock().unwrap() = HeaderValue::from_str(version).ok();
        }
    }
}

fn abort(tasks: &Mutex<Vec<AbortHandle>>) {
    for task in tasks.lock().unwrap().drain(..) {
        task.abort();
    }
}

fn parse_line(line: &str) -> Option<Value> {
    let line = line.trim();
    if line.is_empty() { None } else { serde_json::from_str(line).ok() }
}

// -------------------------------------------------------------------------------------------------
// stdio

pub(crate) struct StdioLink {
    stdin: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
    pgid: i32,
    stderr: Arc<Mutex<String>>,
    tasks: Mutex<Vec<AbortHandle>>,
    /// Kept so the process is reaped; `kill_on_drop` is a last resort behind the group kill.
    _child: Mutex<Option<Child>>,
    closed: AtomicBool,
    groups: Groups,
}

impl StdioLink {
    async fn open(spec: &ServerSpec, cwd: &Path, peer: &Arc<Peer>, groups: &Groups) -> Result<Arc<Link>, Failure> {
        let mut command = Command::new(&spec.command);
        command
            .args(&spec.args)
            .current_dir(cwd)
            .stdin(ProcessStdio::piped())
            .stdout(ProcessStdio::piped())
            .stderr(ProcessStdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        if let Some(environment) = wack_agent::shell_env::wait().await {
            command.envs(environment);
        }
        for name in wack_agent::SECRET_ENV {
            command.env_remove(name);
        }
        // A server's own values reach only its own process.
        command.envs(&spec.env);
        let mut child = command.spawn().map_err(Failure::Spawn)?;
        let pgid = child.id().map(|id| id as i32).unwrap_or(0);
        if pgid > 0 {
            groups.add(pgid);
        }
        let stdin = Arc::new(tokio::sync::Mutex::new(child.stdin.take()));
        let stdout = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let stderr = Arc::new(Mutex::new(String::new()));
        let mut tasks = Vec::new();

        if let Some(mut pipe) = stderr_pipe {
            let tail = stderr.clone();
            tasks.push(
                tokio::spawn(async move {
                    let mut buffer = [0u8; 4096];
                    while let Ok(read) = pipe.read(&mut buffer).await {
                        if read == 0 {
                            break;
                        }
                        let mut tail = tail.lock().unwrap();
                        tail.push_str(&String::from_utf8_lossy(&buffer[..read]));
                        let excess = tail.chars().count().saturating_sub(STDERR_TAIL_CHARS);
                        if excess > 0 {
                            let cut = tail.char_indices().nth(excess).map_or(tail.len(), |(index, _)| index);
                            tail.drain(..cut);
                        }
                    }
                })
                .abort_handle(),
            );
        }
        if let Some(stdout) = stdout {
            let (peer, writer, tail) = (peer.clone(), stdin.clone(), stderr.clone());
            tasks.push(
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let Some(message) = parse_line(&line) else { continue };
                        for reply in peer.receive(message) {
                            let _ = write_line(&writer, &reply).await;
                        }
                    }
                    // Give stderr a moment to catch the server's last words.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    let detail = tail.lock().unwrap().trim().lines().last().map(|line| format!(" Server output: {}", line.trim().chars().take(500).collect::<String>()));
                    peer.close(format!("The server process exited.{}", detail.unwrap_or_default()));
                })
                .abort_handle(),
            );
        }
        Ok(Arc::new(Link::Stdio(StdioLink { stdin, pgid, stderr, tasks: Mutex::new(tasks), _child: Mutex::new(Some(child)), closed: AtomicBool::new(false), groups: groups.clone() })))
    }

    async fn write(&self, message: &Value) -> Result<(), Failure> {
        write_line(&self.stdin, message).await.map_err(|_| Failure::Closed(None))
    }

    fn close(&self, signal: bool) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Ok(mut stdin) = self.stdin.try_lock() {
            stdin.take();
        }
        abort(&self.tasks);
        self.groups.forget(self.pgid);
        if signal && self.pgid > 0 {
            signal_group(self.pgid, libc::SIGTERM);
            let pgid = self.pgid;
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(2));
                signal_group(pgid, libc::SIGKILL);
            });
        }
    }
}

impl Drop for StdioLink {
    fn drop(&mut self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            abort(&self.tasks);
            self.groups.forget(self.pgid);
            signal_group(self.pgid, libc::SIGKILL);
        }
    }
}

async fn write_line(writer: &tokio::sync::Mutex<Option<ChildStdin>>, message: &Value) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    let mut writer = writer.lock().await;
    let stdin = writer.as_mut().ok_or(std::io::ErrorKind::BrokenPipe)?;
    stdin.write_all(&line).await?;
    stdin.flush().await
}

/// The process groups of a hub's stdio servers running now, starting ones and Test connections
/// included, so quitting can end them all.
#[derive(Clone, Default)]
pub(crate) struct Groups(Arc<Mutex<Vec<i32>>>);

impl Groups {
    pub fn live(&self) -> Vec<i32> {
        self.0.lock().unwrap().clone()
    }

    fn add(&self, pgid: i32) {
        self.0.lock().unwrap().push(pgid);
    }

    fn forget(&self, pgid: i32) {
        self.0.lock().unwrap().retain(|live| *live != pgid);
    }
}

/// SIGTERM to every group, then SIGKILL to what is left after a second.
pub(crate) fn terminate_groups(groups: &[i32]) {
    for pgid in groups {
        signal_group(*pgid, libc::SIGTERM);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while groups.iter().any(|pgid| group_alive(*pgid)) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    for pgid in groups {
        signal_group(*pgid, libc::SIGKILL);
    }
}

pub(crate) fn signal_group(pgid: i32, signal: libc::c_int) {
    if pgid > 0 {
        // SAFETY: a plain system call. A negative id addresses the group; if every member has
        // already exited it fails harmlessly.
        unsafe {
            libc::kill(-pgid, signal);
        }
    }
}

pub(crate) fn group_alive(pgid: i32) -> bool {
    // SAFETY: signal 0 only asks whether the group exists.
    pgid > 0 && unsafe { libc::kill(-pgid, 0) } == 0
}

// -------------------------------------------------------------------------------------------------
// HTTP, shared by both HTTP transports

fn client(spec: &ServerSpec) -> Result<(reqwest::Client, HeaderMap), Failure> {
    let mut headers = HeaderMap::new();
    for (name, value) in &spec.headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| Failure::Said(format!("“{name}” isn't a valid header name.")))?;
        let mut value = HeaderValue::from_str(value).map_err(|_| Failure::Said(format!("The value of the header {name} isn't valid.")))?;
        value.set_sensitive(true);
        headers.insert(name, value);
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| Failure::Said(format!("Could not set up the connection: {error}")))?;
    Ok((client, headers))
}

fn parse_url(spec: &ServerSpec) -> Result<Url, Failure> {
    Url::parse(spec.url.trim()).map_err(|_| Failure::Said("The server's URL isn't valid.".into()))
}

/// Sends a request built fresh for each hop, following redirects within `origin` only.
async fn follow(url: &Url, build: impl Fn(Url) -> reqwest::RequestBuilder) -> Result<(Response, Url), Failure> {
    let origin = url.origin();
    let mut url = url.clone();
    for hop in 0..=MAX_REDIRECTS {
        let response = build(url.clone()).send().await.map_err(|error| network(&url, &error))?;
        if !response.status().is_redirection() {
            return Ok((response, url));
        }
        let Some(location) = response.headers().get(LOCATION).and_then(|value| value.to_str().ok()) else { return Ok((response, url)) };
        let next = url.join(location).map_err(|_| Failure::Said("The server redirected to an address that isn't valid.".into()))?;
        if next.origin() != origin {
            return Err(Failure::Said(format!(
                "The server redirected to {}. WackCode only follows redirects within {}, so your headers stay with that server.",
                next.origin().ascii_serialization(),
                origin.ascii_serialization()
            )));
        }
        if hop == MAX_REDIRECTS {
            break;
        }
        url = next;
    }
    Err(Failure::Said("The server redirected too many times.".into()))
}

fn network(url: &Url, error: &reqwest::Error) -> Failure {
    if error.is_timeout() {
        return Failure::Timeout;
    }
    let chain = {
        let mut text = error.to_string();
        let mut source = std::error::Error::source(error);
        while let Some(inner) = source {
            text.push_str(": ");
            text.push_str(&inner.to_string());
            source = inner.source();
        }
        text.to_lowercase()
    };
    let host = url.host_str().unwrap_or("The server").to_string();
    if chain.contains("dns") || chain.contains("failed to lookup") || chain.contains("nodename nor servname") {
        Failure::Said(format!("{host} could not be found."))
    } else if error.is_connect() {
        Failure::Said(format!("Nothing is answering at {}.", url.as_str()))
    } else {
        Failure::Said(format!("Could not reach {host}."))
    }
}

/// Reads a whole JSON body, refusing one too large to hold.
async fn read_json(response: Response) -> Result<Value, Failure> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Failure::Closed(Some("The server closed the connection.".into())))?;
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(Failure::Said("The server's answer was too large.".into()));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| Failure::Said("The server's answer wasn't valid JSON.".into()))
}

fn status_failure(status: StatusCode) -> Failure {
    Failure::Http(status.as_u16())
}

// -------------------------------------------------------------------------------------------------
// Streamable HTTP

pub(crate) struct HttpLink {
    client: reqwest::Client,
    headers: HeaderMap,
    url: Url,
    session: Mutex<Option<HeaderValue>>,
    version: Mutex<Option<HeaderValue>>,
    tasks: Mutex<Vec<AbortHandle>>,
}

impl HttpLink {
    fn open(spec: &ServerSpec, _peer: &Arc<Peer>) -> Result<Arc<Link>, Failure> {
        let (client, headers) = client(spec)?;
        Ok(Arc::new(Link::Http(HttpLink {
            client,
            headers,
            url: parse_url(spec)?,
            session: Mutex::new(None),
            version: Mutex::new(None),
            tasks: Mutex::new(Vec::new()),
        })))
    }

    async fn post(&self, message: &Value) -> Result<Response, Failure> {
        let body = message.to_string();
        let session = self.session.lock().unwrap().clone();
        let version = self.version.lock().unwrap().clone();
        let (response, _) = follow(&self.url, |url| {
            let mut request = self
                .client
                .post(url)
                .headers(self.headers.clone())
                .header(ACCEPT, "application/json, text/event-stream")
                .header(CONTENT_TYPE, "application/json")
                .body(body.clone());
            if let Some(session) = &session {
                request = request.header(SESSION_HEADER, session.clone());
            }
            if let Some(version) = &version {
                request = request.header(VERSION_HEADER, version.clone());
            }
            request
        })
        .await?;
        Ok(response)
    }

    async fn send(link: &Arc<Link>, peer: &Arc<Peer>, message: &Value, id: Option<u64>) -> Result<(), Failure> {
        let Link::Http(http) = &**link else { unreachable!() };
        let response = http.post(message).await?;
        if let Some(session) = response.headers().get(SESSION_HEADER) {
            // Visible ASCII only, as the specification requires.
            if session.as_bytes().iter().all(|byte| (0x21..=0x7e).contains(byte)) {
                *http.session.lock().unwrap() = Some(session.clone());
            }
        }
        let status = response.status();
        if status == StatusCode::NOT_FOUND && http.session.lock().unwrap().is_some() {
            return Err(Failure::SessionExpired);
        }
        if !status.is_success() {
            return Err(status_failure(status));
        }
        let Some(id) = id else { return Ok(()) };
        if status == StatusCode::ACCEPTED {
            return Ok(());
        }
        let kind = response.headers().get(CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or_default().to_ascii_lowercase();
        if kind.starts_with("text/event-stream") {
            let (reader, peer) = (link.clone(), peer.clone());
            let task = tokio::spawn(async move {
                let link = reader;
                let mut parser = SseParser::default();
                let mut stream = response.bytes_stream();
                while let Some(Ok(chunk)) = stream.next().await {
                    for event in parser.push(&chunk) {
                        if !(event.event.is_empty() || event.event == "message") {
                            continue;
                        }
                        let Some(message) = parse_line(&event.data) else { continue };
                        for reply in peer.receive(message) {
                            if let Link::Http(http) = &*link {
                                let _ = http.post(&reply).await;
                            }
                        }
                    }
                    if !peer.is_pending(id) {
                        break;
                    }
                }
                peer.fail(id, Failure::Closed(Some("The server closed the stream before answering.".into())));
            });
            if let Link::Http(http) = &**link {
                let mut tasks = http.tasks.lock().unwrap();
                tasks.retain(|task| !task.is_finished());
                tasks.push(task.abort_handle());
            }
            return Ok(());
        }
        if kind.starts_with("application/json") || kind.is_empty() {
            let message = read_json(response).await?;
            for reply in peer.receive(message) {
                let _ = http.post(&reply).await;
            }
            return Ok(());
        }
        Err(Failure::Said(format!("The server answered with {kind}, not JSON.")))
    }

    fn close(&self) {
        abort(&self.tasks);
        // Ending the session is a courtesy: sent when a runtime is at hand, never waited for.
        let Some(session) = self.session.lock().unwrap().take() else { return };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        let request = self.client.delete(self.url.clone()).headers(self.headers.clone()).header(SESSION_HEADER, session).timeout(Duration::from_secs(2));
        runtime.spawn(async move {
            let _ = request.send().await;
        });
    }
}

// -------------------------------------------------------------------------------------------------
// Legacy HTTP+SSE

pub(crate) struct SseLink {
    client: reqwest::Client,
    headers: HeaderMap,
    endpoint: Url,
    tasks: Mutex<Vec<AbortHandle>>,
}

impl SseLink {
    async fn open(spec: &ServerSpec, peer: &Arc<Peer>) -> Result<Arc<Link>, Failure> {
        let (client, headers) = client(spec)?;
        let url = parse_url(spec)?;
        let open = follow(&url, |url| client.get(url).headers(headers.clone()).header(ACCEPT, "text/event-stream"));
        let (response, base) = tokio::time::timeout(spec.timeout, open).await.map_err(|_| Failure::Timeout)??;
        if !response.status().is_success() {
            return Err(status_failure(response.status()));
        }
        let (found, endpoint) = oneshot::channel::<Result<Url, Failure>>();
        // Replies to the server go out through the link, which exists only once the endpoint is
        // known; until then they wait here.
        let (link_sender, link_receiver) = watch::channel::<Option<Arc<Link>>>(None);
        let reader_peer = peer.clone();
        let origin = url.origin();
        let task = tokio::spawn(async move {
            let mut found = Some(found);
            let mut parser = SseParser::default();
            let mut stream = response.bytes_stream();
            while let Some(Ok(chunk)) = stream.next().await {
                for event in parser.push(&chunk) {
                    if event.event == "endpoint" {
                        let Some(found) = found.take() else { continue };
                        let endpoint = base.join(event.data.trim()).map_err(|_| Failure::Said("The server named an endpoint that isn't a valid address.".into()));
                        let _ = found.send(endpoint.and_then(|endpoint| {
                            if endpoint.origin() == origin {
                                Ok(endpoint)
                            } else {
                                Err(Failure::Said(format!(
                                    "The server asked to be sent messages at {}. WackCode only sends to {}, so your headers stay with that server.",
                                    endpoint.origin().ascii_serialization(),
                                    origin.ascii_serialization()
                                )))
                            }
                        }));
                    } else if event.event.is_empty() || event.event == "message" {
                        let Some(message) = parse_line(&event.data) else { continue };
                        let replies = reader_peer.receive(message);
                        let link = link_receiver.borrow().clone();
                        if let Some(link) = link
                            && let Link::Sse(sse) = &*link
                        {
                            for reply in replies {
                                let _ = sse.post(&reply).await;
                            }
                        }
                    }
                }
            }
            if let Some(found) = found.take() {
                let _ = found.send(Err(Failure::Closed(Some("The server closed the connection before naming its endpoint.".into()))));
            }
            reader_peer.close("The server closed the connection.".into());
        });
        let endpoint = match tokio::time::timeout(spec.timeout, endpoint).await {
            Ok(Ok(Ok(endpoint))) => endpoint,
            Ok(Ok(Err(failure))) => {
                task.abort();
                return Err(failure);
            }
            Ok(Err(_)) => {
                task.abort();
                return Err(Failure::Closed(None));
            }
            Err(_) => {
                task.abort();
                return Err(Failure::Timeout);
            }
        };
        let link = Arc::new(Link::Sse(SseLink { client, headers, endpoint, tasks: Mutex::new(vec![task.abort_handle()]) }));
        let _ = link_sender.send(Some(link.clone()));
        Ok(link)
    }

    async fn post(&self, message: &Value) -> Result<(), Failure> {
        let body = message.to_string();
        let (response, _) =
            follow(&self.endpoint, |url| self.client.post(url).headers(self.headers.clone()).header(CONTENT_TYPE, "application/json").body(body.clone())).await?;
        if response.status().is_success() { Ok(()) } else { Err(status_failure(response.status())) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_peer_answers_pings_refuses_other_requests_and_routes_answers() {
        let peer = Peer::new();
        assert_eq!(peer.receive(json!({ "jsonrpc": "2.0", "id": 7, "method": "ping" })), [json!({ "jsonrpc": "2.0", "id": 7, "result": {} })]);
        let refused = peer.receive(json!({ "jsonrpc": "2.0", "id": "s1", "method": "sampling/createMessage" }));
        assert_eq!(refused[0]["error"]["code"], -32601);
        assert!(peer.receive(json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" })).is_empty());
        assert!(peer.take_tools_changed());
        assert!(!peer.take_tools_changed());

        let (id, mut answer) = peer.register();
        peer.receive(json!({ "jsonrpc": "2.0", "id": id, "result": { "ok": true } }));
        assert_eq!(answer.try_recv().unwrap().unwrap(), json!({ "ok": true }));
        let (id, mut answer) = peer.register();
        peer.receive(json!([{ "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": "Unknown tool: x" } }]));
        assert!(matches!(answer.try_recv().unwrap(), Err(Failure::Rpc { code: -32602, .. })));
    }

    #[test]
    fn closing_fails_every_waiting_request_once() {
        let peer = Peer::new();
        let (_, mut first) = peer.register();
        peer.close("gone".into());
        peer.close("again".into());
        assert!(matches!(first.try_recv().unwrap(), Err(Failure::Closed(Some(reason))) if reason == "gone"));
        assert_eq!(peer.closed().as_deref(), Some("gone"));
    }
}
