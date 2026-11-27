//! The question card: `ask_user_question`'s interview, inline above the composer and never a
//! modal, so the transcript stays readable while the user thinks. Ported from version 2's
//! `QuestionDialog.tsx`.
//!
//! Rules (each one v2's; the draft logic is tested below, the rest was driven in the dev app):
//! - One question at a time, with numbered tabs when there are several; a tab shows a check
//!   once its question is answered.
//! - Picking never advances on its own. Next moves on once the question is answered; Submit
//!   needs every question answered. ↩ on the card does whichever is offered, ⌘↩ submits.
//! - An "Other" answer replaces the preset choices, even in a multi-select question; picking a
//!   preset clears it.
//! - 1–4 pick an option while the card has focus (the options take focus when the card arrives,
//!   so digits never reach the composer). Esc cancels the question; only with no card open does
//!   it stop the run.
//! - In Ultra Plan the card wears the prism and offers "Write the plan now", which ends the
//!   interview: the agent submits its plan on its recommended answers.
//!
//! Its look is version 2.5.3's: the duck in a round tile, bobbing while the question waits and
//! giving off bubbles at each pick; a soft halo of the card's colour round it; the question tabs
//! strung on a line, the one on show filled; every option a bordered row whose marker fills
//! with a tick. The record the transcript keeps of a finished interview ([`asked_view`]) is the
//! same card, settled: the same rows, the picks lit.
//!
//! The drafts live in [`crate::chat::ChatSession`], so switching chats keeps them; this view
//! draws [`QuestionData`] and reports clicks as actions.

use gpui::{AnimationExt, App, FocusHandle, Hsla, KeyBinding, SharedString, Window, div, prelude::*, px};
use wack_core::ask::{Question, QuestionAnswer};

use crate::{
    actions::{CancelQuestion, NextQuestion, PickOption, PreviousQuestion, ShowQuestion, SubmitAnswers, WrapUpQuestion},
    button::{Button, Kind},
    composer::Input,
    icon::Icon,
    tags::Tag,
    theme::Theme,
    typography::{Type, Typography},
    ultra::UltraMark,
};

/// The card's key context: Esc cancels and ⌘↩ submits anywhere in it.
pub const CARD_CONTEXT: &str = "Question";
/// The options' key context: digits pick and ↩ moves on. Kept off the "Other" field, so typing a
/// number there types it.
pub const OPTIONS_CONTEXT: &str = "QuestionOptions";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CancelQuestion, Some(CARD_CONTEXT)),
        KeyBinding::new("cmd-enter", SubmitAnswers, Some(CARD_CONTEXT)),
        KeyBinding::new("enter", NextQuestion, Some(OPTIONS_CONTEXT)),
        KeyBinding::new("1", PickOption { index: 0 }, Some(OPTIONS_CONTEXT)),
        KeyBinding::new("2", PickOption { index: 1 }, Some(OPTIONS_CONTEXT)),
        KeyBinding::new("3", PickOption { index: 2 }, Some(OPTIONS_CONTEXT)),
        KeyBinding::new("4", PickOption { index: 3 }, Some(OPTIONS_CONTEXT)),
    ]);
}

// ── The drafts ────────────────────────────────────────

/// One question's answer so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    /// The labels of the chosen options.
    pub selected: Vec<String>,
    /// The "Other" answer; when it has text it replaces the choices.
    pub custom: String,
}

impl Draft {
    pub fn answered(&self) -> bool {
        !self.selected.is_empty() || !self.custom.trim().is_empty()
    }
}

/// Picks `label`: one option replaces another, unless the question takes several, where a pick
/// toggles. Either way a pick clears the "Other" answer.
pub fn pick(question: &Question, draft: &mut Draft, label: &str) {
    if question.multi_select {
        if let Some(at) = draft.selected.iter().position(|selected| selected == label) {
            draft.selected.remove(at);
        } else {
            draft.selected.push(label.to_string());
        }
    } else {
        draft.selected = vec![label.to_string()];
    }
    draft.custom.clear();
}

