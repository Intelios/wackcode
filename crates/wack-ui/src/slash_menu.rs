//! The `/` and `@` picker card above the composer.
//!
//! - In Code composers, a `/word` at the start or after whitespace opens commands and skills
//!   (`wack_core::skills::active_slash` and `matching`), and an `@word` opens workspace files and
//!   folders (`wack_core::files`: git-tracked + non-ignored untracked files up to 20,000, or a
//!   bounded walk outside Git). Picking a file writes `@path ` (quoted when it has spaces);
//!   picking a folder writes `@path/` (or `@"path/`) and keeps the picker open inside it.
//! - In Chat-area composers, an `@word` lists the Chat area's other chats
//!   (`wack_core::chat_refs::active_mention` and `rank`), and picking one writes `@"Name"` in its
//!   place and keeps the chat's id for the message's referenced-chats section.
//! - While it shows matches it borrows the field's keys (`TextField::set_completing`): ↑ ↓ move
//!   and wrap, ⇥ or ↩ pick, ⎋ closes it for that word. State lives on `AppState::slash`, never in
//!   the view.

use std::ops::Range;

use gpui::{ElementId, ScrollHandle, SharedString, div, prelude::*, px};
use wack_core::{
    chat_refs::ChatRef,
    files::FileEntry,
    skills::{Command, Kind},
};

use crate::{
    actions::{PickCommand, RetryWorkspaceFiles},
    icon::Icon,
    tags::Tag,
    theme::Theme,
    typography::{Type, Typography},
};

/// The menu open over one composer.
pub struct Slash {
    /// Whose composer: a chat's id, or `None` for the new chat's.
    pub owner: Option<String>,
    /// The `/word` or `@word` in the composer's text.
    pub range: Range<usize>,
    /// What follows the `/` or `@`, up to the caret.
    pub query: String,
    pub matches: Matches,
    pub selected: usize,
    pub scroll: ScrollHandle,
}

/// What the menu offers: commands for `/word`, workspace files/folders for Code's `@word`, or
/// other chats (each with its age) for the Chat area's `@word`.
pub enum Matches {
    Commands(Vec<Command>),
    Chats(Vec<(ChatRef, String)>),
    Files {
        entries: Vec<FileEntry>,
        loading: bool,
        truncated: bool,
        error: Option<String>,
    },
}

impl Matches {
    pub fn len(&self) -> usize {
        match self {
            Matches::Commands(commands) => commands.len(),
            Matches::Chats(chats) => chats.len(),
            Matches::Files { entries, .. } => entries.len(),
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
                error: None,
                footnote: None,
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
                        mono: false,
                        accent_icon: false,
                    })
                    .collect(),
                selected: self.selected,
                scroll: self.scroll.clone(),
                heading: Some("Chats"),
                empty: if self.query.is_empty() { "No other chats to reference" } else { "No matching chats" },
                error: None,
                footnote: None,
            },
            Matches::Files { entries, loading, truncated, error } => SlashData {
                rows: entries.iter().map(SlashRow::from).collect(),
                selected: self.selected,
                scroll: self.scroll.clone(),
                heading: Some("Files"),
                empty: if *loading {
                    "Loading files…"
                } else if self.query.is_empty() {
                    "No files"
                } else {
                    "No matching files"
                },
                error: error.clone().map(Into::into),
                footnote: truncated.then(|| "Only the first 20,000 files are listed.".into()),
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
    /// A small heading over the rows ("Files" or "Chats" for the `@` picker).
    pub heading: Option<&'static str>,
    /// What shows when nothing matches.
    pub empty: &'static str,
    /// A load error (shown with a Retry button instead of the empty line).
    pub error: Option<SharedString>,
    /// A quiet note pinned beneath the list (e.g. the 20,000-file cap).
    pub footnote: Option<SharedString>,
}

