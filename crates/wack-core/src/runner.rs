//! User-launched Build and Run project commands, executed on real macOS PTYs.
//!
//! Each project folder (`canonical_cwd`) may have independent active `Build` and `Run` sessions,
//! so building never stops or replaces a running dev server.
//!
//! Invariants:
//! - Provider API keys (`wack_tools::SECRET_ENV`) are removed from child environments before exec.
//! - Exit is determined from `child.wait()` independently of PTY EOF so background jobs spawned by
//!   a command cannot disguise shell completion or falsely look failed on user Stop.
//! - User-initiated `stop()` signals all process groups attached to the session's PTY (`SIGINT` ->
//!   `SIGTERM` -> `SIGKILL`) off the UI thread and resolves the run as [`CommandStatus::Stopped`].
//! - Monotonic `generation` and `revision` counters keep late exit or stop callbacks from clobbering
//!   a restarted run.

use std::{
    collections::{HashMap, HashSet},
    ffi::CStr,
    fs::File,
    io::{Error, Read, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::process::{CommandExt, ExitStatusExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    ptr,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crate::{
    store::CommandKind,
    terminal::{Modes, Screen, SelectionKind, Snapshot, TerminalEvent},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandStatus {
    Idle,
    Running,
    Stopping,
    Finished,
    Stopped,
    Failed {
        exit_code: Option<i32>,
        signal: Option<String>,
    },
}

impl CommandStatus {
    pub fn is_active(&self) -> bool {
        matches!(self, CommandStatus::Running | CommandStatus::Stopping)
    }

    pub fn label(&self, kind: CommandKind) -> String {
        match self {
            CommandStatus::Idle => "Idle".into(),
            CommandStatus::Running => match kind {
                CommandKind::Build => "Building".into(),
                CommandKind::Run => "Running".into(),
            },
            CommandStatus::Stopping => "Stopping…".into(),
            CommandStatus::Finished => "Finished".into(),
            CommandStatus::Stopped => "Stopped".into(),
            CommandStatus::Failed { exit_code, signal } => {
                if let Some(signal) = signal {
                    format!("Failed ({signal})")
                } else if let Some(code) = exit_code {
                    format!("Failed (code {code})")
                } else {
                    "Failed".into()
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandInfo {
    pub kind: CommandKind,
    pub cwd: PathBuf,
    pub command: String,
    pub generation: u64,
    pub revision: u64,
    pub status: CommandStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandEvent {
    Wakeup {
        kind: CommandKind,
        cwd: PathBuf,
        generation: u64,
    },
    StateChanged(CommandInfo),
}

type EventSink = Arc<dyn Fn(CommandEvent) + Send + Sync>;

struct Inner {
    info: Mutex<CommandInfo>,
    exited: Mutex<bool>,
    exited_cv: Condvar,
    screen: Mutex<Screen>,
    master: Mutex<Option<File>>,
    stop_lock: Mutex<()>,
    pid: i32,
    tty: Option<String>,
    generation: u64,
    emit: EventSink,
    has_output: AtomicBool,
}

/// Handle to a live or completed (or fixture-replayed) Build/Run session.
///
/// Cloning keeps a handle to the same session; dropping never kills a live command (only `stop()`,
/// `Runner::stop_project()`, or `Runner::terminate_all()` terminates running commands).
#[derive(Clone)]
pub struct CommandSession {
    inner: Arc<Inner>,
}

impl CommandSession {
    /// Offline replayed session for tests and gallery scenes.
    pub fn fixture(
        kind: CommandKind,
        cwd: impl Into<PathBuf>,
        command: impl Into<String>,
        status: CommandStatus,
        columns: usize,
        rows: usize,
        bytes: &[u8],
    ) -> Self {
        let info = CommandInfo {
            kind,
            cwd: cwd.into(),
            command: command.into(),
            generation: 1,
            revision: 1,
            status,
        };
        let has_output = !bytes.is_empty();
        Self {
            inner: Arc::new(Inner {
                info: Mutex::new(info),
                exited: Mutex::new(true),
                exited_cv: Condvar::new(),
                screen: Mutex::new(Screen::replay(columns, rows, bytes)),
                master: Mutex::new(None),
                stop_lock: Mutex::new(()),
                pid: 0,
                tty: None,
                generation: 1,
                emit: Arc::new(|_| {}),
                has_output: AtomicBool::new(has_output),
            }),
        }
    }

    pub fn info(&self) -> CommandInfo {
        self.inner.info.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn generation(&self) -> u64 {
        self.inner.generation
    }

    pub fn is_active(&self) -> bool {
        self.info().status.is_active()
    }

    pub fn has_output(&self) -> bool {
        self.inner.has_output.load(Ordering::Acquire)
    }

    pub fn snapshot(&self) -> Snapshot {
        self.inner.screen.lock().unwrap_or_else(|e| e.into_inner()).snapshot()
    }

    pub fn modes(&self) -> Modes {
        self.inner.screen.lock().unwrap_or_else(|e| e.into_inner()).modes()
    }

    pub fn resize(&self, columns: usize, rows: usize, cell: (u16, u16)) {
        let changed = {
            let mut screen = self.inner.screen.lock().unwrap_or_else(|e| e.into_inner());
            screen.resize(columns, rows, cell)
        };
        if !changed {
            return;
        }
        if let Ok(master) = self.inner.master.lock()
            && let Some(file) = master.as_ref()
        {
            let winsize = libc::winsize {
                ws_row: rows.max(1) as u16,
                ws_col: columns.max(2) as u16,
                ws_xpixel: (columns.max(2) * cell.0 as usize) as u16,
                ws_ypixel: (rows.max(1) * cell.1 as usize) as u16,
            };
            unsafe {
                libc::ioctl(file.as_raw_fd(), libc::TIOCSWINSZ as _, &winsize);
            }
        }
    }

    pub fn write(&self, bytes: &[u8]) {
        if bytes.is_empty() || self.info().status != CommandStatus::Running {
            return;
        }
        self.inner.screen.lock().unwrap_or_else(|e| e.into_inner()).scroll_to_bottom();
        if let Ok(mut master) = self.inner.master.lock()
            && let Some(file) = master.as_mut()
        {
            let _ = file.write_all(bytes);
        }
    }

    pub fn paste(&self, text: &str) {
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.modes().bracketed_paste {
            let text = text.replace("\x1b[201~", "");
            self.write(format!("\x1b[200~{text}\x1b[201~").as_bytes());
        } else {
            self.write(text.as_bytes());
        }
    }

    pub fn clear_output(&self) {
        self.inner.screen.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.inner.has_output.store(false, Ordering::Release);
        let info = self.info();
        (self.inner.emit)(CommandEvent::Wakeup {
            kind: info.kind,
            cwd: info.cwd,
            generation: info.generation,
        });
    }

    pub fn scroll(&self, lines: i32) {
        self.inner.screen.lock().unwrap_or_else(|e| e.into_inner()).scroll(lines);
    }

    pub fn select_from(&self, row: usize, column: usize, right_half: bool, kind: SelectionKind) {
        self.inner
            .screen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .select_from(row, column, right_half, kind);
    }

    pub fn select_to(&self, row: usize, column: usize, right_half: bool) {
        self.inner
            .screen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .select_to(row, column, right_half);
    }

    pub fn clear_selection(&self) {
        self.inner.screen.lock().unwrap_or_else(|e| e.into_inner()).clear_selection();
    }

    pub fn selected_text(&self) -> Option<String> {
        self.inner.screen.lock().unwrap_or_else(|e| e.into_inner()).selected_text()
    }

    fn mark_stopping(&self) -> bool {
        let changed = {
            let mut info = self.inner.info.lock().unwrap_or_else(|e| e.into_inner());
            if info.status != CommandStatus::Running {
                return false;
            }
            info.status = CommandStatus::Stopping;
            info.revision += 1;
            info.clone()
        };
        (self.inner.emit)(CommandEvent::StateChanged(changed));
        true
    }

    fn foreground_pgid(&self) -> Option<i32> {
        let master = self.inner.master.lock().ok()?;
        let file = master.as_ref()?;
        let pgid = unsafe { libc::tcgetpgrp(file.as_raw_fd()) };
        (pgid > 0).then_some(pgid)
    }

    fn wait_for_exit(&self, timeout: Duration) -> bool {
        let guard = self.inner.exited.lock().unwrap_or_else(|e| e.into_inner());
        let (done, _) = self
            .inner
            .exited_cv
            .wait_timeout_while(guard, timeout, |exited| !*exited)
            .unwrap_or_else(|e| e.into_inner());
        *done
    }

    /// Terminates this run cleanly with escalating signals (`SIGINT` -> `SIGTERM` -> `SIGKILL`)
    /// across all process groups associated with its PTY. Must run off the UI thread.
    pub fn stop_blocking(&self) -> Result<CommandInfo, String> {
        let kind_name = self.info().kind.name();
        let _stop = self
            .inner
            .stop_lock
            .lock()
            .map_err(|_| format!("Could not lock this {kind_name}."))?;
        if !self.is_active() {
            return Ok(self.info());
        }
        self.mark_stopping();

        let processes = process_groups();
        let tty = self
            .inner
            .tty
            .clone()
            .or_else(|| tty_for_pid(&processes, self.inner.pid))
            .filter(|name| valid_tty(name));

        let mut groups = HashSet::new();
        if self.inner.pid > 0 {
            groups.insert(self.inner.pid);
        }
        if let Some(tty) = &tty {
            groups.extend(groups_on_terminal(&processes, tty));
        }

        if let Some(foreground) = self.foreground_pgid() {
            groups.insert(foreground);
            send_signal(foreground, libc::SIGINT);
        } else if self.inner.pid > 0 {
            send_signal(self.inner.pid, libc::SIGINT);
        }
        wait_groups(&groups, Duration::from_secs(2));

        if let Some(tty) = &tty {
            groups.extend(groups_on_terminal(&process_groups(), tty));
        }
        if groups.iter().any(|pgid| group_alive(*pgid)) {
            if let Some(foreground) = self.foreground_pgid() {
                groups.insert(foreground);
            }
            for pgid in &groups {
                send_signal(*pgid, libc::SIGTERM);
            }
            wait_groups(&groups, Duration::from_secs(1));
            if let Some(tty) = &tty {
                groups.extend(groups_on_terminal(&process_groups(), tty));
            }
            for pgid in &groups {
                if group_alive(*pgid) {
                    send_signal(*pgid, libc::SIGKILL);
                }
            }
        }

        let exited = self.wait_for_exit(Duration::from_secs(1));
        if !exited {
            return Err(format!("Could not stop this {kind_name}. Try Stop again."));
        }

        let stopped = {
            let mut info = self.inner.info.lock().unwrap_or_else(|e| e.into_inner());
            info.status = CommandStatus::Stopped;
            info.revision += 1;
            info.clone()
        };
        // Closing our master writer unblocks any lingering readers while leaving the screen intact.
        self.inner.master.lock().unwrap_or_else(|e| e.into_inner()).take();
        (self.inner.emit)(CommandEvent::StateChanged(stopped.clone()));
        Ok(stopped)
    }
}

/// Initial PTY geometry when starting a Build/Run session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtySize {
    pub columns: usize,
    pub rows: usize,
    pub cell: (u16, u16),
}

impl Default for PtySize {
    fn default() -> Self {
        Self {
            columns: 100,
            rows: 24,
            cell: (8, 16),
        }
    }
}

/// Coordinates all Build and Run sessions across open projects in the engine.
#[derive(Clone, Default)]
pub struct Runner {
    sessions: Arc<Mutex<HashMap<(PathBuf, CommandKind), CommandSession>>>,
    next_generation: Arc<AtomicU64>,
    shutting_down: Arc<AtomicBool>,
}

impl Runner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn canonical_cwd(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    pub fn session(&self, cwd: &Path, kind: CommandKind) -> Option<CommandSession> {
        let key = (Self::canonical_cwd(cwd), kind);
        self.sessions
            .lock()
            .ok()
            .and_then(|sessions| sessions.get(&key).cloned())
    }

    /// Starts `command` in `cwd` if no active session of `kind` is already running there;
    /// otherwise returns the active session without spawning a duplicate.
    pub fn start(
        &self,
        cwd: &Path,
        kind: CommandKind,
        command: &str,
        size: PtySize,
        emit: impl Fn(CommandEvent) + Send + Sync + 'static,
    ) -> Result<CommandSession, String> {
        let canonical = cwd
            .canonicalize()
            .map_err(|_| "This command's folder no longer exists.".to_string())?;
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return Err(format!("Set a project {} command first.", kind.name()));
        }
        if trimmed.contains('\0') {
            return Err("The command cannot contain null characters.".into());
        }

        let key = (canonical.clone(), kind);
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Could not lock project commands.".to_string())?;
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err("WackCode is quitting.".into());
        }
        if let Some(existing) = sessions.get(&key).filter(|session| session.is_active()) {
            return Ok(existing.clone());
        }

        let generation = self.next_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let emit: EventSink = Arc::new(emit);
        let (session, child, reader) = spawn_pty(&canonical, kind, trimmed, generation, size, emit.clone())?;
        let info = session.info();
        sessions.insert(key, session.clone());
        drop(sessions);

        emit(CommandEvent::StateChanged(info));
        spawn_reader_thread(session.clone(), reader);
        spawn_waiter_thread(session.clone(), child);
        Ok(session)
    }

    /// Requests cancellation on an active session and finishes signal escalation on a worker thread.
    pub fn stop(&self, cwd: &Path, kind: CommandKind) -> Option<CommandSession> {
        let session = self.session(cwd, kind)?;
        if !session.is_active() {
            return Some(session);
        }
        session.mark_stopping();
        let target = session.clone();
        thread::spawn(move || {
            let _ = target.stop_blocking();
        });
        Some(session)
    }

    /// Stops and removes both Build and Run sessions for a project folder (called on `remove_project`).
    pub fn stop_project(&self, cwd: &Path) {
        let canonical = Self::canonical_cwd(cwd);
        let removed: Vec<CommandSession> = {
            let Ok(mut sessions) = self.sessions.lock() else {
                return;
            };
            let mut list = Vec::new();
            for kind in [CommandKind::Build, CommandKind::Run] {
                if let Some(session) = sessions.remove(&(canonical.clone(), kind)) {
                    session.mark_stopping();
                    list.push(session);
                }
            }
            list
        };
        for session in removed {
            thread::spawn(move || {
                let _ = session.stop_blocking();
            });
        }
    }

    /// Stops every running Build and Run command concurrently on app quit, bounded to one grace
    /// escalation window.
    pub fn terminate_all(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let sessions: Vec<CommandSession> = self
            .sessions
            .lock()
            .map(|mut map| map.drain().map(|(_, session)| session).collect())
            .unwrap_or_default();
        if sessions.is_empty() {
            return;
        }
        let handles: Vec<_> = sessions
            .into_iter()
            .filter(|session| session.is_active())
            .map(|session| {
                session.mark_stopping();
                thread::spawn(move || {
                    let _ = session.stop_blocking();
                })
            })
            .collect();
        for handle in handles {
            let _ = handle.join();
        }
    }
}

fn spawn_pty(
    cwd: &Path,
    kind: CommandKind,
    command: &str,
    generation: u64,
    size: PtySize,
    emit: EventSink,
) -> Result<(CommandSession, Child, File), String> {
    let title = kind.title();
    let cols = size.columns.max(2);
    let row_count = size.rows.max(1);
    let cell = size.cell;
    let mut master_raw: libc::c_int = -1;
    let mut slave_raw: libc::c_int = -1;
    let mut winsize = libc::winsize {
        ws_row: row_count as u16,
        ws_col: cols as u16,
        ws_xpixel: (cols * cell.0 as usize) as u16,
        ws_ypixel: (row_count * cell.1 as usize) as u16,
    };
    if unsafe { libc::openpty(&mut master_raw, &mut slave_raw, ptr::null_mut(), ptr::null_mut(), &mut winsize) } != 0 {
        return Err(format!(
            "Could not open the {title} terminal: {}.",
            Error::last_os_error()
        ));
    }
    let master_fd = unsafe { OwnedFd::from_raw_fd(master_raw) };
    let slave_fd = unsafe { OwnedFd::from_raw_fd(slave_raw) };

    enable_utf8_termios(master_fd.as_raw_fd());
    let tty = slave_tty_name(slave_fd.as_raw_fd());

    let shell = user_shell();
    let mut cmd = Command::new(&shell);
    cmd.args(["-l", "-i", "-c", command]);
    cmd.current_dir(cwd);
    for var in wack_tools::SECRET_ENV {
        cmd.env_remove(var);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "WackCode");
    if std::env::var_os("LANG").is_none() {
        cmd.env("LANG", "en_US.UTF-8");
    }

    let stdin_fd = slave_fd
        .try_clone()
        .map_err(|error| format!("Could not attach stdin to {title}: {error}."))?;
    let stdout_fd = slave_fd
        .try_clone()
        .map_err(|error| format!("Could not attach stdout to {title}: {error}."))?;
    let stderr_fd = slave_fd
        .try_clone()
        .map_err(|error| format!("Could not attach stderr to {title}: {error}."))?;
    cmd.stdin(Stdio::from(stdin_fd));
    cmd.stdout(Stdio::from(stdout_fd));
    cmd.stderr(Stdio::from(stderr_fd));

    let master_for_exec = master_fd.as_raw_fd();
    let slave_for_exec = slave_fd.as_raw_fd();
    unsafe {
        cmd.pre_exec(move || {
            if libc::setsid() == -1 {
                return Err(Error::last_os_error());
            }
            #[allow(clippy::cast_lossless)]
            if libc::ioctl(slave_for_exec, libc::TIOCSCTTY as _, 0) != 0 {
                return Err(Error::last_os_error());
            }
            libc::close(slave_for_exec);
            libc::close(master_for_exec);

            libc::signal(libc::SIGCHLD, libc::SIG_DFL);
            libc::signal(libc::SIGHUP, libc::SIG_DFL);
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            libc::signal(libc::SIGALRM, libc::SIG_DFL);
            Ok(())
        });
    }

    let child = cmd
        .spawn()
        .map_err(|error| format!("Could not start the {} command: {error}.", kind.name()))?;
    // Close our reference to the slave PTY so child exit closes all slave descriptors cleanly.
    drop(slave_fd);

    let pid = i32::try_from(child.id()).unwrap_or(0);
    let master_file = File::from(master_fd);
    let reader = master_file
        .try_clone()
        .map_err(|error| format!("Could not read {title} output: {error}."))?;

    let screen_emit = emit.clone();
    let reply_file = master_file.try_clone().ok().map(Mutex::new);
    let cwd_owned = cwd.to_path_buf();
    let screen = Screen::new(cols, row_count, cell, move |event| {
        if matches!(event, TerminalEvent::Wakeup) {
            (screen_emit)(CommandEvent::Wakeup {
                kind,
                cwd: cwd_owned.clone(),
                generation,
            });
        }
    });
    if let Some(reply_file) = reply_file {
        screen.set_reply(Some(Arc::new(move |bytes| {
            if let Ok(mut file) = reply_file.lock() {
                let _ = file.write_all(&bytes);
            }
        })));
    }

    let session = CommandSession {
        inner: Arc::new(Inner {
            info: Mutex::new(CommandInfo {
                kind,
                cwd: cwd.to_path_buf(),
                command: command.to_string(),
                generation,
                revision: 1,
                status: CommandStatus::Running,
            }),
            exited: Mutex::new(false),
            exited_cv: Condvar::new(),
            screen: Mutex::new(screen),
            master: Mutex::new(Some(master_file)),
            stop_lock: Mutex::new(()),
            pid,
            tty,
            generation,
            emit,
            has_output: AtomicBool::new(false),
        }),
    };
    Ok((session, child, reader))
}

fn spawn_reader_thread(session: CommandSession, mut reader: File) {
    thread::spawn(move || {
        let mut buf = [0u8; 32 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    session.inner.has_output.store(true, Ordering::Release);
                    let redraw = session
                        .inner
                        .screen
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .advance(&buf[..n]);
                    if redraw {
                        let info = session.info();
                        (session.inner.emit)(CommandEvent::Wakeup {
                            kind: info.kind,
                            cwd: info.cwd,
                            generation: info.generation,
                        });
                    }
                }
                Err(error) => match error.kind() {
                    std::io::ErrorKind::Interrupted => continue,
                    // macOS returns EIO when the last slave PTY fd closes.
                    _ if error.raw_os_error() == Some(libc::EIO) => break,
                    _ => break,
                },
            }
        }
    });
}

fn spawn_waiter_thread(session: CommandSession, mut child: Child) {
    thread::spawn(move || {
        let status = child.wait().ok();
        let exit_code = status.as_ref().and_then(|s| s.code());
        let signal = status
            .as_ref()
            .and_then(|s| s.signal())
            .map(signal_name);

        let changed = {
            let mut info = session.inner.info.lock().unwrap_or_else(|e| e.into_inner());
            if info.status != CommandStatus::Stopping {
                info.status = if exit_code == Some(0) && signal.is_none() {
                    CommandStatus::Finished
                } else {
                    CommandStatus::Failed { exit_code, signal }
                };
            }
            info.revision += 1;
            info.clone()
        };

        if let Ok(mut exited) = session.inner.exited.lock() {
            *exited = true;
            session.inner.exited_cv.notify_all();
        }
        if changed.status != CommandStatus::Stopping {
            session.inner.master.lock().unwrap_or_else(|e| e.into_inner()).take();
        }
        (session.inner.emit)(CommandEvent::StateChanged(changed));
    });
}

fn user_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL")
        && !shell.trim().is_empty()
        && Path::new(&shell).exists()
    {
        return shell;
    }
    if Path::new("/bin/zsh").exists() {
        "/bin/zsh".into()
    } else {
        "/bin/sh".into()
    }
}

fn enable_utf8_termios(fd: libc::c_int) {
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) } == 0 {
        let mut termios = unsafe { termios.assume_init() };
        termios.c_iflag |= libc::IUTF8;
        unsafe {
            libc::tcsetattr(fd, libc::TCSANOW, &termios);
        }
    }
}

fn slave_tty_name(fd: libc::c_int) -> Option<String> {
    let mut buf = [0 as libc::c_char; 128];
    if unsafe { libc::ttyname_r(fd, buf.as_mut_ptr(), buf.len()) } != 0 {
        return None;
    }
    let name = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy();
    let short = name.strip_prefix("/dev/").unwrap_or(&name);
    valid_tty(short).then(|| short.to_string())
}

fn valid_tty(tty: &str) -> bool {
    !tty.is_empty() && !matches!(tty, "?" | "??" | "-")
}

fn process_groups() -> Vec<(i32, i32, String)> {
    let Ok(output) = Command::new("/bin/ps").args(["-axo", "pid=,pgid=,tty="]).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            Some((
                columns.next()?.parse().ok()?,
                columns.next()?.parse().ok()?,
                columns.next()?.to_string(),
            ))
        })
        .collect()
}

fn tty_for_pid(processes: &[(i32, i32, String)], pid: i32) -> Option<String> {
    processes
        .iter()
        .find(|(p, _, _)| *p == pid)
        .map(|(_, _, tty)| tty.clone())
}

fn groups_on_terminal(processes: &[(i32, i32, String)], tty: &str) -> HashSet<i32> {
    processes
        .iter()
        .filter(|(_, pgid, terminal)| *pgid > 0 && valid_tty(tty) && terminal == tty)
        .map(|(_, pgid, _)| *pgid)
        .collect()
}

fn send_signal(pgid: i32, signal: libc::c_int) {
    if pgid > 0 {
        unsafe {
            libc::killpg(pgid, signal);
        }
    }
}

fn group_alive(pgid: i32) -> bool {
    pgid > 0 && unsafe { libc::killpg(pgid, 0) } == 0
}

fn wait_groups(groups: &HashSet<i32>, duration: Duration) {
    let deadline = Instant::now() + duration;
    while groups.iter().any(|pgid| group_alive(*pgid)) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
}

fn signal_name(sig: i32) -> String {
    match sig {
        libc::SIGHUP => "SIGHUP".into(),
        libc::SIGINT => "SIGINT".into(),
        libc::SIGQUIT => "SIGQUIT".into(),
        libc::SIGKILL => "SIGKILL".into(),
        libc::SIGTERM => "SIGTERM".into(),
        other => format!("SIG{other}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    fn line_texts(session: &CommandSession) -> Vec<String> {
        let snapshot = session.snapshot();
        (0..snapshot.rows)
            .map(|row| {
                snapshot.lines[row]
                    .iter()
                    .filter(|cell| !cell.spacer)
                    .map(|cell| cell.character)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    fn wait_until_status(
        session: &CommandSession,
        rx: &mpsc::Receiver<CommandEvent>,
        timeout: Duration,
        pred: impl Fn(&CommandStatus) -> bool,
    ) -> CommandInfo {
        let deadline = Instant::now() + timeout;
        loop {
            let info = session.info();
            if pred(&info.status) {
                return info;
            }
            assert!(Instant::now() < deadline, "timed out waiting for status, current: {info:?}");
            let _ = rx.recv_timeout(Duration::from_millis(100));
        }
    }

    #[test]
    fn runs_command_finishes_and_strips_secret_env() {
        let work = tempfile::tempdir().unwrap();
        let runner = Runner::new();
        let (tx, rx) = mpsc::channel();
        // Temporarily test with a dummy env check via `env` command (don't rely on ambient keys,
        // verify bash::SECRET_ENV is absent even if checked in parent shell if present).
        let session = runner
            .start(
                work.path(),
                CommandKind::Build,
                "echo build-ok; printf 'ENV=%s\\n' \"${OPENAI_API_KEY:-NONE}\"",
                PtySize::default(),
                move |event| {
                    let _ = tx.send(event);
                },
            )
            .unwrap();

        let done = wait_until_status(&session, &rx, Duration::from_secs(15), |s| !s.is_active());
        assert_eq!(done.status, CommandStatus::Finished);

        // Give the reader thread a brief window to flush final PTY bytes.
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let lines = line_texts(&session);
            if lines.iter().any(|l| l.contains("build-ok")) && lines.iter().any(|l| l.contains("ENV=NONE")) {
                return;
            }
            let _ = rx.recv_timeout(Duration::from_millis(50));
        }
        panic!("missing expected PTY output: {:?}", line_texts(&session));
    }

    #[test]
    fn failed_command_reports_exit_code() {
        let work = tempfile::tempdir().unwrap();
        let runner = Runner::new();
        let (tx, rx) = mpsc::channel();
        let session = runner
            .start(
                work.path(),
                CommandKind::Build,
                "exit 42",
                PtySize::default(),
                move |event| {
                    let _ = tx.send(event);
                },
            )
            .unwrap();
        let done = wait_until_status(&session, &rx, Duration::from_secs(15), |s| !s.is_active());
        assert_eq!(
            done.status,
            CommandStatus::Failed {
                exit_code: Some(42),
                signal: None
            }
        );
    }

    #[test]
    fn build_and_run_are_independent_and_stop_terminates_children() {
        let work = tempfile::tempdir().unwrap();
        let pid_file = work.path().join("child.pid");
        let runner = Runner::new();
        let (run_tx, run_rx) = mpsc::channel();
        let cmd = format!("sleep 30 & echo $! > {}; echo READY; wait", pid_file.display());
        let run_session = runner
            .start(work.path(), CommandKind::Run, &cmd, PtySize::default(), move |event| {
                let _ = run_tx.send(event);
            })
            .unwrap();

        let (build_tx, build_rx) = mpsc::channel();
        let build_session = runner
            .start(work.path(), CommandKind::Build, "echo concurrent-build", PtySize::default(), move |event| {
                let _ = build_tx.send(event);
            })
            .unwrap();
        let build_done = wait_until_status(&build_session, &build_rx, Duration::from_secs(15), |s| !s.is_active());
        assert_eq!(build_done.status, CommandStatus::Finished);
        assert!(run_session.is_active(), "Build must not interfere with active Run");

        let deadline = Instant::now() + Duration::from_secs(15);
        while !pid_file.exists() || !line_texts(&run_session).iter().any(|l| l.contains("READY")) {
            assert!(Instant::now() < deadline, "run process never became ready");
            let _ = run_rx.recv_timeout(Duration::from_millis(50));
        }
        let child_pid: i32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();

        // Starting the same kind while active returns the same generation rather than duplicating.
        let duplicate = runner
            .start(work.path(), CommandKind::Run, "echo nope", PtySize::default(), |_| {})
            .unwrap();
        assert_eq!(duplicate.generation(), run_session.generation());

        let stopped = run_session.stop_blocking().unwrap();
        assert_eq!(stopped.status, CommandStatus::Stopped);

        // Give the kernel a brief tick to reap the child group if needed.
        let kill_deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { libc::kill(child_pid, 0) } == 0 && Instant::now() < kill_deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert_ne!(unsafe { libc::kill(child_pid, 0) }, 0, "background sleep child must be killed on Stop");
    }

    #[test]
    fn clear_output_resets_screen_without_stopping_command() {
        let work = tempfile::tempdir().unwrap();
        let runner = Runner::new();
        let (tx, rx) = mpsc::channel();
        let session = runner
            .start(
                work.path(),
                CommandKind::Run,
                "echo first-line; sleep 0.4; echo second-line",
                PtySize::default(),
                move |event| {
                    let _ = tx.send(event);
                },
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        while !line_texts(&session).iter().any(|l| l.contains("first-line")) {
            assert!(Instant::now() < deadline);
            let _ = rx.recv_timeout(Duration::from_millis(50));
        }
        assert!(session.has_output());
        session.clear_output();
        assert!(!line_texts(&session).iter().any(|l| l.contains("first-line")));
        assert!(session.is_active(), "clear_output must not interrupt the running command");

        let done = wait_until_status(&session, &rx, Duration::from_secs(15), |s| !s.is_active());
        assert_eq!(done.status, CommandStatus::Finished);

        let tail_deadline = Instant::now() + Duration::from_secs(2);
        while !line_texts(&session).iter().any(|l| l.contains("second-line")) && Instant::now() < tail_deadline {
            let _ = rx.recv_timeout(Duration::from_millis(50));
        }
        assert!(line_texts(&session).iter().any(|l| l.contains("second-line")));
    }
}
