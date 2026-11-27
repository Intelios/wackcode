//! The hub: every live MCP connection, shared by all the chats of a project.
//!
//! Version 2 started every server once per chat. Here a connection is keyed by the server, its
//! connection fingerprint and the project folder (a stdio server's working directory), and every
//! run in that folder shares it. The app owns one hub for its lifetime; `wack run` makes its own.
//!
//! Invariants:
//! - Connecting is the hub's work, not a run's: a run that is stopped stops waiting, never the
//!   connection, which another chat may be waiting on too.
//! - Connections start lazily, when a run in that folder asks ([`Hub::prepare`]), and stay up
//!   across runs. One that fails is not retried for five minutes, so a broken server can't add its
//!   timeout to every message; one that drops reconnects on the next message. One unused for
//!   [`IDLE_AFTER`] is closed (it restarts on the next message, its catalogue cached).
//! - A changed fingerprint (command, URL, headers, variables) means a new connection; the old one
//!   is closed. [`Hub::retain`] closes connections whose server was removed, switched off or
//!   changed in Settings.
//! - What went wrong is kept as news, once per failure or drop, for `McpFeature` to tell each
//!   chat that uses the server, and cleared when the server connects.
//! - [`Hub::terminate_all`] (quitting) asks every stdio server to stop and kills what is left a
//!   second later, including servers still starting and Test connections.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::{
    cache,
    client::Connection,
    spec::{ServerSpec, ToolDecl},
    transport,
};

/// A server that failed to connect is left alone this long.
pub const RETRY_FAILED_AFTER: Duration = Duration::from_secs(5 * 60);
/// A connection no run has used for this long is closed.
pub const IDLE_AFTER: Duration = Duration::from_secs(30 * 60);
/// Test connection never runs longer than this, whatever the server's own timeout.
const MAX_PROBE: Duration = Duration::from_secs(600);

#[derive(Clone)]
pub enum Status {
    /// Not started, or the connection dropped: it starts with the next run.
    Idle,
    Connecting,
    Connected(Arc<Connection>),
    Failed { error: String, at: Instant },
}

/// How one connection stands, for Settings › MCP servers.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    Connecting,
    Connected { tools: usize },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConnectionState {
    pub server: String,
    pub folder: PathBuf,
    pub state: State,
}

#[derive(Clone, Copy, PartialEq)]
enum NewsKind {
    Failed,
    Stopped,
}

struct News {
    kind: NewsKind,
    stamp: u64,
    text: String,
}

struct Entry {
    server: String,
    fingerprint: String,
    folder: PathBuf,
    /// The latest settings: the name, timeout and tool switches change without reconnecting.
    spec: Mutex<ServerSpec>,
    status: watch::Sender<Status>,
    last_used: Mutex<Instant>,
    busy: AtomicUsize,
    news: Mutex<Option<News>>,
    /// The catalogue cache, read once; `None` until read.
    cached: Mutex<Option<Option<Vec<ToolDecl>>>>,
    retired: AtomicBool,
}

impl Entry {
    fn touch(&self) {
        *self.last_used.lock().unwrap() = Instant::now();
    }

    fn name(&self) -> String {
        self.spec.lock().unwrap().name.clone()
    }

    /// Closes the connection and takes the entry out of service.
    fn retire(&self) {
        self.retired.store(true, Ordering::SeqCst);
        if let Status::Connected(connection) = self.status.send_replace(Status::Idle) {
            connection.close();
        }
    }
}

#[derive(Clone)]
pub struct Hub {
    inner: Arc<Inner>,
}

struct Inner {
    cache: Option<PathBuf>,
    idle_after: Duration,
    entries: Mutex<Vec<Arc<Entry>>>,
    reaper: AtomicBool,
    groups: transport::Groups,
}

impl Hub {
    /// A hub whose catalogues are cached in `cache` (`v3/mcp-cache`), or nowhere.
    pub fn new(cache: Option<PathBuf>) -> Self {
        Self::with_idle_after(cache, IDLE_AFTER)
    }

    pub fn with_idle_after(cache: Option<PathBuf>, idle_after: Duration) -> Self {
        Self { inner: Arc::new(Inner { cache, idle_after, entries: Mutex::new(Vec::new()), reaper: AtomicBool::new(false), groups: transport::Groups::default() }) }
    }