impl SlashData {
    /// The `/` menu over `rows`.
    pub fn commands(rows: Vec<SlashRow>) -> Self {
        Self {
            rows,
            selected: 0,
            scroll: ScrollHandle::new(),
            heading: None,
            empty: "No matching commands",
            error: None,
            footnote: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SlashRow {
    pub name: SharedString,
    pub hint: Option<SharedString>,
    pub description: SharedString,
    pub source: SharedString,
    pub icon: Icon,
    /// Draw `description` in the mono face (a file's parent folder).
    pub mono: bool,
    /// Tint the row's icon with the accent even when not selected (a folder).
    pub accent_icon: bool,
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
            mono: false,
            accent_icon: false,
        }
    }
}

impl From<&FileEntry> for SlashRow {
    fn from(entry: &FileEntry) -> Self {
        Self {
            name: entry.name().to_string().into(),
            hint: None,
            description: entry.parent().to_string().into(),
            source: SharedString::default(),
            icon: if entry.directory { Icon::Folder } else { Icon::File },
            mono: true,
            accent_icon: entry.directory,
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
    if let Some(error) = data.error {
        list = list.child(
            div()
                .min_h(px(ROW_HEIGHT))
                .px(px(10.))
                .py(px(6.))
                .flex()
                .items_center()
                .gap(px(10.))
                .child(div().flex_1().min_w_0().typography(Type::Ui).text_color(theme.text_dim).child(error))
                .child(
                    div()
                        .id("retry-workspace-files")
                        .flex_none()
                        .h(px(26.))
                        .px(px(10.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .bg(theme.hover)
                        .border_1()
                        .border_color(theme.border)
                        .hover(|button| button.border_color(theme.border_strong))
                        .typography(Type::Small)
                        .text_color(theme.text)
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(RetryWorkspaceFiles), cx))
                        .child("Retry")
                        .tag("retry-files"),
                )
                .tag("slash-menu-error"),
        );
    } else if data.rows.is_empty() {
        list = list.child(div().px(px(10.)).h(px(ROW_HEIGHT)).flex().items_center().typography(Type::Ui).text_color(theme.text_dim).child(data.empty));
    }
    for (index, row) in data.rows.into_iter().enumerate() {
        let selected = index == data.selected;
        let has_hint = row.hint.is_some();
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
                .child(row.icon.view(px(14.), if selected || row.accent_icon { theme.accent_text } else { theme.text_dim }))
                .child(
                    div()
                        .flex_none()
                        .max_w(px(260.))
                        .flex()
                        .items_baseline()
                        .gap(px(6.))
                        .overflow_hidden()
                        .child(
                            div()
                                .when(!has_hint, |name| name.min_w_0().truncate())
                                .when(has_hint, |name| name.flex_none())
                                .typography(Type::UiStrong)
                                .text_color(theme.text)
                                .child(row.name),
                        )
                        .when_some(row.hint, |name, hint| {
                            name.child(div().min_w_0().truncate().typography(Type::Small).italic().text_color(theme.text_faint).child(hint))
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .typography(if row.mono { Type::Mono } else { Type::Small })
                        .text_color(theme.text_dim)
                        .child(row.description),
                )
                .when(!row.source.is_empty(), |item| {
                    item.child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(row.source))
                }),
        );
    }
    // Keep the heading and footnote outside `slash-list` so they stay visible while scrolling and
    // child index `i` of `slash-list` always matches row `i` for `scroll_to_item`.
    let body = div()
        .flex()
        .flex_col()
        .when_some(data.heading, |body, heading| {
            body.child(div().flex_none().px(px(16.)).pt(px(10.)).pb(px(2.)).typography(Type::Label).text_color(theme.text_faint).child(heading))
        })
        .child(list)
        .when_some(data.footnote, |body, footnote| {
            body.child(
                div()
                    .px(px(16.))
                    .py(px(6.))
                    .border_t_1()
                    .border_color(theme.border)
                    .typography(Type::Small)
                    .text_color(theme.text_faint)
                    .child(footnote)
                    .tag("slash-menu-footnote"),
            )
        });
    picker_card("slash-menu", body, theme).w_full().tag("slash-menu")
}

/// A picker's card: raised, round, the list inside it and the keys that drive it at its foot.
/// The `/` menu and the model catalogue's suggestions (`model_catalog.rs`) share it.
pub(crate) fn picker_card(id: &'static str, list: impl IntoElement, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .occlude()
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
}
