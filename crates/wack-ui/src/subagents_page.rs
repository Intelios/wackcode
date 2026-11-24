//! Settings › Sub-agents: how freely the agent hands work to sub-agents, the model the no-edit
//! roles run on, the agents themselves, and other tools' agent folders. The Automatic titles card
//! lives here too: both are about the small helper requests a chat makes beside its own.
//!
//! - **Delegation**: when it helps (the model decides), only when the user asks, or off (no
//!   sub-agent tools at all).
//! - **Helper model**: roles whose file says `model: helper` (Explore and Review) run on it; with
//!   none they use the chat's model. A pick must be a model of a connection that is on.
//! - **Agents**: every role by the name it answers to. A built-in opens in the editor; saving it
//!   writes the user's own file, which then answers to the name ("Customised", with Restore). The
//!   user's own files are edited and trashed here; other tools' are read, never written.
//! - The list is read from disk when the section opens and after every change; the editor's
//!   fields live in [`AgentEditor`], owned by the app state, so leaving the page keeps them.

use std::path::PathBuf;

use gpui::{AnyElement, App, AppContext, Entity, SharedString, div, prelude::*, px};
use wack_core::{
    Connection,
    agents::{AgentDef, AgentDraft, AgentEntry, AgentsConfig, BUILTIN_LABEL, Delegation, ModelPref, Problem, TOOL_FOLDERS},
};

use crate::{
    actions::{
        AskDeleteAgent, CancelAgentDelete, ChooseAgentModel, ChooseDelegation, ChooseHelperModel, CloseAgent, ConfirmAgentDelete, DuplicateAgent,
        NewAgent, OpenAgent, RevealMemory, SaveAgent, SwitchAgent, SwitchAgentFolder, ToggleHelperModels,
    },
    button::{Button, Kind as ButtonKind},
    icon::Icon,
    settings::{field, text_area, text_input},
    settings_kit::{card, dispatch, head, pill, row, segmented},
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    titles_card::{TitleModel, TitlesData},
    tools_page::switch,
    tooltip::tip,
    typography::{Type, Typography},
};

/// The agent being written or edited.
pub struct AgentEditor {
    /// The name it was opened under; `None` for a new agent or a copy.
    pub previous: Option<String>,
    pub name: Entity<TextField>,
    pub description: Entity<TextField>,
    pub tools: Entity<TextField>,
    pub steps: Entity<TextField>,
    pub prompt: Entity<TextField>,
    pub model: ModelPref,
    /// The one opened is the user's own file.
    pub yours: bool,
    /// A built-in shares its name, so trashing the user's file restores it.
    pub over_builtin: bool,
    pub error: Option<SharedString>,
    pub confirming: bool,
}

impl AgentEditor {
    /// An editor on `agent` (a copy when `copy`), or a new one.
    pub fn new(agent: Option<&AgentDef>, copy: bool, over_builtin: bool, cx: &mut App) -> Self {
        let name = cx.new(|cx| TextField::new(Type::Mono, "docs-check", cx));
        let description = cx.new(|cx| TextField::new(Type::Ui, "Checks the docs match the code. Makes no edits.", cx).multiline(3));
        let tools = cx.new(|cx| TextField::new(Type::Mono, "Every tool", cx));
        let steps = cx.new(|cx| TextField::new(Type::Mono, "60", cx));
        let prompt = cx.new(|cx| TextField::new(Type::Mono, "You are Docs Check. Read the docs the task names, then the code they describe, and report every place they disagree with file:line.", cx).multiline(16));
        let mut model = ModelPref::Chat;
        if let Some(agent) = agent {
            let mut draft = AgentDraft::of(agent);
            if copy {
                draft.name = format!("{}-copy", draft.name);
            }
            name.update(cx, |field, cx| field.set_text(&draft.name, cx));
            description.update(cx, |field, cx| field.set_text_at_start(&draft.description, cx));
            tools.update(cx, |field, cx| field.set_text(&draft.tools, cx));
            steps.update(cx, |field, cx| field.set_text(&draft.max_turns, cx));
            prompt.update(cx, |field, cx| field.set_text_at_start(&draft.prompt, cx));
            model = draft.model;
        }
        let yours = agent.is_some_and(|agent| !agent.builtin()) && !copy;
        Self {
            previous: agent.filter(|_| !copy).map(|agent| agent.name.clone()),
            name,
            description,
            tools,
            steps,
            prompt,
            model,
            yours,
            over_builtin: over_builtin && !copy,
            error: None,
            confirming: false,
        }
    }

