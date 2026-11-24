//! The sidebar: projects and their chats, or the Chat area's chats.
//!
//! The window has two areas, Code and Chat, and the sidebar shows one at a time: in Code its
//! projects and their chats, in Chat the area's chats in calendar groups (Today, Yesterday,
//! Previous 7 days, Earlier; `chat_area::group_by_recency`). [`AreaSwitch`], in the sidebar's
//! title strip, moves between them, and marks the other area while one of its chats waits for an
//! answer.
//!
//! Round, like the rest of the frame: New chat is the one accent pill at the
//! top, every row is a pill, and a project wears a round monogram, lit while it is open. An open
//! project's chats hang from a dotted rail that drops from its monogram. Pinned projects come
//! first under their own label. The selected chat is the only filled row, in the accent wash, so
//! the eye finds it at once. A row's own controls (new chat, pin, remove) appear only while the
//! pointer is over it.
//!
//! Renders from plain data ([`SidebarData`]) and reports clicks as actions (`crate::actions`); it
//! holds no state of its own.

use gpui::{AnimationExt, App, Entity, FontWeight, Hsla, MouseButton, SharedString, Window, div, prelude::*, px};
use wack_core::ChatKind;

use crate::{
    actions::{
        AddFolder, DeleteChat, EnterChatArea, EnterCodeArea, NewChat, NewChatIn, OpenSettings, PinProject, RemoveProject, RenameChatAt, SelectChat,
        ToggleProject,
    },
    tooltip::tip,
    text_field::TextField,
    icon::Icon,
    tags::Tag,
    theme::Theme,
    typography::{Type, Typography},
};

pub const SIDEBAR_WIDTH: gpui::Pixels = px(252.);
/// A project's monogram, and the padding before it: its chats' rail drops from its middle.
const MONOGRAM: f32 = 20.;
const ROW_INSET: f32 = 4.;

#[derive(Clone, Debug, Default)]
pub struct SidebarData {
    /// The area on screen: Code lists projects, Chat lists `groups`.
    pub area: ChatKind,
    pub pinned: Vec<ProjectRow>,
    pub projects: Vec<ProjectRow>,
    /// The Chat area's chats in calendar groups, newest first.
    pub groups: Vec<ChatGroup>,
    /// A chat in the other area waits for the user to answer its question.
    pub other_waiting: bool,
}

/// One of the Chat area's calendar groups.
#[derive(Clone, Debug)]
pub struct ChatGroup {
    pub label: SharedString,
    pub chats: Vec<ChatRow>,
}

/// What the Chat area's empty sidebar says, as version 2 did.
pub const CHAT_AREA_EMPTY: &str = "Ask anything. A chat here has no project: just a model, a few tools and a scratchpad.";

#[derive(Clone, Debug)]
pub struct ProjectRow {
    pub id: SharedString,
    pub name: SharedString,
    /// Shown only when `open`.
    pub chats: Vec<ChatRow>,
    pub open: bool,
    pub pinned: bool,
}

/// How a chat's title is drawn, on its tab and its sidebar row.
#[derive(Clone, Debug, Default)]
pub enum TitleLook {
    #[default]
    Plain,
    /// The model's title arriving.
    Swap(crate::motion::SwapLook),
    /// The user is renaming the chat here.
    Editing(Entity<TextField>),
}

/// The rename field in place of a title: Esc cancels inside it, and clicks stay in it rather
/// than selecting or dragging what it sits on.
pub fn rename_field(field: Entity<TextField>, theme: &Theme) -> impl IntoElement {
    div()
        .key_context(crate::actions::RENAME_CONTEXT)
        .flex_1()
        .min_w(px(96.))
        .h(px(22.))
        .px(px(6.))
        .rounded(px(6.))
        .bg(theme.surface)
        .border_1()
        .border_color(theme.accent_text)
        .flex()
        .items_center()
        .overflow_hidden()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(field)
        .tag("rename-field")
}

#[derive(Clone, Debug)]
pub struct ChatRow {
    pub id: SharedString,
    pub title: SharedString,
    pub look: TitleLook,
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
        if self.data.area == ChatKind::Chat {
            if self.data.groups.is_empty() {
                list = list.child(
                    div().px(px(10.)).pt(px(8.)).typography(Type::Ui).text_color(theme.text_dim).child(CHAT_AREA_EMPTY),
                );
            }
            for group in self.data.groups {
                list = list.child(label_text(group.label, &theme));
                list = list.child(
                    div().flex().flex_col().gap(px(1.)).children(group.chats.into_iter().map(|chat| chat_row(chat, &theme))),
                );
            }
        } else if !self.data.pinned.is_empty() {
            list = list.child(label("Pinned", false, &theme));
            for project in self.data.pinned {
                list = list.child(project_rows(project, &theme));
            }
        }
        if self.data.area == ChatKind::Code && (!self.data.projects.is_empty() || empty) {
            list = list.child(label("Projects", true, &theme));
            for project in self.data.projects {
                list = list.child(project_rows(project, &theme));
            }
        }
        if empty && self.data.area == ChatKind::Code {
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
            .w_full()
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
                    .pt(px(4.))
                    .child(new_chat_pill(&theme)),
            )
            .child(div().id("sidebar-list").flex_1().min_h_0().overflow_y_scroll().child(list))
            .child(
                div()
                    .flex_none()
                    .px(px(10.))
                    .pb(px(12.))
                    .child(settings_row(&theme)),
            )
    }
}

