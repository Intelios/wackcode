//! Settings › MCP servers, after version 2's section: the servers the user added, what each
//! offers, and the projects each reaches.
//!
//! - The overview's head says how many servers are on and offers New server. With none yet, two
//!   tiles start one: "A command on your Mac" or "A server at a URL".
//! - Each server is a row: its type, name and what starts or reaches it, how it stands (from the
//!   app's MCP hub, refreshed every few seconds while the page shows, or the last Test
//!   connection), the projects it reaches, and its switch. A row opens to its details, its tools
//!   (each with a switch, and a Read-only badge where the server marks one), and Test connection,
//!   Edit and Delete (which asks first).
//! - The editor replaces the overview: name, type, the command and its arguments and variables or
//!   the URL and its headers, the timeout, and which projects it reaches. Test connection runs the
//!   draft as it stands; saving tests it again in the background so its tools are listed.
//!
//! A header or variable value is typed into a masked field and handed to the store once. Editing
//! a saved server leaves the value fields empty: blank keeps the saved value, and nothing ever
//! reads one back (docs/security.md). The editor's fields live in [`McpState`], owned by the app,
//! so leaving the page keeps what was typed.

use std::collections::HashMap;

use gpui::{AnyElement, App, AppContext, Entity, SharedString, div, prelude::*, px};
use wack_core::mcp::{DEFAULT_TIMEOUT_MS, McpDraft, McpServer, McpTransport as Transport, Projects, SecretEntry};

use crate::{
    actions::{
        AddMcpEntry, AddMcpServer, AskDeleteMcpServer, CancelDeleteMcpServer, ChooseMcpScope, ChooseMcpTransport, CloseMcpEditor, ConfirmDeleteMcpServer, EditMcpServer,
        ExpandMcpServer, RemoveMcpEntry, SaveMcpServer, TestMcpDraft, TestMcpServer, ToggleMcpProject, ToggleMcpServer, ToggleMcpTool,
    },
    button::{Button, Kind as ButtonKind},
    icon::Icon,
    settings::{field, text_area, text_input},
    settings_kit::{card, dispatch, head, pill, segmented, staged_head},
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    tools_page::switch,
    typography::{Type, Typography},
};

/// What a Test connection found, or why it failed.
#[derive(Clone, Debug, PartialEq)]
pub enum ProbeOutcome {
    Found { elapsed_ms: u64, tools: Vec<(SharedString, bool)> },
    Failed(SharedString),
}

/// A row's last Test connection.
#[derive(Clone, Debug, PartialEq)]
pub enum RowTest {
    Testing,
    Passed(usize),
    Failed(SharedString),
}

/// How a server's connections stand, from the hub.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Live {
    /// Nothing running: it starts when a chat in a project it reaches uses it.
    #[default]
    Idle,
    Connecting,
    /// Connected in this many project folders, offering this many tools.
    Connected { tools: usize, folders: usize },
    Failed(SharedString),
}

/// One header or variable in the editor.
pub struct EntryField {
    pub name: Entity<TextField>,
    pub value: Entity<TextField>,
    /// The name its value was saved under, when it has one.
    pub saved: Option<String>,
}

impl EntryField {
    fn new(name: &str, saved: Option<String>, cx: &mut App) -> Self {
        let field = cx.new(|cx| TextField::new(Type::Mono, "NAME", cx));
        field.update(cx, |field, cx| field.set_text(name, cx));
        // A saved value is never shown: its field starts empty and says so.
        let placeholder = if saved.is_some() { "Saved · leave blank to keep" } else { "Value" };
        Self { name: field, value: cx.new(|cx| TextField::new(Type::Ui, placeholder, cx).masked()), saved }
    }
}

/// The server being added or edited, kept while the user looks elsewhere.
pub struct McpEditor {
    /// The saved server being edited; `None` while adding one.
    pub editing: Option<String>,
    pub transport: Transport,
    pub name: Entity<TextField>,
    pub command: Entity<TextField>,
    pub args: Entity<TextField>,
    pub url: Entity<TextField>,
    pub timeout: Entity<TextField>,
    pub env: Vec<EntryField>,
    pub headers: Vec<EntryField>,
    pub projects: Projects,
    pub error: Option<SharedString>,
    pub testing: bool,
    pub test: Option<ProbeOutcome>,
    /// Tells one Test connection's answer from a later one's.
    pub test_serial: u64,
}

