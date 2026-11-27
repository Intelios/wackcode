//! Rewinding, editing and retrying the user's messages, in the transcript.
//!
//! - Under each message, shown while the pointer is over it, a row of small pills: Copy, and
//!   while no run holds the chat, Edit, Rewind and (on the latest) Retry. Right-clicking the
//!   message offers the same (`context_menu::prompt_entries`).
//! - Edit turns the bubble into a field holding its words, in place: ↩ sends, Esc leaves it.
//!   The pictures and files it carried go again with the new words.
//! - Rewind, and an edit or retry whose files changed since the message, opens a card under the
//!   message: what will happen, and the files changed since it, each ticked to go back as it
//!   was. The engine does the work (`wack_core::rewind`), off the UI thread.
//! - After a rewind, until anything runs, its row at the end of the transcript offers Undo.
//!
//! The state behind these lives in the chat (`ChatSession`); this module only draws it.

use std::collections::BTreeSet;

use gpui::{AnyElement, ElementId, Entity, SharedString, div, prelude::*, px, relative};
use wack_core::{checkpoints::Action, rewind::Preview};

use crate::{
    actions::{CancelEdit, CancelRewind, ConfirmRewind, EditPrompt, RetryPrompt, RewindPrompt, SendEdit, ToggleRewindFile, ToggleRewindFiles, UndoRewind},
    button::{Button, Kind},
    icon::Icon,
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    tooltip::tip,
    typography::{Type, Typography},
};

/// The key context of a message being edited, where Esc leaves it.
pub const EDIT_CONTEXT: &str = "PromptEditor";

/// What the user's messages offer as the transcript draws them, and the one being edited or
/// asked about.
#[derive(Clone, Default)]
pub struct Prompts {
    /// The messages are the chat's own, which it can rewind: off for a sub-agent's, in the
    /// Agents pane, and in scenes, which offer only Copy.
    pub chat: bool,
    /// No run holds the chat and none is being rewound: Edit, Rewind and Retry are offered.
    pub idle: bool,
    /// How many messages the user has sent: the last offers Retry.
    pub count: usize,
    pub editing: Option<(usize, Entity<TextField>)>,
    pub card: Option<Card>,
    /// For scenes: the message whose actions show as if the pointer were over it.
    pub hovered: Option<usize>,
}

/// What the card under a message is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Back to before the message, whose words return to the composer.
    Rewind,
    /// The message again, as edited.
    Edit,
    /// The message again, as it was.
    Retry,
}

/// The card under a message, asking what a rewind, an edit or a retry does.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    /// Which of the user's messages (from 0).
    pub number: usize,
    pub purpose: Purpose,
    /// What the engine found; `None` while it looks.
    pub preview: Option<Preview>,
    /// The changed files to put back as they were.
    pub ticked: BTreeSet<String>,
    /// The rewind is under way.
    pub working: bool,
    pub error: Option<SharedString>,
}

impl Card {
    pub fn new(number: usize, purpose: Purpose) -> Self {
        Self { number, purpose, preview: None, ticked: BTreeSet::new(), working: false, error: None }
    }

    /// The engine has looked: every changed file starts ticked.
    pub fn found(&mut self, preview: Preview) {
        self.ticked = preview.changes.iter().map(|change| change.path.clone()).collect();
        self.preview = Some(preview);
    }

    /// Files changed since the message that a rewind could put back.
    pub fn has_changes(&self) -> bool {
        self.preview.as_ref().is_some_and(|preview| preview.checkpoint.is_some() && !preview.changes.is_empty())
    }
}

/// The row of pills under a message, shown while the pointer is over it (its group is
/// `group`). `copy` is what Copy copies.
pub fn actions(number: usize, copy: SharedString, prompts: &Prompts, group: SharedString, theme: &Theme) -> AnyElement {
    let latest = number + 1 == prompts.count;
    let pill = |id: &'static str, icon: Icon, label: &'static str, hint: &'static str, action: Box<dyn gpui::Action>| {
        div()
            .id(ElementId::NamedInteger(id.into(), number as u64))
            .flex_none()
            .h(px(22.))
            .px(px(8.))
            .rounded_full()
            .flex()
            .items_center()
            .gap(px(5.))
            .typography(Type::Small)
            .text_color(theme.text_dim)
            .cursor_pointer()
            .hover(|pill| pill.bg(theme.hover).text_color(theme.text))
            .tooltip(tip(hint))
            .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            .child(icon.view(px(12.), theme.text_dim))
            .child(label)
            .tag(id)
    };
    div()
        .h(px(ACTIONS_HEIGHT))
        .flex()
        .items_center()
        .justify_end()
        .gap(px(2.))
        .when(prompts.hovered != Some(number), |row| row.invisible().group_hover(group, |row| row.visible()))
        .child(crate::copy::CopyButton::new(format!("prompt-{number}"), copy, "Copy", "prompt-copy"))
        .when(prompts.chat && prompts.idle, |row| {
            row.child(pill("prompt-edit", Icon::Pencil, "Edit", "Change this message and send it again", Box::new(EditPrompt { number })))
                .child(pill("prompt-rewind", Icon::Rewind, "Rewind", "Take the chat back to before this message", Box::new(RewindPrompt { number })))
                .when(latest, |row| row.child(pill("prompt-retry", Icon::Reload, "Retry", "Ask again", Box::new(RetryPrompt { number }))))
        })
        .into_any_element()
}

