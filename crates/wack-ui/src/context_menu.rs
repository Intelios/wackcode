//! A right-click menu: a raised card of rows at the pointer, over everything, as the pickers'
//! popovers are. A click anywhere else closes it. Each row reports its choice as an action, as
//! the rest of the UI does, so whoever opened the menu handles it.
//!
//! A chat's menu ([`chat_entries`]) is version 2's, less what version 3 cut (forking, worktrees):
//! Rename, the chat's folder in the Finder, its session file, its path, Close tab and Archive.
//! Sidebar rows, title-strip tabs and the chat header open it.
//!
//! A message's menu ([`prompt_entries`]), from right-clicking one of the user's messages, is
//! version 2's too, less forking: Copy, Edit, Rewind to here and, on the latest, Retry.

use gpui::{Action, AnchoredPositionMode, AnyElement, MouseButton, Pixels, Point, SharedString, Size, anchored, deferred, div, prelude::*, px};

use crate::{
    actions::{ChatPick, ChatMenuPick, CopyPrompt, EditPrompt, RetryPrompt, RewindPrompt},
    icon::Icon,
    tags::Tag,
    theme::Theme,
    typography::{Type, Typography},
};

/// One row of a menu, or a rule between groups.
pub enum Entry {
    Item { id: SharedString, label: SharedString, icon: Icon, enabled: bool, action: Box<dyn Action> },
    Separator,
}

/// The menu at `position` (window points), with `close` dispatched by a click elsewhere and
/// before any row's own action.
pub fn menu(name: &'static str, position: Point<Pixels>, entries: Vec<Entry>, close: Box<dyn Action>, viewport: Size<Pixels>, theme: &Theme) -> AnyElement {
    let backdrop_close = close.boxed_clone();
    let backdrop = anchored().position_mode(AnchoredPositionMode::Window).position(gpui::point(px(0.), px(0.))).child(
        div()
            .id(SharedString::from(format!("{name}-backdrop")))
            .w(viewport.width)
            .h(viewport.height)
            .on_mouse_down(MouseButton::Left, {
                let close = backdrop_close.boxed_clone();
                move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(close.boxed_clone(), cx);
                }
            })
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(backdrop_close.boxed_clone(), cx);
            }),
    );
    let mut card = div()
        .id(SharedString::from(name))
        // Presses inside stay inside: the click-away layer beneath must not close it first.
        .occlude()
        .w(px(MENU_WIDTH))
        .p(px(5.))
        .rounded(px(12.))
        .bg(theme.raised)
        .border_1()
        .border_color(theme.border_strong)
        .shadow_lg()
        .flex()
        .flex_col();
    for entry in entries {
        card = card.child(match entry {
            Entry::Separator => div().my(px(4.)).mx(px(8.)).h(px(1.)).bg(theme.border).into_any_element(),
            Entry::Item { id, label, icon, enabled, action } => {
                let close = close.boxed_clone();
                div()
                    .id(id)
                    .h(px(30.))
                    .px(px(10.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .when(enabled, |row| {
                        row.cursor_pointer().hover(|row| row.bg(theme.hover)).on_click(move |_, window, cx| {
                            window.dispatch_action(close.boxed_clone(), cx);
                            window.dispatch_action(action.boxed_clone(), cx);
                        })
                    })
                    .child(div().w(px(14.)).flex_none().flex().justify_center().child(icon.view(px(13.), if enabled { theme.text_dim } else { theme.text_faint })))
                    .child(div().flex_1().min_w_0().truncate().typography(Type::Ui).text_color(if enabled { theme.text } else { theme.text_faint }).child(label))
                    .into_any_element()
            }
        });
    }
    let card = anchored()
        .position_mode(AnchoredPositionMode::Window)
        .position(position)
        .snap_to_window_with_margin(px(8.))
        .child(crate::motion::enter(SharedString::from(format!("arrive-{name}")), div().child(card.tag("context-menu"))));
    div().child(deferred(backdrop).with_priority(3)).child(deferred(card).with_priority(4)).into_any_element()
}

const MENU_WIDTH: f32 = 224.;

/// What a chat's menu offers, as [`chat_entries`] lists it.
pub struct ChatFacts {
    pub chat: String,
    /// A Chat area chat: its folder is a scratchpad, not a project.
    pub chat_area: bool,
    /// Its session file is on disk (a chat writes one once the model has answered).
    pub has_file: bool,
    /// It has a tab in the title strip.
    pub tab_open: bool,
    /// Opened from its sidebar row: Rename edits the row there.
    pub sidebar: bool,
}

pub fn chat_entries(facts: &ChatFacts) -> Vec<Entry> {
    let item = |pick: ChatPick, label: &'static str, icon: Icon, enabled: bool| Entry::Item {
        id: SharedString::from(format!("chat-menu-{pick:?}")),
        label: label.into(),
        icon,
        enabled,
        action: Box::new(ChatMenuPick { chat: facts.chat.clone(), pick, sidebar: facts.sidebar }),
    };
    let mut entries = vec![
        item(ChatPick::Rename, "Rename", Icon::Pencil, true),
        Entry::Separator,
        item(ChatPick::Reveal, if facts.chat_area { "Reveal scratchpad" } else { "Reveal in Finder" }, Icon::Folder, true),
        item(ChatPick::RevealFile, "Reveal chat file", Icon::File, facts.has_file),
        item(ChatPick::CopyPath, if facts.chat_area { "Copy scratchpad path" } else { "Copy path" }, Icon::Copy, true),
        Entry::Separator,
    ];
    if facts.tab_open {
        entries.push(item(ChatPick::CloseTab, "Close tab", Icon::Close, true));
    }
    // Deleting is deliberately absent: a chat goes through Archived before it can be removed.
    entries.push(item(ChatPick::Archive, "Archive", Icon::Archive, true));
    entries
}

/// What a message's menu offers, as [`prompt_entries`] lists it.
#[derive(Clone, Copy, Debug)]
pub struct PromptFacts {
    /// Which of the user's messages (from 0).
    pub number: usize,
    /// No run holds the chat: it can be edited, rewound or retried.
    pub idle: bool,
    /// The user's latest message, which Retry asks again.
    pub latest: bool,
}

pub fn prompt_entries(facts: &PromptFacts) -> Vec<Entry> {
    let number = facts.number;
    let item = |id: &'static str, label: &'static str, icon: Icon, enabled: bool, action: Box<dyn Action>| Entry::Item { id: id.into(), label: label.into(), icon, enabled, action };
    let mut entries = vec![
        item("prompt-menu-copy", "Copy", Icon::Copy, true, Box::new(CopyPrompt { number })),
        Entry::Separator,
        item("prompt-menu-edit", "Edit", Icon::Pencil, facts.idle, Box::new(EditPrompt { number })),
        item("prompt-menu-rewind", "Rewind to here", Icon::Rewind, facts.idle, Box::new(RewindPrompt { number })),
    ];
    if facts.latest {
        entries.push(item("prompt-menu-retry", "Retry", Icon::Reload, facts.idle, Box::new(RetryPrompt { number })));
    }
    entries
}
