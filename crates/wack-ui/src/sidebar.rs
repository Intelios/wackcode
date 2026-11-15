//! The sidebar: projects and their chats.
//!
//! Typographic, not boxed: no monogram tiles, no pill buttons. Pinned projects come first under
//! their own label; a project's chats are listed under it when it is open. The selected chat is
//! the only filled row, in the accent wash, so the eye finds it at once. A row's own controls (new
//! chat, pin, remove) appear only while the pointer is over it.
//!
//! Renders from plain data ([`SidebarData`]) and reports clicks as actions (`crate::actions`); it
//! holds no state of its own.

use gpui::{App, FontWeight, Hsla, SharedString, Window, div, prelude::*, px};

use crate::{
    actions::{AddFolder, DeleteChat, NewChat, NewChatIn, OpenSettings, PinProject, RemoveProject, SelectChat, ToggleProject},
    icon::Icon,
    tags::Tag,
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
    pub id: SharedString,
    pub name: SharedString,
    /// Shown only when `open`.
    pub chats: Vec<ChatRow>,
    pub open: bool,
    pub pinned: bool,
}

#[derive(Clone, Debug)]
pub struct ChatRow {
    pub id: SharedString,
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

fn dispatch(action: impl gpui::Action) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static {
    let action = action.boxed_clone();
    move |_, window, cx| {
        cx.stop_propagation();
        window.dispatch_action(action.boxed_clone(), cx)
    }
}

impl RenderOnce for Sidebar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let empty = self.data.pinned.is_empty() && self.data.projects.is_empty();

        let mut list = div().flex().flex_col().px(px(10.)).pt(px(10.)).pb(px(16.)).gap(px(2.));
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
            // No ground of its own: the shell lays the sidebar's, a tint when a picture is behind.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px(px(10.))
                    .gap(px(1.))
                    .child(action("new-chat", Icon::Plus, "New chat", "⌘N", true, NewChat, &theme)),
            )
            .child(div().id("sidebar-list").flex_1().min_h_0().overflow_y_scroll().child(list))
            .child(
                div()
                    .flex_none()
                    .px(px(10.))
                    .pb(px(12.))
                    .child(action("settings", Icon::Settings, "Settings", "⌘,", false, OpenSettings, &theme)),
            )
    }
}

/// A command row: icon, name, shortcut.
fn action(
    tag: &'static str,
    icon: Icon,
    name: &'static str,
    keys: &'static str,
    strong: bool,
    action: impl gpui::Action,
    theme: &Theme,
) -> impl IntoElement {
    div()
        .id(tag)
        .h(px(30.))
        .px(px(10.))
        .rounded(px(7.))
        .flex()
        .items_center()
        .gap(px(9.))
        .hover(|row| row.bg(theme.hover))
        .on_click(dispatch(action))
        .child(icon.view(px(15.), if strong { theme.text } else { theme.text_dim }))
        .child(
            div()
                .flex_1()
                .typography(if strong { Type::UiStrong } else { Type::Ui })
                .text_color(if strong { theme.text } else { theme.text_soft })
                .child(name),
        )
        .child(div().typography(Type::Small).text_color(theme.text_faint).child(keys))
        .tag(tag)
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
        .child(div().typography(Type::Label).text_color(theme.text_dim).child(name))
        .when(with_add, |row| {
            row.child(
                small_button("add-folder", Icon::Plus, theme.text_faint, theme)
                    .on_click(dispatch(AddFolder))
                    .tag("add-folder"),
            )
        })
}

/// A 20-point icon button, for a row's own controls.
fn small_button(id: impl Into<gpui::ElementId>, icon: Icon, colour: Hsla, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(20.))
        .rounded(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .hover(|button| button.bg(theme.border))
        .child(icon.view(px(13.), colour))
}

