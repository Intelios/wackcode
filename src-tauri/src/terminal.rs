//! A chat's user-facing terminal: one login shell on a PTY, kept alive while the app runs.
//!
//! The Terminal side-panel view attaches to this; the agent never sees it. A session lives
//! until the app quits, the chat is deleted/archived/converted to a worktree, or the user ends
//! it — hiding the panel only detaches, and reopening replays a scrollback ring buffer before
//! live output resumes.
//!
//! Ordering is pinned by one lock: `Core` holds the scrollback and the attached channel
//! together, and the reader thread appends and streams inside a single critical section, so an
//! attach that replays under the same lock can neither lose nor duplicate a byte.

use crate::models::{
    OpenTerminalInput, ResizeTerminalInput, TaskRecord, TerminalExit, TerminalFrame, TerminalInfo, WriteTerminalInput,
};
use crate::storage::MetadataState;
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{ipc::Channel, AppHandle, Emitter, State};
use uuid::Uuid;

/// Bytes kept per session for reattach replay — a few hundred lines of typical output.
const SCROLLBACK_BYTES: usize = 256 * 1024;
/// Reads this large keep IPC chatter low without delaying a keystroke's echo.
const READ_CHUNK: usize = 16 * 1024;
/// How often the busy poll asks the PTY which process group owns its foreground.
const BUSY_POLL: Duration = Duration::from_millis(750);

/// Whether the shell is up, and how it ended when it did.
#[derive(Clone)]
enum TerminalStatus {
    Running,
    Exited { code: i32, signal: Option<String> },
}

/// Scrollback plus the live sink under one lock: replay can't race a live chunk.
struct Core {
    bytes: VecDeque<u8>,
    sink: Option<Channel<TerminalFrame>>,
}

impl Core {
    fn push(&mut self, data: &[u8]) {
        self.bytes.extend(data);
        let excess = self.bytes.len().saturating_sub(SCROLLBACK_BYTES);
        self.bytes.drain(..excess);
        if let Some(sink) = &self.sink {
            let _ = sink.send(TerminalFrame::Output { data: String::from_utf8_lossy(data).into_owned() });
        }
    }
}

struct Session {
    /// Sessions are keyed by chat in the map, but restart replaces them; events carry this id
    /// so the renderer can drop a late frame meant for a shell that no longer exists.
    id: String,
    core: Mutex<Core>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    /// A handle for signalling the child without holding the lock the reader blocks `wait()` on.
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// Some while the PTY is open; closing it hangs up the slave and ends the reader.
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    /// Keystrokes from the panel, written to the PTY master.
    writer: Mutex<Box<dyn Write + Send>>,
    status: Mutex<TerminalStatus>,
    /// Set once the session is removed: reader and poller stand down instead of racing a respawn.
    closed: AtomicBool,
    busy: AtomicBool,
    shell: String,
    cwd: PathBuf,
    pid: u32,
}

/// One shell per chat, keyed by task id.
#[derive(Default)]
pub struct TerminalState {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize { rows: rows.clamp(2, 500), cols: cols.clamp(2, 500), pixel_width: 0, pixel_height: 0 }
}

/// What the reader hands to `Core::push`: complete UTF-8 text, with any codepoint split across
/// reads held back for next time. Invalid bytes decode to U+FFFD so nothing is skipped.
fn split_utf8(buffer: &[u8]) -> (String, Vec<u8>) {
    match std::str::from_utf8(buffer) {
        Ok(text) => (text.to_string(), Vec::new()),
        Err(error) => {
            let boundary = error.valid_up_to();
            let mut text = String::from_utf8_lossy(&buffer[..boundary]).into_owned();
            match error.error_len() {
                // Incomplete sequence at the end: keep it for the next chunk.
                None => (text, buffer[boundary..].to_vec()),
                Some(invalid) => {
                    text.push('\u{FFFD}');
                    let (rest, tail) = split_utf8(&buffer[boundary + invalid..]);
                    text.push_str(&rest);
                    (text, tail)
                }
            }
        }
    }
}

fn emit(app: &AppHandle, value: serde_json::Value) {
    let _ = app.emit("terminal-event", value);
}

