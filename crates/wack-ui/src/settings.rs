//! Settings: a full page with its own navigation down the left, one section at a time.
//!
//! Settings › Providers is here: the connections to model APIs, and the form that adds one.
//! Settings › Appearance is `appearance_page.rs`.
//!
//! A key is typed into a masked field, handed to the store once, and the field is cleared: it is
//! never drawn, copied or kept anywhere else (docs/security.md). Adding a connection sends
//! nothing over the network.
//!
//! The form's fields live in [`ProviderForm`], owned by the app state, so leaving Settings
//! half-way keeps what was typed. The page itself draws from plain data and dispatches actions.

use gpui::{App, AppContext, Entity, Pixels, SharedString, Window, div, prelude::*, px};
use wack_core::{Connection, Model, NewConnection, WireFormat};

use crate::{
    actions::{AddConnection, ChooseFormat, OpenSettingsSection, RemoveConnection},
    appearance_page::{self, AppearanceData},
    shell::model_name,
    tags::Tag,
    text_field::TextField,
    theme::Theme,
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

/// Everything the page shows.
#[derive(Clone)]
pub struct ProvidersData {
    pub connections: Vec<Connection>,
    pub name: Entity<TextField>,
    pub address: Entity<TextField>,
    pub models: Entity<TextField>,
    pub context: Entity<TextField>,
    pub key: Entity<TextField>,
    pub format: WireFormat,
    pub error: Option<SharedString>,
}

impl ProvidersData {
    pub fn new(connections: &[Connection], form: &ProviderForm) -> Self {
        Self {
            connections: connections.to_vec(),
            name: form.name.clone(),
            address: form.address.clone(),
            models: form.models.clone(),
            context: form.context.clone(),
            key: form.key.clone(),
            format: form.format,
            error: form.error.clone(),
        }
    }
}

fn dispatch(action: impl gpui::Action) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static {
    let action = action.boxed_clone();
    move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)
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
}

/// Everything the Settings page shows.
#[derive(Clone)]
pub struct SettingsData {
    pub section: Section,
    pub providers: ProvidersData,
    pub appearance: AppearanceData,
    pub tools: crate::tools_page::ToolsData,
}

/// The title strip's words: `Settings › Providers`.
pub fn strip(section: Section, inset: Pixels, theme: &Theme) -> impl IntoElement {
    div()
        .size_full()
        .pl(inset)
        .pr(px(20.))
        .flex()
        .items_center()
        .gap(px(8.))
        .typography(Type::UiStrong)
        .child(div().text_color(theme.text_dim).child("Settings"))
        .child(div().text_color(theme.text_faint).child("›"))
        .child(div().text_color(theme.text).child(section.name()))
        .child(div().flex_1())
        .child(div().typography(Type::Small).text_color(theme.text_faint).child("esc to close"))
}

/// The navigation's width, the gap after it, and the open section's widest line.
const NAV_WIDTH: f32 = 150.;
const NAV_GAP: f32 = 40.;
const CONTENT_WIDTH: f32 = 720.;

/// The page: the sections down the left (across the top when narrow), the one open beside them.
pub fn page(data: SettingsData, inset: Pixels, narrow: bool, theme: &Theme) -> impl IntoElement {
    let nav = div()
        .flex_none()
        .flex()
        .when(narrow, |nav| nav.gap(px(22.)))
        .when(!narrow, |nav| nav.w(px(NAV_WIDTH)).flex_col().items_start().gap(px(10.)).pt(px(8.)))
        .children(Section::ALL.map(|section| {
            appearance_page::word(format!("section-{}", section.name()), section.name(), section == data.section, theme)
                .on_click(dispatch(OpenSettingsSection { section }))
        }));
    let content = match data.section {
        Section::Appearance => appearance_page::page(data.appearance, theme).into_any_element(),
        Section::Providers => providers(data.providers, theme).into_any_element(),
        Section::Tools => crate::tools_page::page(data.tools, theme).into_any_element(),
    };
    div().id("settings-page").size_full().overflow_y_scroll().child(
        div()
            .pl(inset)
            .pr(px(if narrow { 24. } else { 48. }))
            .pt(px(28.))
            .pb(px(64.))
            .flex()
            .when(narrow, |page| page.flex_col().gap(px(26.)))
            .when(!narrow, |page| page.gap(px(NAV_GAP)))
            .child(nav)
            .child(div().flex_1().min_w_0().max_w(px(CONTENT_WIDTH)).child(content)),
    )
}