/// The row of pills' height: the space kept under every message.
pub const ACTIONS_HEIGHT: f32 = 26.;

/// A message being edited, in place of its bubble.
pub fn editor(field: &Entity<TextField>, share: f32, theme: &Theme) -> AnyElement {
    div()
        .key_context(EDIT_CONTEXT)
        .w(relative(share))
        .flex()
        .flex_col()
        .gap(px(10.))
        .px(px(14.))
        .pt(px(11.))
        .pb(px(10.))
        .rounded(px(16.))
        .bg(theme.surface)
        .border_1()
        .border_color(theme.accent_line)
        .child(div().typography(Type::Body).text_color(theme.text).child(field.clone()))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().min_w_0().truncate().typography(Type::Small).text_color(theme.text_faint).child("↩ sends · ⇧↩ new line · Esc leaves it"))
                .child(Button::new("edit-cancel", "Cancel", Kind::Secondary).action(CancelEdit))
                .child(Button::new("edit-send", "Send", Kind::Primary).action(SendEdit)),
        )
        .tag("prompt-editor")
        .into_any_element()
}

/// The card under a message: what the rewind, edit or retry does, and the files changed since.
pub fn card(card: &Card, theme: &Theme) -> AnyElement {
    let (title, body) = match card.purpose {
        Purpose::Rewind => (
            "Rewind to before this message?",
            "The conversation goes back to just before it, and its words come back to the composer. You can undo it until you send.",
        ),
        Purpose::Edit => ("Send the edited message?", "Everything after it goes, and the agent answers the new words."),
        Purpose::Retry => ("Retry this message?", "Everything after it goes, and the agent answers it again."),
    };
    let busy = card.working || card.preview.is_none();
    let files = files(card, theme);
    let ticked = !card.ticked.is_empty();
    let (keep, restore) = match card.purpose {
        Purpose::Rewind => ("Rewind, keep files", "Rewind and restore files"),
        Purpose::Edit => ("Keep files", "Restore files and send"),
        Purpose::Retry => ("Keep files", "Restore files and retry"),
    };
    let buttons = div()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(8.))
        .child(Button::new("rewind-cancel", "Cancel", Kind::Secondary).enabled(!card.working).action(CancelRewind))
        .map(|row| {
            if card.has_changes() {
                row.child(Button::new("rewind-keep", keep, Kind::Secondary).enabled(!busy).action(ConfirmRewind { files: false }))
                    .child(
                        Button::new("rewind-restore", restore, Kind::Primary)
                            .enabled(!busy && ticked)
                            .tooltip(if ticked { "The ticked files go back as they were" } else { "Tick the files to put back" })
                            .action(ConfirmRewind { files: true }),
                    )
            } else {
                let go = match card.purpose {
                    Purpose::Rewind => "Rewind",
                    Purpose::Edit => "Send",
                    Purpose::Retry => "Retry",
                };
                row.child(Button::new("rewind-go", go, Kind::Primary).enabled(!busy).action(ConfirmRewind { files: false }))
            }
        });
    div()
        .w(relative(CARD_SHARE))
        .mt(px(6.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .p(px(14.))
        .rounded(px(14.))
        .bg(theme.raised)
        .border_1()
        .border_color(theme.border_strong)
        .shadow_md()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(9.))
                .child(
                    div()
                        .flex_none()
                        .size(px(24.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme.accent_wash)
                        .child(Icon::Rewind.view(px(13.), theme.accent_text)),
                )
                .child(div().flex_1().min_w_0().typography(Type::UiStrong).text_color(theme.text).child(title)),
        )
        .child(div().typography(Type::Small).text_color(theme.text_dim).child(body))
        .child(files)
        .when_some(card.error.clone(), |column, error| column.child(div().typography(Type::Small).text_color(theme.danger).child(error)))
        .child(buttons)
        .tag("rewind-card")
        .into_any_element()
}

