//! Panes: a chat's Changes, Terminal, Tasks, Agents, Browser and Canvas views beside and below its transcript.
//!
//! `layout` is the tree of docks, splits and tab stacks, as data; `solve` turns it and the window
//! size into rectangles. Both are pure and property-tested. `dock` draws them, and each pane's
//! own state lives in a model kept here, so hiding or moving a pane never loses it.
//!
//! Invariants:
//! - [`ChatPanes`] holds a model for every pane that has been shown, and only for panes still in
//!   the layout: closing a pane drops its model (a terminal's shell ends with it).
//! - A model is made the first time its pane is shown, so a saved layout full of terminals costs
//!   nothing until the user looks at them.

pub mod agents;
pub mod browser;
pub mod canvas;
pub mod changes;
pub mod dock;
pub mod grid;
pub mod layout;
pub mod run;
pub mod solve;
pub mod tasks;
pub mod terminal;

use std::{collections::HashMap, path::PathBuf};

use gpui::{App, AppContext, Entity, FocusHandle, SharedString, Subscription};
use wack_core::{CommandKind, CommandSession, CommandStatus, Runner, agents::Agents, tasks::Tasks};

pub use agents::AgentsPane;
pub use browser::BrowserPane;
pub use canvas::CanvasPane;
pub use changes::ChangesPane;
pub use layout::{Axis, Dock, Edge, Layout, Node, Pane, PaneId, PaneKind, Stack, Target};
pub use run::RunPane;
pub use solve::{Rect, Solution, solve};
pub use tasks::TasksPane;
pub use terminal::TerminalPane;

#[derive(Clone)]
pub enum PaneModel {
    Changes(Entity<ChangesPane>),
    Terminal(Entity<TerminalPane>),
    Build(Entity<RunPane>),
    Run(Entity<RunPane>),
    Tasks(Entity<TasksPane>),
    Agents(Entity<AgentsPane>),
    Browser(Entity<BrowserPane>),
    Canvas(Entity<CanvasPane>),
}

impl PaneModel {
    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self {
            PaneModel::Changes(pane) => pane.read(cx).focus.clone(),
            PaneModel::Terminal(pane) => pane.read(cx).focus.clone(),
            PaneModel::Build(pane) | PaneModel::Run(pane) => pane.read(cx).focus.clone(),
            PaneModel::Tasks(pane) => pane.read(cx).focus.clone(),
            PaneModel::Agents(pane) => pane.read(cx).focus.clone(),
            PaneModel::Browser(pane) => pane.read(cx).focus.clone(),
            PaneModel::Canvas(pane) => pane.read(cx).focus.clone(),
        }
    }
}

pub struct ChatPanes {
    pub layout: Layout,
    folder: PathBuf,
    runner: Runner,
    build_command: Option<String>,
    run_command: Option<String>,
    /// The chat's background commands, for its Tasks pane.
    tasks: Option<Tasks>,
    /// The chat's sub-agents, for its Agents pane.
    agents: Option<Agents>,
    /// The chat's Team as its Agents pane shows it.
    team: Option<agents::TeamSource>,
    /// The chat's browser tabs, for its Browser pane.
    browser: Option<wack_browser::ChatTabs>,
    /// The chat's canvases, for its Canvas pane.
    canvases: Option<wack_canvas::Canvases>,
    models: HashMap<PaneId, (PaneModel, Subscription)>,
}

impl ChatPanes {
    pub fn new(layout: Layout, folder: PathBuf) -> Self {
        Self {
            layout,
            folder,
            runner: Runner::new(),
            build_command: None,
            run_command: None,
            tasks: None,
            agents: None,
            team: None,
            browser: None,
            canvases: None,
            models: HashMap::new(),
        }
    }

    pub fn set_tasks(&mut self, tasks: Tasks) {
        self.tasks = Some(tasks);
    }

    pub fn set_agents(&mut self, agents: Agents) {
        self.agents = Some(agents);
    }

    /// Where the Agents pane reads the chat's Team from, now and for one made later.
    pub fn set_team_source(&mut self, source: agents::TeamSource, cx: &mut App) {
        self.team = Some(source.clone());
        for (model, _) in self.models.values() {
            if let PaneModel::Agents(pane) = model {
                pane.update(cx, |pane, cx| pane.set_team_source(Some(source.clone()), cx));
            }
        }
    }