    pub fn fields(&self) -> [Entity<TextField>; 5] {
        [self.name.clone(), self.description.clone(), self.tools.clone(), self.steps.clone(), self.prompt.clone()]
    }

    pub fn draft(&self, cx: &App) -> AgentDraft {
        let text = |field: &Entity<TextField>| field.read(cx).text().to_string();
        AgentDraft {
            name: text(&self.name),
            description: text(&self.description),
            tools: text(&self.tools),
            model: self.model,
            max_turns: text(&self.steps),
            prompt: text(&self.prompt),
        }
    }

    pub fn issue(&self, cx: &App) -> Option<SharedString> {
        let draft = self.draft(cx);
        if draft.name.trim().is_empty() {
            return Some("Give the agent a name.".into());
        }
        if draft.description.trim().is_empty() {
            return Some("Say when to use it: the agent chooses by its description.".into());
        }
        if draft.prompt.trim().is_empty() {
            return Some("Write its instructions.".into());
        }
        draft.file_text(None).err().map(Into::into)
    }

    pub fn view(&self, cx: &App) -> EditorData {
        let draft = self.draft(cx);
        let probe = AgentDef {
            name: String::new(),
            description: String::new(),
            prompt: String::new(),
            tools: draft.tool_list(),
            model: draft.model,
            max_turns: 0,
            file: None,
            source: String::new(),
        };
        let access: SharedString = match (probe.tools.is_none(), probe.edits()) {
            (true, _) => "Every tool, edits included. Plan mode won't start it.".into(),
            (false, true) => "It can edit files. Plan mode won't start it.".into(),
            (false, false) => "It makes no edits: Plan mode can start it. Its shell still runs builds and tests.".into(),
        };
        EditorData {
            new: self.previous.is_none(),
            title: match (&self.previous, self.yours) {
                (None, _) => "New agent".into(),
                (Some(name), false) => format!("Customise {name}").into(),
                (Some(_), true) => "Edit agent".into(),
            },
            name: self.name.clone(),
            description: self.description.clone(),
            tools: self.tools.clone(),
            steps: self.steps.clone(),
            prompt: self.prompt.clone(),
            model: self.model,
            access,
            restore: self.yours && self.over_builtin,
            delete: self.yours && !self.over_builtin,
            issue: self.error.clone().or_else(|| self.issue(cx)),
            error: self.error.is_some(),
            confirming: self.confirming,
        }
    }
}

#[derive(Clone)]
pub struct EditorData {
    pub new: bool,
    pub title: SharedString,
    pub name: Entity<TextField>,
    pub description: Entity<TextField>,
    pub tools: Entity<TextField>,
    pub steps: Entity<TextField>,
    pub prompt: Entity<TextField>,
    pub model: ModelPref,
    /// What its tools let it do, as the user is choosing them.
    pub access: SharedString,
    /// Trashing the user's file brings back the built-in.
    pub restore: bool,
    pub delete: bool,
    pub issue: Option<SharedString>,
    pub error: bool,
    pub confirming: bool,
}

/// Settings › Sub-agents' state, owned by the app so leaving the page keeps it.
#[derive(Default)]
pub struct SubagentsState {
    pub entries: Vec<AgentEntry>,
    pub problems: Vec<Problem>,
    /// Where the user's agents live.
    pub folder: PathBuf,
    pub editor: Option<AgentEditor>,
    pub error: Option<SharedString>,
    pub helper_open: bool,
}