impl McpEditor {
    /// `saved` names the values already in the secret store.
    pub fn new(draft: &McpDraft, saved: &[String], cx: &mut App) -> Self {
        let text = |kind: Type, placeholder: &'static str, value: &str, cx: &mut App| {
            let field = cx.new(|cx| TextField::new(kind, placeholder, cx));
            field.update(cx, |field, cx| field.set_text(value, cx));
            field
        };
        let args = cx.new(|cx| TextField::new(Type::Mono, "-y\n@modelcontextprotocol/server-filesystem\n/Users/me/Documents", cx).multiline(6));
        args.update(cx, |field, cx| field.set_text(&draft.args.join("\n"), cx));
        let entries = |list: &[SecretEntry], cx: &mut App| -> Vec<EntryField> {
            list.iter().map(|entry| EntryField::new(&entry.name, saved.contains(&entry.name).then(|| entry.name.clone()), cx)).collect()
        };
        Self {
            editing: draft.id.clone(),
            transport: draft.transport,
            name: text(Type::Ui, "GitHub", &draft.name, cx),
            command: text(Type::Mono, "npx", &draft.command, cx),
            args,
            url: text(Type::Mono, "https://example.com/mcp", &draft.url, cx),
            timeout: text(Type::Ui, "120", &seconds(draft.timeout_ms), cx),
            env: entries(&draft.env, cx),
            headers: entries(&draft.headers, cx),
            projects: draft.projects.clone(),
            error: None,
            testing: false,
            test: None,
            test_serial: 0,
        }
    }

    /// Every field, for the app to watch as they change.
    pub fn fields(&self) -> Vec<Entity<TextField>> {
        let mut fields = vec![self.name.clone(), self.command.clone(), self.args.clone(), self.url.clone(), self.timeout.clone()];
        for entry in self.env.iter().chain(&self.headers) {
            fields.push(entry.name.clone());
            fields.push(entry.value.clone());
        }
        fields
    }

    /// The entries this server's type uses: variables for a command, headers for a URL.
    pub fn entries_mut(&mut self) -> &mut Vec<EntryField> {
        if self.transport == Transport::Stdio { &mut self.env } else { &mut self.headers }
    }

    pub fn add_entry(&mut self, cx: &mut App) -> Entity<TextField> {
        let entry = EntryField::new("", None, cx);
        let name = entry.name.clone();
        self.entries_mut().push(entry);
        name
    }

    /// What the editor describes, or the first thing to fix. The store checks again on save.
    pub fn draft(&self, cx: &App) -> Result<McpDraft, String> {
        let text = |field: &Entity<TextField>| field.read(cx).text().trim().to_string();
        let timeout = text(&self.timeout);
        let timeout_ms = if timeout.is_empty() {
            DEFAULT_TIMEOUT_MS
        } else {
            let seconds: f64 = timeout.trim_end_matches('s').trim().parse().map_err(|_| "Enter the timeout in seconds, such as 120.".to_string())?;
            if !seconds.is_finite() || seconds < 0. {
                return Err("Enter the timeout in seconds, such as 120.".into());
            }
            (seconds * 1000.).round() as u64
        };
        let entries = |list: &[EntryField]| {
            list.iter()
                .map(|entry| SecretEntry { name: text(&entry.name), value: Some(entry.value.read(cx).text().to_string()).filter(|value| !value.trim().is_empty()) })
                .collect()
        };
        Ok(McpDraft {
            id: self.editing.clone(),
            name: text(&self.name),
            transport: self.transport,
            timeout_ms,
            command: text(&self.command),
            args: self.args.read(cx).text().lines().map(str::trim).filter(|arg| !arg.is_empty()).map(str::to_string).collect(),
            url: text(&self.url),
            headers: entries(&self.headers),
            env: entries(&self.env),
            projects: self.projects.clone(),
        })
    }
}

fn seconds(ms: u64) -> String {
    if ms.is_multiple_of(1000) { (ms / 1000).to_string() } else { format!("{}", ms as f64 / 1000.) }
}