/// The answers to send, once every question is answered.
pub fn answers(questions: &[Question], drafts: &[Draft]) -> Option<Vec<QuestionAnswer>> {
    if questions.len() != drafts.len() || !drafts.iter().all(Draft::answered) {
        return None;
    }
    Some(
        questions
            .iter()
            .zip(drafts)
            .map(|(question, draft)| {
                let custom = draft.custom.trim();
                QuestionAnswer {
                    question_id: question.id.clone(),
                    selected: if custom.is_empty() { draft.selected.clone() } else { Vec::new() },
                    custom: (!custom.is_empty()).then(|| custom.to_string()),
                }
            })
            .collect(),
    )
}

// ── The card ──────────────────────────────────────────

#[derive(Clone)]
pub struct QuestionData {
    pub questions: Vec<Question>,
    /// The question on show.
    pub tab: usize,
    /// One per question. A live card's "Other" text is read from its field.
    pub drafts: Vec<Draft>,
    /// Ultra Plan: the prism, and "Write the plan now".
    pub wrap_up: bool,
    /// The "Other" field of the question on show.
    pub other: Input,
    /// Where the options take focus, so 1–4 reach them. `None` in a scene.
    pub focus: Option<FocusHandle>,
    /// How many picks so far: each one nods the duck.
    pub nods: usize,
}

#[derive(IntoElement)]
pub struct QuestionCard {
    data: QuestionData,
}

impl QuestionCard {
    pub fn new(data: QuestionData) -> Self {
        Self { data }
    }
}