impl SubagentsState {
    pub fn data(&self, config: &AgentsConfig, connections: &[Connection], titles: TitlesData, cx: &App) -> SubagentsData {
        let models: Vec<TitleModel> = connections
            .iter()
            .filter(|connection| connection.enabled)
            .flat_map(|connection| {
                connection.models.iter().map(|model| TitleModel {
                    connection: connection.id.clone(),
                    connection_name: connection.name.clone().into(),
                    model: model.id.clone().into(),
                })
            })
            .collect();
        let picked = config.helper.as_ref().and_then(|choice| choice.connection.clone().zip(choice.model.clone()));
        let helper = picked.as_ref().and_then(|(connection, model)| {
            models.iter().find(|listed| &listed.connection == connection && listed.model.as_ref() == model).map(|listed| listed.model.clone())
        });
        let helper_gone = picked.is_some() && helper.is_none();
        SubagentsData {
            delegation: config.delegation,
            helper,
            helper_gone,
            picked,
            models,
            helper_open: self.helper_open,
            titles,
            entries: self.entries.clone(),
            problems: self.problems.clone(),
            folder: self.folder.clone(),
            folders: TOOL_FOLDERS.iter().map(|(id, label, path)| (id.to_string(), *label, format!("~/{path}"), config.tool_folders.iter().any(|on| on == id))).collect(),
            editor: self.editor.as_ref().map(|editor| editor.view(cx)),
            error: self.error.clone(),
        }
    }
}