    /// The servers a run in `folder` uses, each started if it isn't connected or connecting
    /// (unless it failed recently). Never waits. Call on Tokio.
    pub fn prepare(&self, specs: &[ServerSpec], folder: &Path) -> Vec<Server> {
        self.ensure_reaper();
        let mut servers = Vec::new();
        let mut entries = self.inner.entries.lock().unwrap();
        for spec in specs {
            let fingerprint = spec.fingerprint();
            entries.retain(|entry| {
                let stale = entry.server == spec.id && entry.folder == folder && entry.fingerprint != fingerprint;
                if stale {
                    entry.retire();
                }
                !stale
            });
            let entry = match entries.iter().find(|entry| entry.server == spec.id && entry.folder == folder) {
                Some(entry) => entry.clone(),
                None => {
                    let entry = Arc::new(Entry {
                        server: spec.id.clone(),
                        fingerprint,
                        folder: folder.to_path_buf(),
                        spec: Mutex::new(spec.clone()),
                        status: watch::channel(Status::Idle).0,
                        last_used: Mutex::new(Instant::now()),
                        busy: AtomicUsize::new(0),
                        news: Mutex::new(None),
                        cached: Mutex::new(None),
                        retired: AtomicBool::new(false),
                    });
                    entries.push(entry.clone());
                    entry
                }
            };
            *entry.spec.lock().unwrap() = spec.clone();
            entry.touch();
            let start = match &*entry.status.borrow() {
                Status::Idle => true,
                Status::Failed { at, .. } => at.elapsed() >= RETRY_FAILED_AFTER,
                Status::Connected(connection) => connection.closed().is_some(),
                Status::Connecting => false,
            };
            if start {
                self.start(&entry);
            } else if let Status::Connected(connection) = &*entry.status.borrow() {
                // A server that said its tools changed is listed again, for the next run.
                let (connection, entry, cache) = (connection.clone(), entry.clone(), self.inner.cache.clone());
                tokio::spawn(async move {
                    if connection.refresh_if_changed().await == Ok(true) {
                        let tools = connection.tools();
                        if let Some(cache) = &cache {
                            cache::write(cache, &entry.spec.lock().unwrap().clone(), &tools);
                        }
                    }
                });
            }
            servers.push(Server { entry, cache: self.inner.cache.clone() });
        }
        servers
    }

    fn start(&self, entry: &Arc<Entry>) {
        entry.status.send_replace(Status::Connecting);
        {
            let mut news = entry.news.lock().unwrap();
            if news.as_ref().is_some_and(|news| news.kind == NewsKind::Stopped) {
                *news = None;
            }
        }
        let spec = entry.spec.lock().unwrap().clone();
        let (entry, cache, groups) = (entry.clone(), self.inner.cache.clone(), self.inner.groups.clone());
        tokio::spawn(async move {
            let result = Connection::connect_in(&spec, &entry.folder, &groups).await;
            if entry.retired.load(Ordering::SeqCst) {
                if let Ok(connection) = result {
                    connection.close();
                }
                return;
            }
            match result {
                Ok(connection) => {
                    let connection = Arc::new(connection);
                    let tools = connection.tools();
                    if let Some(cache) = &cache {
                        cache::write(cache, &spec, &tools);
                    }
                    *entry.cached.lock().unwrap() = Some(Some(tools));
                    *entry.news.lock().unwrap() = None;
                    entry.status.send_replace(Status::Connected(connection.clone()));
                    let reason = connection.wait_closed().await;
                    let dropped = matches!(&*entry.status.borrow(), Status::Connected(current) if Arc::ptr_eq(current, &connection));
                    if dropped && !entry.retired.load(Ordering::SeqCst) {
                        entry.status.send_replace(Status::Idle);
                        let text = format!("MCP server \"{}\" stopped: {reason} It reconnects with your next message.", entry.name());
                        *entry.news.lock().unwrap() = Some(News { kind: NewsKind::Stopped, stamp: stamp(), text });
                    }
                }
                Err(error) => {
                    let text = format!("MCP server \"{}\" isn't available: {error} Check it in Settings › MCP servers.", entry.name());
                    *entry.news.lock().unwrap() = Some(News { kind: NewsKind::Failed, stamp: stamp(), text });
                    entry.status.send_replace(Status::Failed { error, at: Instant::now() });
                }
            }
        });
    }

    /// Closes every connection for which `keep(server_id, fingerprint, folder)` returns false:
    /// the server was removed, switched off, reconfigured, or scoped away from that folder.
    pub fn retain(&self, mut keep: impl FnMut(&str, &str, &Path) -> bool) {
        self.inner.entries.lock().unwrap().retain(|entry| {
            let kept = keep(&entry.server, &entry.fingerprint, &entry.folder);
            if !kept {
                entry.retire();
            }
            kept
        });
    }

    /// Closes connections no run has used for the idle time. Runs on its own every minute once
    /// the hub has prepared a run.
    pub fn reap_idle(&self) {
        let idle_after = self.inner.idle_after;
        self.inner.entries.lock().unwrap().retain(|entry| {
            let idle = entry.busy.load(Ordering::SeqCst) == 0
                && !matches!(&*entry.status.borrow(), Status::Connecting)
                && entry.last_used.lock().unwrap().elapsed() >= idle_after;
            if idle {
                entry.retire();
            }
            !idle
        });
    }

