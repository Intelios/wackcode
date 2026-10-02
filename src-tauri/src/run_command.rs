//! User-launched foreground commands, shared by canonical checkout, never by the worker.
//!
//! The registry lock serializes starts. A run's command/cwd are immutable snapshots; editing
//! configuration affects the next launch. Output replay and attachment changes share Core's
//! lock. Session + attachment identities prevent a dying panel from detaching its successor.
//! Child wait, independently of PTY EOF, determines completion. Lifecycle cleanup runs before
//! deleting a worktree, and only stops a shared local run when its last eligible chat leaves.

use crate::{
    models::{ProjectRecord, RunEvent, RunFrame, RunInfo, RunLookup, RunStatus, TerminalExit},
    pty_output::{append_scrollback, read_output, size},
    storage::MetadataState,
};
use nix::{sys::signal::{killpg, Signal}, unistd::Pid};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::{ipc::Channel, AppHandle, Emitter, State};
use uuid::Uuid;

type Emit = Arc<dyn Fn(RunEvent) + Send + Sync>;
type Output = Arc<dyn Fn(RunFrame) + Send + Sync>;

struct Attachment { id: String, send: Output }
#[derive(Default)]
struct Core { bytes: VecDeque<u8>, sink: Option<Attachment> }

struct Session {
    info: Mutex<RunInfo>,
    finished: Condvar,
    core: Mutex<Core>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    stop_lock: Mutex<()>,
    pid: i32,
    emit: Emit,
}