#[derive(Clone, Default)]
pub struct SubagentsData {
    pub delegation: Delegation,
    /// The helper model's id, when it is still listed.
    pub helper: Option<SharedString>,
    /// A helper was chosen whose connection or model is gone: helper roles use the chat's model.
    pub helper_gone: bool,
    pub picked: Option<(String, String)>,
    pub models: Vec<TitleModel>,
    pub helper_open: bool,
    pub titles: TitlesData,
    pub entries: Vec<AgentEntry>,
    pub problems: Vec<Problem>,
    pub folder: PathBuf,
    /// Other tools' folders: id, name, where, on.
    pub folders: Vec<(String, &'static str, String, bool)>,
    pub editor: Option<EditorData>,
    pub error: Option<SharedString>,
}

pub fn page(data: SubagentsData, theme: &Theme) -> AnyElement {
    if let Some(editor) = &data.editor {
        return editor_page(editor, theme).into_any_element();
    }
    let new = pill("agent-new", "New agent", Some(Icon::Plus), true, theme).on_click(dispatch(NewAgent)).tag("agent-new");
    let mut page = div().flex().flex_col().gap(px(18.)).child(head(
        "Sub-agents",
        "Agents the agent can hand work to. Each works in its own context and reports back, so the chat keeps the conclusion, not the search. Chat mode chats work alone and never hand work off.",
        Some(new.into_any_element()),
        theme,
    ));
    if let Some(error) = data.error.clone() {
        page = page.child(div().typography(Type::Ui).text_color(theme.danger).child(error));
    }
    page.child(delegation_card(&data, theme))
        .child(agents_card(&data, theme))
        .child(folders_card(&data, theme))
        .child(crate::titles_card::card_view(data.titles, theme))
        .into_any_element()
}

fn delegation_card(data: &SubagentsData, theme: &Theme) -> impl IntoElement {
    let about = match data.delegation {
        Delegation::Auto => "The agent sends one when it pays: wide searches, side tasks, a second look at a change.",
        Delegation::OnRequest => "The agent sends one only when you ask it to.",
        Delegation::Off => "No sub-agents: the agent does all the work in the chat itself.",
    };
    let choices = [(Delegation::Auto, "When it helps"), (Delegation::OnRequest, "Only when I ask"), (Delegation::Off, "Off")];
    let picker = pill(
        "helper-model",
        data.helper.clone().unwrap_or_else(|| "The chat’s model".into()),
        Some(if data.helper_open { Icon::ChevronDown } else { Icon::ChevronRight }),
        false,
        theme,
    )
    .max_w(px(260.))
    .on_click(dispatch(ToggleHelperModels))
    .tag("helper-model");
    let helper_about = if data.helper_gone {
        "The model chosen is gone, so these use the chat’s model. Choose another."
    } else {
        "Explore and Review run on it. A small, fast model is plenty; with none they use the chat’s."
    };
    card(None, None, None, theme)
        .child(
            row("Delegation", Some(about))
                .first()
                .control(div().child(segmented("delegation", &choices, data.delegation, |delegation| Box::new(ChooseDelegation { delegation }), theme)).tag("delegation"))
                .view(theme),
        )
        .child(row("Helper model", Some(helper_about)).control(picker).view(theme))
        .when(data.helper_open, |card| card.child(helper_list(data, theme)))
}

/// Tags for the automation socket, which takes only fixed names.
const HELPER_TAGS: [&str; 6] = ["helper-model-0", "helper-model-1", "helper-model-2", "helper-model-3", "helper-model-4", "helper-model-5"];

fn helper_list(data: &SubagentsData, theme: &Theme) -> impl IntoElement {
    let item = |id: (&'static str, usize), label: SharedString, ticked: bool, action: ChooseHelperModel, tag: &'static str| {
        div()
            .id(id)
            .px(px(10.))
            .h(px(30.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .hover(|row| row.bg(theme.hover))
            .on_click(dispatch(action))
            .child(div().flex_1().min_w_0().truncate().typography(Type::Ui).text_color(theme.text).child(label))
            .when(ticked, |row| row.child(Icon::Check.view(px(14.), theme.accent_text)))
            .tag(tag)
    };
    let mut list = div()
        .mx(px(18.))
        .mb(px(14.))
        .p(px(6.))
        .rounded(px(12.))
        .bg(theme.window)
        .border_1()
        .border_color(theme.border)
        .flex()
        .flex_col()
        .child(item(("helper-model-row", usize::MAX), "The chat’s model".into(), data.picked.is_none(), ChooseHelperModel { connection: None, model: None }, "helper-model-chat"));
    let mut last: Option<&str> = None;
    for (index, listed) in data.models.iter().enumerate() {
        if last != Some(listed.connection.as_str()) {
            last = Some(listed.connection.as_str());
            list = list.child(div().px(px(10.)).pt(px(10.)).pb(px(4.)).typography(Type::Label).text_color(theme.text_faint).child(listed.connection_name.clone()));
        }
        let ticked = data.picked.as_ref().is_some_and(|(connection, model)| connection == &listed.connection && model == listed.model.as_ref());
        let action = ChooseHelperModel { connection: Some(listed.connection.clone()), model: Some(listed.model.to_string()) };
        list = list.child(item(("helper-model-row", index), listed.model.clone(), ticked, action, HELPER_TAGS.get(index).copied().unwrap_or("helper-model-more")));
    }
    list
}

fn agents_card(data: &SubagentsData, theme: &Theme) -> impl IntoElement {
    let on = data.entries.iter().filter(|entry| entry.enabled).count();
    let mut agents = card(Some("Agents"), Some(format!("{on} on").into()), Some("Off keeps one from being sent"), theme);
    for entry in &data.entries {
        agents = agents.child(agent_row(entry, theme));
    }
    for problem in &data.problems {
        agents = agents.child(
            div()
                .px(px(18.))
                .py(px(10.))
                .border_t_1()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().truncate().typography(Type::UiStrong).text_color(theme.danger).child(problem.file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()))
                .child(div().typography(Type::Small).text_color(theme.text_dim).child(problem.message.clone()))
                .tag("agent-problem"),
        );
    }
    agents.child(
        div()
            .px(px(18.))
            .py(px(10.))
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().truncate().typography(Type::Small).text_color(theme.text_faint).child(shown(&data.folder)))
            .child(
                div()
                    .id("agents-reveal")
                    .flex_none()
                    .size(px(28.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|button| button.bg(theme.hover))
                    .tooltip(tip("Show the folder in Finder"))
                    .on_click(dispatch(RevealMemory { path: data.folder.clone() }))
                    .child(Icon::Folder.view(px(14.), theme.text_dim)),
            ),
    )
}

/// A small word beside an agent's name.
fn badge(text: impl Into<SharedString>, theme: &Theme) -> impl IntoElement {
    div().flex_none().h(px(18.)).px(px(7.)).rounded_full().flex().items_center().bg(theme.hover).typography(Type::Small).text_color(theme.text_dim).child(text.into())
}

fn agent_row(entry: &AgentEntry, theme: &Theme) -> impl IntoElement {
    let agent = &entry.agent;
    let id = SharedString::from(format!("agent-{}", agent.name));
    let source: SharedString = if entry.customised {
        "Customised".into()
    } else if agent.builtin() {
        BUILTIN_LABEL.into()
    } else if entry.yours() {
        "Yours".into()
    } else {
        agent.source.clone().into()
    };
    // Other tools' files are theirs to change.
    let opens = agent.builtin() || entry.yours();
    let (name, on) = (agent.name.clone(), entry.enabled);
    div()
        .id(id.clone())
        .px(px(18.))
        .py(px(11.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(14.))
        .when(opens, |row| row.cursor_pointer().hover(|row| row.bg(theme.hover)).on_click(dispatch(OpenAgent { name: name.clone() })))
        .when(!opens, |row| row.tooltip(tip(format!("Edit it where {} keeps it", agent.source))))
        .child(Icon::Duck.view(px(14.), theme.text_dim))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .when(!on, |text| text.opacity(0.6))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(div().min_w_0().truncate().typography(Type::UiStrong).text_color(theme.text).child(agent.name.clone()))
                        .child(badge(source, theme))
                        .child(badge(if agent.edits() { "Edits" } else { "No edits" }, theme))
                        .when(agent.model == ModelPref::Helper, |line| line.child(badge("Helper model", theme))),
                )
                .child(div().typography(Type::Small).text_color(theme.text_dim).child(agent.description.clone())),
        )
        .child(
            div()
                .id(SharedString::from(format!("{id}-switch")))
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(Box::new(SwitchAgent { name: name.clone(), on: !on }), cx);
                })
                .child(switch(on, theme))
                .tag("agent-switch"),
        )
        .tag("agent-row")
}