fn project_rows(project: ProjectRow, theme: &Theme) -> impl IntoElement {
    let has_selection = project.chats.iter().any(|chat| chat.selected);
    let group: SharedString = format!("project-{}", project.id).into();
    let id = project.id.to_string();
    // Over the end of the row while it is hovered, on the hover colour so the name runs under it.
    let controls = div()
        .absolute()
        .top_0()
        .bottom_0()
        .right(px(4.))
        .pl(px(6.))
        .bg(theme.hover)
        .flex()
        .items_center()
        .gap(px(2.))
        .invisible()
        .group_hover(group.clone(), |controls| controls.visible())
        .child(
            small_button(SharedString::from(format!("{id}-new")), Icon::Plus, theme.text_dim, theme)
                .on_click(dispatch(NewChatIn { project: id.clone() })),
        )
        .child(
            small_button(
                SharedString::from(format!("{id}-pin")),
                Icon::Pin,
                if project.pinned { theme.accent_text } else { theme.text_dim },
                theme,
            )
            .on_click(dispatch(PinProject { id: id.clone(), pinned: !project.pinned })),
        )
        .child(
            small_button(SharedString::from(format!("{id}-remove")), Icon::Close, theme.text_dim, theme)
                .on_click(dispatch(RemoveProject { id: id.clone() })),
        );
    let count = (!project.open && !project.chats.is_empty()).then(|| {
        div().pr(px(6.)).typography(Type::Small).text_color(theme.text_faint).child(project.chats.len().to_string())
    });
    let header = div()
        .id(SharedString::from(format!("{id}-row")))
        .group(group)
        .relative()
        .h(px(28.))
        .pl(px(10.))
        .pr(px(4.))
        .rounded(px(7.))
        .flex()
        .items_center()
        .gap(px(8.))
        .hover(|row| row.bg(theme.hover))
        .on_click(dispatch(ToggleProject { id: id.clone() }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .typography(Type::UiStrong)
                .text_color(if has_selection || project.open { theme.text } else { theme.text_soft })
                .child(project.name),
        )
        .children(count)
        .child(controls);

    let mut rows = div().flex().flex_col().child(header);
    if project.open {
        for chat in project.chats {
            rows = rows.child(chat_row(chat, theme));
        }
        rows = rows.child(div().h(px(6.)));
    }
    rows
}

fn chat_row(chat: ChatRow, theme: &Theme) -> impl IntoElement {
    let title_colour = if chat.selected || chat.state == ChatState::Unread { theme.text } else { theme.text_soft };
    let group: SharedString = format!("chat-{}", chat.id).into();
    let id = chat.id.to_string();
    let marker = div().flex_none().w(px(14.)).flex().justify_center().map(|marker| match chat.state {
        ChatState::Working => marker.child(crate::motion::paddling(div().child(Icon::Duck.view(px(12.), theme.accent_text)))),
        ChatState::Unread => marker.child(div().size(px(6.)).rounded_full().bg(theme.accent_text)),
        ChatState::Idle => marker,
    });
    div()
        .id(SharedString::from(format!("{id}-chat")))
        .group(group.clone())
        .relative()
        .h(px(28.))
        .ml(px(6.))
        .pl(px(4.))
        .pr(px(4.))
        .rounded(px(7.))
        .flex()
        .items_center()
        .gap(px(6.))
        .map(|row| if chat.selected { row.bg(theme.accent_wash) } else { row.hover(|row| row.bg(theme.hover)) })
        .on_click(dispatch(SelectChat { id: id.clone() }))
        .child(marker)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .typography(Type::Ui)
                .when(chat.state == ChatState::Unread, |text| text.font_weight(FontWeight::MEDIUM))
                .text_color(title_colour)
                .child(chat.title),
        )
        .child(
            div()
                .flex_none()
                .pr(px(6.))
                .group_hover(group.clone(), |age| age.invisible())
                .typography(Type::Small)
                .text_color(if chat.state == ChatState::Working { theme.accent_text } else { theme.text_faint })
                .child(chat.age),
        )
        .child(
            // Over the age, in its place, while the row is hovered.
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(4.))
                .flex()
                .items_center()
                .invisible()
                .group_hover(group, |controls| controls.visible())
                .child(
                small_button(SharedString::from(format!("{id}-delete")), Icon::Close, theme.text_dim, theme)
                    .on_click(dispatch(DeleteChat { id })),
            ),
        )
}
