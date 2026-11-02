//! The menu bar follows both Code and Chat activity. Native menu APIs synchronously use the
//! main thread: never call them while holding activity, metadata or computer-use locks.
//! Refreshes are coalesced onto main, then render an owned snapshot with every lock released.
use crate::{models::AppData, storage::MetadataState};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{atomic::{AtomicBool, Ordering}, Mutex},
};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    App, AppHandle, Emitter, Manager,
};

const TRAY_ID: &str = "wackcode-menu-bar";
const CHAT_PREFIX: &str = "menu-chat:";
const OPEN_ID: &str = "menu-open";
const QUIT_ID: &str = "menu-quit";
const STOP_COMPUTER_ID: &str = "menu-stop-computer-use";
const RECENT_LIMIT: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunPhase {
    Idle,
    Working,
    Stopping,
}

impl Default for RunPhase {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunOutcome {
    Finished,
    Stopped,
    Failed,
    Interrupted,
}

impl RunOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Finished => "Finished",
            Self::Stopped => "Stopped",
            Self::Failed => "Failed",
            Self::Interrupted => "Interrupted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayStatus {
    Waiting,
    Working,
    Stopping,
    Recent(RunOutcome),
}

impl DisplayStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Waiting => "Waiting for user",
            Self::Working => "Working",
            Self::Stopping => "Stopping",
            Self::Recent(outcome) => outcome.label(),
        }
    }

    fn active_rank(self) -> Option<u8> {
        match self {
            Self::Waiting => Some(0),
            Self::Working => Some(1),
            Self::Stopping => Some(2),
            Self::Recent(_) => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ChatActivity {
    phase: RunPhase,
    pending_dialogs: HashSet<String>,
    plan_ready: bool,
    outcome: Option<RunOutcome>,
    touched: u64,
}

impl ChatActivity {
    fn status(&self) -> Option<DisplayStatus> {
        if self.phase == RunPhase::Stopping {
            Some(DisplayStatus::Stopping)
        } else if !self.pending_dialogs.is_empty() {
            Some(DisplayStatus::Waiting)
        } else if self.phase == RunPhase::Working {
            Some(DisplayStatus::Working)
        } else if self.plan_ready {
            Some(DisplayStatus::Waiting)
        } else {
            self.outcome.map(DisplayStatus::Recent)
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ActivityStore {
    chats: HashMap<String, ChatActivity>,
    recent: VecDeque<String>,
    clock: u64,
}

impl ActivityStore {
    fn touch(&mut self, task_id: &str) -> &mut ChatActivity {
        self.clock = self.clock.saturating_add(1);
        let touched = self.clock;
        let chat = self.chats.entry(task_id.to_string()).or_default();
        chat.touched = touched;
        chat
    }

    fn start(&mut self, task_id: &str) {
        let chat = self.touch(task_id);
        chat.phase = RunPhase::Working;
        chat.outcome = None;
        self.recent.retain(|id| id != task_id);
    }

    fn stopping(&mut self, task_id: &str) {
        self.touch(task_id).phase = RunPhase::Stopping;
    }

    fn idle(&mut self, task_id: &str) {
        self.touch(task_id).phase = RunPhase::Idle;
    }

    fn finish(&mut self, task_id: &str, outcome: RunOutcome) {
        let chat = self.touch(task_id);
        chat.phase = RunPhase::Idle;
        chat.outcome = Some(outcome);
        self.recent.retain(|id| id != task_id);
        self.recent.push_front(task_id.to_string());
        self.recent.truncate(RECENT_LIMIT);
    }

    fn request_dialog(&mut self, task_id: &str, request_id: &str) {
        self.touch(task_id)
            .pending_dialogs
            .insert(request_id.to_string());
    }

    fn resolve_dialog(&mut self, task_id: &str, request_id: &str) {
        self.touch(task_id).pending_dialogs.remove(request_id);
    }

    fn clear_dialogs(&mut self, task_id: &str) {
        if let Some(chat) = self.chats.get_mut(task_id) {
            chat.pending_dialogs.clear();
        }
    }

    fn set_plan_ready(&mut self, task_id: &str, ready: bool) {
        self.touch(task_id).plan_ready = ready;
    }

    fn remove(&mut self, task_id: &str) {
        self.chats.remove(task_id);
        self.recent.retain(|id| id != task_id);
    }

    fn active(&self) -> Vec<(&str, DisplayStatus)> {
        let mut rows = self
            .chats
            .iter()
            .filter_map(|(id, chat)| {
                let status = chat.status()?;
                status
                    .active_rank()
                    .map(|rank| (id.as_str(), status, rank, chat.touched))
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|(_, _, rank, touched)| (*rank, std::cmp::Reverse(*touched)));
        rows.into_iter()
            .map(|(id, status, _, _)| (id, status))
            .collect()
    }

    fn recent(&self) -> Vec<(&str, DisplayStatus)> {
        self.recent
            .iter()
            .filter_map(|id| {
                let chat = self.chats.get(id)?;
                match chat.status()? {
                    DisplayStatus::Recent(outcome) => {
                        Some((id.as_str(), DisplayStatus::Recent(outcome)))
                    }
                    _ => None,
                }
            })
            .collect()
    }
}

#[derive(Default)]
pub struct MenuBarState {
    activity: Mutex<ActivityStore>,
    pending_navigation: Mutex<Option<String>>,
    refresh_pending: AtomicBool,
}

impl MenuBarState {
    fn queue_refresh(&self, dispatch: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        if !self.request_refresh() {
            return Ok(());
        }
        dispatch().map_err(|error| {
            self.begin_refresh();
            error
        })
    }

    fn request_refresh(&self) -> bool {
        !self.refresh_pending.swap(true, Ordering::AcqRel)
    }

    fn begin_refresh(&self) {
        // Clear before sampling: an event arriving during the snapshot or native update must
        // queue a later refresh, rather than disappear behind the in-progress one.
        self.refresh_pending.store(false, Ordering::Release);
    }
}

pub fn setup(app: &App) -> Result<(), String> {
    let menu = with_snapshot(
        &app.state::<MenuBarState>().activity,
        &app.state::<MetadataState>().data,
        false,
        |snapshot| build_menu(app.handle(), &snapshot),
    )?;
    let icon = tray_icon()?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true)
        .tooltip("WackCode")
        .show_menu_on_left_click(true)
        .menu(&menu)
        .on_menu_event(|app, event| handle_menu_event(app, &event.id.0))
        .build(app)
        .map_err(|error| format!("Could not create the menu bar icon: {error}"))?;
    Ok(())
}

/// Derives a transparent mask from the dark duck in the existing app icon. macOS treats its
/// alpha channel as a template, so the symbol follows both light and dark menu bars.
fn tray_icon() -> Result<Image<'static>, String> {
    let source = Image::from_bytes(include_bytes!("../icons/32x32.png"))
        .map_err(|error| format!("Could not read the menu bar icon: {error}"))?;
    let mut rgba = source.rgba().to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        let dark = pixel[0] < 96 && pixel[1] < 96 && pixel[2] < 96;
        pixel[0] = 0;
        pixel[1] = 0;
        pixel[2] = 0;
        if !dark {
            pixel[3] = 0;
        }
    }
    Ok(Image::new_owned(rgba, source.width(), source.height()))
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    if id == OPEN_ID {
        show_main_window(app);
    } else if id == STOP_COMPUTER_ID {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { crate::computer_use::emergency_stop(&app, "menu bar").await });
    } else if id == QUIT_ID {
        app.exit(0);
    } else if let Some(task_id) = id.strip_prefix(CHAT_PREFIX) {
        let exists = app
            .state::<MetadataState>()
            .data
            .lock()
            .ok()
            .is_some_and(|data| {
                data.tasks
                    .iter()
                    .any(|task| task.id == task_id && !task.archived)
            });
        if !exists {
            return;
        }
        if let Ok(mut pending) = app.state::<MenuBarState>().pending_navigation.lock() {
            *pending = Some(task_id.to_string());
        }
        show_main_window(app);
        let _ = app.emit(
            "native-chat-navigation",
            serde_json::json!({ "taskId": task_id }),
        );
    }
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_window("main") {
        // Showing a hidden window is "opening the app": shuffle first, then tell the renderer
        // what it now shows. A window that was already on screen (menu bar while chatting, a
        // dock click with the window visible) keeps its background.
        if !window.is_visible().unwrap_or(true) {
            if let Some(config) = crate::commands::shuffle_if_enabled(&app.state::<MetadataState>()) {
                let _ = app.emit("background-shuffled", &config);
            }
        }
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
pub fn take_menu_navigation(state: tauri::State<'_, MenuBarState>) -> Option<String> {
    state
        .pending_navigation
        .lock()
        .ok()
        .and_then(|mut pending| pending.take())
}

pub fn handle_worker_event(app: &AppHandle, task_id: &str, value: &Value) {
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    let state = app.state::<MenuBarState>();
    let changed = if let Ok(mut activity) = state.activity.lock() {
        match event_type {
            "run_state" => match value.get("state").and_then(Value::as_str) {
                Some("running") => {
                    activity.start(task_id);
                    true
                }
                Some("stopping") => {
                    activity.stopping(task_id);
                    true
                }
                Some("idle") => {
                    activity.idle(task_id);
                    true
                }
                Some("interrupted") => {
                    activity.finish(task_id, RunOutcome::Interrupted);
                    true
                }
                _ => false,
            },
            "run_finished" => {
                let outcome = match value.get("outcome").and_then(Value::as_str) {
                    Some("completed") => Some(RunOutcome::Finished),
                    Some("stopped") => Some(RunOutcome::Stopped),
                    Some("failed") => Some(RunOutcome::Failed),
                    _ => None,
                };
                if let Some(outcome) = outcome {
                    activity.finish(task_id, outcome);
                    true
                } else {
                    false
                }
            }
            // A computer-use access card waits on the user just like an extension dialog.
            "extension_ui_request" | "computer_access_request" => {
                if let Some(request_id) = value.get("requestId").and_then(Value::as_str) {
                    activity.request_dialog(task_id, request_id);
                    true
                } else {
                    false
                }
            }
            "extension_ui_resolved" | "computer_access_resolved" => {
                if let Some(request_id) = value.get("requestId").and_then(Value::as_str) {
                    activity.resolve_dialog(task_id, request_id);
                    true
                } else {
                    false
                }
            }
            "plan_state" => {
                activity.set_plan_ready(
                    task_id,
                    value.get("phase").and_then(Value::as_str) == Some("ready"),
                );
                true
            }
            "ready" | "snapshot" => value
                .pointer("/snapshot/planState/phase")
                .and_then(Value::as_str)
                .map(|phase| {
                    activity.set_plan_ready(task_id, phase == "ready");
                })
                .is_some(),
            "snapshot_delta" => value
                .pointer("/delta/planState/phase")
                .and_then(Value::as_str)
                .map(|phase| {
                    activity.set_plan_ready(task_id, phase == "ready");
                })
                .is_some(),
            _ => false,
        }
    } else {
        false
    };
    if changed {
        let _ = refresh(app);
    }
}

pub fn worker_stopped(app: &AppHandle, task_id: &str, interrupted: bool) {
    if let Ok(mut activity) = app.state::<MenuBarState>().activity.lock() {
        if interrupted {
            activity.finish(task_id, RunOutcome::Interrupted);
        } else {
            activity.clear_dialogs(task_id);
        }
    }
    let _ = refresh(app);
}

pub fn remove_chat(app: &AppHandle, task_id: &str) {
    if let Ok(mut activity) = app.state::<MenuBarState>().activity.lock() {
        activity.remove(task_id);
    }
    let _ = refresh(app);
}

pub fn refresh(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<MenuBarState>();
    let queued_app = app.clone();
    state.queue_refresh(|| {
        app.run_on_main_thread(move || {
            queued_app.state::<MenuBarState>().begin_refresh();
            if let Err(error) = refresh_on_main(&queued_app) {
                eprintln!("Could not refresh the menu bar: {error}");
            }
        }).map_err(|error| format!("Could not queue the menu bar update: {error}"))
    })
}

fn refresh_on_main(app: &AppHandle) -> Result<(), String> {
    // Sample computer use before taking metadata: access answers hold computer-use state
    // while saving metadata. Reversing that order here would deadlock a menu refresh with an
    // answer or engine policy check. This advisory snapshot needs no overlapping locks.
    let computer_active = app.state::<crate::computer_use::ComputerUseManager>().any_session();
    with_snapshot(
        &app.state::<MenuBarState>().activity,
        &app.state::<MetadataState>().data,
        computer_active,
        |snapshot| {
            let menu = build_menu(app, &snapshot)?;
            if let Some(tray) = app.tray_by_id(TRAY_ID) {
                tray.set_menu(Some(menu))
                    .map_err(|error| format!("Could not update the menu bar: {error}"))?;
            }
            Ok(())
        },
    )
}

struct MenuSnapshot {
    computer_active: bool,
    active: Vec<MenuRow>,
    recent: Vec<MenuRow>,
}

/// The sink can create native menus or reenter window callbacks: no shared lock may reach it.
fn with_snapshot<T>(
    activity: &Mutex<ActivityStore>,
    metadata: &Mutex<AppData>,
    computer_active: bool,
    render: impl FnOnce(MenuSnapshot) -> Result<T, String>,
) -> Result<T, String> {
    let activity = activity.lock().map_err(|_| "Menu bar state lock was poisoned".to_string())?.clone();
    let snapshot = {
        let data = metadata.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        MenuSnapshot {
            computer_active,
            active: visible_rows(activity.active(), &data),
            recent: visible_rows(activity.recent(), &data),
        }
    };
    render(snapshot)
}

fn build_menu(
    manager: &impl Manager<tauri::Wry>,
    snapshot: &MenuSnapshot,
) -> Result<Menu<tauri::Wry>, String> {
    let menu = Menu::new(manager).map_err(|error| error.to_string())?;
    if snapshot.computer_active {
        menu.append(
            &MenuItem::with_id(manager, STOP_COMPUTER_ID, "Stop Computer Use  ⌃⌥⌘.", true, None::<&str>)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        append_separator(manager, &menu)?;
    }
    append_label(manager, &menu, "menu-active-heading", "Active Chats")?;
    if snapshot.active.is_empty() {
        append_label(manager, &menu, "menu-no-active", "No active chats")?;
    } else {
        for row in &snapshot.active {
            append_chat(manager, &menu, row)?;
        }
    }
    if !snapshot.recent.is_empty() {
        append_separator(manager, &menu)?;
        append_label(manager, &menu, "menu-recent-heading", "Recent Chats")?;
        for row in &snapshot.recent {
            append_chat(manager, &menu, row)?;
        }
    }
    append_separator(manager, &menu)?;
    menu.append(
        &MenuItem::with_id(manager, OPEN_ID, "Open WackCode", true, None::<&str>)
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    menu.append(
        &MenuItem::with_id(manager, QUIT_ID, "Quit WackCode", true, Some("Cmd+Q"))
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(menu)
}

struct MenuRow {
    task_id: String,
    task_name: String,
    project_name: Option<String>,
    status: DisplayStatus,
}

fn visible_rows(rows: Vec<(&str, DisplayStatus)>, data: &AppData) -> Vec<MenuRow> {
    rows.into_iter()
        .filter_map(|(task_id, status)| {
            let task = data
                .tasks
                .iter()
                .find(|task| task.id == task_id && !task.archived)?;
            let project_name = task.project_id.as_deref().and_then(|project_id| {
                data.projects
                    .iter()
                    .find(|project| project.id == project_id)
                    .map(|project| project.name.clone())
            });
            Some(MenuRow {
                task_id: task_id.to_string(),
                task_name: task.name.clone(),
                project_name,
                status,
            })
        })
        .collect()
}

fn append_chat(
    manager: &impl Manager<tauri::Wry>,
    menu: &Menu<tauri::Wry>,
    row: &MenuRow,
) -> Result<(), String> {
    let title = truncate(
        &row.task_name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        42,
    );
    let label = match row.project_name.as_deref() {
        Some(project) => format!(
            "{title} — {} — {}",
            truncate(project, 28),
            row.status.label()
        ),
        None => format!("{title} — {}", row.status.label()),
    };
    let item = MenuItem::with_id(
        manager,
        format!("{CHAT_PREFIX}{}", row.task_id),
        label,
        true,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    menu.append(&item).map_err(|error| error.to_string())
}

fn append_label(
    manager: &impl Manager<tauri::Wry>,
    menu: &Menu<tauri::Wry>,
    id: &str,
    label: &str,
) -> Result<(), String> {
    let item = MenuItem::with_id(manager, id, label, false, None::<&str>)
        .map_err(|error| error.to_string())?;
    menu.append(&item).map_err(|error| error.to_string())
}

fn append_separator(
    manager: &impl Manager<tauri::Wry>,
    menu: &Menu<tauri::Wry>,
) -> Result<(), String> {
    let item = PredefinedMenuItem::separator(manager).map_err(|error| error.to_string())?;
    menu.append(&item).map_err(|error| error.to_string())
}

fn truncate(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let prefix = chars.by_ref().take(max).collect::<String>();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, kind: crate::models::TaskKind) -> crate::models::TaskRecord {
        serde_json::from_value(serde_json::json!({
            "id": id, "kind": kind, "name": id, "workspacePath": "/tmp",
            "usesWorktree": false, "providerId": "test", "modelId": "test",
            "thinkingLevel": "off", "createdAt": "", "updatedAt": ""
        })).unwrap()
    }

    #[test]
    fn native_menu_sink_can_reenter_focus_and_activity_without_locks() {
        let state = MenuBarState::default();
        state.activity.lock().unwrap().start("code");
        let mut data = AppData::default();
        data.tasks.push(task("code", crate::models::TaskKind::Code));
        let metadata = Mutex::new(data);

        with_snapshot(&state.activity, &metadata, true, |snapshot| {
            // Native menu APIs can wait on AppKit while a focus callback needs metadata.
            // Acquiring both here exercises the same boundary without hanging the suite.
            let mut metadata = metadata.try_lock().expect("native menu retained metadata");
            let mut activity = state.activity.try_lock().expect("native menu retained activity");
            metadata.tasks[0].name = "Renamed while building".into();
            activity.finish("code", RunOutcome::Finished);
            assert!(snapshot.computer_active);
            assert_eq!(snapshot.active[0].task_name, "code");
            assert_eq!(snapshot.active[0].status, DisplayStatus::Working);
            Ok(())
        }).unwrap();
    }

    #[test]
    fn queued_refresh_coalesces_bursts_and_renders_latest_code_and_chat_state() {
        let state = MenuBarState::default();
        let mut data = AppData::default();
        data.tasks.push(task("code", crate::models::TaskKind::Code));
        data.tasks.push(task("chat", crate::models::TaskKind::Chat));
        let metadata = Mutex::new(data);

        state.activity.lock().unwrap().start("code");
        assert!(state.request_refresh());
        // The main-thread callback has not run yet; both later events share that refresh.
        state.activity.lock().unwrap().start("chat");
        assert!(!state.request_refresh());
        state.activity.lock().unwrap().finish("code", RunOutcome::Finished);
        assert!(!state.request_refresh());

        state.begin_refresh();
        with_snapshot(&state.activity, &metadata, false, |snapshot| {
            assert_eq!(snapshot.active.len(), 1);
            assert_eq!(snapshot.active[0].task_id, "chat");
            assert_eq!(snapshot.recent[0].task_id, "code");
            // An event during native construction must queue a follow-up, and its snapshot
            // must be taken when that callback runs rather than reusing the stale first one.
            state.activity.lock().unwrap().finish("chat", RunOutcome::Stopped);
            assert!(state.request_refresh());
            assert!(!state.request_refresh());
            Ok(())
        }).unwrap();

        state.begin_refresh();
        with_snapshot(&state.activity, &metadata, false, |snapshot| {
            assert!(snapshot.active.is_empty());
            assert_eq!(snapshot.recent[0].task_id, "chat");
            assert_eq!(snapshot.recent[0].status, DisplayStatus::Recent(RunOutcome::Stopped));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn failed_queue_can_be_requested_again() {
        let state = MenuBarState::default();
        let error = state.queue_refresh(|| Err("Event loop stopped".into())).unwrap_err();
        assert_eq!(error, "Event loop stopped");
        let mut dispatched = false;
        state.queue_refresh(|| {
            dispatched = true;
            Ok(())
        }).unwrap();
        assert!(dispatched);
    }

    #[test]
    fn idle_initialization_is_not_visible() {
        let mut store = ActivityStore::default();
        store.idle("untouched");
        assert!(store.active().is_empty());
        assert!(store.recent().is_empty());
    }

    #[test]
    fn waiting_precedes_working_and_stopping() {
        let mut store = ActivityStore::default();
        store.start("working");
        store.start("waiting");
        store.request_dialog("waiting", "question");
        store.start("stopping");
        store.stopping("stopping");
        assert_eq!(
            store.active().iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec!["waiting", "working", "stopping"]
        );
    }

    #[test]
    fn stopping_beats_a_pending_dialog() {
        let mut store = ActivityStore::default();
        store.start("chat");
        store.request_dialog("chat", "question");
        store.stopping("chat");
        assert_eq!(store.active(), vec![("chat", DisplayStatus::Stopping)]);
    }

    #[test]
    fn plan_waits_only_after_work_settles() {
        let mut store = ActivityStore::default();
        store.start("chat");
        store.set_plan_ready("chat", true);
        assert_eq!(store.active(), vec![("chat", DisplayStatus::Working)]);
        store.finish("chat", RunOutcome::Finished);
        assert_eq!(store.active(), vec![("chat", DisplayStatus::Waiting)]);
        assert!(store.recent().is_empty());
    }

    #[test]
    fn resolved_dialog_returns_to_working() {
        let mut store = ActivityStore::default();
        store.start("chat");
        store.request_dialog("chat", "question");
        store.resolve_dialog("chat", "question");
        assert_eq!(store.active(), vec![("chat", DisplayStatus::Working)]);
    }

    #[test]
    fn outcomes_survive_later_idle_frames() {
        let mut store = ActivityStore::default();
        store.start("failed");
        store.finish("failed", RunOutcome::Failed);
        store.idle("failed");
        assert_eq!(
            store.recent(),
            vec![("failed", DisplayStatus::Recent(RunOutcome::Failed))]
        );
        store.finish("interrupted", RunOutcome::Interrupted);
        assert_eq!(
            store.recent()[0],
            (
                "interrupted",
                DisplayStatus::Recent(RunOutcome::Interrupted)
            )
        );
    }

    #[test]
    fn another_run_removes_a_chat_from_recent() {
        let mut store = ActivityStore::default();
        store.finish("chat", RunOutcome::Finished);
        store.start("chat");
        assert!(store.recent().is_empty());
        assert_eq!(store.active(), vec![("chat", DisplayStatus::Working)]);
    }

    #[test]
    fn recent_history_keeps_ten_distinct_chats() {
        let mut store = ActivityStore::default();
        for index in 0..12 {
            store.finish(&format!("chat-{index}"), RunOutcome::Finished);
        }
        assert_eq!(store.recent().len(), 10);
        assert_eq!(store.recent()[0].0, "chat-11");
        store.finish("chat-5", RunOutcome::Stopped);
        assert_eq!(store.recent().len(), 10);
        assert_eq!(
            store.recent()[0],
            ("chat-5", DisplayStatus::Recent(RunOutcome::Stopped))
        );
    }

    #[test]
    fn removing_a_chat_cleans_active_and_recent_state() {
        let mut store = ActivityStore::default();
        store.start("active");
        store.finish("recent", RunOutcome::Finished);
        store.remove("active");
        store.remove("recent");
        assert!(store.active().is_empty());
        assert!(store.recent().is_empty());
    }
}