fn folders_card(data: &SubagentsData, theme: &Theme) -> impl IntoElement {
    let mut folders = card(Some("Agents from other tools"), None, Some("Read, never changed"), theme);
    for (index, (id, label, path, on)) in data.folders.iter().enumerate() {
        let (id, on) = (id.clone(), *on);
        let control = div()
            .id(("agent-folder", index))
            .cursor_pointer()
            .on_click(move |_, window, cx| window.dispatch_action(Box::new(SwitchAgentFolder { id: id.clone(), on: !on }), cx))
            .child(switch(on, theme))
            .tag("agent-folder");
        folders = folders.child(
            div()
                .px(px(18.))
                .py(px(13.))
                .border_t_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .gap(px(18.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().typography(Type::UiStrong).text_color(theme.text).child(*label))
                        .child(div().typography(Type::Small).text_color(theme.text_dim).child(format!("{path}. Your own agent of the same name wins."))),
                )
                .child(control),
        );
    }
    folders
}

fn shown(path: &std::path::Path) -> String {
    let path = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path,
    }
}

fn editor_page(editor: &EditorData, theme: &Theme) -> impl IntoElement {
    let section = |content: AnyElement| div().px(px(18.)).py(px(13.)).border_t_1().border_color(theme.border).child(content);
    let back = div()
        .id("back-agents")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .pb(px(18.))
        .cursor_pointer()
        .typography(Type::Ui)
        .text_color(theme.text_dim)
        .hover(|link| link.text_color(theme.text))
        .on_click(dispatch(CloseAgent))
        .child(Icon::ChevronRight.view(px(13.), theme.text_faint).with_transformation(gpui::Transformation::rotate(gpui::radians(std::f32::consts::PI))))
        .child("All agents")
        .tag("back-agents");
    let status: AnyElement = match &editor.issue {
        Some(issue) => div().flex_1().typography(Type::Small).text_color(if editor.error { theme.danger } else { theme.text_dim }).child(issue.clone()).into_any_element(),
        None => div().flex_1().into_any_element(),
    };
    let mut bar = div().px(px(18.)).py(px(12.)).border_t_1().border_color(theme.border).flex().items_center().gap(px(8.)).child(status);
    if editor.restore {
        bar = bar.child(Button::new("agent-restore", "Restore built-in", ButtonKind::Secondary).action(AskDeleteAgent).tag("agent-restore"));
    }
    if editor.delete {
        bar = bar.child(Button::new("agent-delete", "Delete", ButtonKind::Secondary).action(AskDeleteAgent).tag("agent-delete"));
    }
    if !editor.new {
        bar = bar.child(Button::new("agent-duplicate", "Duplicate", ButtonKind::Secondary).action(DuplicateAgent).tag("agent-duplicate"));
    }
    bar = bar.child(Button::new("agent-cancel", "Cancel", ButtonKind::Secondary).action(CloseAgent).tag("agent-cancel")).child(
        Button::new("agent-save", if editor.new { "Create agent" } else { "Save" }, ButtonKind::Primary)
            .action(SaveAgent)
            .enabled(editor.issue.is_none() || editor.error)
            .tag("agent-save"),
    );
    let models = [(ModelPref::Helper, "Helper model"), (ModelPref::Chat, "Chat’s model")];
    let mut form = card(None, None, None, theme)
        .child(
            div()
                .px(px(18.))
                .py(px(13.))
                .child(field("Name", Some("How the agent asks for it. Lowercase words joined by hyphens.".into()), text_input(editor.name.clone(), "agent-name", theme), theme)),
        )
        .child(section(
            field("When to use it", Some("The agent reads this to choose. Say what it is good for, and what it won’t do.".into()), text_area(editor.description.clone(), 44., theme).tag("agent-description"), theme)
                .into_any_element(),
        ))
        .child(section(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(field(
                    "Tools",
                    Some("Names separated by commas: read, grep, find, ls, bash, edit, write, web_fetch… mcp__* takes every MCP tool. Empty is every tool.".into()),
                    text_input(editor.tools.clone(), "agent-tools", theme),
                    theme,
                ))
                .child(div().flex().items_center().gap(px(6.)).child(Icon::Duck.view(px(12.), theme.text_faint)).child(div().typography(Type::Small).text_color(theme.text_soft).child(editor.access.clone())).tag("agent-access"))
                .into_any_element(),
        ))
        .child(
            row("Model", Some("The helper model is chosen on the Sub-agents page; the chat’s is whatever the chat runs on."))
                .control(div().child(segmented("agent-model", &models, editor.model, |model| Box::new(ChooseAgentModel { helper: model == ModelPref::Helper }), theme)).tag("agent-model"))
                .view(theme),
        )
        .child(section(
            field("Steps", Some("Model requests it may make in one round before it must report. Empty is 60.".into()), div().w(px(120.)).child(text_input(editor.steps.clone(), "agent-steps", theme)), theme).into_any_element(),
        ))
        .child(section(
            field(
                "Instructions",
                Some("Who it is and how it works. Say what its report should hold: the agent that sent it acts on the report alone.".into()),
                text_area(editor.prompt.clone(), 160., theme).tag("agent-prompt"),
                theme,
            )
            .into_any_element(),
        ));
    if editor.confirming {
        let (question, words) = if editor.restore {
            ("Restore the built-in?", "Your version moves to the Trash, and WackCode’s answers to the name again.")
        } else {
            ("Delete this agent?", "Its file moves to the Trash. You can put it back from the Trash.")
        };
        form = form.child(
            div()
                .mx(px(18.))
                .my(px(12.))
                .p(px(14.))
                .rounded(px(12.))
                .bg(theme.window)
                .border_1()
                .border_color(theme.border_strong)
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(div().typography(Type::UiStrong).text_color(theme.text).child(question))
                .child(div().typography(Type::Ui).text_color(theme.text_dim).child(words))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(Button::new("agent-keep", "Keep", ButtonKind::Secondary).action(CancelAgentDelete))
                        .child(Button::new("agent-trash", if editor.restore { "Restore" } else { "Move to Trash" }, ButtonKind::Danger).action(ConfirmAgentDelete).tag("agent-trash")),
                ),
        );
    }
    div()
        .flex()
        .flex_col()
        .child(back)
        .child(head_owned(editor.title.clone(), theme))
        .child(form.child(bar))
}

/// The editor's head, whose title names the agent.
fn head_owned(title: SharedString, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .pb(px(22.))
        .child(div().typography(Type::Title).text_color(theme.text).child(title))
        .child(div().max_w(px(540.)).typography(Type::Body).text_color(theme.text_soft).child("A role the agent can hand work to: when to use it, what it may use, and how it works."))
}
