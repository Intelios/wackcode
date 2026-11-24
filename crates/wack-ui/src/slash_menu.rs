//! The `/` menu: the commands and skills a `/word` in the composer could mean, in a card that
//! rises above the composer's.
//!
//! - It opens on a `/` at the start of the text or after a space, with the caret in the word, and
//!   follows what is typed (`wack_core::skills::active_slash` and `matching`: names that start
//!   with it first). A second `/` makes the word a path, which never opens it.
//! - While it shows matches it has the field's keys (`TextField::set_completing`): ↑ ↓ move and
//!   wrap, ⇥ or ↩ pick, ⎋ closes it for that word. Picking moves the command to the front of the
//!   message, the rest becoming its arguments, as version 2 did.
//! - The catalogue is read from disk each time the menu opens, so a skill saved in another app
//!   shows at once; the state lives on `AppState::slash`, never in the view.
//! - In a Chat chat the same card is the `@` picker: an `@word` lists the Chat area's other chats
//!   (`wack_core::chat_refs::active_mention` and `rank`), and picking one writes `@"Name"` in its
//!   place and keeps the chat's id for the message's referenced-chats section.

use std::ops::Range;

use gpui::{ElementId, ScrollHandle, SharedString, div, prelude::*, px};
use wack_core::{
    chat_refs::ChatRef,
    skills::{Command, Kind},
};

use crate::{
    actions::PickCommand,
    icon::Icon,
    tags::Tag,
    theme::Theme,
    typography::{Type, Typography},
};

/// The menu open over one composer.
pub struct Slash {
    /// Whose composer: a chat's id, or `None` for the new chat's.
    pub owner: Option<String>,
    /// The `/word` in the composer's text.
    pub range: Range<usize>,
    /// What follows the `/` or `@`, up to the caret.
    pub query: String,
    pub matches: Matches,
    pub selected: usize,
    pub scroll: ScrollHandle,
}

/// What the menu offers: commands for a `/word`, chats (each with its age) for an `@word`.
pub enum Matches {
    Commands(Vec<Command>),
    Chats(Vec<(ChatRef, String)>),
}