/// The most of the column the card takes.
const CARD_SHARE: f32 = 0.86;
/// The file list scrolls past about seven files.
const FILES_HEIGHT: f32 = 212.;

fn files(card: &Card, theme: &Theme) -> AnyElement {
    let note = |text: String| div().typography(Type::Small).text_color(theme.text_faint).child(text).into_any_element();
    let Some(preview) = &card.preview else {
        return note("Looking at what changed since…".into());
    };
    if preview.checkpoint.is_none() {
        return note("Files stay as they are: there's no snapshot of them from before this message.".into());
    }
    if let Some(problem) = &preview.problem {
        return note(format!("Files stay as they are: {problem}"));
    }
    if preview.changes.is_empty() {
        return note("No files have changed since.".into());
    }
    let all = card.ticked.len() == preview.changes.len();
    let count = preview.changes.len();
    let heading = div()
        .id("rewind-files-all")
        .h(px(28.))
        .px(px(6.))
        .rounded(px(8.))
        .flex()
        .items_center()
        .gap(px(9.))
        .cursor_pointer()
        .hover(|row| row.bg(theme.hover))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleRewindFiles), cx))
        .child(tick(if all { Tick::On } else if card.ticked.is_empty() { Tick::Off } else { Tick::Some }, theme))
        .child(
            div()
                .flex_1()
                .typography(Type::UiStrong)
                .text_color(theme.text)
                .child(if count == 1 { "1 file changed since".to_string() } else { format!("{count} files changed since") }),
        )
        .child(div().typography(Type::Small).text_color(theme.text_faint).child(format!("{} to restore", card.ticked.len())))
        .tag("rewind-files-all");
    let rows = div()
        .id("rewind-files")
        .max_h(px(FILES_HEIGHT))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .children(preview.changes.iter().enumerate().map(|(index, change)| {
            let ticked = card.ticked.contains(&change.path);
            let path = change.path.clone();
            div()
                .id(("rewind-file", index))
                .h(px(28.))
                .px(px(6.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .gap(px(9.))
                .cursor_pointer()
                .hover(|row| row.bg(theme.hover))
                .on_click(move |_, window, cx| window.dispatch_action(Box::new(ToggleRewindFile { path: path.clone() }), cx))
                .child(tick(if ticked { Tick::On } else { Tick::Off }, theme))
                .child(div().flex_1().min_w_0().truncate().typography(Type::Mono).text_color(if ticked { theme.text_soft } else { theme.text_faint }).child(change.path.clone()))
                .child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(match change.action {
                    Action::Revert => "changed",
                    Action::Delete => "new",
                    Action::Recreate => "deleted",
                }))
                .tag("rewind-file")
        }));
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .py(px(4.))
        .border_t_1()
        .border_b_1()
        .border_color(theme.border)
        .child(heading)
        .child(rows)
        .into_any_element()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tick {
    On,
    /// Some of the files under it, for the tick over them all.
    Some,
    Off,
}

/// A round tick: filled with the accent when on (a dash when only some are), a ring when off.
fn tick(tick: Tick, theme: &Theme) -> impl IntoElement {
    div()
        .flex_none()
        .size(px(16.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .map(|circle| match tick {
            Tick::On => circle.bg(theme.accent).child(Icon::Check.view(px(10.), theme.on_accent)),
            Tick::Some => circle.bg(theme.accent).child(Icon::Minus.view(px(10.), theme.on_accent)),
            Tick::Off => circle.border_1().border_color(theme.border_strong),
        })
}

/// The row a rewind leaves at the end of the transcript until anything runs: Undo brings back
/// what it took away.
pub fn rewound(files: bool, theme: &Theme) -> AnyElement {
    div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(9.))
        .rounded(px(12.))
        .border_1()
        .border_color(theme.border)
        .child(Icon::Rewind.view(px(14.), theme.accent_text))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().typography(Type::UiStrong).text_color(theme.text).child("Rewound"))
                .child(div().typography(Type::Small).text_color(theme.text_dim).child(if files {
                    "Undo brings back the messages after this point, and the files as they were."
                } else {
                    "Undo brings back the messages after this point."
                })),
        )
        .child(Button::new("undo-rewind", "Undo", Kind::Secondary).action(UndoRewind))
        .tag("rewound")
        .into_any_element()
}
