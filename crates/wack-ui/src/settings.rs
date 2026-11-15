//! Settings: one section at a time on the stage, its navigation in the sidebar's pane.
//!
//! The navigation groups the sections (Look, Models, Agent, Reach) and lists those still to come,
//! dimmed, so it is laid out for the dozen-odd sections planned. Each page is a head and cards of
//! rows, built from `settings_kit.rs`.
//!
//! Settings › Providers is here: the connections to model APIs, each with an on / off switch (off
//! keeps the key and models, out of the model menu), and the form that adds one, folded away
//! behind "New connection" until asked for. Settings › Appearance is `appearance_page.rs`.
//!
//! A key is typed into a masked field, handed to the store once, and the field is cleared: it is
//! never drawn, copied or kept anywhere else (docs/security.md). Adding a connection sends
//! nothing over the network.
//!
//! The form's fields live in [`ProviderForm`], owned by the app state, so leaving Settings
//! half-way keeps what was typed. The page itself draws from plain data and dispatches actions.

use std::f32::consts::PI;

use gpui::{App, AppContext, Entity, Pixels, SharedString, Transformation, div, prelude::*, px, radians};
use wack_core::{Connection, Model, NewConnection, WireFormat};

use crate::{
    actions::{AddConnection, ChooseFormat, CloseConnectionForm, CloseSettings, OpenConnectionForm, OpenSettingsSection, RemoveConnection, SwitchConnection},
    appearance_page::{self, AppearanceData},
    icon::Icon,
    settings_kit::{PAGE_WIDTH, card, dispatch, head, pill, row as settings_row, segmented},
    shell::model_name,
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    tools_page::switch,
    typography::{Type, Typography},
    work,
};

pub struct ProviderForm {
    pub name: Entity<TextField>,
    pub address: Entity<TextField>,
    pub models: Entity<TextField>,
    pub context: Entity<TextField>,
    pub key: Entity<TextField>,
    pub format: WireFormat,
    pub error: Option<SharedString>,
    /// "New connection" was pressed. With no connections yet the form shows regardless.
    pub open: bool,
}

impl ProviderForm {
    pub fn new(cx: &mut App) -> Self {
        let field = |placeholder: &'static str, cx: &mut App| cx.new(|cx| TextField::new(Type::Ui, placeholder, cx));
        Self {
            name: field("Work, Local, OpenRouter…", cx),
            address: field("https://api.example.com/v1", cx),
            models: field("model-id, another-model-id", cx),
            context: field("200k", cx),
            key: cx.new(|cx| TextField::new(Type::Ui, "Paste the key", cx).masked()),
            format: WireFormat::OpenaiChat,
            error: None,
            open: false,
        }
    }

    pub fn fields(&self) -> [&Entity<TextField>; 5] {
        [&self.name, &self.address, &self.models, &self.context, &self.key]
    }

    /// What the form describes, and the key, or why it is not complete.
    pub fn read(&self, cx: &App) -> Result<(NewConnection, String), String> {
        let text = |field: &Entity<TextField>| field.read(cx).text().trim().to_string();
        let context = text(&self.context);
        let context_window = if context.is_empty() {
            None
        } else {
            Some(parse_tokens(&context).ok_or("Write the context window as a number of tokens, such as 200000 or 200k.")?)
        };
        let models = text(&self.models)
            .split([',', ' ', '\n', '\t'])
            .filter(|id| !id.is_empty())
            .map(|id| Model { id: id.to_string(), context_window })
            .collect();
        Ok((NewConnection { name: text(&self.name), format: self.format, base_url: text(&self.address), models }, text(&self.key)))
    }

    pub fn clear(&mut self, cx: &mut App) {
        for field in self.fields() {
            field.update(cx, |field, cx| field.set_text("", cx));
        }
        self.error = None;
        self.open = false;
    }
}