/// Settings › MCP servers' state, owned by the app so leaving the page keeps it.
#[derive(Default)]
pub struct McpState {
    pub editor: Option<McpEditor>,
    /// The row opened to its details and tools.
    pub expanded: Option<String>,
    /// The server whose Delete is waiting for a yes.
    pub confirming_delete: Option<String>,
    /// Each server's last Test connection, by id.
    pub tests: HashMap<String, RowTest>,
    /// A failure from the overview: a switch.
    pub error: Option<SharedString>,
}

/// One header or variable as the editor draws it.
#[derive(Clone)]
pub struct EntryData {
    pub name: Entity<TextField>,
    pub value: Entity<TextField>,
}

/// The editor as the page draws it.
#[derive(Clone)]
pub struct EditorData {
    /// The saved server being edited; `None` for a new one.
    pub id: Option<String>,
    pub new: bool,
    pub transport: Transport,
    pub name: Entity<TextField>,
    pub command: Entity<TextField>,
    pub args: Entity<TextField>,
    pub url: Entity<TextField>,
    pub timeout: Entity<TextField>,
    pub entries: Vec<EntryData>,
    pub projects: Projects,
    /// Every project, by id and name, to choose from.
    pub all_projects: Vec<(String, SharedString)>,
    pub error: Option<SharedString>,
    pub testing: bool,
    pub test: Option<ProbeOutcome>,
    pub confirming_delete: bool,
}

impl EditorData {
    pub fn of(editor: &McpEditor, all_projects: Vec<(String, SharedString)>, confirming_delete: bool) -> Self {
        let list = if editor.transport == Transport::Stdio { &editor.env } else { &editor.headers };
        Self {
            id: editor.editing.clone(),
            new: editor.editing.is_none(),
            transport: editor.transport,
            name: editor.name.clone(),
            command: editor.command.clone(),
            args: editor.args.clone(),
            url: editor.url.clone(),
            timeout: editor.timeout.clone(),
            entries: list
                .iter()
                .map(|entry| EntryData { name: entry.name.clone(), value: entry.value.clone() })
                .collect(),
            projects: editor.projects.clone(),
            all_projects,
            error: editor.error.clone(),
            testing: editor.testing,
            test: editor.test.clone(),
            confirming_delete,
        }
    }
}

/// One server as the overview lists it.
#[derive(Clone)]
pub struct ServerRow {
    pub server: McpServer,
    pub live: Live,
    /// "All projects", or the names of the ones it reaches.
    pub reach: SharedString,
    pub test: Option<RowTest>,
}

/// Everything the page shows.
#[derive(Clone, Default)]
pub struct McpData {
    pub servers: Vec<ServerRow>,
    pub expanded: Option<String>,
    pub confirming_delete: Option<String>,
    pub editor: Option<EditorData>,
    pub error: Option<SharedString>,
}

/// Where a server reaches, in words.
pub fn reach(projects: &Projects, names: &[(String, SharedString)]) -> SharedString {
    match projects {
        Projects::All => "All projects".into(),
        Projects::Only(ids) => {
            let chosen: Vec<&str> = ids.iter().filter_map(|id| names.iter().find(|(project, _)| project == id)).map(|(_, name)| name.as_ref()).collect();
            if chosen.is_empty() { "No projects".into() } else { chosen.join(", ").into() }
        }
    }
}

pub fn page(data: McpData, theme: &Theme) -> AnyElement {
    if let Some(editor) = &data.editor {
        return editor_page(editor, theme).into_any_element();
    }
    let new = pill("mcp-new", "New server", Some(Icon::Plus), true, theme).on_click(dispatch(AddMcpServer { transport: Transport::Stdio })).tag("mcp-new").into_any_element();
    div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .child(staged_head(crate::settings::Section::Mcp, 
            "MCP servers",
            "Tools from servers that speak the Model Context Protocol. The agent finds them when it needs them. Changes apply from a chat’s next message.",
            Some(new),
            theme,
        ))
        .when(data.servers.is_empty(), |page| page.child(start_tiles(theme)))
        .when(!data.servers.is_empty(), |page| page.child(server_list(&data, theme)))
        .children(data.error.clone().map(|error| div().typography(Type::Small).text_color(theme.danger).child(error)))
        .into_any_element()
}