impl RenderOnce for QuestionCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let data = self.data;
        let count = data.questions.len();
        let tab = data.tab.min(count.saturating_sub(1));
        let Some(question) = data.questions.get(tab).cloned() else {
            return div().into_any_element();
        };
        let draft = data.drafts.get(tab).cloned().unwrap_or_default();
        let all_answered = data.drafts.len() == count && data.drafts.iter().all(Draft::answered);
        let last = tab + 1 == count;
        let ultra = data.wrap_up;
        // The card's own colour: the accent, or the prism in Ultra Plan.
        let tone = Tone::of(ultra, &theme);
        let (tone_text, wash, line) = (tone.text, tone.wash, tone.line);
        let still = cx.reduce_motion();

        let duck = duck_tile(&tone, data.nods, !draft.answered() && !still, still);

        let eyebrow = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .typography(Type::Small)
            .text_color(tone_text)
            .when(ultra, |eyebrow| eyebrow.child(UltraMark::new("question-ultra-mark", px(10.))))
            .child(if ultra { "Ultra Plan · Question" } else { "Question" });
        let heading = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(eyebrow)
            .child(div().truncate().typography(Type::UiStrong).text_color(theme.text).child(question.header.clone()));

        // The tabs are strung on a line; the one on show is filled.
        let tabs = (count > 1).then(|| {
            let thread = div().absolute().left(px(12.)).right(px(12.)).top(px(12.)).h(px(1.)).bg(line);
            div().flex_none().relative().flex().gap(px(7.)).child(thread).children(data.questions.iter().enumerate().map(|(index, entry)| {
                let answered = data.drafts.get(index).is_some_and(Draft::answered);
                let active = index == tab;
                let ink = if active { tone.ink } else if answered { tone_text } else { theme.text_dim };
                div()
                    .id(("question-tab", index))
                    .size(px(25.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_1()
                    .map(|tab| {
                        if active {
                            tab.bg(tone.fill).border_color(tone.fill)
                        } else {
                            tab.bg(theme.surface).border_color(if answered { line } else { theme.border }).hover(|tab| tab.border_color(line))
                        }
                    })
                    .tooltip(crate::tooltip::tip(entry.header.clone()))
                    .on_click(dispatch(ShowQuestion { index }))
                    .child(if answered && !active {
                        Icon::Check.view(px(12.), ink).into_any_element()
                    } else {
                        div().typography(Type::Small).font_weight(gpui::FontWeight::SEMIBOLD).text_color(ink).child((index + 1).to_string()).into_any_element()
                    })
            }))
        });

        let head = div().flex().items_center().gap(px(12.)).child(duck).child(heading).children(tabs);

        let mut options = div()
            .id(SharedString::from(format!("question-options-{}", question.id)))
            .key_context(OPTIONS_CONTEXT)
            .when_some(data.focus.as_ref(), |options, focus| options.track_focus(focus))
            .flex()
            .flex_col()
            .gap(px(6.));
        for (index, option) in question.options.iter().enumerate() {
            let selected = draft.selected.contains(&option.label);
            let marker_id = SharedString::from(format!("question-tick-{}-{index}", question.id));
            options = options.child(
                option_row(marker_id, option.label.clone().into(), Some(option.description.clone().into()), selected, question.multi_select, &tone, still, &theme)
                    .id(("question-option", index))
                    .cursor_pointer()
                    .when(!selected, |row| row.hover(|row| row.bg(wash).border_color(line)))
                    .on_click(dispatch(PickOption { index }))
                    .when(index < 4, |row| {
                        row.child(
                            div()
                                .flex_none()
                                .min_w(px(18.))
                                .h(px(18.))
                                .rounded(px(5.))
                                .bg(theme.hover)
                                .flex()
                                .items_center()
                                .justify_center()
                                .typography(Type::Small)
                                .text_color(if selected { tone_text } else { theme.text_faint })
                                .child((index + 1).to_string()),
                        )
                    })
                    .tag("question-option"),
            );
        }

        let has_custom = !draft.custom.trim().is_empty();
        let other_field = match data.other {
            Input::Field(field) => div().flex_1().min_w_0().child(field),
            Input::Drawn { text, placeholder } => div().flex_1().min_w_0().typography(Type::Ui).map(|field| match text {
                Some(text) => field.text_color(theme.text).child(text),
                None => field.text_color(theme.text_faint).child(placeholder),
            }),
        };
        let other = div()
            .px(px(10.))
            .py(px(9.))
            .rounded(px(11.))
            .flex()
            .items_start()
            .gap(px(10.))
            .border_1()
            .border_color(if has_custom { line } else { theme.border })
            .bg(if has_custom { wash } else { theme.surface })
            .child(marker(SharedString::from(format!("question-tick-{}-other", question.id)), has_custom, false, &tone, still, &theme))
            .child(other_field)
            .tag("question-other");

        // A new question slides its options in; the list is the part that scrolls when room
        // is short, so it is the card's own child.
        let options = crate::motion::enter(SharedString::from(format!("question-stage-{}", question.id)), options);
        let title = div().flex_none().pb(px(2.)).typography(Type::Heading).text_color(theme.text).child(question.question.clone());

        let answered = draft.answered();
        let mut actions = div().flex().items_center().justify_end().gap(px(8.));
        if ultra {
            actions = actions.child(
                Button::new("question-wrap-up", "Write the plan now", Kind::Secondary)
                    .tooltip("Stop the interview — the agent submits its plan using its recommended answers")
                    .action(WrapUpQuestion),
            );
            actions = actions.child(div().flex_1());
        }
        actions = actions.child(Button::new("question-cancel", "Cancel", Kind::Secondary).action(CancelQuestion));
        if tab > 0 {
            actions = actions.child(Button::new("question-back", "Back", Kind::Secondary).action(PreviousQuestion));
        }
        let primary = if ultra { Kind::Ultra } else { Kind::Primary };
        actions = actions.child(if last {
            Button::new("question-submit", "Submit", primary).enabled(all_answered).action(SubmitAnswers)
        } else {
            Button::new("question-next", "Next", primary).enabled(answered).action(NextQuestion)
        });

        let card = div()
            .id("question-card")
            .key_context(CARD_CONTEXT)
            .w_full()
            .flex()
            .flex_col()
            .gap(px(14.))
            .px(px(16.))
            .pt(px(14.))
            .pb(px(12.))
            .rounded(px(16.))
            .bg(theme.surface)
            .border_1()
            .border_color(line)
            // A soft halo of the card's colour, over the drop that lifts it off the chat.
            .shadow(vec![
                gpui::BoxShadow { color: Hsla { a: 0.10, ..tone.fill }, offset: gpui::point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(4.), inset: false },
                gpui::BoxShadow { color: theme.glass_shadow, offset: gpui::point(px(0.), px(10.)), blur_radius: px(36.), spread_radius: px(0.), inset: false },
            ])
            .min_h_0()
            .child(head.flex_none())
            // The body scrolls when room is short; the head and the buttons always show.
            .child(
                div()
                    .id(SharedString::from(format!("question-body-{}", question.id)))
                    .flex_shrink(1.)
                    .min_h(px(56.))
                    .max_h(px(380.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(title)
                    .child(options)
                    .child(other),
            )
            .child(actions.flex_none())
            .tag("question-card");
        crate::motion::enter(
            SharedString::from(format!("question-card-{}", data.questions[0].id)),
            div().w_full().min_h_0().flex().flex_col().child(card),
        )
            .into_any_element()
    }
}

fn dispatch(action: impl gpui::Action) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static {
    let action = action.boxed_clone();
    move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)
}

// ── The record ────────────────────────────────────────

/// What was asked and what the user chose, for the work log.
#[derive(Clone, Debug, PartialEq)]
pub struct Asked {
    pub question: SharedString,
    pub header: SharedString,
    pub options: Vec<AskedOption>,
    pub multi_select: bool,
    pub custom: Option<SharedString>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AskedOption {
    pub label: SharedString,
    pub description: SharedString,
    pub picked: bool,
}

/// The record of a finished `ask_user_question` call: its questions with the picks marked, when
/// the result says what was picked.
pub fn asked(questions: &[Question], answers: Option<&[QuestionAnswer]>) -> Vec<Asked> {
    questions
        .iter()
        .map(|question| {
            let answer = answers.and_then(|answers| answers.iter().find(|answer| answer.question_id == question.id));
            Asked {
                question: question.question.clone().into(),
                header: question.header.clone().into(),
                options: question
                    .options
                    .iter()
                    .map(|option| AskedOption {
                        label: option.label.clone().into(),
                        description: option.description.clone().into(),
                        picked: answer.is_some_and(|answer| answer.selected.contains(&option.label)),
                    })
                    .collect(),
                multi_select: question.multi_select,
                custom: answer.and_then(|answer| answer.custom.clone()).map(Into::into),
            }
        })
        .collect()
}

/// Draws the record: the question card, settled. Each question under its header, its options in
/// the card's rows with the picks lit and the rest faded, and an "Other" answer as a row of its own.
pub fn asked_view(asked: &[Asked], theme: &Theme) -> impl IntoElement {
    let tone = Tone::of(false, theme);
    div()
        .flex()
        .flex_col()
        .gap(px(14.))
        .px(px(14.))
        .py(px(12.))
        .rounded(px(14.))
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .children(asked.iter().enumerate().map(|(number, question)| {
            let answered = question.custom.is_some() || question.options.iter().any(|option| option.picked);
            let head = div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .flex_none()
                        .size(px(26.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_1()
                        .border_color(tone.line)
                        .bg(tone.wash)
                        .child(Icon::Duck.view(px(15.), tone.text)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().typography(Type::Small).text_color(tone.text).child(question.header.clone()))
                        .child(div().typography(Type::UiStrong).text_color(theme.text).child(question.question.clone())),
                )
                .when(!answered, |head| head.child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child("No answer")));
            let rows = question.options.iter().enumerate().map(|(index, option)| {
                option_row(SharedString::from(format!("asked-tick-{number}-{index}")), option.label.clone(), Some(option.description.clone()), option.picked, question.multi_select, &tone, true, theme)
                    .when(!option.picked, |row| row.opacity(0.62))
                    .into_any_element()
            });
            let custom = question.custom.clone().map(|custom| {
                option_row(SharedString::from(format!("asked-tick-{number}-other")), custom, Some("Your own answer".into()), true, false, &tone, true, theme).into_any_element()
            });
            div().flex().flex_col().gap(px(6.)).child(head.mb(px(4.))).children(rows).children(custom)
        }))
        .tag("asked-card")
}

/// The card's colour: the accent, or the prism in Ultra Plan.
struct Tone {
    fill: Hsla,
    text: Hsla,
    wash: Hsla,
    line: Hsla,
    /// What is drawn on `fill`.
    ink: Hsla,
}

impl Tone {
    fn of(ultra: bool, theme: &Theme) -> Self {
        if ultra {
            Self { fill: theme.ultra, text: theme.ultra, wash: Hsla { a: 0.10, ..theme.ultra }, line: Hsla { a: 0.42, ..theme.ultra }, ink: theme.on_ultra }
        } else {
            Self { fill: theme.accent, text: theme.accent_text, wash: theme.accent_wash, line: theme.accent_line, ink: theme.on_accent }
        }
    }
}

/// An option as the card offers it and its record keeps it, one shape (as version 2 kept them):
/// a bordered row, a marker that fills with a tick, the row tinted when picked.
#[allow(clippy::too_many_arguments)]
fn option_row(marker_id: SharedString, label: SharedString, description: Option<SharedString>, selected: bool, multi: bool, tone: &Tone, still: bool, theme: &Theme) -> gpui::Div {
    div()
        .px(px(10.))
        .py(px(9.))
        .rounded(px(11.))
        .flex()
        .items_start()
        .gap(px(10.))
        .border_1()
        .border_color(if selected { tone.line } else { theme.border })
        .bg(if selected { tone.wash } else { theme.surface })
        .child(marker(marker_id, selected, multi, tone, still, theme))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().typography(Type::UiStrong).text_color(theme.text).child(label))
                .children(description.filter(|text| !text.is_empty()).map(|text| div().typography(Type::Small).text_color(theme.text_dim).child(text))),
        )
}