/// "200000", "200k", "1M", "1.5m" in tokens.
pub fn parse_tokens(text: &str) -> Option<u64> {
    let text = text.trim().replace(['_', ','], "");
    let (number, scale) = match text.chars().last()?.to_ascii_lowercase() {
        'k' => (&text[..text.len() - 1], 1_000.),
        'm' => (&text[..text.len() - 1], 1_000_000.),
        _ => (text.as_str(), 1.),
    };
    let value: f64 = number.trim().parse().ok()?;
    (value > 0.).then(|| (value * scale).round() as u64)
}

pub fn format_name(format: WireFormat) -> &'static str {
    match format {
        WireFormat::OpenaiChat => "Chat Completions",
        WireFormat::OpenaiResponses => "OpenAI Responses",
        WireFormat::AnthropicMessages => "Anthropic Messages",
    }
}

/// A connection as its row shows it.
#[derive(Clone, Debug)]
pub struct ConnectionRow {
    pub connection: Connection,
    /// A key is kept for it. The key itself never reaches the page.
    pub has_key: bool,
}

/// Everything the page shows.
#[derive(Clone)]
pub struct ProvidersData {
    pub connections: Vec<ConnectionRow>,
    pub name: Entity<TextField>,
    pub address: Entity<TextField>,
    pub models: Entity<TextField>,
    pub context: Entity<TextField>,
    pub key: Entity<TextField>,
    pub format: WireFormat,
    pub error: Option<SharedString>,
    /// The new-connection form is on the page.
    pub form_open: bool,
}

