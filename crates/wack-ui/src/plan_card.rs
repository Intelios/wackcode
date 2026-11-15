//! The plan card: where Plan mode lands. The transcript's one hero card, lifted out of the work
//! log, ported from version 2's `PlanCard.tsx`.
//!
//! Only the proposal still awaiting a decision has actions, the halo, the lit head and "Awaiting
//! you"; earlier or revised proposals keep the shape and drop the light, so there is never more
//! than one decision on screen. Approve & implement switches the chat to Build and sends
//! "Implement the plan."; Discard asks in the card's own footer, then returns to Build. A plan
//! from Ultra Plan wears the prism and the Ultra mark.

use std::rc::Rc;

use gpui::{BoxShadow, Hsla, SharedString, div, point, prelude::*, px};

use crate::{
    actions::{ApprovePlan, ConfirmDiscardPlan, CopyPlan, DiscardPlan, KeepPlan, SavePlan},
    button::{Button, Kind},
    icon::Icon,
    markdown,
    tags::Tag,
    theme::Theme,
    transcript::Handlers,
    typography::{Type, Typography},
    ultra::UltraMark,
};

/// What Approve sends.
pub const IMPLEMENT: &str = "Implement the plan.";

#[derive(Clone, Debug)]
pub struct PlanCard {
    pub key: SharedString,
    pub blocks: Rc<Vec<markdown::Block>>,
    pub ultra: bool,
    /// The proposal awaiting a decision.
    pub current: bool,
    /// Discard was pressed: the footer asks before it happens.
    pub confirming_discard: bool,
    /// A word after Copy or Save PLAN.md.
    pub notice: Option<SharedString>,
}

pub fn plan_card(card: &PlanCard, theme: &Theme, handlers: &Handlers) -> impl IntoElement {
    let current = card.current;
    let (tone, tone_text) = if card.ultra { (theme.ultra, theme.ultra) } else { (theme.accent, theme.accent_text) };
    let line = if current { Hsla { a: 0.5, ..tone } } else { theme.border_strong };

    let mark = div()
        .flex_none()
        .size(px(34.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(if current { Hsla { a: 0.14, ..tone } } else { theme.hover })
        .when(current, |mark| mark.border_1().border_color(Hsla { a: 0.3, ..tone }))
        .child(if card.ultra {
            UltraMark::new(SharedString::from(format!("{}-mark", card.key)), px(15.)).animated(current).into_any_element()
        } else {
            Icon::Brain.view(px(17.), if current { tone_text } else { theme.text_soft }).into_any_element()
        });

    let title = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(div().typography(Type::Heading).text_color(theme.text).child("Proposed plan"))
        .when(current, |title| {
            title.child(div().typography(Type::Small).text_color(theme.text_dim).child("Approve it, or type feedback below to revise it."))
        });

    let status = current.then(|| {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(9.))
            .h(px(22.))
            .rounded_full()
            .bg(Hsla { a: 0.12, ..tone })
            .child(div().size(px(6.)).rounded_full().bg(tone))
            .child(div().typography(Type::Small).text_color(tone_text).child("Awaiting you"))
    });

    let head = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(16.))
        .py(px(12.))
        .border_b_1()
        .border_color(if current { Hsla { a: 0.4, ..tone } } else { theme.border })
        .when(current, |head| {
            // Lit from its corner while a decision is owed.
            head.bg(gpui::linear_gradient(
                115.,
                gpui::linear_color_stop(Hsla { a: 0.16, ..tone }, 0.),
                gpui::linear_color_stop(Hsla { a: 0., ..tone }, 0.6),
            ))
        })
        .child(mark)
        .child(title)
        .children(status);

    let body = div().px(px(18.)).py(px(14.)).child(crate::transcript::reply(&card.blocks, theme, handlers));

    let footer = current.then(|| {
        let primary = if card.ultra { Kind::Ultra } else { Kind::Primary };
        let row = div().flex().items_center().flex_wrap().gap(px(8.)).px(px(16.)).py(px(12.)).border_t_1().border_color(theme.border);
        if card.confirming_discard {
            row.child(
                div()
                    .flex_1()
                    .min_w(px(220.))
                    .flex()
                    .flex_col()
                    .child(div().typography(Type::UiStrong).text_color(theme.text).child("Discard this plan?"))
                    .child(div().typography(Type::Small).text_color(theme.text_dim).child(
                        "The task switches back to Build mode and the proposed plan is abandoned. The conversation is kept.",
                    )),
            )
            .child(Button::new("plan-keep", "Keep it", Kind::Secondary).action(KeepPlan))
            .child(Button::new("plan-discard-confirm", "Discard plan", Kind::Danger).action(ConfirmDiscardPlan))
            .into_any_element()
        } else {
            row.child(div().child(Button::new("plan-approve", "Approve & implement", primary).action(ApprovePlan)).tag("plan-approve"))
                .child(div().child(Button::new("plan-save", "Save PLAN.md", Kind::Secondary).action(SavePlan)).tag("plan-save"))
                .child(div().child(Button::new("plan-copy", "Copy", Kind::Secondary).action(CopyPlan)).tag("plan-copy"))
                .child(div().child(Button::new("plan-discard", "Discard", Kind::Danger).action(DiscardPlan)).tag("plan-discard"))
                .child(div().flex_1())
                .when_some(card.notice.clone(), |row, notice| {
                    row.child(div().min_w_0().truncate().typography(Type::Small).text_color(theme.text_dim).child(notice))
                })
                .into_any_element()
        }
    });

    let card_view = div()
        .rounded(px(16.))
        .bg(theme.surface)
        .border_1()
        .border_color(line)
        .overflow_hidden()
        .when(current, |card| {
            card.shadow(vec![
                BoxShadow { color: Hsla { a: 0.10, ..tone }, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(4.), inset: false },
                BoxShadow {
                    color: Hsla { a: 0.22, ..theme.sidebar },
                    offset: point(px(0.), px(12.)),
                    blur_radius: px(32.),
                    spread_radius: px(0.),
                    inset: false,
                },
            ])
        })
        .child(head)
        .child(body)
        .children(footer);
    div().py(px(4.)).child(card_view).tag("plan-card")
}