    /// Turns the Agents pane, if one has been made, to the Team overview.
    pub fn select_team(&self, cx: &mut App) {
        for (model, _) in self.models.values() {
            if let PaneModel::Agents(pane) = model {
                pane.update(cx, |pane, cx| pane.select_team(cx));
            }
        }
    }

    pub fn set_browser(&mut self, tabs: wack_browser::ChatTabs) {
        self.browser = Some(tabs);
    }

    pub fn set_canvases(&mut self, canvases: wack_canvas::Canvases) {
        self.canvases = Some(canvases);
    }

    /// The chat's canvases, if this tree was given them.
    pub fn canvases(&self) -> Option<&wack_canvas::Canvases> {
        self.canvases.as_ref()
    }

    /// Redraws the Canvas pane: a canvas changed, or a picture was drawn.
    pub fn refresh_canvas(&self, cx: &mut App) {
        for (model, _) in self.models.values() {
            if let PaneModel::Canvas(pane) = model {
                pane.update(cx, |pane, cx| pane.refresh(cx));
            }
        }
    }

    /// Tells the Canvas pane what the model is drawing now (`None`: nothing).
    pub fn canvas_writing(&self, writing: Option<crate::conversation::Writing>, cx: &mut App) {
        for (model, _) in self.models.values() {
            if let PaneModel::Canvas(pane) = model {
                pane.update(cx, |pane, cx| pane.set_writing(writing.clone(), cx));
            }
        }
    }

    /// Shows a canvas in the Canvas pane, if one has been made.
    pub fn show_canvas(&self, canvas: &str, artboards: &[String], cx: &mut App) {
        for (model, _) in self.models.values() {
            if let PaneModel::Canvas(pane) = model {
                pane.update(cx, |pane, cx| pane.show(canvas, artboards, cx));
            }
        }
    }

    /// The tabs the Browser pane shows, if this tree was given any.
    pub fn browser(&self) -> Option<&wack_browser::ChatTabs> {
        self.browser.as_ref()
    }

    /// Redraws the Browser pane: a tab changed.
    pub fn refresh_browser(&self, cx: &mut App) {
        for (model, _) in self.models.values() {
            if let PaneModel::Browser(pane) = model {
                pane.update(cx, |pane, cx| pane.refresh(cx));
            }
        }
    }

    /// Shows an agent's work in the Agents pane, if one has been made.
    pub fn select_agent(&self, id: &str, cx: &mut App) {
        for (model, _) in self.models.values() {
            if let PaneModel::Agents(pane) = model {
                pane.update(cx, |pane, cx| pane.select(id.to_string(), cx));
            }
        }
    }

    /// Redraws the Agents pane: an agent took a step.
    pub fn refresh_agents(&self, cx: &mut App) {
        for (model, _) in self.models.values() {
            if let PaneModel::Agents(pane) = model {
                pane.update(cx, |_, cx| cx.notify());
            }
        }
    }

    /// Shows a task's output in the Tasks pane, if one has been made.
    pub fn select_task(&self, id: &str, cx: &mut App) {
        for (model, _) in self.models.values() {
            if let PaneModel::Tasks(pane) = model {
                pane.update(cx, |pane, cx| pane.select(id.to_string(), cx));
            }
        }
    }

    pub fn set_runner(&mut self, runner: Runner) {
        self.runner = runner;
    }

    pub fn set_commands(&mut self, build_command: Option<String>, run_command: Option<String>, cx: &mut App) {
        self.build_command = build_command.clone();
        self.run_command = run_command.clone();
        for (model, _) in self.models.values() {
            match model {
                PaneModel::Build(pane) => pane.update(cx, |pane, cx| pane.set_configured_command(build_command.clone(), cx)),
                PaneModel::Run(pane) => pane.update(cx, |pane, cx| pane.set_configured_command(run_command.clone(), cx)),
                _ => {}
            }
        }
    }

    pub fn folder(&self) -> &PathBuf {
        &self.folder
    }

    pub fn model(&self, id: PaneId) -> Option<&PaneModel> {
        self.models.get(&id).map(|(model, _)| model)
    }