fn spawn_session(cwd: &Path, cols: u16, rows: u16) -> Result<Arc<Session>, String> {
    let pair = native_pty_system()
        .openpty(size(cols, rows))
        .map_err(|error| format!("Could not open a terminal: {error}"))?;
    let shell = std::env::var("SHELL").ok().filter(|shell| shell.starts_with('/')).unwrap_or_else(|| "/bin/zsh".into());
    let mut command = CommandBuilder::new(&shell);
    command.arg("-l");
    command.cwd(cwd);
    // A login shell on a real PTY is interactive, so it sources the user's profiles itself —
    // the environment `shell_env` captures for non-shell children arrives here on its own.
    // A real terminal also declares itself; without TERM full-screen programs refuse to run.
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "WackCode");
    if std::env::var_os("LANG").is_none() { command.env("LANG", "en_US.UTF-8"); }
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("Could not start {shell}: {error}"))?;
    let writer = pair.master.take_writer().map_err(|error| error.to_string())?;
    let killer = child.clone_killer();
    let pid = child.process_id().unwrap_or(0);
    // Closing the master sends the session SIGHUP; the slave can go now that it has spawned.
    drop(pair.slave);
    Ok(Arc::new(Session {
        id: Uuid::new_v4().to_string(),
        core: Mutex::new(Core { bytes: VecDeque::new(), sink: None }),
        child: Mutex::new(child),
        killer: Mutex::new(killer),
        master: Mutex::new(Some(pair.master)),
        writer: Mutex::new(writer),
        status: Mutex::new(TerminalStatus::Running),
        closed: AtomicBool::new(false),
        busy: AtomicBool::new(false),
        shell,
        cwd: cwd.to_path_buf(),
        pid,
    }))
}

/// The blocking read loop: append to scrollback and stream to the attached sink inside one
/// lock. EOF (shell exit or a closed master) reaps the child and reports the exit once.
fn start_reader(app: &AppHandle, task_id: &str, session: &Arc<Session>, mut reader: Box<dyn Read + Send>) {
    let app = app.clone();
    let task_id = task_id.to_string();
    let session = session.clone();
    let session_id = session.id.clone();
    thread::spawn(move || {
        let mut chunk = [0u8; READ_CHUNK];
        // Bytes of a UTF-8 codepoint split across two reads, decoded on the next one.
        let mut tail: Vec<u8> = Vec::new();
        loop {
            let Ok(read) = reader.read(&mut chunk) else { break };
            if read == 0 { break; }
            let mut pending = std::mem::take(&mut tail);
            pending.extend_from_slice(&chunk[..read]);
            let (text, rest) = split_utf8(&pending);
            tail = rest;
            if !text.is_empty() {
                session.core.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(text.as_bytes());
            }
        }
        if !tail.is_empty() {
            let text = String::from_utf8_lossy(&tail).into_owned();
            session.core.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(text.as_bytes());
        }
        let status = session.child.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).wait().ok()
            .map(|status| TerminalStatus::Exited { code: status.exit_code() as i32, signal: status.signal().map(str::to_string) })
            .unwrap_or(TerminalStatus::Exited { code: -1, signal: None });
        if let Ok(mut record) = session.status.lock() { *record = status.clone(); }
        let mut core = session.core.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let TerminalStatus::Exited { code, signal } = status {
            if let Some(sink) = &core.sink { let _ = sink.send(TerminalFrame::Exit { code: Some(code), signal: signal.clone() }); }
            emit(&app, json!({ "type": "terminal_exited", "taskId": task_id, "sessionId": session_id, "code": code, "signal": signal }));
        }
        core.sink = None;
    });
}

/// Polls the PTY's foreground process group; anything other than the shell's own group means a
/// command is running. Transitions emit a global event so the header can hint at work in a
/// terminal that isn't on screen, and also reach the panel over the channel.
fn start_busy_poller(app: &AppHandle, task_id: &str, session: &Arc<Session>) {
    let app = app.clone();
    let task_id = task_id.to_string();
    let session_id = session.id.clone();
    let session = Arc::downgrade(session);
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(BUSY_POLL);
        loop {
            interval.tick().await;
            let Some(session) = session.upgrade() else { return };
            let exited = session.status.lock().map(|status| matches!(*status, TerminalStatus::Exited { .. })).unwrap_or(true);
            if session.closed.load(Ordering::Relaxed) || exited { return; }
            let busy = session.master.lock().ok()
                .and_then(|master| master.as_ref().and_then(|master| master.process_group_leader()))
                .is_some_and(|leader| leader >= 0 && leader as u32 != session.pid);
            if busy != session.busy.swap(busy, Ordering::Relaxed) {
                emit(&app, json!({ "type": "terminal_busy", "taskId": task_id, "sessionId": session_id, "busy": busy }));
                let core = session.core.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(sink) = &core.sink { let _ = sink.send(TerminalFrame::Busy { busy }); }
            }
        }
    });
}

