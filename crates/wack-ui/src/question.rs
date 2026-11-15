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
        let (tone, tone_text, wash, line) = if ultra {
            (theme.ultra, theme.ultra, Hsla { a: 0.10, ..theme.ultra }, Hsla { a: 0.42, ..theme.ultra })
        } else {
            (theme.accent, theme.accent_text, theme.accent_wash, theme.accent_line)
        };

        // The duck nods at each pick.
        let duck = Icon::Duck.view(px(18.), tone_text);
        let duck = if data.nods > 0 {
            duck.with_animation(("question-nod", data.nods), gpui::Animation::new(std::time::Duration::from_millis(420)), |duck, t| {
                let turn = crate::composer::keyframes(&[(0., 0.), (0.35, 12.), (0.7, -5.), (1., 0.)], t);
                duck.with_transformation(gpui::Transformation::rotate(gpui::radians(turn.to_radians())))
            })
            .into_any_element()
        } else {
            duck.into_any_element()
        };
        let tile = div().flex_none().size(px(34.)).rounded(px(10.)).flex().items_center().justify_center().bg(wash).child(duck);

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
            .gap(px(1.))
            .child(eyebrow)
            .child(div().truncate().typography(Type::UiStrong).text_color(theme.text).child(question.header.clone()));

        let tabs = (count > 1).then(|| {
            div().flex_none().flex().gap(px(4.)).children(data.questions.iter().enumerate().map(|(index, entry)| {
                let answered = data.drafts.get(index).is_some_and(Draft::answered);
                let active = index == tab;
                div()
                    .id(("question-tab", index))
                    .size(px(24.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(active, |tab| tab.bg(wash).border_1().border_color(line))
                    .when(!active, |tab| tab.hover(|tab| tab.bg(theme.hover)))
                    .tooltip(crate::tooltip::tip(entry.header.clone()))
                    .on_click(dispatch(ShowQuestion { index }))
                    .child(if answered {
                        Icon::Check.view(px(12.), tone_text).into_any_element()
                    } else {
                        div()
                            .typography(Type::Small)
                            .text_color(if active { tone_text } else { theme.text_dim })
                            .child((index + 1).to_string())
                            .into_any_element()
                    })
            }))
        });

        let head = div().flex().items_center().gap(px(12.)).child(tile).child(heading).children(tabs);

        let marker = |selected: bool| {
            let size = px(16.);
            let shape = div().flex_none().size(size).flex().items_center().justify_center();
            let shape = if question.multi_select { shape.rounded(px(4.)) } else { shape.rounded_full() };
            if selected {
                shape.bg(tone).child(Icon::Check.view(px(11.), if ultra { theme.on_ultra } else { theme.on_accent }))
            } else {
                shape.border_1().border_color(theme.border_strong)
            }
        };

        let mut options = div()
            .id(SharedString::from(format!("question-options-{}", question.id)))
            .key_context(OPTIONS_CONTEXT)
            .when_some(data.focus.as_ref(), |options, focus| options.track_focus(focus))
            .flex()
            .flex_col()
            .gap(px(4.));
        for (index, option) in question.options.iter().enumerate() {
            let selected = draft.selected.contains(&option.label);
            options = options.child(
                div()
                    .id(("question-option", index))
                    .px(px(10.))
                    .py(px(7.))
                    .rounded(px(9.))
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .border_1()
                    .border_color(if selected { line } else { gpui::transparent_black() })
                    .when(selected, |row| row.bg(wash))
                    .when(!selected, |row| row.hover(|row| row.bg(theme.hover)))
                    .on_click(dispatch(PickOption { index }))
                    .child(div().pt(px(1.)).child(marker(selected)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().typography(Type::UiStrong).text_color(theme.text).child(option.label.clone()))
                            .child(div().typography(Type::Small).text_color(theme.text_dim).child(option.description.clone())),
                    )
                    .when(index < 4, |row| {
                        row.child(
                            div()
                                .flex_none()
                                .min_w(px(18.))
                                .h(px(18.))
                                .rounded(px(5.))
                                .border_1()
                                .border_color(theme.border)
                                .flex()
                                .items_center()
                                .justify_center()
                                .typography(Type::Small)
                                .text_color(theme.text_faint)
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
            .py(px(7.))
            .rounded(px(9.))
            .flex()
            .items_start()
            .gap(px(10.))
            .border_1()
            .border_color(if has_custom { line } else { theme.border })
            .when(has_custom, |row| row.bg(wash))
            .child(div().pt(px(1.)).child(marker(has_custom)))
            .child(other_field)
            .tag("question-other");

        // A new question slides its options in; the list is the part that scrolls when room
        // is short, so it is the card's own child.
        let options = crate::motion::enter(SharedString::from(format!("question-stage-{}", question.id)), options);
        let title = div().flex_none().typography(Type::Heading).text_color(theme.text).child(question.question.clone());

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
            .rounded(px(14.))
            .bg(theme.surface)
            .border_1()
            .border_color(line)
            .shadow_md()
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

/// Draws the record: each question, its options, the picks lit.
pub fn asked_view(asked: &[Asked], theme: &Theme) -> impl IntoElement {
    div().flex().flex_col().gap(px(12.)).children(asked.iter().map(|question| {
        let marker = |picked: bool| {
            let shape = div().flex_none().size(px(12.)).mt(px(3.));
            let shape = if question.multi_select { shape.rounded(px(3.)) } else { shape.rounded_full() };
            if picked { shape.bg(theme.accent_fill()) } else { shape.border_1().border_color(theme.border_strong) }
        };
        div()
            .flex()
            .flex_col()
            .gap(px(5.))
            .child(div().typography(Type::UiStrong).text_color(theme.text_soft).child(question.question.clone()))
            .children(question.options.iter().map(|option| {
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .child(marker(option.picked))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .typography(Type::Ui)
                            .text_color(if option.picked { theme.text } else { theme.text_dim })
                            .child(option.label.clone()),
                    )
            }))
            .when_some(question.custom.clone(), |column, custom| {
                column.child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .child(marker(true))
                        .child(div().flex_1().min_w_0().typography(Type::Ui).text_color(theme.text).child(format!("Other: {custom}"))),
                )
            })
    }))
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