fn providers(data: ProvidersData, theme: &Theme) -> impl IntoElement {
    let mut list = div().flex().flex_col();
    if data.connections.is_empty() {
        list = list.child(
            div()
                .py(px(14.))
                .typography(Type::Body)
                .text_color(theme.text_dim)
                .child("No connections yet. Add one below to start chatting."),
        );
    }
    for (index, connection) in data.connections.iter().enumerate() {
        list = list.child(connection_row(index, connection, theme));
    }

    div()
            .flex()
            .flex_col()
            .child(div().typography(Type::Title).text_color(theme.text).child("Providers"))
            .child(div().pt(px(8.)).max_w(px(560.)).typography(Type::Body).text_color(theme.text_soft).child(
                "Connections to model APIs. A key is kept in WackCode’s own secrets file and only ever \
                 sent to the address it belongs to.",
            ))
            .child(div().pt(px(24.)).child(list))
            .child(div().pt(px(40.)).typography(Type::Heading).text_color(theme.text).child("Add a connection"))
            .child(form(data, theme))
}

fn connection_row(index: usize, connection: &Connection, theme: &Theme) -> impl IntoElement {
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
    div()
        .id(("connection", index))
        .group("connection")
        .py(px(14.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_start()
        .gap(px(16.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().typography(Type::UiStrong).text_color(theme.text).child(connection.name.clone()))
                .child(
                    div()
                        .flex()
                        .gap(px(10.))
                        .typography(Type::Small)
                        .text_color(theme.text_dim)
                        .child(format_name(connection.format))
                        .child(div().text_color(theme.text_faint).child("·"))
                        .child(div().min_w_0().truncate().child(host)),
                )
                .child(div().pt(px(2.)).typography(Type::Mono).text_color(theme.text_soft).child(models)),
        )
        .child(
            div()
                .id(("remove-connection", index))
                .flex_none()
                .typography(Type::Small)
                .text_color(theme.text_faint)
                .hover(|word| word.text_color(theme.danger))
                .on_click(dispatch(RemoveConnection { id: connection.id.clone() }))
                .child("Remove"),
        )
}

fn form(data: ProvidersData, theme: &Theme) -> impl IntoElement {
    let label_width = px(120.);
    let row = |label: &'static str, control: gpui::AnyElement, note: Option<&'static str>| {
        div()
            .pt(px(14.))
            .flex()
            .items_start()
            .child(div().w(label_width).flex_none().pt(px(7.)).typography(Type::Ui).text_color(theme.text_soft).child(label))
            .child(
                div().flex_1().min_w_0().flex().flex_col().gap(px(5.)).child(control).when_some(note, |column, note| {
                    column.child(div().typography(Type::Small).text_color(theme.text_faint).child(note))
                }),
            )
    };
    let field = |field: Entity<TextField>| {
        div()
            .w_full()
            .px(px(10.))
            .py(px(6.))
            .rounded(px(8.))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .child(field)
            .into_any_element()
    };
    let formats = div()
        .flex()
        .gap(px(18.))
        .pt(px(6.))
        .children([WireFormat::OpenaiChat, WireFormat::OpenaiResponses, WireFormat::AnthropicMessages].map(|format| {
            let chosen = format == data.format;
            div()
                .id(format_name(format))
                .pb(px(3.))
                .border_b_2()
                .border_color(if chosen { theme.accent } else { gpui::Hsla { a: 0., ..theme.window } })
                .typography(if chosen { Type::UiStrong } else { Type::Ui })
                .text_color(if chosen { theme.text } else { theme.text_dim })
                .hover(|word| word.text_color(theme.text))
                .on_click(dispatch(ChooseFormat { format }))
                .child(format_name(format))
        }))
        .into_any_element();

    div()
        .flex()
        .flex_col()
        .max_w(label_width + px(520.))
        .child(row("Name", field(data.name), None))
        .child(row("Format", formats, Some("Chat Completions suits most servers, including local ones.")))
        .child(row("Address", field(data.address), Some("The API’s base address, without /chat/completions.")))
        .child(row("Models", field(data.models), Some("Model ids, separated by commas or spaces.")))
        .child(row("Context", field(data.context), Some("Tokens each model can read, such as 200k. Optional; it decides when chats compact.")))
        .child(row("API key", field(data.key), None))
        .child(
            div()
                .pt(px(20.))
                .pl(label_width)
                .flex()
                .items_center()
                .gap(px(16.))
                .child(
                    div()
                        .id("add-connection")
                        .h(px(32.))
                        .px(px(14.))
                        .rounded(px(8.))
                        .flex()
                        .items_center()
                        .bg(theme.accent)
                        .hover(|button| button.bg(theme.accent_hover))
                        .on_click(dispatch(AddConnection))
                        .child(div().typography(Type::UiStrong).text_color(theme.on_accent).child("Add connection"))
                        .tag("add-connection"),
                )
                .when_some(data.error, |row, error| {
                    row.child(div().flex_1().min_w_0().typography(Type::Ui).text_color(theme.danger).child(error))
                }),
        )
}

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
