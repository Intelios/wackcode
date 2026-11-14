//! The sidebar: projects and their chats.
//!
//! Typographic, not boxed: no monogram tiles, no pill buttons. Pinned projects come first under
//! their own label; a project's chats are listed under it when it is open. The selected chat is
//! the only filled row, in the accent wash, so the eye finds it at once.
//!
//! Renders from plain data ([`SidebarData`]); it holds no state of its own.

use gpui::{App, FontWeight, SharedString, Window, div, prelude::*, px};

use crate::{
    icon::Icon,
    theme::Theme,
    typography::{Type, Typography},
};

pub const SIDEBAR_WIDTH: gpui::Pixels = px(252.);

#[derive(Clone, Debug, Default)]
pub struct SidebarData {
    pub pinned: Vec<ProjectRow>,
    pub projects: Vec<ProjectRow>,
}

#[derive(Clone, Debug)]
pub struct ProjectRow {
    pub name: SharedString,
    /// Shown only when `open`.
    pub chats: Vec<ChatRow>,
    pub open: bool,
}

#[derive(Clone, Debug)]
pub struct ChatRow {
    pub title: SharedString,
    /// "now", "2h", "3d".
    pub age: SharedString,
    pub state: ChatState,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatState {
    Idle,
    /// The agent is running in this chat.
    Working,
    /// Finished since the user last looked.
    Unread,
}

#[derive(IntoElement)]
pub struct Sidebar {
    data: SidebarData,
}

impl Sidebar {
    pub fn new(data: SidebarData) -> Self {
        Self { data }
    }
}

impl RenderOnce for Sidebar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let empty = self.data.pinned.is_empty() && self.data.projects.is_empty();

        let mut list = div().flex().flex_col().px(px(10.)).pt(px(10.)).gap(px(2.));
        if !self.data.pinned.is_empty() {
            list = list.child(label("Pinned", false, &theme));
            for project in self.data.pinned {
                list = list.child(project_rows(project, &theme));
            }
        }
        if !self.data.projects.is_empty() || empty {
            list = list.child(label("Projects", true, &theme));
            for project in self.data.projects {
                list = list.child(project_rows(project, &theme));
            }
        }
        if empty {
            list = list.child(
                div()
                    .px(px(10.))
                    .pt(px(4.))
                    .typography(Type::Ui)
                    .text_color(theme.text_dim)
                    .child("Add a folder to start a chat in it."),
            );
        }

        div()
            .flex_none()
            .w(SIDEBAR_WIDTH)
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme.sidebar)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px(px(10.))
                    .gap(px(1.))
                    .child(action(Icon::Plus, "New chat", "⌘N", true, &theme))
                    .child(action(Icon::Search, "Search", "⌘K", false, &theme)),
            )
            .child(div().flex_1().min_h_0().overflow_hidden().child(list))
            .child(
                div()
                    .flex_none()
                    .px(px(10.))
                    .pb(px(12.))
                    .child(action(Icon::Settings, "Settings", "⌘,", false, &theme)),
            )
    }
}

/// A command row: icon, name, shortcut.
fn action(
    icon: Icon,
    name: &'static str,
    keys: &'static str,
    strong: bool,
    theme: &Theme,
) -> impl IntoElement {
    div()
        .h(px(30.))
        .px(px(10.))
        .rounded(px(7.))
        .flex()
        .items_center()
        .gap(px(9.))
        .child(icon.view(px(15.), if strong { theme.text } else { theme.text_dim }))
        .child(
            div()
                .flex_1()
                .typography(if strong { Type::UiStrong } else { Type::Ui })
                .text_color(if strong { theme.text } else { theme.text_soft })
                .child(name),
        )
        .child(keys_hint(keys, theme))
}

fn keys_hint(keys: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .typography(Type::Small)
        .text_color(theme.text_faint)
        .child(keys)
}

/// A section name, set in the serif's italic.
fn label(name: &'static str, with_add: bool, theme: &Theme) -> impl IntoElement {
    div()
        .pt(px(14.))
        .pb(px(4.))
        .px(px(10.))
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .typography(Type::Label)
                .text_color(theme.text_dim)
                .child(name),
        )
        .when(with_add, |row| row.child(Icon::Plus.view(px(14.), theme.text_faint)))
}

fn project_rows(project: ProjectRow, theme: &Theme) -> impl IntoElement {
    let has_selection = project.chats.iter().any(|chat| chat.selected);
    let mut rows = div().flex().flex_col().child(
        div()
            .h(px(28.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .typography(Type::UiStrong)
                    .text_color(if has_selection || project.open {
                        theme.text
                    } else {
                        theme.text_soft
                    })
                    .child(project.name),
            )
            .when(!project.open && !project.chats.is_empty(), |row| {
                row.child(
                    div()
                        .typography(Type::Small)
                        .text_color(theme.text_faint)
                        .child(project.chats.len().to_string()),
                )
            }),
    );
    if project.open {
        for chat in project.chats {
            rows = rows.child(chat_row(chat, theme));
        }
        rows = rows.child(div().h(px(6.)));
    }
    rows
}

fn chat_row(chat: ChatRow, theme: &Theme) -> impl IntoElement {
    let title_colour = if chat.selected || chat.state == ChatState::Unread {
        theme.text
    } else {
        theme.text_soft
    };
    let marker = div()
        .flex_none()
        .w(px(14.))
        .flex()
        .justify_center()
        .map(|marker| match chat.state {
            ChatState::Working => marker.child(Icon::Duck.view(px(12.), theme.accent_text)),
            ChatState::Unread => {
                marker.child(div().size(px(6.)).rounded_full().bg(theme.accent_text))
            }
            ChatState::Idle => marker,
        });
    div()
        .h(px(28.))
        .ml(px(6.))
        .pl(px(4.))
        .pr(px(10.))
        .rounded(px(7.))
        .flex()
        .items_center()
        .gap(px(6.))
        .when(chat.selected, |row| row.bg(theme.accent_wash))
        .child(marker)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .typography(Type::Ui)
                .when(chat.state == ChatState::Unread, |text| {
                    text.font_weight(FontWeight::MEDIUM)
                })
                .text_color(title_colour)
                .child(chat.title),
        )
        .child(
            div()
                .flex_none()
                .typography(Type::Small)
                .text_color(if chat.state == ChatState::Working {
                    theme.accent_text
                } else {
                    theme.text_faint
                })
                .child(chat.age),
        )
}