fn session_for(state: &State<'_, TerminalState>, task_id: &str) -> Result<Arc<Session>, String> {
    state.sessions.lock().map_err(|_| "Terminal lock was poisoned".to_string())?
        .get(task_id).cloned()
        .ok_or_else(|| "This chat has no terminal session".to_string())
}

fn info_for(session: &Session, fresh: bool) -> TerminalInfo {
    let status = session.status.lock().map(|status| match *status {
        TerminalStatus::Running => None,
        TerminalStatus::Exited { code, ref signal } => Some(TerminalExit { code, signal: signal.clone() }),
    }).unwrap_or(None);
    TerminalInfo {
        session_id: session.id.clone(),
        shell: session.shell.rsplit('/').next().unwrap_or(&session.shell).to_string(),
        cwd: session.cwd.to_string_lossy().into_owned(),
        fresh,
        exit: status,
        busy: session.busy.load(Ordering::Relaxed),
    }
}

fn kill(session: &Arc<Session>) {
    session.closed.store(true, Ordering::Relaxed);
    // Detach first: a restart has already handed the renderer a new channel, and the reader's
    // exit frame for the dying shell must not reach it (it would flash a stale "Exited").
    if let Ok(mut core) = session.core.lock() { core.sink = None; }
    if let Ok(mut master) = session.master.lock() { *master = None; }
    if let Ok(mut killer) = session.killer.lock() { let _ = killer.kill(); }
}

impl TerminalState {
    /// The chat went away or its folder moved (delete, archive, worktree): its shell goes too.
    /// The idle-worker reaper never calls this — a user's idle shell is cheap to keep.
    pub fn kill_for_task(&self, app: &AppHandle, task_id: &str) {
        let session = self.sessions.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(task_id);
        if let Some(session) = session {
            emit(app, json!({ "type": "terminal_closed", "taskId": task_id, "sessionId": session.id }));
            kill(&session);
        }
    }

    pub fn terminate_all(&self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            for (_, session) in sessions.drain() { kill(&session); }
        }
    }
}

/// Attach the channel, spawn if needed, then replay the scrollback — all while holding the
/// core lock so no live byte can slip between the replay and the attach.
fn attach(session: &Arc<Session>, on_frame: Channel<TerminalFrame>) {
    let mut core = session.core.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !core.bytes.is_empty() {
        let _ = on_frame.send(TerminalFrame::Output { data: String::from_utf8_lossy(core.bytes.make_contiguous()).into_owned() });
    }
    core.sink = Some(on_frame);
}

fn open(
    app: &AppHandle,
    state: &State<'_, TerminalState>,
    task: &TaskRecord,
    input: &OpenTerminalInput,
    on_frame: Channel<TerminalFrame>,
) -> Result<TerminalInfo, String> {
    // The map stays locked across spawn so two racing opens can't each start a shell; the
    // guard drops when this block ends, before attach touches the channel.
    let spawned = {
        let mut sessions = state.sessions.lock().map_err(|_| "Terminal lock was poisoned".to_string())?;
        match sessions.get(&input.task_id) {
            Some(session) => {
                // Reattach at the panel's size; the SIGWINCH makes full-screen apps repaint.
                if let Ok(mut master) = session.master.lock() {
                    if let Some(master) = master.as_mut() { let _ = master.resize(size(input.cols, input.rows)); }
                }
                (session.clone(), None)
            }
            None => {
                let cwd = PathBuf::from(&task.workspace_path);
                if !cwd.is_dir() { return Err("This chat's folder no longer exists.".into()); }
                let session = spawn_session(&cwd, input.cols, input.rows)?;
                let reader = session.master.lock().map_err(|_| "Terminal lock was poisoned".to_string())?
                    .as_ref().and_then(|master| master.try_clone_reader().ok())
                    .ok_or_else(|| "Could not read from the terminal".to_string())?;
                emit(app, json!({ "type": "terminal_started", "taskId": input.task_id, "sessionId": session.id, "shell": session.shell, "cwd": session.cwd }));
                sessions.insert(input.task_id.clone(), session.clone());
                (session, Some(reader))
            }
        }
    };
    let (session, reader) = spawned;
    let fresh = reader.is_some();
    if let Some(reader) = reader {
        start_reader(app, &input.task_id, &session, reader);
        start_busy_poller(app, &input.task_id, &session);
    }
    attach(&session, on_frame);
    Ok(info_for(&session, fresh))
}