impl Session {
    fn info(&self) -> RunInfo {
        self.info.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn active(&self) -> bool {
        matches!(self.info().status, RunStatus::Running | RunStatus::Stopping)
    }

    fn push(&self, data: &str) {
        let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
        append_scrollback(&mut core.bytes, data.as_bytes());
        if let Some(sink) = &core.sink {
            (sink.send)(RunFrame {
                session_id: self.info().session_id,
                attachment_id: sink.id.clone(),
                data: data.to_string(),
            });
        }
    }

    fn attach(&self, id: String, send: Output) -> RunInfo {
        let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
        let info = self.info();
        if !core.bytes.is_empty() {
            send(RunFrame {
                session_id: info.session_id.clone(), attachment_id: id.clone(),
                data: String::from_utf8_lossy(core.bytes.make_contiguous()).into_owned(),
            });
        }
        core.sink = Some(Attachment { id, send });
        info
    }

    fn detach(&self, id: &str) {
        let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
        if core.sink.as_ref().is_some_and(|sink| sink.id == id) { core.sink = None; }
    }

    fn mark_stopping(&self) {
        let changed = {
            let mut info = self.info.lock().unwrap_or_else(|e| e.into_inner());
            if info.status != RunStatus::Running { return; }
            info.status = RunStatus::Stopping;
            info.revision += 1;
            info.clone()
        };
        (self.emit)(RunEvent::Changed { run: changed });
    }

    fn foreground(&self) -> Option<i32> {
        self.master.lock().ok().and_then(|master|
            master.as_ref().and_then(|master| master.process_group_leader())
        ).filter(|pid| *pid > 0)
    }

    fn wait_for_exit(&self, duration: Duration) {
        let info = self.info.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.finished.wait_timeout_while(info, duration, |info|
            info.exit.is_none()
        );
    }

    /// Serialize repeat Stop/cleanup requests; signal jobs as well as their parent shell.
    fn stop(&self) -> Result<RunInfo, String> {
        let _stop = self.stop_lock.lock().map_err(|_| "Could not lock this run.".to_string())?;
        if !self.active() { return Ok(self.info()); }
        self.mark_stopping();
        let processes = process_groups();
        let tty = processes.iter().find(|(pid, _, _)| *pid == self.pid)
            .map(|(_, _, tty)| tty.clone()).filter(|tty| valid_tty(tty));
        let mut groups = HashSet::from([self.pid]);
        if let Some(tty) = &tty { groups.extend(groups_on_terminal(&processes, tty)); }
        if let Some(foreground) = self.foreground() {
            groups.insert(foreground);
            let _ = killpg(Pid::from_raw(foreground), Signal::SIGINT);
        }
        wait_groups(&groups, Duration::from_secs(2));
        if let Some(tty) = &tty { groups.extend(groups_on_terminal(&process_groups(), tty)); }
        // An exited shell can leave a child holding the PTY open. Always check its group too.
        if groups.iter().any(|pid| group_alive(*pid)) {
            if let Some(foreground) = self.foreground() { groups.insert(foreground); }
            for pid in &groups { let _ = killpg(Pid::from_raw(*pid), Signal::SIGTERM); }
            wait_groups(&groups, Duration::from_secs(1));
            if let Some(tty) = &tty { groups.extend(groups_on_terminal(&process_groups(), tty)); }
            for pid in &groups {
                if group_alive(*pid) { let _ = killpg(Pid::from_raw(*pid), Signal::SIGKILL); }
            }
        }
        self.wait_for_exit(Duration::from_secs(1));
        let stopped = {
            let mut info = self.info.lock().unwrap_or_else(|e| e.into_inner());
            if info.exit.is_none() { return Err("Could not stop this run. Try Stop again.".into()); }
            info.status = RunStatus::Stopped;
            info.revision += 1;
            info.clone()
        };
        (self.emit)(RunEvent::Changed { run: stopped.clone() });
        // The reader has its own descriptor and can drain the command's final output.
        self.writer.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.master.lock().unwrap_or_else(|e| e.into_inner()).take();
        Ok(stopped)
    }
}

fn valid_tty(tty: &str) -> bool {
    !tty.is_empty() && !matches!(tty, "?" | "??" | "-")
}

/// Interactive shells put background jobs in their own groups. Keep the PTY identity before
/// interrupting the shell so reparented jobs are still found; detached sessions have no tty.
fn process_groups() -> Vec<(i32, i32, String)> {
    let Ok(output) = Command::new("/bin/ps").args(["-axo", "pid=,pgid=,tty="]).output() else { return Vec::new(); };
    if !output.status.success() { return Vec::new(); }
    String::from_utf8_lossy(&output.stdout).lines().filter_map(|line| {
        let mut columns = line.split_whitespace();
        Some((columns.next()?.parse().ok()?, columns.next()?.parse().ok()?, columns.next()?.to_string()))
    }).collect()
}

fn groups_on_terminal(processes: &[(i32, i32, String)], tty: &str) -> HashSet<i32> {
    processes.iter().filter(|(_, pgid, terminal)| *pgid > 0 && valid_tty(tty) && terminal == tty)
        .map(|(_, pgid, _)| *pgid).collect()
}

fn group_alive(pid: i32) -> bool {
    pid > 0 && killpg(Pid::from_raw(pid), None).is_ok()
}

fn wait_groups(groups: &HashSet<i32>, duration: Duration) {
    let deadline = Instant::now() + duration;
    while groups.iter().any(|pid| group_alive(*pid)) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
}

#[derive(Clone, Default)]
pub struct RunState {
    sessions: Arc<Mutex<HashMap<PathBuf, Arc<Session>>>>,
    generation: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
}

fn canonical_folder(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn emitter(app: &AppHandle) -> Emit {
    let app = app.clone();
    Arc::new(move |event| { let _ = app.emit("run-event", event); })
}

fn spawn(
    cwd: &Path, command: &str, generation: u64, emit: Emit, shell: &str, login: bool,
) -> Result<(Arc<Session>, Box<dyn portable_pty::Child + Send + Sync>, Box<dyn std::io::Read + Send>), String> {
    let pair = native_pty_system().openpty(size(80, 24))
        .map_err(|e| format!("Could not open the Run terminal: {e}"))?;
    let reader = pair.master.try_clone_reader().map_err(|e| format!("Could not read Run output: {e}"))?;
    let writer = pair.master.take_writer().map_err(|e| format!("Could not write to Run: {e}"))?;
    let mut builder = CommandBuilder::new(shell);
    if login { builder.arg("-l"); }
    builder.args(["-i", "-c", command]);
    builder.cwd(cwd);
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder.env("TERM_PROGRAM", "WackCode");
    if std::env::var_os("LANG").is_none() { builder.env("LANG", "en_US.UTF-8"); }
    let mut child = pair.slave.spawn_command(builder)
        .map_err(|e| format!("Could not start the run: {e}"))?;
    let Some(pid) = child.process_id().filter(|pid| *pid > 0) else {
        let _ = child.kill();
        return Err("Could not identify the run's process.".into());
    };
    drop(pair.slave);
    let session = Arc::new(Session {
        info: Mutex::new(RunInfo {
            session_id: Uuid::new_v4().to_string(), generation, revision: 1,
            cwd: cwd.to_string_lossy().into_owned(), command: command.to_string(),
            status: RunStatus::Running, exit: None,
        }),
        finished: Condvar::new(), core: Mutex::new(Core::default()),
        master: Mutex::new(Some(pair.master)), writer: Mutex::new(Some(writer)),
        stop_lock: Mutex::new(()), pid: pid as i32, emit,
    });
    Ok((session, child, reader))
}

impl RunState {
    fn start(&self, cwd: &Path, command: &str, emit: Emit, shell: &str, login: bool) -> Result<RunInfo, String> {
        let cwd = cwd.canonicalize().map_err(|_| "This chat's folder no longer exists.".to_string())?;
        let mut sessions = self.sessions.lock().map_err(|_| "Could not lock the runs.".to_string())?;
        if self.shutdown.load(Ordering::SeqCst) { return Err("WackCode is quitting.".into()); }
        if let Some(session) = sessions.get(&cwd).filter(|session| session.active()) { return Ok(session.info()); }
        if command.trim().is_empty() { return Err("Set a project run command first.".into()); }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let (session, mut child, reader) = spawn(&cwd, command, generation, emit, shell, login)?;
        let info = session.info();
        sessions.insert(cwd, session.clone());
        (session.emit)(RunEvent::Changed { run: info.clone() });
        let output_session = session.clone();
        thread::spawn(move || read_output(reader, |text| output_session.push(text)));
        thread::spawn(move || {
            let exit = child.wait().ok()
                .map(|exit| TerminalExit { code: exit.exit_code() as i32, signal: exit.signal().map(str::to_string) })
                .unwrap_or(TerminalExit { code: -1, signal: None });
            let info = {
                let mut info = session.info.lock().unwrap_or_else(|e| e.into_inner());
                // Stop owns the final transition: a shell can exit while its job still lives.
                info.status = if info.status == RunStatus::Stopping { RunStatus::Stopping }
                    else if exit.code == 0 && exit.signal.is_none() { RunStatus::Finished } else { RunStatus::Failed };
                info.exit = Some(exit);
                info.revision += 1;
                info.clone()
            };
            session.finished.notify_all();
            (session.emit)(RunEvent::Changed { run: info });
        });
        Ok(info)
    }

    fn by_id(&self, id: &str) -> Result<Arc<Session>, String> {
        self.sessions.lock().map_err(|_| "Could not lock the runs.".to_string())?
            .values().find(|session| session.info().session_id == id).cloned()
            .ok_or_else(|| "This run has ended or been replaced.".to_string())
    }

    /// Metadata is checked before reserving cleanup; no metadata/registry lock crosses a wait.
    pub async fn release_workspace(&self, state: &MetadataState, workspace: &str, removing: bool) -> Result<(), String> {
        let cwd = canonical_folder(Path::new(workspace));
        let session = {
            let data = state.data.lock().map_err(|_| "Could not read the chats.".to_string())?;
            if !removing && has_chat(&data.tasks, &cwd) { return Ok(()); }
            let sessions = self.sessions.lock().map_err(|_| "Could not lock the runs.".to_string())?;
            let session = sessions.get(&cwd).cloned();
            if let Some(session) = &session { session.mark_stopping(); }
            session
        };
        if let Some(session) = session {
            let stopping = session.clone();
            tauri::async_runtime::spawn_blocking(move || stopping.stop()).await
                .map_err(|e| format!("Could not stop the run: {e}"))??;
            let data = state.data.lock().map_err(|_| "Could not read the chats.".to_string())?;
            if removing || !has_chat(&data.tasks, &cwd) {
                let mut sessions = self.sessions.lock().map_err(|_| "Could not lock the runs.".to_string())?;
                if sessions.get(&cwd).is_some_and(|current| Arc::ptr_eq(current, &session)) {
                    sessions.remove(&cwd);
                    (session.emit)(RunEvent::Removed { cwd: session.info().cwd, session_id: session.info().session_id, generation: session.info().generation });
                }
            }
        }
        Ok(())
    }

    pub fn terminate_all(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let sessions: Vec<_> = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).drain().map(|(_, s)| s).collect();
        // Stop different checkouts together so quit takes one grace period, not one per run.
        let stops: Vec<_> = sessions.into_iter().map(|session| thread::spawn(move || { let _ = session.stop(); })).collect();
        for stop in stops { let _ = stop.join(); }
    }
}

fn has_chat(tasks: &[crate::models::TaskRecord], cwd: &Path) -> bool {
    tasks.iter().any(|task| !task.archived && task.project_id.is_some() && canonical_folder(Path::new(&task.workspace_path)) == cwd)
}

fn resolve(state: &MetadataState, task_id: &str) -> Result<(PathBuf, Option<String>), String> {
    let data = state.data.lock().map_err(|_| "Could not read the project.".to_string())?;
    let task = data.tasks.iter().find(|task| task.id == task_id && !task.archived)
        .ok_or_else(|| "This chat is missing or archived.".to_string())?;
    let project = task.project_id.as_ref().and_then(|id| data.projects.iter().find(|project| &project.id == id))
        .ok_or_else(|| "Choose a project before running a command.".to_string())?;
    let cwd = Path::new(&task.workspace_path).canonicalize()
        .map_err(|_| "This chat's folder no longer exists.".to_string())?;
    if !cwd.is_dir() { return Err("This chat's folder no longer exists.".into()); }
    Ok((cwd, project.run_command.clone()))
}

#[tauri::command]
pub fn save_project_run_command(state: State<'_, MetadataState>, project_id: String, command: String) -> Result<ProjectRecord, String> {
    save_command(&state, &project_id, &command)
}

fn save_command(state: &MetadataState, project_id: &str, command: &str) -> Result<ProjectRecord, String> {
    if command.contains('\0') { return Err("The run command contains an invalid character.".into()); }
    state.mutate(|data| {
        let project = data.projects.iter_mut().find(|project| project.id == project_id)
            .ok_or_else(|| "This project no longer exists.".to_string())?;
        project.run_command = if command.trim().is_empty() { None } else { Some(command.trim().to_string()) };
        Ok(project.clone())
    })
}

#[tauri::command]
pub async fn start_run(app: AppHandle, state: State<'_, MetadataState>, runs: State<'_, RunState>, task_id: String) -> Result<RunInfo, String> {
    let _task = crate::commands::task_lock(&app, &task_id).lock_owned().await;
    let (cwd, command) = resolve(&state, &task_id)?;
    let shell = std::env::var("SHELL").ok().filter(|shell| shell.starts_with('/')).unwrap_or_else(|| "/bin/zsh".into());
    let runs = runs.inner().clone();
    let emit = emitter(&app);
    tauri::async_runtime::spawn_blocking(move || runs.start(&cwd, command.as_deref().unwrap_or(""), emit, &shell, true))
        .await.map_err(|e| format!("Could not start the run: {e}"))?
}

#[tauri::command]
pub fn get_run(state: State<'_, MetadataState>, runs: State<'_, RunState>, task_id: String) -> Result<RunLookup, String> {
    let (cwd, _) = resolve(&state, &task_id)?;
    let sessions = runs.sessions.lock().map_err(|_| "Could not lock the runs.".to_string())?;
    let run = sessions.get(&cwd).map(|s| s.info());
    let generation = runs.generation.load(Ordering::SeqCst);
    Ok(RunLookup { generation, cwd: cwd.to_string_lossy().into_owned(), run })
}

#[tauri::command]
pub async fn stop_run(runs: State<'_, RunState>, session_id: String) -> Result<RunInfo, String> {
    let session = runs.by_id(&session_id)?;
    tauri::async_runtime::spawn_blocking(move || session.stop()).await.map_err(|e| format!("Could not stop the run: {e}"))?
}

#[tauri::command]
pub fn attach_run(runs: State<'_, RunState>, session_id: String, attachment_id: String, cols: u16, rows: u16, on_frame: Channel<RunFrame>) -> Result<RunInfo, String> {
    let session = runs.by_id(&session_id)?;
    if let Some(master) = session.master.lock().map_err(|_| "Could not resize Run.".to_string())?.as_ref() {
        master.resize(size(cols, rows)).map_err(|e| format!("Could not resize Run: {e}"))?;
    }
    Ok(session.attach(attachment_id, Arc::new(move |frame| { let _ = on_frame.send(frame); })))
}

#[tauri::command]
pub fn detach_run(runs: State<'_, RunState>, session_id: String, attachment_id: String) -> Result<(), String> {
    if let Ok(session) = runs.by_id(&session_id) { session.detach(&attachment_id); }
    Ok(())
}

#[tauri::command]
pub fn write_run(runs: State<'_, RunState>, session_id: String, data: String) -> Result<(), String> {
    let session = runs.by_id(&session_id)?;
    if session.info().status != RunStatus::Running { return Err("This run is no longer accepting input.".into()); }
    let mut writer = session.writer.lock().map_err(|_| "Could not write to Run.".to_string())?;
    writer.as_mut().ok_or_else(|| "This run has ended.".to_string())?
        .write_all(data.as_bytes()).map_err(|e| format!("Could not write to Run: {e}"))
}

#[tauri::command]
pub fn resize_run(runs: State<'_, RunState>, session_id: String, cols: u16, rows: u16) -> Result<(), String> {
    let session = runs.by_id(&session_id)?;
    if let Some(master) = session.master.lock().map_err(|_| "Could not resize Run.".to_string())?.as_ref() {
        master.resize(size(cols, rows)).map_err(|e| format!("Could not resize Run: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::{AppData, TaskRecord}, secrets::SecretStore};
    use serde_json::json;
    use std::fs;

    struct Harness(RunState);
    impl Harness {
        fn new() -> Self { Self(RunState::default()) }
        fn start(&self, cwd: &Path, command: &str) -> RunInfo {
            // Test real PTYs without sourcing the owner's login profiles.
            self.0.start(cwd, command, Arc::new(|_| {}), "/bin/sh", false).unwrap()
        }
    }
    impl Drop for Harness { fn drop(&mut self) { self.0.terminate_all(); } }

    fn eventually(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(6);
        while !condition() {
            assert!(Instant::now() < deadline, "Timed out waiting for the run");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn task(id: &str, cwd: &Path) -> TaskRecord {
        serde_json::from_value(json!({
            "id": id, "projectId": "p", "name": id, "workspacePath": cwd.to_string_lossy(),
            "usesWorktree": false, "providerId": "", "modelId": "", "thinkingLevel": "off",
            "createdAt": "now", "updatedAt": "now"
        })).unwrap()
    }

    fn metadata(directory: &Path, tasks: Vec<TaskRecord>) -> MetadataState {
        let project: ProjectRecord = serde_json::from_value(json!({
            "id": "p", "name": "Project", "path": directory.to_string_lossy(),
            "gitHasHead": false, "createdAt": "now"
        })).unwrap();
        MetadataState {
            data: Mutex::new(AppData { projects: vec![project], tasks, ..AppData::default() }),
            data_path: directory.join("wackcode.json"), secrets: SecretStore::load(directory).unwrap(),
        }
    }

    #[test]
    fn old_projects_load_and_configuration_persists_without_touching_a_running_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let state = metadata(directory.path(), vec![task("a", directory.path())]);
        assert!(state.data.lock().unwrap().projects[0].run_command.is_none());
        save_command(&state, "p", "  printf '%s' 'hello world'  ").unwrap();
        let data: AppData = serde_json::from_slice(&fs::read(&state.data_path).unwrap()).unwrap();
        assert_eq!(data.projects[0].run_command.as_deref(), Some("printf '%s' 'hello world'"));
        let runs = Harness::new();
        let running = runs.start(directory.path(), "sleep 30");
        save_command(&state, "p", "printf next").unwrap();
        assert_eq!(runs.0.by_id(&running.session_id).unwrap().info().command, "sleep 30");
        save_command(&state, "p", " \n ").unwrap();
        assert!(state.data.lock().unwrap().projects[0].run_command.is_none());
        assert!(save_command(&state, "p", "bad\0command").is_err());
    }

    #[test]
    fn sharing_uses_canonical_folders_and_worktrees_are_independent() {
        let directory = tempfile::tempdir().unwrap();
        let local = directory.path().join("local");
        let tree = directory.path().join("tree");
        let alias = directory.path().join("alias");
        fs::create_dir(&local).unwrap();
        fs::create_dir(&tree).unwrap();
        std::os::unix::fs::symlink(&local, &alias).unwrap();
        let runs = Harness::new();
        let first = runs.start(&local, "sleep 30");
        let shared = runs.start(&alias, "echo changed");
        let independent = runs.start(&tree, "sleep 30");
        assert_eq!(first.session_id, shared.session_id);
        assert_eq!(shared.command, "sleep 30");
        assert_ne!(first.session_id, independent.session_id);
        assert_ne!(first.cwd, independent.cwd);
    }

    #[test]
    fn concurrent_starts_spawn_one_command() {
        let directory = tempfile::tempdir().unwrap();
        let runs = Harness::new();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let starts: Vec<_> = (0..2).map(|_| {
            let state = runs.0.clone();
            let barrier = barrier.clone();
            let cwd = directory.path().to_path_buf();
            thread::spawn(move || {
                barrier.wait();
                state.start(&cwd, "sleep 30", Arc::new(|_| {}), "/bin/sh", false).unwrap()
            })
        }).collect();
        barrier.wait();
        let ids: Vec<_> = starts.into_iter().map(|start| start.join().unwrap().session_id).collect();
        assert_eq!(ids[0], ids[1]);
    }

    #[test]
    fn command_quoting_cwd_utf8_exit_and_replacement_are_preserved() {
        let directory = tempfile::Builder::new().prefix("run folder ").tempdir().unwrap();
        let runs = Harness::new();
        let failed = runs.start(directory.path(), "printf '%s\\n' 'hello world 🦆'; pwd; exit 7");
        let session = runs.0.by_id(&failed.session_id).unwrap();
        eventually(|| session.info().exit.is_some());
        eventually(|| {
            let core = session.core.lock().unwrap();
            String::from_utf8_lossy(&core.bytes.iter().copied().collect::<Vec<_>>()).contains("hello world 🦆")
        });
        let info = session.info();
        assert_eq!(info.status, RunStatus::Failed);
        assert_eq!(info.exit.unwrap().code, 7);
        let output = session.core.lock().unwrap().bytes.iter().copied().collect::<Vec<_>>();
        assert!(String::from_utf8_lossy(&output).contains(&failed.cwd));
        let replacement = runs.start(directory.path(), "exit 0");
        assert_ne!(failed.session_id, replacement.session_id);
        assert!(runs.0.by_id(&failed.session_id).is_err());
        let session = runs.0.by_id(&replacement.session_id).unwrap();
        eventually(|| session.info().status == RunStatus::Finished);
    }

    #[test]
    fn reattach_replays_output_and_a_stale_detach_leaves_the_new_attachment_alone() {
        let directory = tempfile::tempdir().unwrap();
        let runs = Harness::new();
        let info = runs.start(directory.path(), "read reply; printf '%s\\n' \"$reply\"");
        let session = runs.0.by_id(&info.session_id).unwrap();
        session.writer.lock().unwrap().as_mut().unwrap().write_all(b"from input\n").unwrap();
        eventually(|| session.info().exit.is_some());
        eventually(|| session.core.lock().unwrap().bytes.len() > 0);
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let received = received.clone();
            Arc::new(move |frame: RunFrame| received.lock().unwrap().push(frame)) as Output
        };
        session.attach("old".into(), sink.clone());
        session.attach("new".into(), sink);
        session.detach("old");
        session.push("live");
        let frames = received.lock().unwrap();
        assert!(frames[0].data.contains("from input"));
        assert_eq!(frames.last().unwrap().attachment_id, "new");
        assert_eq!(frames.last().unwrap().session_id, info.session_id);
        assert_eq!(frames.last().unwrap().data, "live");
        assert_eq!(frames.len(), 3);
    }

    #[test]
    fn stop_escalates_for_a_foreground_job_and_its_children() {
        let directory = tempfile::tempdir().unwrap();
        let runs = Harness::new();
        let info = runs.start(directory.path(),
            "/bin/sh -c 'trap \"\" INT TERM; echo $$ > job.pid; sleep 30 & echo $! > child.pid; wait'");
        eventually(|| directory.path().join("child.pid").exists());
        let job: i32 = fs::read_to_string(directory.path().join("job.pid")).unwrap().trim().parse().unwrap();
        let child: i32 = fs::read_to_string(directory.path().join("child.pid")).unwrap().trim().parse().unwrap();
        let session = runs.0.by_id(&info.session_id).unwrap();
        let stopped = session.stop().unwrap();
        assert_eq!(stopped.status, RunStatus::Stopped);
        eventually(|| !group_alive(job));
        eventually(|| nix::sys::signal::kill(Pid::from_raw(child), None).is_err());
        assert_eq!(session.stop().unwrap().status, RunStatus::Stopped);
    }

    #[test]
    fn stop_cleans_up_background_jobs_while_the_command_waits_for_them() {
        let directory = tempfile::tempdir().unwrap();
        let runs = Harness::new();
        let info = runs.start(directory.path(), "sleep 30 & echo $! > child.pid; wait");
        eventually(|| directory.path().join("child.pid").exists());
        let child: i32 = fs::read_to_string(directory.path().join("child.pid")).unwrap().trim().parse().unwrap();
        let session = runs.0.by_id(&info.session_id).unwrap();
        session.stop().unwrap();
        wait_groups(&HashSet::from([child]), Duration::from_secs(1));
        let alive = nix::sys::signal::kill(Pid::from_raw(child), None).is_ok();
        // Clean up even when this assertion catches a regression.
        if alive { let _ = nix::sys::signal::kill(Pid::from_raw(child), Signal::SIGKILL); }
        assert!(!alive, "A background job outlived its foreground command");
    }

    #[test]
    fn cleanup_group_discovery_excludes_other_terminals_and_detached_sessions() {
        let processes = vec![(1, 1, "??".into()), (10, 10, "ttys001".into()),
            (11, 11, "ttys001".into()), (12, 12, "ttys002".into())];
        assert_eq!(groups_on_terminal(&processes, "ttys001"), HashSet::from([10, 11]));
        assert!(groups_on_terminal(&processes, "??").is_empty());
    }

    #[test]
    fn cleanup_retains_shared_runs_and_stops_before_worktree_removal() {
        tauri::async_runtime::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let cwd = directory.path().join("checkout");
        fs::create_dir(&cwd).unwrap();
        let metadata = metadata(directory.path(), vec![task("a", &cwd), task("b", &cwd)]);
        let runs = Harness::new();
        let run = runs.start(&cwd, "sleep 30");
        metadata.mutate(|data| { data.tasks[0].archived = true; Ok(()) }).unwrap();
        runs.0.release_workspace(&metadata, cwd.to_str().unwrap(), false).await.unwrap();
        assert!(runs.0.by_id(&run.session_id).unwrap().active());
        metadata.mutate(|data| { data.tasks[1].archived = true; Ok(()) }).unwrap();
        runs.0.release_workspace(&metadata, cwd.to_str().unwrap(), false).await.unwrap();
        assert!(runs.0.by_id(&run.session_id).is_err());
        let run = runs.start(&cwd, "sleep 30");
        metadata.mutate(|data| { data.tasks[0].archived = false; Ok(()) }).unwrap();
        runs.0.release_workspace(&metadata, cwd.to_str().unwrap(), true).await.unwrap();
        assert!(runs.0.by_id(&run.session_id).is_err());
        fs::remove_dir(&cwd).unwrap();
        });
    }

    #[test]
    fn moving_the_last_chat_releases_its_old_checkout() {
        tauri::async_runtime::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let old = directory.path().join("old");
        let new = directory.path().join("new");
        fs::create_dir(&old).unwrap();
        fs::create_dir(&new).unwrap();
        let metadata = metadata(directory.path(), vec![task("a", &old)]);
        let runs = Harness::new();
        let run = runs.start(&old, "sleep 30");
        metadata.mutate(|data| { data.tasks[0].workspace_path = new.to_string_lossy().into_owned(); Ok(()) }).unwrap();
        runs.0.release_workspace(&metadata, old.to_str().unwrap(), false).await.unwrap();
        assert!(runs.0.by_id(&run.session_id).is_err());
        assert!(resolve(&metadata, "a").unwrap().0.ends_with("new"));
        });
    }

    #[test]
    fn shutdown_stops_every_checkout_and_rejects_new_starts() {
        let directory = tempfile::tempdir().unwrap();
        let runs = Harness::new();
        let run = runs.start(directory.path(), "sleep 30");
        let session = runs.0.by_id(&run.session_id).unwrap();
        runs.0.terminate_all();
        assert!(!session.active());
        assert!(runs.0.start(directory.path(), "echo never", Arc::new(|_| {}), "/bin/sh", false).is_err());
        runs.0.terminate_all();
    }

    #[test]
    fn missing_directories_and_archived_chats_cannot_launch() {
        let directory = tempfile::tempdir().unwrap();
        let metadata = metadata(directory.path(), vec![task("a", directory.path())]);
        metadata.mutate(|data| { data.tasks[0].archived = true; Ok(()) }).unwrap();
        assert!(resolve(&metadata, "a").is_err());
        let runs = Harness::new();
        assert!(runs.0.start(&directory.path().join("missing"), "echo never", Arc::new(|_| {}), "/bin/sh", false).is_err());
    }
}