/// The two ways to start: a server's README gives one or the other.
fn start_tiles(theme: &Theme) -> impl IntoElement {
    let tile = |id: &'static str, icon: Icon, title: &'static str, about: &'static str, transport: Transport| {
        div()
            .id(id)
            .flex_1()
            .min_w_0()
            .p(px(18.))
            .rounded(px(16.))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(10.))
            .cursor_pointer()
            .hover(|tile| tile.border_color(theme.accent).bg(theme.accent_wash))
            .on_click(dispatch(AddMcpServer { transport }))
            .child(div().size(px(34.)).rounded_full().bg(theme.hover).flex().items_center().justify_center().child(icon.view(px(16.), theme.text_soft)))
            .child(div().typography(Type::UiStrong).text_color(theme.text).child(title))
            .child(div().typography(Type::Small).text_color(theme.text_dim).child(about))
            .tag(id)
    };
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().typography(Type::Label).text_color(theme.text_dim).child("No servers yet"))
                .child(div().typography(Type::Small).text_color(theme.text_faint).child("A server’s README usually gives a command to run or a URL to connect to. Start with whichever yours has.")),
        )
        .child(
            div()
                .flex()
                .gap(px(12.))
                .child(tile("mcp-start-command", Icon::Terminal, "A command on your Mac", "Runs a program such as npx, uvx or docker in the chat’s project folder, with your permissions.", Transport::Stdio))
                .child(tile("mcp-start-url", Icon::Link, "A server at a URL", "Connects to a server over Streamable HTTP or SSE, with any headers it needs to sign you in.", Transport::Http)),
        )
}

fn server_list(data: &McpData, theme: &Theme) -> impl IntoElement {
    let on = data.servers.iter().filter(|row| row.server.enabled).count();
    let badge: SharedString = format!("{on} of {} on", data.servers.len()).into();
    let mut list = card(Some("Servers"), Some(badge), Some("Off keeps a server and its values, out of every chat."), theme);
    for (index, row) in data.servers.iter().enumerate() {
        let expanded = data.expanded.as_deref() == Some(row.server.id.as_str());
        let confirming = data.confirming_delete.as_deref() == Some(row.server.id.as_str());
        list = list.child(server_row(row, index == 0, expanded, confirming, theme));
    }
    list
}

/// The words for how a server stands: a Test connection's answer while it is fresh, else the hub's.
fn standing(row: &ServerRow) -> (SharedString, Tone) {
    if !row.server.enabled {
        return ("Off".into(), Tone::Quiet);
    }
    // A connection in use says most; then the user's own last test; then what the hub saw.
    match (&row.test, &row.live) {
        (Some(RowTest::Testing), _) => ("Testing the connection…".into(), Tone::Quiet),
        (_, Live::Connected { tools, folders }) => {
            let projects = if *folders > 1 { format!(" in {folders} projects") } else { String::new() };
            (format!("Connected{projects} · {}", plural(*tools, "tool", "tools")).into(), Tone::Good)
        }
        (Some(RowTest::Passed(tools)), _) => (format!("Connection works · {}", plural(*tools, "tool", "tools")).into(), Tone::Good),
        (Some(RowTest::Failed(error)), _) => (error.clone(), Tone::Bad),
        (None, Live::Connecting) => ("Connecting…".into(), Tone::Quiet),
        (None, Live::Failed(error)) => (error.clone(), Tone::Bad),
        (None, Live::Idle) => ("Starts when a chat uses it".into(), Tone::Quiet),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Quiet,
    Good,
    Bad,
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// What starts or reaches a server, in one line.
fn summary(server: &McpServer) -> String {
    match server.transport {
        Transport::Stdio => std::iter::once(server.command.as_str()).chain(server.args.iter().map(String::as_str)).collect::<Vec<_>>().join(" "),
        _ => server.url.clone(),
    }
}

fn badge(text: impl Into<SharedString>, theme: &Theme) -> impl IntoElement {
    div().flex_none().h(px(18.)).px(px(7.)).rounded_full().flex().items_center().bg(theme.hover).typography(Type::Small).text_color(theme.text_soft).child(text.into())
}

fn server_row(row: &ServerRow, first: bool, expanded: bool, confirming: bool, theme: &Theme) -> impl IntoElement {
    let server = &row.server;
    let id = server.id.clone();
    let (status, tone) = standing(row);
    let dot = match tone {
        Tone::Good => theme.success,
        Tone::Bad => theme.danger,
        Tone::Quiet => theme.text_faint,
    };
    let chevron = Icon::ChevronRight.view(px(13.), theme.text_faint).with_transformation(gpui::Transformation::rotate(gpui::radians(if expanded { std::f32::consts::FRAC_PI_2 } else { 0. })));
    let head = div()
        .id(SharedString::from(format!("mcp-row-{id}")))
        .px(px(18.))
        .py(px(12.))
        .flex()
        .items_center()
        .gap(px(12.))
        .cursor_pointer()
        .hover(|line| line.bg(theme.hover))
        .on_click(dispatch(ExpandMcpServer { id: id.clone() }))
        .child(chevron)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().typography(Type::UiStrong).text_color(if server.enabled { theme.text } else { theme.text_dim }).child(server.name.clone()))
                        .child(badge(server.transport.label(), theme)),
                )
                .child(div().truncate().typography(Type::Mono).text_color(theme.text_dim).child(summary(server)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .typography(Type::Small)
                        .child(div().flex_none().size(px(6.)).rounded_full().bg(dot))
                        .child(div().min_w_0().truncate().text_color(if tone == Tone::Bad { theme.danger } else { theme.text_dim }).child(status))
                        .child(div().flex_none().text_color(theme.text_faint).child("·"))
                        .child(div().flex_none().text_color(theme.text_faint).child(row.reach.clone())),
                ),
        )
        .child(
            div()
                .id(SharedString::from(format!("mcp-switch-{id}")))
                .flex_none()
                .cursor_pointer()
                // The row opens on a click; its switch only switches.
                .on_click({
                    let id = id.clone();
                    move |_, window, cx| {
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(ToggleMcpServer { id: id.clone() }), cx);
                    }
                })
                .child(switch(server.enabled, theme))
                .tag("mcp-switch"),
        );
    div()
        .flex()
        .flex_col()
        .when(!first, |row| row.border_t_1().border_color(theme.border))
        .child(head)
        .when(expanded, |block| block.child(details(row, confirming, theme)))
        .tag("mcp-server")
}