impl Matches {
    pub fn len(&self) -> usize {
        match self {
            Matches::Commands(commands) => commands.len(),
            Matches::Chats(chats) => chats.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Slash {
    pub fn data(&self) -> SlashData {
        match &self.matches {
            Matches::Commands(commands) => SlashData {
                rows: commands.iter().map(SlashRow::from).collect(),
                selected: self.selected,
                scroll: self.scroll.clone(),
                heading: None,
                empty: "No matching commands",
            },
            Matches::Chats(chats) => SlashData {
                rows: chats
                    .iter()
                    .map(|(chat, age)| SlashRow {
                        name: chat.name.clone().into(),
                        hint: None,
                        description: SharedString::default(),
                        source: age.clone().into(),
                        icon: Icon::Comment,
                    })
                    .collect(),
                selected: self.selected,
                scroll: self.scroll.clone(),
                heading: Some("Chats"),
                empty: if self.query.is_empty() { "No other chats to reference" } else { "No matching chats" },
            },
        }
    }
}

/// What the menu draws.
#[derive(Clone, Debug)]
pub struct SlashData {
    pub rows: Vec<SlashRow>,
    pub selected: usize,
    pub scroll: ScrollHandle,
    /// A small heading over the rows ("Chats" for the `@` picker).
    pub heading: Option<&'static str>,
    /// What shows when nothing matches.
    pub empty: &'static str,
}

impl SlashData {
    /// The `/` menu over `rows`.
    pub fn commands(rows: Vec<SlashRow>) -> Self {
        Self { rows, selected: 0, scroll: ScrollHandle::new(), heading: None, empty: "No matching commands" }
    }
}

#[derive(Clone, Debug)]
pub struct SlashRow {
    pub name: SharedString,
    pub hint: Option<SharedString>,
    pub description: SharedString,
    pub source: SharedString,
    pub icon: Icon,
}

impl From<&Command> for SlashRow {
    fn from(command: &Command) -> Self {
        Self {
            name: format!("/{}", command.name).into(),
            hint: command.hint.clone().map(Into::into),
            // A long description reads as one line here; Settings shows it whole.
            description: command.description.split_whitespace().collect::<Vec<_>>().join(" ").into(),
            source: command.source.clone().into(),
            icon: icon(command.kind),
        }
    }
}

/// The note under a bare `/command`: what it takes after it.
pub fn hint_note(command: &Command) -> Option<SharedString> {
    let hint = command.hint.as_deref()?;
    Some(
        match command.kind {
            Kind::Skill => format!("{hint}  ·  added after the skill's instructions"),
            _ if hint.starts_with('<') => format!("{hint}  ·  needed"),
            _ if hint.starts_with('[') => format!("{hint}  ·  optional"),
            _ => hint.to_string(),
        }
        .into(),
    )
}

pub fn icon(kind: Kind) -> Icon {
    match kind {
        Kind::Builtin => Icon::Slash,
        Kind::Template => Icon::Pencil,
        Kind::Skill => Icon::Book,
    }
}

/// About seven rows; more scroll, the picked one kept in view.
const LIST_MAX_HEIGHT: f32 = 300.;
const ROW_HEIGHT: f32 = 38.;

/// The card. It sits wherever its holder puts it (above the composer card, full width).
pub fn menu(data: SlashData, theme: &Theme) -> impl IntoElement {
    let mut list = div()
        .id("slash-list")
        .max_h(px(LIST_MAX_HEIGHT))
        .overflow_y_scroll()
        .track_scroll(&data.scroll)
        .flex()
        .flex_col()
        .px(px(6.))
        .py(px(6.));
    if let Some(heading) = data.heading {
        list = list.child(div().flex_none().px(px(10.)).pt(px(4.)).pb(px(2.)).typography(Type::Label).text_color(theme.text_faint).child(heading));
    }
    if data.rows.is_empty() {
        list = list.child(div().px(px(10.)).h(px(ROW_HEIGHT)).flex().items_center().typography(Type::Ui).text_color(theme.text_dim).child(data.empty));
    }
    for (index, row) in data.rows.into_iter().enumerate() {
        let selected = index == data.selected;
        list = list.child(
            div()
                .id(ElementId::Name(format!("slash-row-{index}").into()))
                .flex_none()
                .h(px(ROW_HEIGHT))
                .px(px(10.))
                .rounded(px(10.))
                .flex()
                .items_center()
                .gap(px(10.))
                .when(selected, |row| row.bg(theme.hover))
                .when(!selected, |row| row.hover(|row| row.bg(theme.hover)))
                .on_click(move |_, window, cx| window.dispatch_action(Box::new(PickCommand { index }), cx))
                .child(row.icon.view(px(14.), if selected { theme.accent_text } else { theme.text_dim }))
                .child(
                    div()
                        .flex_none()
                        .max_w(px(260.))
                        .flex()
                        .items_baseline()
                        .gap(px(6.))
                        .overflow_hidden()
                        .child(div().flex_none().typography(Type::UiStrong).text_color(theme.text).child(row.name))
                        .when_some(row.hint, |name, hint| {
                            name.child(div().min_w_0().truncate().typography(Type::Small).italic().text_color(theme.text_faint).child(hint))
                        }),
                )
                .child(div().flex_1().min_w_0().truncate().typography(Type::Small).text_color(theme.text_dim).child(row.description))
                .child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(row.source)),
        );
    }
    div()
        .id("slash-menu")
        .occlude()
        .w_full()
        .rounded(px(16.))
        .bg(theme.raised)
        .border_1()
        .border_color(theme.border_strong)
        .shadow_lg()
        .flex()
        .flex_col()
        .child(list)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(14.))
                .px(px(16.))
                .py(px(7.))
                .border_t_1()
                .border_color(theme.border)
                .typography(Type::Small)
                .text_color(theme.text_faint)
                .child("↑↓ move")
                .child("↩ pick")
                .child("⎋ close"),
        )
        .tag("slash-menu")
}