    /// Makes models for panes now on screen and drops those of closed panes. `on_change` is how
    /// the owner hears that a pane redrew. Returns the panes that were just made.
    pub fn sync<T: 'static>(
        &mut self,
        cx: &mut gpui::Context<T>,
        on_change: impl Fn(&mut T, &mut gpui::Context<T>) + Clone + 'static,
    ) -> Vec<PaneId> {
        let present: Vec<PaneId> = self.layout.panes().iter().map(|(_, pane)| pane.id).collect();
        self.models.retain(|id, _| present.contains(id));
        let mut made = Vec::new();
        for pane in self.layout.visible() {
            if self.models.contains_key(&pane.id) {
                continue;
            }
            let folder = self.folder.clone();
            let model = match pane.kind {
                PaneKind::Changes => PaneModel::Changes(cx.new(|cx| ChangesPane::new(folder, cx))),
                PaneKind::Terminal => PaneModel::Terminal(cx.new(|cx| TerminalPane::new(Some(pane.id), folder, cx))),
                PaneKind::Build => {
                    let configured = self.build_command.clone();
                    let runner = self.runner.clone();
                    PaneModel::Build(cx.new(|cx| RunPane::new(CommandKind::Build, folder, &runner, configured, cx)))
                }
                PaneKind::Run => {
                    let configured = self.run_command.clone();
                    let runner = self.runner.clone();
                    PaneModel::Run(cx.new(|cx| RunPane::new(CommandKind::Run, folder, &runner, configured, cx)))
                }
                PaneKind::Tasks => {
                    let tasks = self.tasks.clone().unwrap_or_else(Tasks::scratch);
                    PaneModel::Tasks(cx.new(|cx| TasksPane::new(tasks, cx)))
                }
                PaneKind::Agents => {
                    let agents = self.agents.clone().unwrap_or_default();
                    let team = self.team.clone();
                    PaneModel::Agents(cx.new(|cx| {
                        let mut pane = AgentsPane::new(agents, folder, cx);
                        pane.set_team_source(team, cx);
                        pane
                    }))
                }
                PaneKind::Browser => {
                    let tabs = self.browser.clone().unwrap_or_else(|| wack_browser::ChatTabs::new(None));
                    PaneModel::Browser(cx.new(|cx| BrowserPane::new(tabs, cx)))
                }
                PaneKind::Canvas => {
                    // A new chat's screen has no canvases until its first message makes the chat.
                    let canvases = self.canvases.clone().unwrap_or_else(|| wack_canvas::Canvases::new(std::env::temp_dir().join("wackcode-no-canvas"), None));
                    PaneModel::Canvas(cx.new(|cx| CanvasPane::new(canvases, cx)))
                }
            };
            let subscription = match &model {
                PaneModel::Changes(entity) => {
                    let on_change = on_change.clone();
                    cx.observe(entity, move |owner, _, cx| on_change(owner, cx))
                }
                PaneModel::Terminal(entity) => {
                    let on_change = on_change.clone();
                    cx.observe(entity, move |owner, _, cx| on_change(owner, cx))
                }
                PaneModel::Build(entity) | PaneModel::Run(entity) => {
                    let on_change = on_change.clone();
                    cx.observe(entity, move |owner, _, cx| on_change(owner, cx))
                }
                PaneModel::Tasks(entity) => {
                    let on_change = on_change.clone();
                    cx.observe(entity, move |owner, _, cx| on_change(owner, cx))
                }
                PaneModel::Agents(entity) => {
                    let on_change = on_change.clone();
                    cx.observe(entity, move |owner, _, cx| on_change(owner, cx))
                }
                PaneModel::Browser(entity) => {
                    let on_change = on_change.clone();
                    cx.observe(entity, move |owner, _, cx| on_change(owner, cx))
                }
                PaneModel::Canvas(entity) => {
                    let on_change = on_change.clone();
                    cx.observe(entity, move |owner, _, cx| on_change(owner, cx))
                }
            };
            self.models.insert(pane.id, (model, subscription));
            made.push(pane.id);
        }
        made
    }

    pub fn sync_command_session(
        &self,
        kind: CommandKind,
        session: Option<CommandSession>,
        configured: Option<String>,
        cx: &mut App,
    ) {
        for (model, _) in self.models.values() {
            match (kind, model) {
                (CommandKind::Build, PaneModel::Build(pane)) | (CommandKind::Run, PaneModel::Run(pane)) => {
                    pane.update(cx, |pane, cx| pane.sync(session.clone(), configured.clone(), cx));
                }
                _ => {}
            }
        }
    }

    pub fn wake_command(&self, kind: CommandKind, cx: &mut App) {
        for pane in self.layout.visible() {
            match (kind, self.model(pane.id)) {
                (CommandKind::Build, Some(PaneModel::Build(model)))
                | (CommandKind::Run, Some(PaneModel::Run(model))) => {
                    model.update(cx, |model, cx| model.wake(cx));
                }
                _ => {}
            }
        }
    }

    /// Puts a model in for a pane, for gallery scenes.
    pub fn insert<T: 'static>(&mut self, id: PaneId, model: PaneModel, cx: &mut gpui::Context<T>) {
        let subscription = match &model {
            PaneModel::Changes(entity) => cx.observe(entity, |_, _, cx| cx.notify()),
            PaneModel::Terminal(entity) => cx.observe(entity, |_, _, cx| cx.notify()),
            PaneModel::Build(entity) | PaneModel::Run(entity) => cx.observe(entity, |_, _, cx| cx.notify()),
            PaneModel::Tasks(entity) => cx.observe(entity, |_, _, cx| cx.notify()),
            PaneModel::Agents(entity) => cx.observe(entity, |_, _, cx| cx.notify()),
            PaneModel::Browser(entity) => cx.observe(entity, |_, _, cx| cx.notify()),
            PaneModel::Canvas(entity) => cx.observe(entity, |_, _, cx| cx.notify()),
        };
        self.models.insert(id, (model, subscription));
    }

    /// Asks every Changes pane on screen to look again.
    pub fn refresh_changes(&self, cx: &mut App) {
        for pane in self.layout.visible() {
            if let Some(PaneModel::Changes(changes)) = self.model(pane.id) {
                changes.update(cx, |changes, cx| changes.refresh(cx));
            }
        }
    }

    /// Selects `path` in every Changes pane on screen if Git lists it (or once its next refresh lands).
    pub fn select_change(&self, path: &str, cx: &mut App) {
        for pane in self.layout.visible() {
            if let Some(PaneModel::Changes(changes)) = self.model(pane.id) {
                changes.update(cx, |changes, cx| changes.select_if_present(path, cx));
            }
        }
    }

    /// The words a pane's tab shows: a terminal's own title, numbered when untitled and not alone.
    pub fn title(&self, pane: Pane, cx: &App) -> SharedString {
        match (pane.kind, self.model(pane.id)) {
            (PaneKind::Terminal, Some(PaneModel::Terminal(terminal))) if terminal.read(cx).title().is_some() => {
                terminal.read(cx).title().unwrap_or_default()
            }
            (PaneKind::Terminal, _) => {
                let terminals: Vec<PaneId> =
                    self.layout.panes().iter().filter(|(_, other)| other.kind == PaneKind::Terminal).map(|(_, other)| other.id).collect();
                match terminals.iter().position(|id| *id == pane.id) {
                    Some(index) if terminals.len() > 1 => format!("Terminal {}", index + 1).into(),
                    _ => "Terminal".into(),
                }
            }
            (PaneKind::Changes, _) => "Changes".into(),
            (PaneKind::Build, _) => "Build".into(),
            (PaneKind::Run, _) => "Run".into(),
            (PaneKind::Tasks, _) => "Tasks".into(),
            (PaneKind::Agents, _) => "Agents".into(),
            (PaneKind::Browser, _) => "Browser".into(),
            (PaneKind::Canvas, _) => "Canvas".into(),
        }
    }

    /// A small count beside a tab's title: how many files changed.
    pub fn badge(&self, pane: Pane, cx: &App) -> Option<SharedString> {
        match self.model(pane.id) {
            Some(PaneModel::Changes(changes)) => changes.read(cx).count().filter(|count| *count > 0).map(|count| count.to_string().into()),
            Some(PaneModel::Tasks(tasks)) => Some(tasks.read(cx).running()).filter(|count| *count > 0).map(|count| count.to_string().into()),
            Some(PaneModel::Agents(agents)) => Some(agents.read(cx).working()).filter(|count| *count > 0).map(|count| count.to_string().into()),
            _ => None,
        }
    }

    /// Status indicator for Build/Run tabs.
    pub fn command_status(&self, pane: Pane, cx: &App) -> Option<CommandStatus> {
        match self.model(pane.id) {
            Some(PaneModel::Build(model)) | Some(PaneModel::Run(model)) => Some(model.read(cx).status()),
            _ => None,
        }
    }
}