fn details(row: &ServerRow, confirming: bool, theme: &Theme) -> impl IntoElement {
    let server = &row.server;
    let id = server.id.clone();
    let fact = |label: &'static str, value: String, mono: bool| {
        div()
            .flex()
            .gap(px(12.))
            .child(div().flex_none().w(px(96.)).typography(Type::Small).text_color(theme.text_faint).child(label))
            .child(div().flex_1().min_w_0().typography(if mono { Type::Mono } else { Type::Small }).text_color(theme.text_soft).child(value))
    };
    let mut facts = div().flex().flex_col().gap(px(6.));
    match server.transport {
        Transport::Stdio => {
            facts = facts.child(fact("Command", server.command.clone(), true));
            if !server.args.is_empty() {
                facts = facts.child(fact("Arguments", server.args.join(" "), true));
            }
            if !server.env.is_empty() {
                facts = facts.child(fact("Environment", server.env.join(", "), true));
            }
        }
        _ => {
            facts = facts.child(fact("URL", server.url.clone(), true));
            if !server.headers.is_empty() {
                facts = facts.child(fact("Headers", server.headers.join(", "), true));
            }
        }
    }
    facts = facts.child(fact("Timeout", format!("{} s", seconds(server.timeout_ms)), false)).child(fact("Projects", row.reach.to_string(), false));

    let tools: AnyElement = if server.tools.is_empty() {
        div().typography(Type::Small).text_color(theme.text_faint).child("Test the connection to list this server’s tools. They’re all on until you switch one off.").into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .rounded(px(12.))
            .border_1()
            .border_color(theme.border)
            .children(server.tools.iter().enumerate().map(|(index, tool)| {
                let on = !server.disabled_tools.contains(&tool.name);
                let toggle = ToggleMcpTool { id: id.clone(), tool: tool.name.clone() };
                div()
                    .id(SharedString::from(format!("mcp-tool-{id}-{index}")))
                    .px(px(12.))
                    .py(px(9.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .when(index > 0, |line| line.border_t_1().border_color(theme.border))
                    .cursor_pointer()
                    .on_click(dispatch(toggle))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(div().typography(Type::Mono).text_color(if on { theme.text } else { theme.text_dim }).child(tool.name.clone()))
                                    .when(tool.read_only, |line| line.child(badge("Read-only", theme))),
                            )
                            .when(!tool.description.trim().is_empty(), |words| {
                                words.child(div().truncate().typography(Type::Small).text_color(theme.text_faint).child(tool.description.lines().next().unwrap_or_default().to_string()))
                            }),
                    )
                    .child(switch(on, theme))
                    .tag("mcp-tool")
            }))
            .into_any_element()
    };

    let actions: AnyElement = if confirming {
        div()
            .p(px(12.))
            .rounded(px(12.))
            .bg(theme.window)
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().min_w_0().typography(Type::Ui).text_color(theme.text).child("Delete this server? Its tools stop being offered, and its saved headers and environment values are deleted from this Mac."))
            .child(Button::new("mcp-keep", "Keep it", ButtonKind::Secondary).action(CancelDeleteMcpServer).tag("mcp-keep"))
            .child(Button::new("mcp-delete-confirm", "Delete", ButtonKind::Danger).action(ConfirmDeleteMcpServer).tag("mcp-delete-confirm"))
            .into_any_element()
    } else {
        let testing = matches!(row.test, Some(RowTest::Testing));
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(pill(SharedString::from(format!("mcp-test-{id}")), if testing { "Testing…" } else { "Test connection" }, Some(Icon::Play), false, theme).on_click(dispatch(TestMcpServer { id: id.clone() })).tag("mcp-row-test"))
            .child(pill(SharedString::from(format!("mcp-edit-{id}")), "Edit", Some(Icon::Pencil), false, theme).on_click(dispatch(EditMcpServer { id: id.clone() })).tag("mcp-row-edit"))
            .child(div().flex_1())
            .child(pill(SharedString::from(format!("mcp-delete-{id}")), "Delete", Some(Icon::Trash), false, theme).on_click(dispatch(AskDeleteMcpServer { id: id.clone() })).tag("mcp-row-delete"))
            .into_any_element()
    };

    div()
        .px(px(18.))
        .pl(px(43.))
        .pb(px(16.))
        .pt(px(2.))
        .flex()
        .flex_col()
        .gap(px(14.))
        .child(facts)
        .child(div().flex().flex_col().gap(px(8.)).child(div().typography(Type::Label).text_color(theme.text_dim).child("Tools")).child(tools))
        .child(actions)
}

