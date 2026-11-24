//! The computer-use card: the agent asks to use one of the user's other apps, inline above the
//! composer like the question card, never a modal. Ported from version 2's access card.
//!
//! Rules:
//! - Four answers, quietest first: Never allow, Deny, Always allow, and Allow for this chat, the
//!   one filled button. Nothing has focus when it arrives, so a keystroke meant for the composer
//!   never answers it; Esc denies (`Shell::back`).
//! - It waits as long as the user takes. Stop takes it down with the run.
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
            .child(match data.icon.clone() {
                Some(icon) => gpui::img(icon).size(px(28.)).into_any_element(),
                None => Icon::Cursor.view(px(17.), theme.accent_text).into_any_element(),
            });
        let heading = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(1.))
            .child(div().typography(Type::Small).text_color(theme.accent_text).child(if data.look_only { "Computer use · Look only" } else { "Computer use" }))
            .child(div().truncate().typography(Type::UiStrong).text_color(theme.text).child(format!("Let the agent use {}?", data.app)));
        let head = div().flex().items_center().gap(px(12.)).child(tile).child(heading);
        let body = div()
            .flex()
            .flex_col()
            .gap(px(4.))
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
            .child(div().child(Button::new("computer-always", "Always allow", Kind::Secondary).action(answer(Answer::AlwaysAllow))).tag("computer-always"))
            .child(div().child(Button::new("computer-allow", "Allow for this chat", Kind::Primary).action(answer(Answer::Allow))).tag("computer-allow"));
        let actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(div().child(Button::new("computer-never", "Never allow", Kind::Secondary).action(answer(Answer::NeverAllow))).tag("computer-never"))
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