/// A choice's marker: a ring (a rounded square when several can be picked) that fills with a
/// tick, the tick landing with a small pop.
fn marker(id: SharedString, selected: bool, multi: bool, tone: &Tone, still: bool, theme: &Theme) -> gpui::AnyElement {
    let shape = div().flex_none().mt(px(1.)).size(px(16.)).flex().items_center().justify_center();
    let shape = if multi { shape.rounded(px(5.)) } else { shape.rounded_full() };
    if !selected {
        return shape.border_1().border_color(theme.border_strong).into_any_element();
    }
    let ink = tone.ink;
    let filled = shape.bg(tone.fill);
    if still {
        return filled.child(Icon::Check.view(px(11.), ink)).into_any_element();
    }
    filled
        .child(div().child(Icon::Check.view(px(11.), ink)).with_animation(id, gpui::Animation::new(std::time::Duration::from_millis(260)), |tick, t| {
            tick.opacity((t * 3.).min(1.)).mt(px((1. - crate::motion::pop(t)) * 3.))
        }))
        .into_any_element()
}

/// The duck in its round tile: it nods at each pick (`nods`), bobs while the question waits
/// (`idle`), and a pick sends up a few bubbles. With Reduce Motion it holds still.
fn duck_tile(tone: &Tone, nods: usize, idle: bool, still: bool) -> gpui::AnyElement {
    let duck = Icon::Duck.view(px(24.), tone.text);
    let duck = if nods > 0 && !still {
        duck.with_animation(("question-nod", nods), gpui::Animation::new(std::time::Duration::from_millis(420)), |duck, t| {
            let turn = crate::composer::keyframes(&[(0., 0.), (0.35, 12.), (0.7, -5.), (1., 0.)], t);
            duck.with_transformation(gpui::Transformation::rotate(gpui::radians(turn.to_radians())))
        })
        .into_any_element()
    } else {
        duck.into_any_element()
    };
    // The bob is the tile's contents rising and falling a few points, over and over.
    let duck = if idle {
        div()
            .relative()
            .child(duck)
            .with_animation("question-bob", gpui::Animation::new(std::time::Duration::from_millis(3200)).repeat(), |duck, t| {
                duck.top(px(-1.5 * (t * std::f32::consts::TAU).sin()))
            })
            .into_any_element()
    } else {
        duck
    };
    let ring = tone.fill;
    let bubbles = (nods > 0 && !still).then(|| {
        div()
            .absolute()
            .top(px(2.))
            .right(px(-2.))
            .size(px(12.))
            .with_animation(("question-bubbles", nods), gpui::Animation::new(std::time::Duration::from_millis(650)), move |bubbles, t| {
                // Three bubbles, each a beat after the last, rise and fade.
                let bubble = |delay: f32, x: f32, size: f32| {
                    let local = ((t - delay) / (1. - delay)).clamp(0., 1.);
                    let opacity = if local < 0.15 { local / 0.15 * 0.8 } else { 0.8 * (1. - (local - 0.15) / 0.85) };
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(3. - 15. * local))
                        .size(px(size * (1. - 0.3 * local)))
                        .rounded_full()
                        .border_1()
                        .border_color(Hsla { a: opacity.max(0.), ..ring })
                };
                bubbles.child(bubble(0., 0., 5.)).child(bubble(0.14, 6., 3.)).child(bubble(0.28, -4., 5.))
            })
    });
    div()
        .flex_none()
        .relative()
        .size(px(42.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(tone.line)
        .bg(tone.wash)
        .child(duck)
        .children(bubbles)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wack_core::ask::QuestionOption;

    fn question(id: &str, multi: bool) -> Question {
        Question {
            id: id.into(),
            header: id.into(),
            question: format!("{id}?"),
            options: ["A", "B", "C"].iter().map(|label| QuestionOption { label: (*label).into(), description: "…".into() }).collect(),
            multi_select: multi,
        }
    }

    #[test]
    fn a_pick_replaces_the_last_unless_the_question_takes_several() {
        let single = question("one", false);
        let mut draft = Draft::default();
        pick(&single, &mut draft, "A");
        pick(&single, &mut draft, "B");
        assert_eq!(draft.selected, vec!["B"]);

        let multi = question("many", true);
        let mut draft = Draft::default();
        pick(&multi, &mut draft, "A");
        pick(&multi, &mut draft, "C");
        pick(&multi, &mut draft, "A");
        assert_eq!(draft.selected, vec!["C"]);
    }

    #[test]
    fn a_pick_clears_the_other_answer_and_other_replaces_the_picks() {
        let multi = question("many", true);
        let mut draft = Draft { selected: Vec::new(), custom: "my own".into() };
        pick(&multi, &mut draft, "A");
        assert_eq!(draft.custom, "");

        let questions = vec![multi];
        let drafts = vec![Draft { selected: vec!["A".into()], custom: "  something else  ".into() }];
        let answers = answers(&questions, &drafts).unwrap();
        assert!(answers[0].selected.is_empty());
        assert_eq!(answers[0].custom.as_deref(), Some("something else"));
    }

    #[test]
    fn submitting_needs_every_question_answered() {
        let questions = vec![question("one", false), question("two", false)];
        let mut drafts = vec![Draft::default(), Draft::default()];
        pick(&questions[0], &mut drafts[0], "A");
        assert!(answers(&questions, &drafts).is_none());
        drafts[1].custom = "   ".into();
        assert!(answers(&questions, &drafts).is_none(), "blank text is no answer");
        drafts[1].custom = "mine".into();
        let answers = answers(&questions, &drafts).unwrap();
        assert_eq!(answers[0].selected, vec!["A"]);
        assert_eq!(answers[1].custom.as_deref(), Some("mine"));
    }

    #[test]
    fn the_record_marks_what_was_picked() {
        let questions = vec![question("one", false)];
        let picked = vec![QuestionAnswer { question_id: "one".into(), selected: vec!["B".into()], custom: None }];
        let record = asked(&questions, Some(&picked));
        assert_eq!(record[0].options.iter().map(|option| option.picked).collect::<Vec<_>>(), vec![false, true, false]);
        let unanswered = asked(&questions, None);
        assert!(unanswered[0].options.iter().all(|option| !option.picked));
    }
}