impl ProvidersData {
    pub fn new(connections: &[Connection], has_key: impl Fn(&str) -> bool, form: &ProviderForm) -> Self {
        Self {
            connections: connections
                .iter()
                .map(|connection| ConnectionRow { connection: connection.clone(), has_key: has_key(&connection.id) })
                .collect(),
            name: form.name.clone(),
            address: form.address.clone(),
            models: form.models.clone(),
            context: form.context.clone(),
            key: form.key.clone(),
            format: form.format,
            error: form.error.clone(),
            form_open: form.open || connections.is_empty(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Appearance,
    Providers,
    Tools,
}

impl Section {
    pub const ALL: [Section; 3] = [Section::Appearance, Section::Providers, Section::Tools];

    pub fn name(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Providers => "Providers",
            Section::Tools => "Tools",
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Section::Appearance => "section-appearance",
            Section::Providers => "section-providers",
            Section::Tools => "section-tools",
        }
    }
}

/// The navigation's groups. `None` is a section still to come: listed, dimmed, not yet a place
/// to go, so the shape of Settings is clear from the start and the list never reflows as they
/// arrive.
/// A section in the navigation: where it goes (none yet for one to come), and its name.
type Entry = (Option<Section>, &'static str);

const GROUPS: [(&str, &[Entry]); 4] = [
    ("Look", &[(Some(Section::Appearance), "Appearance")]),
    ("Models", &[(Some(Section::Providers), "Providers"), (None, "Prompts")]),
    (
        "Agent",
        &[(Some(Section::Tools), "Tools"), (None, "Memory"), (None, "Skills"), (None, "Commands"), (None, "Sub-agents"), (None, "Git")],
    ),
    ("Reach", &[(None, "MCP servers"), (None, "Web"), (None, "Computer use")]),
];

/// Everything the Settings page shows.
#[derive(Clone)]
pub struct SettingsData {
    pub section: Section,
    pub providers: ProvidersData,
    pub appearance: AppearanceData,
    pub tools: crate::tools_page::ToolsData,
}

/// The title strip's words: `Settings › Providers`.
/// Over a picture the strip's words sit on `ground`, the panes' glass, so they read on the frame.
pub fn strip(section: Section, inset: Pixels, ground: Option<gpui::Hsla>, theme: &Theme) -> impl IntoElement {
    let pill = |content: gpui::Div| content.h(px(28.)).px(px(12.)).rounded_full().flex().items_center().when_some(ground, |pill, ground| pill.bg(ground));
    div()
        .size_full()
        .pl((inset - px(12.)).max(px(4.)))
        .pr(px(4.))
        .flex()
        .items_center()
        .typography(Type::UiStrong)
        .child(
            pill(div().gap(px(8.)))
                .child(div().text_color(theme.text_dim).child("Settings"))
                .child(div().text_color(theme.text_faint).child("›"))
                .child(div().text_color(theme.text).child(section.name())),
        )
        .child(div().flex_1())
        .child(pill(div()).typography(Type::Small).text_color(theme.text_faint).child("esc to close"))
}

/// The navigation, in the sidebar's pane while Settings is open: back to the chats, then the
/// sections in their groups. `connections` is counted beside Providers.
pub fn nav(section: Section, connections: usize, theme: &Theme) -> impl IntoElement {
    let back = div()
        .id("settings-back")
        .h(px(30.))
        .px(px(10.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(8.))
        .cursor_pointer()
        .hover(|row| row.bg(theme.hover))
        .on_click(dispatch(CloseSettings))
        .child(Icon::ChevronRight.view(px(13.), theme.text_dim).with_transformation(Transformation::rotate(radians(PI))))
        .child(div().flex_1().typography(Type::Ui).text_color(theme.text_soft).child("Back to chats"))
        .child(div().typography(Type::Small).text_color(theme.text_faint).child("esc"))
        .tag("settings-back");

    let mut list = div().flex().flex_col().gap(px(1.)).px(px(10.)).pb(px(16.));
    for (group, entries) in GROUPS {
        list = list.child(div().px(px(10.)).pt(px(16.)).pb(px(4.)).typography(Type::Label).text_color(theme.text_dim).child(group));
        for (target, name) in entries.iter().copied() {
            let row = div().h(px(30.)).px(px(10.)).rounded_full().flex().items_center().gap(px(8.));
            list = list.child(match target {
                Some(target) => {
                    let lit = target == section;
                    row.id(SharedString::from(format!("section-{name}")))
                        .cursor_pointer()
                        .when(lit, |row| row.bg(theme.accent_wash))
                        .when(!lit, |row| row.hover(|row| row.bg(theme.hover)))
                        .on_click(dispatch(OpenSettingsSection { section: target }))
                        .child(
                            div()
                                .flex_1()
                                .typography(if lit { Type::UiStrong } else { Type::Ui })
                                .text_color(if lit { theme.accent_text } else { theme.text_soft })
                                .child(name),
                        )
                        .when(target == Section::Providers && connections > 0, |row| {
                            row.child(div().typography(Type::Small).text_color(theme.text_faint).child(connections.to_string()))
                        })
                        .tag(target.tag())
                        .into_any_element()
                }
                None => row
                    .child(div().flex_1().typography(Type::Ui).text_color(theme.text_faint).child(name))
                    .child(div().typography(Type::Small).text_color(theme.text_faint).child("soon"))
                    .into_any_element(),
            });
        }
    }
    div()
        .flex_1()
        .min_h_0()
        .w_full()
        .flex()
        .flex_col()
        .child(div().flex_none().px(px(10.)).pt(px(4.)).child(back))
        .child(div().id("settings-nav").flex_1().min_h_0().overflow_y_scroll().child(list))
}

/// The page, centred on the stage. With the sidebar put away the sections are a row of pills
/// above it instead (`inline_nav`), since the sidebar's pane is where they otherwise live.
pub fn page(data: SettingsData, inline_nav: bool, narrow: bool, theme: &Theme) -> impl IntoElement {
    let section = data.section;
    let nav = inline_nav.then(|| {
        let choices: Vec<(Section, &'static str)> = Section::ALL.iter().map(|section| (*section, section.name())).collect();
        div().pb(px(26.)).flex().child(segmented("section", &choices, section, |section| Box::new(OpenSettingsSection { section }), theme))
    });
    let content = match section {
        Section::Appearance => appearance_page::page(data.appearance, theme).into_any_element(),
        Section::Providers => providers(data.providers, theme).into_any_element(),
        Section::Tools => crate::tools_page::page(data.tools, theme).into_any_element(),
    };
    div().id("settings-page").size_full().overflow_y_scroll().child(
        div()
            .w_full()
            .px(px(if narrow { 24. } else { 48. }))
            .pt(px(28.))
            .pb(px(64.))
            .flex()
            .justify_center()
            .child(div().flex_1().min_w_0().max_w(px(PAGE_WIDTH)).flex().flex_col().children(nav).child(content)),
    )
}

fn providers(data: ProvidersData, theme: &Theme) -> impl IntoElement {
    let on = data.connections.iter().filter(|row| row.connection.enabled).count();
    let action = (!data.form_open).then(|| {
        pill("new-connection", "New connection", Some(Icon::Plus), true, theme)
            .on_click(dispatch(OpenConnectionForm))
            .tag("new-connection")
            .into_any_element()
    });
    let mut connections = None;
    if !data.connections.is_empty() {
        let mut list = card(Some("Connections"), Some(format!("{on} on").into()), Some("Off keeps the key and models, out of the model menu"), theme);
        for (index, row) in data.connections.iter().enumerate() {
            list = list.child(connection_row(index, row, theme));
        }
        connections = Some(list);
    }
    let adding = data.form_open.then(|| form(&data, theme));
    div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .child(head(
            "Providers",
            "Connections to model APIs. A key is kept in WackCode’s own secrets file and only ever sent to the address it belongs to.",
            action,
            theme,
        ))
        .children(adding)
        .children(connections)
}

/// A round monogram: the name's first letter.
fn monogram(name: &str, on: bool, theme: &Theme) -> impl IntoElement {
    let letter: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default();
    div()
        .flex_none()
        .size(px(30.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(if on { theme.accent_wash } else { theme.hover })
        .typography(Type::UiStrong)
        .text_color(if on { theme.accent_text } else { theme.text_faint })
        .child(letter)
}

/// Tags for the automation socket, which takes only fixed names.
const SWITCH_TAGS: [&str; 4] = ["switch-connection-0", "switch-connection-1", "switch-connection-2", "switch-connection-3"];
const REMOVE_TAGS: [&str; 4] = ["remove-connection-0", "remove-connection-1", "remove-connection-2", "remove-connection-3"];

fn connection_row(index: usize, row: &ConnectionRow, theme: &Theme) -> impl IntoElement {
    let connection = &row.connection;
    let on = connection.enabled;
    let host = connection.base_url.trim_start_matches("https://").trim_start_matches("http://").to_string();
    let models = connection
        .models
        .iter()
        .map(|model| match model.context_window {
            Some(window) => format!("{}  {}", model_name(&model.id), work::count(window)),
            None => model_name(&model.id),
        })
        .collect::<Vec<_>>()
        .join("   ·   ");
    let count = connection.models.len();
    let status = if row.has_key {
        div().typography(Type::Small).text_color(theme.text_faint).child(format!("Key saved · {count} model{}", if count == 1 { "" } else { "s" }))
    } else {
        div().typography(Type::Small).text_color(theme.warning).child("No key")
    };
    let id = connection.id.clone();
    div()
        .id(("connection", index))
        .group("connection")
        .px(px(18.))
        .py(px(13.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(14.))
        .child(monogram(&connection.name, on, theme))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().typography(Type::UiStrong).text_color(if on { theme.text } else { theme.text_dim }).child(connection.name.clone()))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .typography(Type::Small)
                        .text_color(theme.text_dim)
                        .child(div().flex_none().child(format_name(connection.format)))
                        .child(div().flex_none().text_color(theme.text_faint).child("·"))
                        .child(div().min_w_0().truncate().typography(Type::Mono).child(host)),
                )
                .child(div().pt(px(2.)).truncate().typography(Type::Mono).text_color(if on { theme.text_soft } else { theme.text_faint }).child(models)),
        )
        .child(status.flex_none())
        .child(
            div()
                .id(("remove-connection", index))
                .flex_none()
                .size(px(26.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .invisible()
                .group_hover("connection", |button| button.visible())
                .hover(|button| button.bg(theme.hover))
                .tooltip(crate::tooltip::tip("Remove, and delete its key"))
                .on_click(dispatch(RemoveConnection { id: id.clone() }))
                .child(Icon::Trash.view(px(14.), theme.text_dim))
                .tag(REMOVE_TAGS.get(index).copied().unwrap_or("remove-connection")),
        )
        .child(
            div()
                .id(("switch-connection", index))
                .flex_none()
                .cursor_pointer()
                .on_click(dispatch(SwitchConnection { id, on: !on }))
                .child(switch(on, theme))
                .tag(SWITCH_TAGS.get(index).copied().unwrap_or("switch-connection")),
        )
}

fn form(data: &ProvidersData, theme: &Theme) -> impl IntoElement {
    let field = |field: Entity<TextField>| {
        div()
            .w(px(FIELD_WIDTH))
            .px(px(12.))
            .py(px(6.))
            .rounded_full()
            .bg(theme.window)
            .border_1()
            .border_color(theme.border)
            .child(field)
            .into_any_element()
    };
    let formats: Vec<(WireFormat, &'static str)> =
        [WireFormat::OpenaiChat, WireFormat::OpenaiResponses, WireFormat::AnthropicMessages].map(|format| (format, format_name(format))).to_vec();
    let formats = segmented("format", &formats, data.format, |format| Box::new(ChooseFormat { format }), theme);
    let buttons = div()
        .px(px(18.))
        .py(px(14.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(10.))
        .child(pill("add-connection", "Add connection", None, true, theme).on_click(dispatch(AddConnection)).tag("add-connection"))
        .when(!data.connections.is_empty(), |row| {
            row.child(pill("cancel-connection", "Cancel", None, false, theme).on_click(dispatch(CloseConnectionForm)).tag("cancel-connection"))
        })
        .when_some(data.error.clone(), |row, error| {
            row.child(div().flex_1().min_w_0().typography(Type::Ui).text_color(theme.danger).child(error))
        });
    let note = if data.connections.is_empty() { Some("Add one to start chatting") } else { None };
    card(Some("New connection"), None, note, theme)
        .child(settings_row("Name", None).control(field(data.name.clone())).view(theme))
        .child(settings_row("Format", Some("Chat Completions suits most servers, including local ones.")).control(formats).stacked().view(theme))
        .child(settings_row("Address", Some("The API’s base address, without /chat/completions.")).control(field(data.address.clone())).view(theme))
        .child(settings_row("Models", Some("Model ids, separated by commas or spaces.")).control(field(data.models.clone())).view(theme))
        .child(settings_row("Context", Some("Tokens each model can read, such as 200k. Optional; it decides when chats compact.")).control(field(data.context.clone())).view(theme))
        .child(settings_row("API key", Some("Sent only to the address above.")).control(field(data.key.clone())).view(theme))
        .child(buttons)
}

/// The width of the form's text fields.
const FIELD_WIDTH: f32 = 280.;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_token_counts() {
        assert_eq!(parse_tokens("200000"), Some(200_000));
        assert_eq!(parse_tokens("200k"), Some(200_000));
        assert_eq!(parse_tokens("1M"), Some(1_000_000));
        assert_eq!(parse_tokens("1.5m"), Some(1_500_000));
        assert_eq!(parse_tokens("128_000"), Some(128_000));
        assert_eq!(parse_tokens("lots"), None);
        assert_eq!(parse_tokens("0"), None);
        assert_eq!(parse_tokens(""), None);
    }
}
