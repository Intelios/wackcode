//! Suggested-task cards: the follow-ups the agent filed with `suggest_task` while it worked —
//! a bug, a rough edge, a missing bit of polish it noticed but did not touch — drawn as a tray
//! under a settled turn's answer. Ported from version 2's `SuggestionTray.tsx`.
//!
//! A card is a task, not a widget: unlike a visual reply it stays actionable on older turns,
//! because "I'll come back to this" has to survive a few more messages. Its state comes from the
//! conversation and the chat's saved metadata, never from this view: started suggestions are read
//! out of the chat's own later messages, opened chats through `Chat::origin`, dismissals through
//! `Chat::dismissed_suggestions`, and queued sends through the composer's queue. So a deleted
//! chat's card becomes actionable again, and nothing is started by accident — "Start here"
//! queues behind a run like any composer message, rather than racing it.

use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gpui::{Animation, AnimationExt, App, ClickEvent, Hsla, SharedString, Window, div, prelude::*, px};

use crate::{
    actions::{CopySuggestion, DismissSuggestion, OpenChat, StartSuggestion, SuggestionTarget},
    button::{Button, Kind as ButtonKind},
    icon::Icon,
    markdown,
    motion,
    suggestions::{Card, Kind},
    tags::Tag,
    theme::Theme,
    transcript::Handlers,
    typography::{Type, Typography},
    ultra::UltraMark,
};

/// A brief longer than this gets a Show more: the card is a summary, not the whole ticket.
const CLAMP_AT: usize = 260;
/// A clamped brief shows about four lines.
const CLAMP_HEIGHT: f32 = 76.;
/// The bulb's one glint as the tray arrives.
const GLINT: Duration = Duration::from_millis(1100);

/// One turn's suggested tasks, drawn under its answer once it has settled.
#[derive(Clone, Debug)]
pub struct Tray {
    pub cards: Vec<Item>,
}

/// One suggestion as the tray draws it, with its state resolved.
#[derive(Clone, Debug)]
pub struct Item {
    /// The `suggest_task` call id: the card's identity, and its toggle keys' suffix.
    pub id: SharedString,
    pub title: SharedString,
    pub kind: Kind,
    /// The brief, parsed once when the item is built.
    pub brief: Rc<Vec<markdown::Block>>,
    /// Long enough to clamp behind Show more.
    pub long: bool,
    pub state: State,
    /// The "more ways to start" menu is open.
    pub menu_open: bool,
    /// Show more was pressed.
    pub expanded: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Waiting on the user.
    Open,
    /// Sent to this chat once its run ends.
    Queued,
    /// Started right here.
    StartedHere,
    /// Opened in a chat of its own (`Chat::origin`), which the button selects.
    OpenedIn { chat: String, name: SharedString },
    /// Folded away behind a title and a Restore.
    Dismissed,
}

impl Item {
    pub fn new(card: Card, state: State, open: &HashSet<String>) -> Self {
        Self {
            title: card.title.into(),
            kind: card.kind,
            long: card.description.chars().count() > CLAMP_AT,
            brief: Rc::new(markdown::parse(&card.description)),
            state,
            menu_open: open.contains(&menu_key(&card.id)),
            expanded: open.contains(&more_key(&card.id)),
            id: card.id.into(),
        }
    }

    /// Whether this card owns the toggle key, so the session knows which entry to rebuild.
    pub fn holds(&self, key: &str) -> bool {
        key == menu_key(&self.id) || key == more_key(&self.id)
    }
}

/// The `open` key the "more ways" menu rides on — the transcript's toggle does the work, so
/// gallery scenes stay inert.
fn menu_key(id: &str) -> String {
    format!("suggestion-menu-{id}")
}

fn more_key(id: &str) -> String {
    format!("suggestion-more-{id}")
}

fn kind_icon(kind: Kind) -> Icon {
    match kind {
        Kind::Bug => Icon::Alert,
        Kind::Polish => Icon::Spark,
        Kind::Improvement => Icon::Plus,
    }
}

/// The tone a kind wears. A bug keeps the fixed danger tone — the accent recolours interaction,
/// never meaning — the rest take the accent.
fn kind_tone(kind: Kind, theme: &Theme) -> Hsla {
    match kind {
        Kind::Bug => theme.danger,
        _ => theme.accent_text,
    }
}

