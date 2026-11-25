//! The computer-use card: the agent asks to use one of the user's other apps, inline above the
//! composer like the question card, never a modal. Ported from version 2's access card.
//!
//! Rules:
//! - Four answers, quietest first: Never allow, Deny, Always allow, and Allow for this chat, the
//!   one filled button. Nothing has focus when it arrives, so a keystroke meant for the composer
//!   never answers it; Esc denies (`Shell::back`).
//! - It waits as long as the user takes. Stop takes it down with the run.
//! - A clipboard card asks to read, or to write, for this chat: Deny or Allow, nothing saved.
//! - A screen card asks to take the screen (the real pointer and keyboard) until the run ends,
//!   in the agent's own words, and says how to take it back: Deny or Allow, nothing saved.
//! - It says what the agent will be able to do: act in the app in the background, or only look
//!   (web browsers), and whether the app will be started.
//! - Answers go through `AppState::answer_computer_card` with the card's id, so a click on a card
//!   that has already gone grants nothing.

use std::sync::Arc;

use gpui::{App, RenderImage, SharedString, Window, div, prelude::*, px};
use wack_computer::computer::Answer;

use crate::{
    actions::AnswerComputerCard,
    button::{Button, Kind},
    icon::Icon,
    tags::Tag,
    theme::Theme,
    typography::{Type, Typography},
};

/// What the card shows.
#[derive(Clone)]
pub struct CardData {
    pub id: u64,
    pub app: SharedString,
    /// Where the app lives, as the user would recognise it.
    pub path: Option<SharedString>,
    /// Web browsers: look, never act.
    pub look_only: bool,
    /// The app isn't running and will be started in the background.
    pub launching: bool,
    pub icon: Option<Arc<RenderImage>>,
    /// Why the last Always allow or Never allow couldn't be saved; the card stays up.
    pub error: Option<SharedString>,
    /// A clipboard card, not an app's: `Some(true)` to write to it, `Some(false)` to read it.
    /// It offers Deny and Allow for this chat only.
    pub clipboard: Option<bool>,
    /// A screen card, not an app's: the agent's reason, and the apps it may use.
    pub screen: Option<ScreenAsk>,
}

/// What a screen card shows.
#[derive(Clone)]
pub struct ScreenAsk {
    pub reason: SharedString,
    pub apps: SharedString,
}

/// "TextEdit", "TextEdit and Notes", "TextEdit, Notes and Calculator".
pub fn list(names: &[String]) -> String {
    match names {
        [] => "the apps you allowed".into(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl CardData {
    /// Clipboard and screen cards are this chat's alone: no Always or Never.
    fn chat_only(&self) -> bool {
        self.clipboard.is_some() || self.screen.is_some()
    }
}

#[derive(IntoElement)]
pub struct ComputerCard {
    data: CardData,
}

impl ComputerCard {
    pub fn new(data: CardData) -> Self {
        Self { data }
    }
}

/// What the agent may do, in the user's words.
fn reach(data: &CardData) -> String {
    if let Some(screen) = &data.screen {
        return format!(
            "Until it finishes, the agent may bring {} to the front and use your pointer and keyboard there. Move the mouse or press ⌃⌥⌘. to take your screen back.",
            screen.apps
        );
    }
    match data.clipboard {
        Some(true) => return "The agent may put text on the clipboard for this chat, replacing what you last copied.".into(),
        Some(false) => return "The agent may read what you copy, for this chat. Items a password manager marks private are never read.".into(),
        None => {}
    }
    let app = &data.app;
    let mut text = if data.look_only {
        format!("The agent may read {app}'s windows, but never click or type in them: browsers are look-only.")
    } else {
        format!("The agent may read {app}'s windows and work in them in the background, while you keep using your Mac.")
    };
    if data.launching {
        text.push_str(&format!(" {app} will start without coming to the front."));
    }
    text
}

impl RenderOnce for ComputerCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let data = self.data;
        let id = data.id;
        let tile = div()
            .flex_none()
            .size(px(34.))
            .rounded(px(10.))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.accent_wash)
            .child(match (data.icon.clone(), data.clipboard) {
                (_, Some(_)) => Icon::Copy.view(px(17.), theme.accent_text).into_any_element(),
                (Some(icon), None) if data.screen.is_none() => gpui::img(icon).size(px(28.)).into_any_element(),
                _ => Icon::Cursor.view(px(17.), theme.accent_text).into_any_element(),
            });
        let heading = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(1.))
            .child(div().typography(Type::Small).text_color(theme.accent_text).child(if data.look_only {
                "Computer use · Look only"
            } else if data.screen.is_some() {
                "Computer use · Your screen"
            } else {
                "Computer use"
            }))
            .child(div().truncate().typography(Type::UiStrong).text_color(theme.text).child(match (data.clipboard, &data.screen) {
                (_, Some(_)) => "Let the agent take over your screen?".to_string(),
                (Some(true), _) => "Let the agent copy to your clipboard?".to_string(),
                (Some(false), _) => "Let the agent read your clipboard?".to_string(),
                (None, None) => format!("Let the agent use {}?", data.app),
            }));
        let head = div().flex().items_center().gap(px(12.)).child(tile).child(heading);
        let body = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            // The agent's reason, in its own words, before what it may do.
            .when_some(data.screen.as_ref().map(|screen| screen.reason.clone()), |body, reason| body.child(div().typography(Type::Ui).text_color(theme.text).child(format!("“{reason}”"))))
            .child(div().typography(Type::Ui).text_color(theme.text_soft).child(reach(&data)))
            .when_some(data.path.clone(), |body, path| body.child(div().truncate().typography(Type::Mono).text_color(theme.text_faint).child(path)))
            .when_some(data.error.clone(), |body, error| body.child(div().typography(Type::Small).text_color(theme.danger).child(error)));
        let answer = |answer: Answer| AnswerComputerCard { id, answer };
        // In a narrow window the three answers that matter wrap together, still at the right.
        let answers = div()
            .ml_auto()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().child(Button::new("computer-deny", "Deny", Kind::Secondary).action(answer(Answer::Deny))).tag("computer-deny"))
            .when(!data.chat_only(), |answers| {
                answers.child(div().child(Button::new("computer-always", "Always allow", Kind::Secondary).action(answer(Answer::AlwaysAllow))).tag("computer-always"))
            })
            .child(div().child(Button::new("computer-allow", if data.screen.is_some() { "Allow until it’s done" } else { "Allow for this chat" }, Kind::Primary).action(answer(Answer::Allow))).tag("computer-allow"));
        let actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .when(!data.chat_only(), |actions| {
                actions.child(div().child(Button::new("computer-never", "Never allow", Kind::Secondary).action(answer(Answer::NeverAllow))).tag("computer-never"))
            })
            .child(answers);
        let card = div()
            .id("computer-card")
            .w_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .px(px(16.))
            .pt(px(14.))
            .pb(px(12.))
            .rounded(px(14.))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.accent_line)
            .shadow_md()
            .child(head)
            .child(body)
            .child(actions)
            .tag("computer-card");
        crate::motion::enter(SharedString::from(format!("computer-card-{id}")), div().w_full().flex().flex_col().child(card))
    }
}