#[tauri::command]
pub fn open_terminal(
    app: AppHandle,
    state: State<'_, MetadataState>,
    terminals: State<'_, TerminalState>,
    input: OpenTerminalInput,
    on_frame: Channel<TerminalFrame>,
) -> Result<TerminalInfo, String> {
    let task = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .tasks.iter().find(|task| task.id == input.task_id).cloned()
        .ok_or_else(|| "Chat not found".to_string())?;
    open(&app, &terminals, &task, &input, on_frame)
}

#[tauri::command]
pub fn write_terminal(terminals: State<'_, TerminalState>, input: WriteTerminalInput) -> Result<(), String> {
    let session = session_for(&terminals, &input.task_id)?;
    if matches!(*session.status.lock().map_err(|_| "Terminal lock was poisoned".to_string())?, TerminalStatus::Exited { .. }) {
        return Err("This terminal's shell has exited. Start a new one.".into());
    }
    let mut writer = session.writer.lock().map_err(|_| "Terminal lock was poisoned".to_string())?;
    writer.write_all(input.data.as_bytes()).map_err(|error| format!("Could not write to the terminal: {error}"))
}

#[tauri::command]
pub fn resize_terminal(terminals: State<'_, TerminalState>, input: ResizeTerminalInput) -> Result<(), String> {
    let session = session_for(&terminals, &input.task_id)?;
    if let Ok(mut master) = session.master.lock() {
        if let Some(master) = master.as_mut() { let _ = master.resize(size(input.cols, input.rows)); }
    }
    Ok(())
}

#[tauri::command]
pub fn detach_terminal(terminals: State<'_, TerminalState>, task_id: String) -> Result<(), String> {
    if let Some(session) = terminals.sessions.lock().map_err(|_| "Terminal lock was poisoned".to_string())?.get(&task_id) {
        session.core.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).sink = None;
    }
    Ok(())
}

#[tauri::command]
pub fn restart_terminal(
    app: AppHandle,
    state: State<'_, MetadataState>,
    terminals: State<'_, TerminalState>,
    input: OpenTerminalInput,
    on_frame: Channel<TerminalFrame>,
) -> Result<TerminalInfo, String> {
    terminals.kill_for_task(&app, &input.task_id);
    let task = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .tasks.iter().find(|task| task.id == input.task_id).cloned()
        .ok_or_else(|| "Chat not found".to_string())?;
    open(&app, &terminals, &task, &input, on_frame)
}

#[tauri::command]
pub fn close_terminal(app: AppHandle, terminals: State<'_, TerminalState>, task_id: String) -> Result<(), String> {
    terminals.kill_for_task(&app, &task_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_utf8_holds_a_codepoint_split_across_reads() {
        // "🦆" is F0 9F A6 86: feed the first two bytes, then the rest with more text.
        let (text, tail) = split_utf8(&[0xF0, 0x9F]);
        assert_eq!(text, "");
        assert_eq!(tail, vec![0xF0, 0x9F]);
        let mut next = tail;
        next.extend_from_slice(&[0xA6, 0x86, b'!']);
        let (text, tail) = split_utf8(&next);
        assert_eq!(text, "🦆!");
        assert!(tail.is_empty());
    }

    #[test]
    fn split_utf8_lossy_decodes_genuinely_invalid_bytes() {
        let (text, tail) = split_utf8(&[b'a', 0xFF, b'b']);
        assert!(text.starts_with('a') && text.ends_with('b') && text.contains('\u{FFFD}'));
        assert!(tail.is_empty());
    }

    #[test]
    fn scrollback_keeps_only_the_most_recent_bytes() {
        let mut core = Core { bytes: VecDeque::new(), sink: None };
        let big = vec![b'x'; SCROLLBACK_BYTES + 100];
        core.push(&vec![b'y'; 50]);
        core.push(&big);
        assert_eq!(core.bytes.len(), SCROLLBACK_BYTES);
        assert_eq!(core.bytes[0], b'x');
    }

    #[test]
    fn frame_serialises_with_a_type_tag() {
        let value = serde_json::to_value(TerminalFrame::Busy { busy: true }).unwrap();
        assert_eq!(value, json!({ "type": "busy", "busy": true }));
        let value = serde_json::to_value(TerminalFrame::Exit { code: Some(0), signal: None }).unwrap();
        assert_eq!(value["type"], "exit");
        assert_eq!(value["code"], 0);
    }
}