/// One glint when the tray arrives: the bulb catching the light, never a loop.
fn glint(id: &str, theme: &Theme) -> impl IntoElement {
    let ring = div().absolute().rounded_full().border_2().border_color(theme.accent_line);
    ring.with_animation(SharedString::from(format!("{id}-glint")), Animation::new(GLINT), move |ring, t| {
        // Fades in fast, out slow; grows from a tight halo to past the mark.
        let opacity = if t < 0.35 { t / 0.35 } else { (1. - t) / 0.65 };
        let grow = -4. - 10. * t;
        ring.opacity(opacity.clamp(0., 0.9)).left(px(grow)).top(px(grow)).size(px(36. + 20. * t))
    })
}

/// The tray's head: the lit mark, a title, and how many follow-ups are open.
fn head(cards: &[Item], theme: &Theme) -> impl IntoElement {
    let open = cards.iter().filter(|card| card.state != State::Dismissed).count();
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(4.))
        .child(
            div()
                .relative()
                .flex_none()
                .size(px(28.))
                .rounded_full()
                .bg(theme.accent_wash)
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::Bulb.view(px(15.), theme.accent_text))
                .child(glint("suggestion-tray", theme)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(div().typography(Type::UiStrong).text_color(theme.text).child("Spotted along the way"))
                .child(
                    div()
                        .typography(Type::Small)
                        .text_color(theme.text_dim)
                        .child(if open == 1 {
                            "One follow-up the agent left for later.".to_string()
                        } else {
                            format!("{open} follow-ups the agent left for later.")
                        }),
                ),
        )
}

/// A kind's pill: its icon and word on its own tint.
fn kind_pill(kind: Kind, theme: &Theme) -> impl IntoElement {
    let tone = kind_tone(kind, theme);
    div()
        .flex_none()
        .h(px(19.))
        .px(px(7.))
        .rounded_full()
        .bg(Hsla { a: 0.12, ..tone })
        .border_1()
        .border_color(Hsla { a: 0.3, ..tone })
        .flex()
        .items_center()
        .gap(px(4.))
        .child(kind_icon(kind).view(px(10.), tone))
        .child(div().typography(Type::Small).font_weight(gpui::FontWeight::SEMIBOLD).text_color(tone).child(kind.label()))
}

/// A quiet word-as-button, for Restore and Show more.
fn text_button(id: &str, label: impl Into<SharedString>, theme: &Theme, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id.to_string()))
        .flex_none()
        .typography(Type::Small)
        .text_color(theme.accent_text)
        .hover(|button| button.underline())
        .cursor_pointer()
        .on_click(on_click)
        .child(label.into())
}

/// One card's action row or its standing, by state.
fn footing(card: &Item, theme: &Theme, handlers: &Handlers) -> gpui::Div {
    let row = div().px(px(14.)).pb(px(12.)).pt(px(2.)).flex().items_center().gap(px(8.));
    match &card.state {
        State::Open => row
            .child(Button::new(format!("{}-new", card.id), "Start in new chat", ButtonKind::Primary).action(StartSuggestion {
                id: card.id.to_string(),
                target: SuggestionTarget::New,
                mode: wack_core::AgentMode::Build,
            }))
            .child(Button::new(format!("{}-here", card.id), "Start here", ButtonKind::Secondary).action(StartSuggestion {
                id: card.id.to_string(),
                target: SuggestionTarget::Here,
                mode: wack_core::AgentMode::Build,
            }))
            .child(div().flex_1())
            .child(menu_button(card, theme, handlers)),
        State::Queued => row.child(state_line(theme.success, Icon::Check, "Queued for this chat", theme)),
        State::StartedHere => row.child(state_line(theme.success, Icon::Check, "Started in this chat", theme)),
        State::OpenedIn { chat, name } => row
            .child(state_line(theme.success, Icon::Check, "Opened in", theme))
            .child({
                let chat = chat.clone();
                let label = format!("“{name}”");
                text_button(&format!("{}-open", card.id), label, theme, move |_, window, cx| {
                    window.dispatch_action(Box::new(OpenChat { id: chat.clone() }), cx)
                })
            }),
        State::Dismissed => row,
    }
}

