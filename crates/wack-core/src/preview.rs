//! `browser_preview`'s dev server: the project's Run command, through the app's `Runner`.
//!
//! - A Run session already going is used as it is, never restarted; one started here is the
//!   same session the Run button starts, so it shows under Run and outlives the agent's run
//!   (version 2 killed dev servers when a run ended, so every message reloaded the page).
//! - The server's address is what it printed (`Heard`, fed by the runner's reader thread), kept
//!   only when it is on this Mac: a LAN line ("Network: http://192.168.…") is never offered.
//!   A server listening on every interface (`0.0.0.0`, `[::]`) is opened at `localhost`.
//! - The tool waits for the address to accept a connection, and says at once when the command
//!   ends instead, with what it printed last.

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use url::{Host, Url};
use wack_browser::tools::{Preview, Served};

use crate::{
    runner::{CommandEvent, PtySize, Runner},
    store::CommandKind,
};

/// How long `serve` waits for a server to answer.
const LIMIT: Duration = Duration::from_secs(90);
/// Lines of a command's output kept for an error message.
const TAIL_LINES: usize = 30;
const LINE_CHARS: usize = 400;

/// What a command printed, as plain text: the addresses on this Mac in it, and its last lines.
#[derive(Default)]
pub struct Heard {
    /// Oldest first; one printed again moves to the end, so the last is the newest.
    pub addresses: Vec<String>,
    lines: VecDeque<String>,
    line: Vec<u8>,
    escape: Escape,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Escape {
    #[default]
    None,
    /// After ESC.
    Start,
    /// In a CSI sequence (ESC [), until its final byte.
    Csi,
    /// In an OSC string (ESC ]), until BEL or ESC \.
    Osc,
    /// ESC inside an OSC string: the `\` that ends it.
    OscEnd,
}

impl Heard {
    pub fn from_output(bytes: &[u8]) -> Self {
        let mut heard = Self::default();
        heard.take(bytes);
        heard
    }

    /// Reads more output. Terminal escapes are dropped; a line ends at `\n` or `\r`.
    pub fn take(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.escape = match (self.escape, byte) {
                (Escape::None, 0x1b) => Escape::Start,
                (Escape::None, b'\n' | b'\r') => {
                    self.end_line();
                    Escape::None
                }
                (Escape::None, byte) => {
                    if (byte >= 0x20 || byte == b'\t') && self.line.len() < 4 * LINE_CHARS {
                        self.line.push(byte);
                    }
                    Escape::None
                }
                (Escape::Start, b'[') => Escape::Csi,
                (Escape::Start, b']') => Escape::Osc,
                (Escape::Start, _) => Escape::None,
                (Escape::Csi, 0x40..=0x7e) => Escape::None,
                (Escape::Csi, _) => Escape::Csi,
                (Escape::Osc, 0x07) => Escape::None,
                (Escape::Osc, 0x1b) => Escape::OscEnd,
                (Escape::Osc, _) => Escape::Osc,
                (Escape::OscEnd, _) => Escape::None,
            };
        }
    }

    fn end_line(&mut self) {
        let line = String::from_utf8_lossy(&std::mem::take(&mut self.line)).trim_end().to_string();
        if line.is_empty() {
            return;
        }
        for address in local_addresses(&line) {
            self.addresses.retain(|known| *known != address);
            self.addresses.push(address);
        }
        if self.lines.len() == TAIL_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line.chars().take(LINE_CHARS).collect());
    }

    /// The last lines printed, the unfinished one included.
    pub fn tail(&self) -> String {
        let mut lines: Vec<String> = self.lines.iter().cloned().collect();
        let partial = String::from_utf8_lossy(&self.line).trim_end().to_string();
        if !partial.is_empty() {
            lines.push(partial);
        }
        lines.join("\n")
    }
}