fn editor_page(editor: &EditorData, theme: &Theme) -> impl IntoElement {
    let back = div()
        .id("back-mcp")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .pb(px(18.))
        .cursor_pointer()
        .typography(Type::Ui)
        .text_color(theme.text_dim)
        .hover(|link| link.text_color(theme.text))
        .on_click(dispatch(CloseMcpEditor))
        .child(Icon::ChevronRight.view(px(13.), theme.text_faint).with_transformation(gpui::Transformation::rotate(gpui::radians(std::f32::consts::PI))))
        .child("MCP servers")
        .tag("back-mcp");
    let section = |content: AnyElement| div().px(px(18.)).py(px(13.)).border_t_1().border_color(theme.border).child(content);
    let stdio = editor.transport == Transport::Stdio;
    let kind_hint: SharedString = match editor.transport {
        Transport::Stdio => "A program on this Mac that WackCode starts, in the chat’s project folder, with your permissions.".into(),
        Transport::Http => "A server at a URL that speaks Streamable HTTP, the current MCP transport.".into(),
        Transport::Sse => "A server at a URL that speaks the older HTTP+SSE transport.".into(),
    };

    let mut server_card = card(Some("Server"), None, None, theme)
        .child(section(
            div()
                .flex()
                .gap(px(14.))
                .child(field("Name", Some("Shown in chats and in its tools’ names.".into()), text_input(editor.name.clone(), "mcp-name", theme), theme))
                .child(div().w(px(150.)).flex_none().child(field("Timeout", Some("In seconds.".into()), text_input(editor.timeout.clone(), "mcp-timeout", theme), theme)))
                .into_any_element(),
        ))
        .child(section(
            field(
                "Type",
                Some(kind_hint),
                segmented("mcp-transport", &[(Transport::Stdio, "Command"), (Transport::Http, "HTTP"), (Transport::Sse, "SSE")], editor.transport, |transport| Box::new(ChooseMcpTransport { transport }), theme),
                theme,
            )
            .into_any_element(),
        ));
    if stdio {
        server_card = server_card
            .child(section(field("Command", Some("The program to run, such as npx, uvx or docker.".into()), text_input(editor.command.clone(), "mcp-command", theme), theme).into_any_element()))
            .child(section(field("Arguments", Some("One per line.".into()), text_area(editor.args.clone(), 72., theme).tag("mcp-args"), theme).into_any_element()));
    } else {
        server_card = server_card.child(section(
            field("URL", Some("Headers go only to this server, and only over https:// unless it is on this Mac.".into()), text_input(editor.url.clone(), "mcp-url", theme), theme).into_any_element(),
        ));
    }
    server_card = server_card.child(section(entries(editor, theme)));

    div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .child(div().flex().flex_col().child(back).child(head(
            if editor.new { "New MCP server" } else { "Edit MCP server" },
            "Its tools are offered to the agent in the projects you choose. Values you type are kept in WackCode’s secret store and never shown again.",
            None,
            theme,
        )))
        .child(server_card)
        .child(projects_card(editor, theme))
        .children(editor.test.clone().map(|test| test_result(test, theme)))
        .child(footer(editor, theme))
}