/// A settled card's standing: an icon and a line in the success colour.
fn state_line(tone: Hsla, icon: Icon, text: impl IntoElement, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .typography(Type::Small)
        .text_color(theme.text_dim)
        .child(icon.view(px(12.), tone))
        .child(text)
}

/// One row of the "more ways" menu.
fn menu_row(
    id: &str,
    label: SharedString,
    icon: Option<gpui::AnyElement>,
    theme: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id.to_string()))
        .mx(px(6.))
        .px(px(8.))
        .h(px(30.))
        .rounded(px(7.))
        .flex()
        .items_center()
        .gap(px(8.))
        .hover(|row| row.bg(theme.hover))
        .cursor_pointer()
        .on_click(on_click)
        .when_some(icon, |row, icon| row.child(div().w(px(14.)).flex_none().flex().items_center().child(icon)))
        .child(div().flex_1().min_w_0().truncate().typography(Type::Ui).text_color(theme.text).child(label))
}

/// The chevron and its popover: Plan it or Ultra Plan it, here or in a new chat, then Copy.
fn menu_button(card: &Item, theme: &Theme, handlers: &Handlers) -> impl IntoElement {
    let key = menu_key(&card.id);
    let toggle = handlers.toggle.clone();
    let menu_key: SharedString = key.clone().into();
    let menu_toggle = {
        let key = menu_key.clone();
        let toggle = toggle.clone();
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| toggle(key.clone(), window, cx)
    };
    let card_id = card.id.to_string();
    let start = |target: SuggestionTarget, mode| {
        let key = menu_key.clone();
        let toggle = toggle.clone();
        let id = card_id.clone();
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            window.dispatch_action(Box::new(StartSuggestion { id: id.clone(), target, mode }), cx);
            toggle(key.clone(), window, cx);
        }
    };
    let copy = {
        let key = menu_key.clone();
        let toggle = toggle.clone();
        let id = card_id.clone();
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            window.dispatch_action(Box::new(CopySuggestion { id: id.clone() }), cx);
            toggle(key.clone(), window, cx);
        }
    };
    let popover = div()
        .id(SharedString::from(format!("{}-menu", card_id)))
        .occlude()
        .absolute()
        .bottom(px(40.))
        .right(px(0.))
        .min_w(px(220.))
        .py(px(6.))
        .rounded(px(12.))
        .bg(theme.raised)
        .border_1()
        .border_color(theme.border_strong)
        .shadow_lg()
        .flex()
        .flex_col()
        .child(menu_row(
            &format!("{}-plan-new", card.id),
            "Plan it in a new chat".into(),
            Some(Icon::Brain.view(px(13.), theme.text_dim).into_any_element()),
            theme,
            start(SuggestionTarget::New, wack_core::AgentMode::Plan),
        ))
        .child(menu_row(
            &format!("{}-plan-here", card.id),
            "Plan it here".into(),
            Some(Icon::Brain.view(px(13.), theme.text_dim).into_any_element()),
            theme,
            start(SuggestionTarget::Here, wack_core::AgentMode::Plan),
        ))
        .child(menu_row(
            &format!("{}-ultra-new", card.id),
            "Ultra Plan it in a new chat".into(),
            Some(UltraMark::new(SharedString::from(format!("{}-un", card.id)), px(13.)).into_any_element()),
            theme,
            start(SuggestionTarget::New, wack_core::AgentMode::UltraPlan),
        ))
        .child(menu_row(
            &format!("{}-ultra-here", card.id),
            "Ultra Plan it here".into(),
            Some(UltraMark::new(SharedString::from(format!("{}-uh", card.id)), px(13.)).into_any_element()),
            theme,
            start(SuggestionTarget::Here, wack_core::AgentMode::UltraPlan),
        ))
        .child(div().my(px(6.)).h(px(1.)).bg(theme.border))
        .child(menu_row(
            &format!("{}-copy", card.id),
            "Copy as prompt".into(),
            Some(Icon::Copy.view(px(13.), theme.text_dim).into_any_element()),
            theme,
            copy,
        ));

    div()
        .relative()
        .flex_none()
        .child(
            div()
                .id(SharedString::from(format!("{}-more", card.id)))
                .size(px(30.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(theme.border_strong)
                .hover(|button| button.bg(theme.hover))
                .cursor_pointer()
                .tooltip(crate::tooltip::tip("More ways to start"))
                .on_click(menu_toggle)
                .child(Icon::ChevronDown.view(px(13.), theme.text_dim)),
        )
        .when(card.menu_open, |wrap| wrap.child(popover))
}

/// One card, states resolved already.
fn card_view(card: &Item, theme: &Theme, handlers: &Handlers) -> impl IntoElement {
    if card.state == State::Dismissed {
        return div()
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(14.))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .px(px(14.))
            .py(px(10.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .typography(Type::Ui)
                    .text_color(theme.text_faint)
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(Icon::Close.view(px(11.), theme.text_faint))
                    .child(card.title.clone()),
            )
            .child({
                let id = card.id.to_string();
                text_button(&format!("{}-restore", card.id), "Restore", theme, move |_, window, cx| {
                    window.dispatch_action(Box::new(DismissSuggestion { id: id.clone(), dismissed: false }), cx)
                })
            })
            .into_any_element();
    }

    let started = matches!(card.state, State::StartedHere | State::OpenedIn { .. });
    // The kind's tint along the card's left edge; a started card's turns to success.
    let edge = if started { theme.success } else { kind_tone(card.kind, theme) };
    let dismiss = {
        let id = card.id.to_string();
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            window.dispatch_action(Box::new(DismissSuggestion { id: id.clone(), dismissed: true }), cx)
        }
    };
    let more = {
        let key: SharedString = more_key(&card.id).into();
        let toggle = handlers.toggle.clone();
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| toggle(key.clone(), window, cx)
    };

    div()
        .relative()
        .rounded(px(14.))
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .child(div().absolute().left(px(-1.)).top(px(10.)).bottom(px(10.)).w(px(3.)).rounded_r(px(2.)).bg(Hsla { a: 0.75, ..edge }))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(9.))
                .pl(px(15.))
                .pr(px(12.))
                .pt(px(11.))
                .child(kind_pill(card.kind, theme))
                .child(div().flex_1().min_w_0().pt(px(1.)).typography(Type::UiStrong).text_color(theme.text).child(card.title.clone()))
                .when(!started, |head| {
                    head.child(
                        div()
                            .id(SharedString::from(format!("{}-dismiss", card.id)))
                            .flex_none()
                            .size(px(22.))
                            .mt(px(-2.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|button| button.bg(theme.hover))
                            .cursor_pointer()
                            .tooltip(crate::tooltip::tip("Dismiss"))
                            .on_click(dismiss)
                            .child(Icon::Close.view(px(11.), theme.text_dim)),
                    )
                }),
        )
        .child(
            div()
                .relative()
                .pl(px(15.))
                .pr(px(14.))
                .pt(px(4.))
                .when(card.long && !card.expanded, |brief| brief.max_h(px(CLAMP_HEIGHT)).overflow_hidden())
                .child(crate::transcript::reply(&card.brief, theme, handlers))
                // The clamped brief fades to the card, instead of cutting a line in half.
                .when(card.long && !card.expanded, |brief| {
                    brief.child(
                        div()
                            .absolute()
                            .bottom(px(0.))
                            .left(px(0.))
                            .right(px(0.))
                            .h(px(28.))
                            .bg(gpui::linear_gradient(
                                180.,
                                gpui::linear_color_stop(Hsla { a: 0., ..theme.surface }, 0.),
                                gpui::linear_color_stop(theme.surface, 1.),
                            )),
                    )
                }),
        )
        .when(card.long, |card_view| {
            card_view.child(div().pl(px(15.)).pt(px(2.)).child(text_button(
                &format!("{}-show", card.id),
                if card.expanded { "Show less" } else { "Show more" },
                theme,
                more,
            )))
        })
        .child(footing(card, theme, handlers))
        .into_any_element()
}

/// The whole tray for one settled turn.
pub fn tray(tray: &Tray, theme: &Theme, handlers: &Handlers) -> impl IntoElement {
    let mut cards = div().flex().flex_col().gap(px(8.));
    for card in &tray.cards {
        cards = cards.child(card_view(card, theme, handlers));
    }
    motion::enter(
        "suggestion-tray",
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(head(&tray.cards, theme))
            .child(cards),
    )
    .tag("suggestion-tray")
}
