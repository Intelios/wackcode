//! Panes: a chat's Changes and Terminal views beside and below its transcript.
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

pub mod changes;
pub mod dock;
pub mod layout;
pub mod solve;
pub mod terminal;

use std::{collections::HashMap, path::PathBuf};

use gpui::{App, AppContext, Entity, FocusHandle, SharedString, Subscription};

pub use changes::ChangesPane;
pub use layout::{Axis, Dock, Edge, Layout, Node, Pane, PaneId, PaneKind, Stack, Target};
pub use solve::{Rect, Solution, solve};
pub use terminal::TerminalPane;

#[derive(Clone)]
pub enum PaneModel {
    Changes(Entity<ChangesPane>),
    Terminal(Entity<TerminalPane>),
}

impl PaneModel {
    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self {
            PaneModel::Changes(pane) => pane.read(cx).focus.clone(),
            PaneModel::Terminal(pane) => pane.read(cx).focus.clone(),
        }
    }
}

pub struct ChatPanes {
    pub layout: Layout,
    folder: PathBuf,
    models: HashMap<PaneId, (PaneModel, Subscription)>,
}

impl ChatPanes {
    pub fn new(layout: Layout, folder: PathBuf) -> Self {
        Self { layout, folder, models: HashMap::new() }
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
                PaneKind::Terminal => PaneModel::Terminal(cx.new(|cx| TerminalPane::new(folder, cx))),
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
            };
            self.models.insert(pane.id, (model, subscription));
            made.push(pane.id);
        }
        made
    }

    /// Puts a model in for a pane, for gallery scenes.
    pub fn insert<T: 'static>(&mut self, id: PaneId, model: PaneModel, cx: &mut gpui::Context<T>) {
        let subscription = match &model {
            PaneModel::Changes(entity) => cx.observe(entity, |_, _, cx| cx.notify()),
            PaneModel::Terminal(entity) => cx.observe(entity, |_, _, cx| cx.notify()),
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
        }
    }

    /// A small count beside a tab's title: how many files changed.
    pub fn badge(&self, pane: Pane, cx: &App) -> Option<SharedString> {
        match self.model(pane.id) {
            Some(PaneModel::Changes(changes)) => changes.read(cx).count().filter(|count| *count > 0).map(|count| count.to_string().into()),
            _ => None,
        }
    }
}