/// The variables (command) or headers (URL), each a name and a masked value.
fn entries(editor: &EditorData, theme: &Theme) -> AnyElement {
    let stdio = editor.transport == Transport::Stdio;
    let (label, hint, add) = if stdio {
        ("Environment variables", "Given to this server’s process only, such as an API token.", "Add variable")
    } else {
        ("Headers", "Sent with every request to this server, such as Authorization: Bearer <token>.", "Add header")
    };
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(div().flex().flex_col().gap(px(2.)).child(div().typography(Type::Ui).text_color(theme.text_soft).child(label)).child(div().typography(Type::Small).text_color(theme.text_faint).child(hint)))
        .children(editor.entries.iter().enumerate().map(|(index, entry)| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().w(px(300.)).flex_none().child(text_input(entry.name.clone(), "mcp-entry-name", theme)))
                .child(div().flex_1().min_w_0().child(text_input(entry.value.clone(), "mcp-entry-value", theme)))
                .child(
                    div()
                        .id(("mcp-entry-remove", index))
                        .flex_none()
                        .size(px(28.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|button| button.bg(theme.hover))
                        .on_click(dispatch(RemoveMcpEntry { index }))
                        .child(Icon::Close.view(px(12.), theme.text_dim)),
                )
        }))
        .child(div().flex().child(pill("mcp-add-entry", add, Some(Icon::Plus), false, theme).on_click(dispatch(AddMcpEntry)).tag("mcp-add-entry")))
        .into_any_element()
}

fn projects_card(editor: &EditorData, theme: &Theme) -> impl IntoElement {
    let chosen = matches!(editor.projects, Projects::Only(_));
    let mut block = div().flex().flex_col().items_start().gap(px(12.)).child(segmented(
        "mcp-scope",
        &[(false, "All projects"), (true, "Chosen projects")],
        chosen,
        |only| Box::new(ChooseMcpScope { only }),
        theme,
    ));
    if let Projects::Only(ids) = &editor.projects {
        block = block.child(if editor.all_projects.is_empty() {
            div().typography(Type::Small).text_color(theme.text_faint).child("Add a project folder first.").into_any_element()
        } else {
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .children(editor.all_projects.iter().enumerate().map(|(index, (id, name))| {
                    let on = ids.contains(id);
                    pill(("mcp-project", index), name.clone(), on.then_some(Icon::Check), on, theme).on_click(dispatch(ToggleMcpProject { id: id.clone() })).tag("mcp-project")
                }))
                .into_any_element()
        });
    }
    card(Some("Projects"), None, Some("Keeps a server, such as one for a design app, out of unrelated work."), theme)
        .child(div().px(px(18.)).py(px(13.)).border_t_1().border_color(theme.border).child(block))
        .tag("mcp-projects")
}