/// The accent pill at the top: a filled circle holding the plus, the words, the shortcut.
fn new_chat_pill(theme: &Theme) -> impl IntoElement {
    div()
        .id("new-chat")
        .h(px(34.))
        .pl(px(6.))
        .pr(px(12.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(10.))
        .bg(theme.accent_wash)
        .border_1()
        .border_color(theme.accent_line)
        .hover(|pill| pill.border_color(theme.accent))
        .on_click(dispatch(NewChat))
        .child(
            div()
                .flex_none()
                .size(px(22.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.accent_fill())
                .child(Icon::Plus.view(px(12.), theme.on_accent)),
        )
        .child(div().flex_1().typography(Type::UiStrong).text_color(theme.text).child("New chat"))
        .child(div().typography(Type::Small).text_color(theme.text_dim).child("⌘N"))
        .tag("new-chat")
}

/// Settings, at the foot.
fn settings_row(theme: &Theme) -> impl IntoElement {
    div()
        .id("settings")
        .h(px(30.))
        .px(px(10.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(9.))
        .hover(|row| row.bg(theme.hover))
        .on_click(dispatch(OpenSettings))
        .child(Icon::Settings.view(px(15.), theme.text_dim))
        .child(div().flex_1().typography(Type::Ui).text_color(theme.text_soft).child("Settings"))
        .child(div().typography(Type::Small).text_color(theme.text_faint).child("⌘,"))
        .tag("settings")
}

/// A project's round monogram: its name's first letter, lit while the project is open.
fn monogram(name: &str, lit: bool, theme: &Theme) -> impl IntoElement {
    let letter: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default();
    div()
        .flex_none()
        .size(px(MONOGRAM))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(if lit { theme.text } else { theme.hover })
        .typography(Type::Small)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if lit { theme.window } else { theme.text_soft })
        .child(letter)
}

/// A Chat area group's name, as a section name.
fn label_text(name: SharedString, theme: &Theme) -> impl IntoElement {
    div().pt(px(14.)).pb(px(4.)).px(px(10.)).child(div().typography(Type::Label).text_color(theme.text_dim).child(name))
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
        .rounded_full()
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
        .my(px(4.))
        .px(px(2.))
        .rounded_full()
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
        .pl(px(ROW_INSET))
        .pr(px(4.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(8.))
        .hover(|row| row.bg(theme.hover))
        .on_click(dispatch(ToggleProject { id: id.clone() }))
        .child(monogram(&project.name, project.open || has_selection, theme))
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

    let mut rows = div().flex().flex_col().gap(px(2.)).child(header);
    if project.open && !project.chats.is_empty() {
        // The rail runs down from the middle of the monogram.
        let rail = div()
            .ml(px(ROW_INSET + MONOGRAM / 2.))
            .mb(px(6.))
            .pl(px(6.))
            .border_l_1()
            .border_dashed()
            .border_color(theme.border_strong)
            .flex()
            .flex_col()
            .gap(px(1.))
            .children(project.chats.into_iter().map(|chat| chat_row(chat, theme)));
        rows = rows.child(rail);
    }
    rows
}

fn chat_row(chat: ChatRow, theme: &Theme) -> impl IntoElement {
    let title_colour = if chat.selected || chat.state == ChatState::Unread { theme.text } else { theme.text_soft };
    let group: SharedString = format!("chat-{}", chat.id).into();
    let id = chat.id.to_string();
    let editing = matches!(chat.look, TitleLook::Editing(_));
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
        .pl(px(4.))
        .pr(px(4.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(6.))
        .map(|row| if chat.selected { row.bg(theme.accent_wash) } else { row.hover(|row| row.bg(theme.hover)) })
        .on_click({
            let id = id.clone();
            move |event, window, cx| {
                let action: Box<dyn gpui::Action> = if event.click_count() == 2 {
                    Box::new(RenameChatAt { chat: id.clone(), sidebar: true })
                } else {
                    Box::new(SelectChat { id: id.clone() })
                };
                window.dispatch_action(action, cx);
            }
        })
        .child(marker)
        .child(match chat.look {
            TitleLook::Editing(field) => rename_field(field, theme).into_any_element(),
            look => {
                let words = |text: SharedString| {
                    div()
                        .min_w_0()
                        .truncate()
                        .typography(Type::Ui)
                        .when(chat.state == ChatState::Unread, |text| text.font_weight(FontWeight::MEDIUM))
                        .text_color(title_colour)
                        .child(text)
                };
                // The model's title crossfades in over the old one.
                match look {
                    TitleLook::Swap(swap) => div()
                        .flex_1()
                        .min_w_0()
                        .relative()
                        .child(words(chat.title).opacity(swap.row))
                        .child(div().absolute().top_0().left_0().right_0().child(words(swap.from).opacity(1. - swap.row)))
                        .into_any_element(),
                    _ => words(chat.title).flex_1().into_any_element(),
                }
            }
        })
        .when(!editing, |row| row.child(
            div()
                .flex_none()
                .pr(px(6.))
                .group_hover(group.clone(), |age| age.invisible())
                .typography(Type::Small)
                .text_color(if chat.state == ChatState::Working { theme.accent_text } else { theme.text_faint })
                .child(chat.age),
        ))
        .when(!editing, |row| row.child(
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
        ))
}

/// The switch between the window's two areas, in the sidebar's title strip: a round track with
/// a pill that glides to the area on screen. A dot on the other area says one of its chats is
/// waiting for an answer.
#[derive(IntoElement)]
pub struct AreaSwitch {
    pub area: ChatKind,
    pub other_waiting: bool,
}

/// Each segment's width and height, and the track's padding.
const SEGMENT: (f32, f32) = (34., 22.);
const SWITCH_PAD: f32 = 2.;
const GLIDE: std::time::Duration = std::time::Duration::from_millis(240);

/// Which area the switch last showed and how many times it has moved, so a move glides and a
/// redraw does not.
struct Shown {
    area: ChatKind,
    from: ChatKind,
    moves: usize,
}

fn segment_left(area: ChatKind) -> f32 {
    match area {
        ChatKind::Code => SWITCH_PAD,
        ChatKind::Chat => SWITCH_PAD + SEGMENT.0,
    }
}

impl RenderOnce for AreaSwitch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let area = self.area;
        let shown = window.use_keyed_state("area-switch", cx, |_, _| Shown { area, from: area, moves: 0 });
        let (from, moves) = shown.update(cx, |shown, _| {
            if shown.area != area {
                shown.from = shown.area;
                shown.area = area;
                shown.moves += 1;
            }
            (shown.from, shown.moves)
        });
        let (to_left, from_left) = (segment_left(area), segment_left(from));
        let pill = div()
            .absolute()
            .top(px(SWITCH_PAD))
            .w(px(SEGMENT.0))
            .h(px(SEGMENT.1))
            .rounded_full()
            .bg(theme.window)
            .border_1()
            .border_color(theme.border_strong);
        let pill = if moves == 0 {
            pill.left(px(to_left)).into_any_element()
        } else {
            pill.with_animation(("area-pill", moves), gpui::Animation::new(GLIDE).with_easing(crate::motion::spring), move |pill, t| {
                pill.left(px(from_left + (to_left - from_left) * t))
            })
            .into_any_element()
        };
        let segment = |id: &'static str, icon: Icon, target: ChatKind, tooltip: &'static str, action: Box<dyn gpui::Action>| {
            let on = target == area;
            let waiting = self.other_waiting && !on;
            div()
                .id(id)
                .relative()
                .flex_none()
                .w(px(SEGMENT.0))
                .h(px(SEGMENT.1))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .when(!on, |segment| segment.hover(|segment| segment.bg(theme.hover)))
                .tooltip(tip(if waiting { format!("{tooltip} · a chat is waiting for your answer") } else { tooltip.to_string() }))
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(action.boxed_clone(), cx);
                })
                .child(icon.view(px(14.), if on { theme.text } else { theme.text_dim }))
                .when(waiting, |segment| {
                    segment.child(div().absolute().top(px(3.)).right(px(6.)).size(px(6.)).rounded_full().bg(theme.accent_text))
                })
                .tag(id)
        };
        div()
            .id("area-switch")
            .occlude()
            .flex_none()
            .relative()
            .h(px(SEGMENT.1 + 2. * SWITCH_PAD))
            .px(px(SWITCH_PAD))
            .flex()
            .items_center()
            .rounded_full()
            .bg(theme.hover)
            .child(pill)
            .child(segment("area-code", Icon::Code, ChatKind::Code, "Code  ⌥⌘1", Box::new(EnterCodeArea)))
            .child(segment("area-chat", Icon::Comment, ChatKind::Chat, "Chat  ⌥⌘2", Box::new(EnterChatArea)))
            .tag("area-switch")
    }
}