/// The http(s) addresses on this Mac in a line of output, as a browser opens them.
pub fn local_addresses(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("http://").into_iter().chain(rest.find("https://")).min() {
        let candidate = &rest[start..];
        let end = candidate.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`' | ',')).unwrap_or(candidate.len());
        let mut raw = &candidate[..end];
        // "(http://[::]:8000/)" and "…/." end in what wraps or ends the sentence.
        while let Some(trimmed) = raw.strip_suffix([')', '.', ';', ':']) {
            if raw.ends_with(')') && raw.matches('(').count() >= raw.matches(')').count() {
                break;
            }
            raw = trimmed;
        }
        if let Some(address) = on_this_mac(raw) {
            found.push(address);
        }
        rest = &candidate[end.max(1)..];
    }
    found
}

fn on_this_mac(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let every_interface = match url.host()? {
        Host::Domain(domain) => {
            let domain = domain.to_ascii_lowercase();
            if domain != "localhost" && !domain.ends_with(".localhost") {
                return None;
            }
            false
        }
        Host::Ipv4(ip) if ip.is_loopback() => false,
        Host::Ipv6(ip) if ip.is_loopback() => false,
        Host::Ipv4(ip) if ip.is_unspecified() => true,
        Host::Ipv6(ip) if ip.is_unspecified() => true,
        _ => return None,
    };
    if every_interface {
        url.set_host(Some("localhost")).ok()?;
    }
    Some(url.to_string())
}

/// What a run needs to start the project's Run command: the app's runner, and where the Run
/// button's events go, so a session started here shows there.
#[derive(Clone)]
pub struct RunCommands {
    pub runner: Runner,
    pub events: Arc<dyn Fn(CommandEvent) + Send + Sync>,
}

/// `browser_preview`'s server for one project.
pub struct RunPreview {
    pub commands: RunCommands,
    pub cwd: PathBuf,
    /// The project's Run command when the run started; `None` when it has none.
    pub command: Option<String>,
}

#[async_trait]
impl Preview for RunPreview {
    async fn serve(&self, port: Option<u16>) -> Result<Served, String> {
        let runner = &self.commands.runner;
        let (session, started) = match runner.session(&self.cwd, CommandKind::Run).filter(|session| session.is_active()) {
            Some(session) => (session, false),
            None => {
                let command = self.command.as_deref().filter(|command| !command.trim().is_empty()).ok_or(
                    "This project has no Run command yet (the user sets it with the Run button). Start the server with bash and run_in_background instead, then open its address with browser_navigate.",
                )?;
                let events = self.commands.events.clone();
                let session = runner.start(&self.cwd, CommandKind::Run, command, PtySize::default(), move |event| events(event))?;
                (session, true)
            }
        };
        let command = session.info().command;
        let began = Instant::now();
        loop {
            let (addresses, tail) = session.heard();
            let address = match port {
                Some(port) => Some(format!("http://localhost:{port}/")),
                None => addresses.last().cloned(),
            };
            if let Some(url) = address.clone()
                && answers(&url).await
            {
                return Ok(Served { url, command, started });
            }
            if !session.is_active() {
                let said = if tail.is_empty() { "It printed nothing.".to_string() } else { format!("What it printed last:\n{tail}") };
                return Err(format!("The Run command (`{command}`) ended before it served anything. {said}"));
            }
            if began.elapsed() > LIMIT {
                let waited = match address {
                    Some(url) => format!("{url} never answered"),
                    None => "it printed no address on this Mac (pass `port` if it serves one without saying)".into(),
                };
                let said = if tail.is_empty() { String::new() } else { format!(" What it printed last:\n{tail}") };
                return Err(format!(
                    "The Run command (`{command}`) is running, but after {} seconds {waited}. It is still running under Run.{said}",
                    LIMIT.as_secs()
                ));
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }
}

/// Whether something accepts connections at `url`'s host and port.
async fn answers(url: &str) -> bool {
    let Ok(url) = Url::parse(url) else { return false };
    let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) else { return false };
    let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
    matches!(tokio::time::timeout(Duration::from_secs(1), tokio::net::TcpStream::connect((host.as_str(), port))).await, Ok(Ok(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_addresses_on_this_mac_are_heard() {
        assert_eq!(local_addresses("  ➜  Local:   http://localhost:5173/"), ["http://localhost:5173/"]);
        assert!(local_addresses("  ➜  Network: http://192.168.1.20:5173/").is_empty());
        assert_eq!(
            local_addresses("Serving HTTP on :: port 8000 (http://[::]:8000/) ..."),
            ["http://localhost:8000/"]
        );
        assert_eq!(local_addresses("Listening on http://0.0.0.0:3000."), ["http://localhost:3000/"]);
        assert_eq!(local_addresses("ready at http://127.0.0.1:8080/app, and https://app.localhost:4443"), ["http://127.0.0.1:8080/app", "https://app.localhost:4443/"]);
        assert!(local_addresses("Docs: https://vitejs.dev/guide").is_empty());
        assert!(local_addresses("http://localhost").len() == 1, "localhost on its default port");
    }

    #[test]
    fn output_is_read_as_plain_lines() {
        let mut heard = Heard::default();
        heard.take(b"\x1b[32m  VITE v5\x1b[0m ready\r\n  \x1b[1mLocal\x1b[22m:   http://local");
        assert!(heard.addresses.is_empty(), "the line isn't finished");
        heard.take(b"host:5173/\n\x1b]0;title\x07  Network: http://10.0.0.2:5173/\n");
        assert_eq!(heard.addresses, ["http://localhost:5173/"]);
        heard.take(b"Port 5173 is in use, trying another one...\nLocal: http://localhost:5174/\nLocal: http://localhost:5173/\n");
        assert_eq!(heard.addresses.last().map(String::as_str), Some("http://localhost:5173/"), "the newest is last");
        assert!(heard.tail().starts_with("  VITE v5 ready\n  Local:"));
        assert!(!heard.tail().contains('\x1b'));
    }

    #[tokio::test]
    async fn a_running_server_is_used_as_it_is() {
        let folder = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let runner = Runner::new();
        let commands = RunCommands { runner: runner.clone(), events: Arc::new(|_| {}) };
        let command = format!("echo serving on http://127.0.0.1:{port}/; sleep 30");
        let preview = RunPreview { commands, cwd: folder.path().into(), command: Some(command.clone()) };
        let first = preview.serve(None).await.unwrap();
        assert_eq!(first, Served { url: format!("http://127.0.0.1:{port}/"), command: command.clone(), started: true });
        let generation = runner.session(folder.path(), CommandKind::Run).unwrap().generation();
        let again = preview.serve(None).await.unwrap();
        assert!(!again.started, "found running, not started again");
        assert_eq!(runner.session(folder.path(), CommandKind::Run).unwrap().generation(), generation, "never restarted");
        runner.terminate_all();
    }

    #[tokio::test]
    async fn a_command_that_ends_says_what_it_printed() {
        let folder = tempfile::tempdir().unwrap();
        let commands = RunCommands { runner: Runner::new(), events: Arc::new(|_| {}) };
        let preview = RunPreview { commands, cwd: folder.path().into(), command: Some("echo 'error: no such script: dev'; exit 1".into()) };
        let error = preview.serve(None).await.unwrap_err();
        assert!(error.contains("ended before it served anything") && error.contains("no such script"), "{error}");
        let none = RunPreview { commands: RunCommands { runner: Runner::new(), events: Arc::new(|_| {}) }, cwd: folder.path().into(), command: None };
        assert!(none.serve(None).await.unwrap_err().contains("no Run command"));
    }
}