    fn ensure_reaper(&self) {
        if self.inner.reaper.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        let every = (self.inner.idle_after / 4).clamp(Duration::from_millis(50), Duration::from_secs(60));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                let Some(inner) = weak.upgrade() else { return };
                Hub { inner }.reap_idle();
            }
        });
    }

    /// Every connection and how it stands, for Settings.
    pub fn states(&self) -> Vec<ConnectionState> {
        self.inner
            .entries
            .lock()
            .unwrap()
            .iter()
            .map(|entry| ConnectionState {
                server: entry.server.clone(),
                folder: entry.folder.clone(),
                state: match &*entry.status.borrow() {
                    Status::Idle => State::Idle,
                    Status::Connecting => State::Connecting,
                    Status::Connected(connection) if connection.closed().is_some() => State::Idle,
                    Status::Connected(connection) => State::Connected { tools: connection.tools().len() },
                    Status::Failed { error, .. } => State::Failed(error.clone()),
                },
            })
            .collect()
    }

    /// Ends every connection, for quitting: stdio servers (including any still starting, and
    /// Test connections) get SIGTERM, then SIGKILL after a second. Blocks for at most that second.
    pub fn terminate_all(&self) {
        let groups = self.inner.groups.live();
        for entry in self.inner.entries.lock().unwrap().drain(..) {
            entry.retired.store(true, Ordering::SeqCst);
            if let Status::Connected(connection) = entry.status.send_replace(Status::Idle) {
                if connection.pgid().is_some() { connection.abandon() } else { connection.close() }
            }
        }
        transport::terminate_groups(&groups);
    }

    /// Settings' Test connection: a connection of its own (never a shared one) that lists the
    /// server's tools and closes. A stdio server starts in `cwd`. The tool list is cached, so a
    /// new server is offered on the next message without a wait.
    pub async fn probe(&self, spec: &ServerSpec, cwd: &Path) -> Result<Vec<ToolDecl>, String> {
        let deadline = spec.timeout.saturating_mul(2).saturating_add(Duration::from_secs(5)).min(MAX_PROBE);
        let connection = tokio::time::timeout(deadline, Connection::connect_in(spec, cwd, &self.inner.groups))
            .await
            .map_err(|_| format!("The server did not respond within {} ms.", spec.timeout.as_millis()))??;
        let tools = connection.tools();
        if let Some(cache) = &self.inner.cache {
            cache::write(cache, spec, &tools);
        }
        connection.close();
        Ok(tools)
    }
}

/// One server as a run sees it.
#[derive(Clone)]
pub struct Server {
    entry: Arc<Entry>,
    cache: Option<PathBuf>,
}

impl Server {
    pub fn id(&self) -> &str {
        &self.entry.server
    }

    /// The server's latest settings.
    pub fn spec(&self) -> ServerSpec {
        self.entry.spec.lock().unwrap().clone()
    }

    pub fn name(&self) -> String {
        self.entry.name()
    }

    pub fn connecting(&self) -> bool {
        matches!(&*self.entry.status.borrow(), Status::Connecting)
    }

    /// The tools to offer: the live list once connected, otherwise the cached one. A server
    /// that failed offers none.
    pub fn catalogue(&self) -> Option<Vec<ToolDecl>> {
        match &*self.entry.status.borrow() {
            Status::Connected(connection) => return Some(connection.tools()),
            Status::Failed { .. } => return None,
            Status::Idle | Status::Connecting => {}
        }
        let mut cached = self.entry.cached.lock().unwrap();
        if cached.is_none() {
            let spec = self.spec();
            *cached = Some(self.cache.as_ref().and_then(|folder| cache::read(&cache::path(folder, &spec))));
        }
        cached.clone().flatten()
    }

    /// Waits until the server is connected or has failed, or `cancel` fires.
    pub async fn ready(&self, cancel: &CancellationToken) {
        let mut status = self.entry.status.subscribe();
        tokio::select! {
            _ = status.wait_for(|status| !matches!(status, Status::Connecting)) => {}
            () = cancel.cancelled() => {}
        }
    }

    /// The live connection, waiting for it if it is starting.
    pub async fn connection(&self, cancel: &CancellationToken) -> Result<Arc<Connection>, String> {
        self.ready(cancel).await;
        if cancel.is_cancelled() {
            return Err("Stopped.".into());
        }
        let name = self.name();
        match &*self.entry.status.borrow() {
            Status::Connected(connection) => match connection.closed() {
                None => Ok(connection.clone()),
                Some(reason) => Err(format!("MCP server \"{name}\" is not connected. {reason} It reconnects with your next message.")),
            },
            Status::Failed { error, .. } => Err(format!("MCP server \"{name}\" is not connected. {error}")),
            Status::Idle | Status::Connecting => Err(format!("MCP server \"{name}\" is not connected. It reconnects with your next message.")),
        }
    }

    /// What went wrong with the server, once per failure or drop: `(stamp, sentence)`.
    pub fn news(&self) -> Option<(u64, String)> {
        self.entry.news.lock().unwrap().as_ref().map(|news| (news.stamp, news.text.clone()))
    }

    /// Marks the server in use until the guard drops, so it is never closed as idle mid-call.
    pub(crate) fn busy(&self) -> Busy {
        self.entry.busy.fetch_add(1, Ordering::SeqCst);
        self.entry.touch();
        Busy(self.entry.clone())
    }
}

pub(crate) struct Busy(Arc<Entry>);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.touch();
        self.0.busy.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A news stamp: milliseconds since 1970, unique within this process.
fn stamp() -> u64 {
    static LAST: Mutex<u64> = Mutex::new(0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_millis() as u64).unwrap_or(0);
    let mut last = LAST.lock().unwrap();
    *last = now.max(*last + 1);
    *last
}