fn test_result(test: ProbeOutcome, theme: &Theme) -> impl IntoElement {
    let body: AnyElement = match test {
        ProbeOutcome::Failed(message) => div().typography(Type::Ui).text_color(theme.danger).child(message).into_any_element(),
        ProbeOutcome::Found { elapsed_ms, tools } => div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().typography(Type::Small).text_color(theme.success).child(format!("Connected in {elapsed_ms} ms · {}", plural(tools.len(), "tool", "tools"))))
            .child(div().flex().flex_wrap().gap(px(6.)).children(tools.into_iter().map(|(name, read_only)| {
                div()
                    .h(px(22.))
                    .px(px(9.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .bg(theme.hover)
                    .typography(Type::Mono)
                    .text_color(theme.text_soft)
                    .child(name)
                    .when(read_only, |chip| chip.child(div().typography(Type::Small).text_color(theme.text_faint).child("read-only")))
            })))
            .into_any_element(),
    };
    card(Some("Test connection"), None, None, theme).child(div().px(px(18.)).py(px(13.)).border_t_1().border_color(theme.border).child(body)).tag("mcp-test")
}

fn footer(editor: &EditorData, theme: &Theme) -> impl IntoElement {
    if editor.confirming_delete {
        return div()
            .p(px(14.))
            .rounded(px(12.))
            .bg(theme.window)
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().min_w_0().typography(Type::Ui).text_color(theme.text).child("Delete this server and its saved values?"))
            .child(Button::new("mcp-editor-keep", "Keep it", ButtonKind::Secondary).action(CancelDeleteMcpServer).tag("mcp-editor-keep"))
            .child(Button::new("mcp-editor-delete-confirm", "Delete", ButtonKind::Danger).action(ConfirmDeleteMcpServer).tag("mcp-editor-delete-confirm"))
            .into_any_element();
    }
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .children(editor.id.clone().map(|id| Button::new("mcp-editor-delete", "Delete", ButtonKind::Secondary).action(AskDeleteMcpServer { id }).tag("mcp-editor-delete")))
        .child(div().flex_1().min_w_0().typography(Type::Small).text_color(theme.danger).children(editor.error.clone()))
        .child(Button::new("mcp-test-button", if editor.testing { "Testing…" } else { "Test connection" }, ButtonKind::Secondary).action(TestMcpDraft).enabled(!editor.testing).tag("mcp-test-button"))
        .child(Button::new("mcp-cancel", "Cancel", ButtonKind::Secondary).action(CloseMcpEditor).tag("mcp-cancel"))
        .child(Button::new("mcp-save", if editor.new { "Add server" } else { "Save" }, ButtonKind::Primary).action(SaveMcpServer).tag("mcp-save"))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(enabled: bool, live: Live, test: Option<RowTest>) -> ServerRow {
        let server = McpServer {
            id: "mcp-1".into(),
            name: "GitHub".into(),
            enabled,
            transport: Transport::Stdio,
            timeout_ms: 120_000,
            command: "npx".into(),
            args: vec!["-y".into(), "@example/server".into()],
            url: String::new(),
            headers: Vec::new(),
            env: Vec::new(),
            disabled_tools: Vec::new(),
            tools: Vec::new(),
            projects: Projects::All,
        };
        ServerRow { server, live, reach: "All projects".into(), test }
    }

    #[test]
    fn a_row_says_how_its_server_stands() {
        assert_eq!(standing(&row(false, Live::Connected { tools: 3, folders: 1 }, None)).0.as_ref(), "Off");
        assert_eq!(standing(&row(true, Live::Idle, None)).0.as_ref(), "Starts when a chat uses it");
        assert_eq!(standing(&row(true, Live::Connected { tools: 1, folders: 1 }, None)).0.as_ref(), "Connected · 1 tool");
        assert_eq!(standing(&row(true, Live::Connected { tools: 5, folders: 2 }, None)).0.as_ref(), "Connected in 2 projects · 5 tools");
        assert_eq!(standing(&row(true, Live::Idle, Some(RowTest::Passed(4)))).0.as_ref(), "Connection works · 4 tools");
        assert_eq!(standing(&row(true, Live::Connecting, Some(RowTest::Testing))).0.as_ref(), "Testing the connection…");
        assert_eq!(summary(&row(true, Live::Idle, None).server), "npx -y @example/server");
    }

    #[test]
    fn reach_names_the_chosen_projects() {
        let names = vec![("p1".to_string(), SharedString::from("WackCode")), ("p2".to_string(), SharedString::from("Site"))];
        assert_eq!(reach(&Projects::All, &names).as_ref(), "All projects");
        assert_eq!(reach(&Projects::Only(vec!["p2".into(), "p1".into()]), &names).as_ref(), "Site, WackCode");
        assert_eq!(reach(&Projects::Only(vec!["gone".into()]), &names).as_ref(), "No projects");
        assert_eq!(seconds(120_000), "120");
        assert_eq!(seconds(1_500), "1.5");
    }
}
