//! The composer: where the user writes to the agent.
//!
//! One card of round controls: the mode track, the model's pill, the effort's own pill (which
//! opens WackCode 2's slider, Default through Ultra, staying open as it moves), the context meter
//! and the send button, the one filled control (the stop button while the agent works). In Ultra
//! Plan the card wears the prism's edge.
//!
//! Above the card, a row of status pills: what the run is doing and for how long, the todo list,
//! the waiting messages. The list and the messages open above their pills and fold back into
//! them (`ChatSession::todos_open`, `waiting_open`). An open question comes first: it stacks
//! above the pills, and while it is open the pills only report, with the list and the messages
//! folded into them.
//!
//! The mode toggle (Build | Plan) sits first in the controls, a recessed track with a pill that
//! slides to the mode in force. Clicking Plan again switches to Ultra Plan and back; ⇧Tab cycles
//! all three. It is locked while the agent works: a run keeps the mode it started with, unless
//! the agent itself asks to plan.
//!
//! Renders from plain data ([`ComposerData`]) and reports clicks as actions (`crate::actions`),
//! which the shell handles; in a gallery scene nothing handles them.

use std::{cell::Cell, rc::Rc};

use gpui::{
    AnchoredPositionMode, AnimationExt, App, Bounds, Entity, Focusable, Hsla, MouseButton, Pixels, SharedString, Window, anchored,
    deferred, div, point, prelude::*, px,
};
use wack_core::{AgentMode, Reasoning};

use crate::{
    actions::{
        ChooseEffort, ChooseModel, CloseMenu, Compact, OpenEffortMenu, OpenModelMenu, Send, SetAgentMode, SlideEffort, Stop, ToggleTodos,
        ToggleWaiting, Unqueue,
    },
    app::Menu,
    icon::Icon,
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    tooltip::tip,
    typography::{Type, Typography},
    ultra::{UltraLabel, UltraMark, prism_edge},
};

#[derive(Clone)]
pub enum Input {
    /// A live field.
    Field(Entity<TextField>),
    /// Drawn text, for scenes: the placeholder shows when `text` is `None`.
    Drawn { text: Option<SharedString>, placeholder: SharedString },
}

#[derive(Clone)]
pub struct ComposerData {
    pub input: Input,
    pub model: SharedString,
    pub effort: Option<SharedString>,
    /// The effort is Ultra: its word is drawn in the prism.
    pub ultra_effort: bool,
    /// Build, Plan or Ultra Plan.
    pub mode: AgentMode,
    /// Whose composer this is (a chat's id, or the new chat's), so switching chats never plays
    /// the toggle's slide.
    pub key: SharedString,
    /// "14%" of the context, or a token count when the model's window is unknown.
    pub meter: Option<Meter>,
    pub working: bool,
    /// Messages waiting for the run, oldest first.
    pub waiting: Vec<Waiting>,
    /// A taller card for the new-chat screen.
    pub roomy: bool,
    /// The model or effort menu, when open.
    pub menu: Option<MenuData>,
    /// The question the agent is waiting on, drawn above the waiting messages.
    pub question: Option<crate::question::QuestionData>,
    /// The agent's todo list, at the top of the column.
    pub todos: Option<crate::todo_panel::TodoData>,
    /// While a run is going: how long, and what it is doing now (`ChatSession::activity`).
    pub activity: Option<(SharedString, Option<SharedString>)>,
    /// The todo list and the waiting messages are open above their pills.
    pub todos_open: bool,
    pub waiting_open: bool,
}

#[derive(Clone, Debug)]
pub struct MenuData {
    pub kind: Menu,
    pub groups: Vec<MenuGroup>,
}

#[derive(Clone, Debug)]
pub struct MenuGroup {
    pub title: Option<SharedString>,
    pub items: Vec<MenuItem>,
}

#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub selected: bool,
    pub choice: Choice,
}

#[derive(Clone, Debug)]
pub enum Choice {
    Model { connection: String, model: String },
    Effort(Option<Reasoning>),
}

#[derive(Clone, Debug)]
pub struct Meter {
    pub label: SharedString,
    /// Getting full: drawn in the warning colour.
    pub high: bool,
    /// Offers Compact beside it.
    pub can_compact: bool,
}

#[derive(Clone, Debug)]
pub struct Waiting {
    pub text: SharedString,
    /// Sent to the agent mid-run, rather than queued for after it.
    pub steering: bool,
}

#[derive(IntoElement)]
pub struct Composer {
    data: ComposerData,
}

impl Composer {
    pub fn new(data: ComposerData) -> Self {
        Self { data }
    }
}

fn dispatch(action: impl gpui::Action) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static {
    let action = action.boxed_clone();
    move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)
}

impl RenderOnce for Composer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let data = self.data;
        let viewport = window.viewport_size();
        let menu_for = |kind: Menu| data.menu.clone().filter(|menu| menu.kind == kind);
        let has_text = match &data.input {
            Input::Field(field) => !field.read(cx).text().trim().is_empty(),
            Input::Drawn { text, .. } => text.is_some(),
        };

        let send = if data.working && !has_text {
            div()
                .id("stop")
                .flex_none()
                .size(px(30.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.raised)
                .border_1()
                .border_color(theme.border_strong)
                .hover(|button| button.bg(theme.hover))
                .on_click(dispatch(Stop))
                .child(Icon::Stop.view(px(14.), theme.text))
                .tag("stop")
                .into_any_element()
        } else {
            div()
                .id("send")
                .flex_none()
                .size(px(30.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .when(has_text, |button| button.bg(theme.accent_fill()).hover(|button| button.bg(theme.accent_fill_hover())))
                .when(!has_text, |button| button.bg(theme.hover))
                .on_click(dispatch(Send))
                .child(Icon::ArrowUp.view(px(15.), if has_text { theme.on_accent } else { theme.text_faint }))
                .tag("send")
                .into_any_element()
        };

        let field = match data.input.clone() {
            Input::Field(field) => div().child(field),
            Input::Drawn { text, placeholder } => div().typography(Type::Body).map(|text_box| match text {
                Some(typed) => text_box.text_color(theme.text).child(typed),
                None => text_box.text_color(theme.text_faint).child(placeholder),
            }),
        };
        let focus_target = match &data.input {
            Input::Field(field) => Some(field.clone()),
            Input::Drawn { .. } => None,
        };

        let ultra_plan = data.mode == AgentMode::UltraPlan;
        let field_height = if data.roomy { 64. } else { 22. };
        let mut controls = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(div().flex_none().w(px(24.)).flex().justify_center().child(Icon::Plus.view(px(16.), theme.text_dim)))
            .child(ModeToggle { mode: data.mode, locked: data.working, key: data.key.clone() })
            // In a narrow composer the model's name gives way first; the send button never does.
            .child(picker(
                "model",
                pill("model").min_w(px(MODEL_MIN)).text_color(theme.text_soft).hover(|pill| pill.bg(theme.hover).text_color(theme.text)).child(
                    div().min_w_0().truncate().typography(Type::Ui).child(data.model.clone()),
                ).child(Icon::ChevronDown.view(px(12.), theme.text_faint)),
                menu_for(Menu::Model).map(|menu| menu_view(menu, &theme).into_any_element()),
                true,
                viewport,
            ));
        if let Some(effort) = data.effort.clone() {
            let ultra = data.ultra_effort;
            let button = pill("effort")
                .flex_none()
                .gap(px(6.))
                .when(ultra, |pill| pill.bg(Hsla { a: 0.12, ..theme.ultra }).border_1().border_color(Hsla { a: 0.24, ..theme.ultra }))
                .when(!ultra, |pill| pill.hover(|pill| pill.bg(theme.hover)))
                .child(if ultra {
                    UltraMark::new("effort-ultra-mark", px(12.)).into_any_element()
                } else {
                    Icon::Brain.view(px(13.), theme.accent_text).into_any_element()
                })
                .child(if ultra {
                    UltraLabel::new("effort-ultra", effort, Type::Ui).into_any_element()
                } else {
                    div().typography(Type::Ui).text_color(theme.accent_text).child(effort).into_any_element()
                });
            let slider = menu_for(Menu::Effort).map(|menu| effort_slider(menu, data.model.clone(), &theme).into_any_element());
            controls = controls.child(picker("effort", button, slider, false, viewport));
        }
        controls = controls.child(div().flex_1());
        if let Some(meter) = data.meter {
            controls = controls.child(div().flex_none().pr(px(4.)).child(meter_view(meter, data.working, &theme)));
        }
        let controls = controls.child(send);

        let card = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(CARD_GAP))
            .pl(px(16.))
            .pr(px(12.))
            .pt(px(CARD_TOP))
            .pb(px(10.))
            .rounded(px(CARD_RADIUS))
            .bg(theme.surface)
            .when(!ultra_plan, |card| card.border_1().border_color(theme.border))
            .child(
                field
                    .min_h(px(field_height))
                    .when_some(focus_target, |field, target| {
                        // The whole text area takes a click, not only the line of text.
                        field.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            let handle = target.read(cx).focus_handle(cx);
                            window.focus(&handle, cx);
                        })
                    }),
            )
            .child(controls)
            .tag("composer");
        let card = if ultra_plan { prism_edge(CARD_RADIUS, card, &theme).into_any_element() } else { card.into_any_element() };

        // Inside a capped column the card stays whole and the question gives way first.
        let mut column = div().w_full().min_h_0().flex().flex_col().gap(px(6.));
        let asking = data.question.is_some();
        // While a question is open the pills only report: the question has the room.
        let todos_open = data.todos_open && !asking;
        let todo_pill = data.todos.as_ref().map(|todos| {
            let view = &todos.view;
            let live = todos.busy && view.active;
            status_pill("todos-pill", todos_open, !asking, &theme)
                .when(!asking, |pill| pill.on_click(dispatch(ToggleTodos)))
                .child(div().size(px(6.)).rounded_full().bg(if live { theme.accent_text } else { theme.border_strong }))
                .child(div().typography(Type::Small).text_color(theme.text_soft).child(format!("{} of {} tasks", view.done, view.total)))
                .when(!asking, |pill| pill.child(if todos_open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(11.), theme.text_faint)))
                .tag("todos-pill")
        });
        if let Some(todos) = data.todos
            && todos_open
        {
            column = column.child(div().flex_shrink(1.).min_h(px(40.)).flex().flex_col().child(crate::todo_panel::todo_panel(todos, &theme)));
        }
        if let Some(question) = data.question {
            column = column.child(
                div().min_h(px(QUESTION_MIN_HEIGHT)).flex_shrink(1.).flex().flex_col().pb(px(4.)).child(crate::question::QuestionCard::new(question)),
            );
        }
        let waiting_pill = (!data.waiting.is_empty()).then(|| {
            let steering = data.waiting.iter().filter(|waiting| waiting.steering).count();
            let queued = data.waiting.len() - steering;
            let words = match (steering, queued) {
                (0, queued) => format!("{queued} queued"),
                (steering, 0) => format!("{steering} steering"),
                (steering, queued) => format!("{steering} steering · {queued} queued"),
            };
            status_pill("waiting-pill", data.waiting_open && !asking, !asking, &theme)
                .when(!asking, |pill| pill.on_click(dispatch(ToggleWaiting)))
                .child(div().typography(Type::Small).text_color(theme.text_soft).child(words))
                .when(!asking, |pill| {
                    pill.child(if data.waiting_open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(11.), theme.text_faint))
                })
                .tag("waiting-pill")
        });
        if data.waiting_open && !asking && !data.waiting.is_empty() {
            // A long queue scrolls inside itself.
            let mut rows = div().id("waiting").flex_shrink(1.).min_h(px(36.)).max_h(px(WAITING_MAX_HEIGHT)).overflow_y_scroll().flex().flex_col().gap(px(6.));
            for (index, waiting) in data.waiting.into_iter().enumerate() {
                rows = rows.child(div().flex_none().child(waiting_row(index, waiting, &theme)));
            }
            column = column.child(rows);
        }
        let activity_pill = data.activity.map(|(time, doing)| {
            div()
                .min_w_0()
                .h(px(STATUS_HEIGHT))
                .px(px(12.))
                .rounded_full()
                .flex()
                .items_center()
                .gap(px(7.))
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(crate::motion::paddling(div().flex_none().child(Icon::Duck.view(px(13.), theme.accent_text))))
                .child(div().flex_none().typography(Type::Small).text_color(theme.text).child(time))
                .when_some(doing, |pill, doing| {
                    pill.child(div().min_w_0().truncate().typography(Type::Small).text_color(theme.text_dim).child(doing))
                })
                .tag("activity")
        });
        if activity_pill.is_some() || todo_pill.is_some() || waiting_pill.is_some() {
            column = column.child(
                div()
                    .flex_none()
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .children(activity_pill)
                    .children(todo_pill)
                    .child(div().flex_1())
                    .children(waiting_pill),
            );
        }
        column.child(div().flex_none().child(card))
    }
}

/// The composer card's corners, the space above its field, and between the field and the bar.
const CARD_RADIUS: f32 = 18.;
const CARD_TOP: f32 = 14.;
const CARD_GAP: f32 = 10.;
/// The least of the model's pill a narrow composer keeps.
const MODEL_MIN: f32 = 64.;
/// How far above its pill a picker's popover opens.
const PICKER_GAP: f32 = 6.;
/// A status pill's height.
const STATUS_HEIGHT: f32 = 26.;

/// A round control in the composer's bar.
fn pill(tag: &'static str) -> gpui::Stateful<gpui::Div> {
    div().id(tag).h(px(28.)).px(px(10.)).rounded_full().flex().items_center().gap(px(4.))
}

/// A status pill over the card that opens what it stands for (when `opens`); lit while open.
fn status_pill(tag: &'static str, open: bool, opens: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(tag)
        .flex_none()
        .h(px(STATUS_HEIGHT))
        .px(px(12.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(7.))
        .bg(theme.surface)
        .border_1()
        .border_color(if open { theme.border_strong } else { theme.border })
        .when(opens, |pill| pill.hover(|pill| pill.border_color(theme.border_strong)))
}

/// A message waiting above the card. A queued one can be taken back to edit.
fn waiting_row(index: usize, waiting: Waiting, theme: &Theme) -> impl IntoElement {
    div()
        .id(("waiting", index))
        .group("waiting")
        .mx(px(4.))
        .px(px(14.))
        .py(px(6.))
        .flex()
        .items_center()
        .gap(px(10.))
        .rounded(px(14.))
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .child(
            div()
                .flex_none()
                .typography(Type::Small)
                .text_color(if waiting.steering { theme.accent_text } else { theme.text_dim })
                .child(if waiting.steering { "Steering" } else { "Next" }),
        )
        .child(div().flex_1().min_w_0().truncate().typography(Type::Ui).text_color(theme.text_soft).child(waiting.text))
        .when(!waiting.steering && index != usize::MAX, |row| {
            row.child(
                div()
                    .id(("unqueue", index))
                    .flex_none()
                    .invisible()
                    .group_hover("waiting", |button| button.visible())
                    .typography(Type::Small)
                    .text_color(theme.text_dim)
                    .hover(|button| button.text_color(theme.text))
                    .on_click(dispatch(Unqueue { index }))
                    .child("Edit"),
            )
        })
}

/// "14%" of the context, and Compact beside it when there is something to compact.
fn meter_view(meter: Meter, working: bool, theme: &Theme) -> impl IntoElement {
    let colour: Hsla = if meter.high { theme.warning } else { theme.text_faint };
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .typography(Type::Small)
        .child(div().id("meter").text_color(colour).tooltip(tip("How full the model’s context is")).child(meter.label))
        .when(meter.can_compact && !working, |row| {
            row.child(div().text_color(theme.text_faint).child("·")).child(
                div()
                    .id("compact")
                    .text_color(if meter.high { theme.text_soft } else { theme.text_dim })
                    .hover(|word| word.text_color(theme.accent_text))
                    .on_click(dispatch(Compact))
                    .child("Compact")
                    .tag("compact"),
            )
        })
}

/// About three waiting messages; more scroll.
const WAITING_MAX_HEIGHT: f32 = 118.;
/// The least an open question card keeps: its head, a little of its body, its buttons.
const QUESTION_MIN_HEIGHT: f32 = 196.;

/// A pill that opens a popover upwards from itself (the model's menu, the effort's slider), over
/// everything, its bottom [`PICKER_GAP`] above the pill's top so it opens where the click was; a
/// click anywhere else closes it. `shrinks` lets it give way in a narrow composer.
fn picker(
    tag: &'static str,
    button: gpui::Stateful<gpui::Div>,
    popover: Option<gpui::AnyElement>,
    shrinks: bool,
    viewport: gpui::Size<gpui::Pixels>,
) -> impl IntoElement {
    let open = match tag {
        "model" => Box::new(OpenModelMenu) as Box<dyn gpui::Action>,
        _ => Box::new(OpenEffortMenu),
    };
    let button = button.on_click(move |_, window, cx| window.dispatch_action(open.boxed_clone(), cx)).tag(tag);
    div().relative().when(shrinks, |picker| picker.min_w(px(MODEL_MIN))).when(!shrinks, |picker| picker.flex_none()).child(button).when_some(popover, |picker, popover| {
        let backdrop = anchored().position_mode(AnchoredPositionMode::Window).position(point(px(0.), px(0.))).child(
            div().id("menu-backdrop").w(viewport.width).h(viewport.height).on_mouse_down(MouseButton::Left, |_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(Box::new(CloseMenu), cx);
            }),
        );
        // The card rises into place above the pill that opened it, kept inside the window.
        let card = div().absolute().top_0().left_0().child(
            anchored()
                .anchor(gpui::Anchor::BottomLeft)
                .offset(point(px(0.), px(-PICKER_GAP)))
                .snap_to_window_with_margin(px(8.))
                .child(crate::motion::enter(gpui::ElementId::Name(format!("arrive-{tag}-menu").into()), div().child(popover))),
        );
        picker.child(deferred(backdrop).with_priority(1)).child(deferred(card).with_priority(2))
    })
}

/// The effort slider's width, and its track's inside it.
const SLIDER_WIDTH: f32 = 320.;
const SLIDER_PAD: f32 = 14.;
const TRACK_HEIGHT: f32 = 30.;
const KNOB: f32 = 24.;

/// Where stop `index` of `count` sits along a track `width` wide: the knob's centre.
fn stop_x(index: usize, count: usize, width: f32) -> f32 {
    let inset = TRACK_HEIGHT / 2.;
    inset + (width - 2. * inset) * index as f32 / (count.max(2) - 1) as f32
}

/// The stop nearest `x` along a track `width` wide.
fn nearest_stop(x: f32, count: usize, width: f32) -> usize {
    let inset = TRACK_HEIGHT / 2.;
    let along = ((x - inset) / (width - 2. * inset).max(1.)).clamp(0., 1.);
    (along * (count.max(2) - 1) as f32).round() as usize
}

/// WackCode 2's effort slider, from the effort menu's choices (Default, then Low through Ultra):
/// a head naming the effort and the model, a track with a stop for each choice and a knob, and the
/// stops' names beneath. Clicking or dragging along the track, or clicking a name, moves the
/// effort at once and leaves the slider open (`SlideEffort`).
fn effort_slider(menu: MenuData, model: SharedString, theme: &Theme) -> impl IntoElement {
    let items: Vec<MenuItem> = menu.groups.into_iter().flat_map(|group| group.items).collect();
    let choices: Vec<Option<Reasoning>> = items
        .iter()
        .map(|item| match item.choice {
            Choice::Effort(effort) => effort,
            Choice::Model { .. } => None,
        })
        .collect();
    let count = items.len();
    let current = items.iter().position(|item| item.selected).unwrap_or(0);
    let ultra = choices.get(current).copied().flatten() == Some(Reasoning::Ultra);
    let on = current > 0;
    let width = SLIDER_WIDTH - 2. * SLIDER_PAD;
    let (tone, wash) = if ultra { (theme.ultra, Hsla { a: 0.14, ..theme.ultra }) } else { (theme.accent, theme.accent_wash) };
    let selected = &items[current.min(count.saturating_sub(1))];

    let head = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .flex_none()
                .size(px(30.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(if on { wash } else { theme.hover })
                .child(if ultra {
                    UltraMark::new("slider-ultra-mark", px(14.)).into_any_element()
                } else {
                    Icon::Brain.view(px(15.), if on { theme.accent_text } else { theme.text_dim }).into_any_element()
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(if ultra {
                    UltraLabel::new("slider-ultra", selected.label.clone(), Type::UiStrong).into_any_element()
                } else {
                    div()
                        .typography(Type::UiStrong)
                        .text_color(if on { theme.accent_text } else { theme.text })
                        .child(selected.label.clone())
                        .into_any_element()
                })
                .child(div().truncate().typography(Type::Small).text_color(theme.text_dim).child(model)),
        )
        .when_some(selected.detail.clone(), |head, detail| {
            head.child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(detail))
        });

    // The track remembers where it was drawn, so a press or a drag finds the nearest stop.
    let drawn: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::default();
    let pick = {
        let drawn = drawn.clone();
        let choices = choices.clone();
        move |x: Pixels, window: &mut Window, cx: &mut App| {
            let Some(bounds) = drawn.get() else {
                return;
            };
            let index = nearest_stop(f32::from(x - bounds.left()), count, f32::from(bounds.size.width));
            if index != current {
                window.dispatch_action(Box::new(SlideEffort { effort: choices[index] }), cx);
            }
        }
    };
    let pick: crate::slide::Pick = Rc::new(pick);
    let knob_x = stop_x(current, count, width);
    let fill_ground = if ultra { theme.ultra.into() } else { theme.accent_fill() };
    let mut track = div()
        .id("effort-track")
        .relative()
        .h(px(TRACK_HEIGHT))
        .rounded_full()
        .bg(theme.hover)
        .child(crate::slide::follow("effort-track", drawn, pick.clone()).absolute().left_0().top_0().size_full())
        .when(on, |track| {
            track.child(div().absolute().left_0().top_0().bottom_0().w(px(knob_x + TRACK_HEIGHT / 2.)).rounded_full().bg(fill_ground))
        });
    for index in 0..count {
        let lit = on && index <= current;
        track = track.child(
            div()
                .absolute()
                .top(px(TRACK_HEIGHT / 2. - 2.))
                .left(px(stop_x(index, count, width) - 2.))
                .size(px(4.))
                .rounded_full()
                .bg(if lit { Hsla { a: 0.45, ..theme.on_accent } } else { theme.border_strong }),
        );
    }
    let track = track
        .child(
            div()
                .absolute()
                .top(px((TRACK_HEIGHT - KNOB) / 2.))
                .left(px(knob_x - KNOB / 2.))
                .size(px(KNOB))
                .rounded_full()
                .bg(theme.raised)
                .border_1()
                .border_color(theme.border_strong)
                .shadow_sm(),
        )
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            cx.stop_propagation();
            crate::slide::begin("effort-track");
            pick(event.position.x, window, cx)
        })
        .tag("effort-track");

    let mut names = div().relative().h(px(16.)).mt(px(7.));
    for (index, item) in items.iter().enumerate() {
        let effort = choices[index];
        let chosen = index == current;
        let label = item.label.clone();
        names = names.child(
            div()
                .id(("effort-stop", index))
                .absolute()
                .top_0()
                .left(px(stop_x(index, count, width) - STOP_LABEL / 2.))
                .w(px(STOP_LABEL))
                .flex()
                .justify_center()
                .on_click(move |_, window, cx| window.dispatch_action(Box::new(SlideEffort { effort }), cx))
                .child(if effort == Some(Reasoning::Ultra) {
                    UltraLabel::new(("slider-stop-ultra", index), label, Type::Small).decode_on_arrival(false).into_any_element()
                } else {
                    div()
                        .typography(Type::Small)
                        .when(chosen, |name| name.font_weight(gpui::FontWeight::SEMIBOLD))
                        .text_color(if chosen { theme.text } else { theme.text_faint })
                        .hover(|name| name.text_color(theme.text))
                        .child(label)
                        .into_any_element()
                }),
        );
    }

    div()
        .id("effort-slider")
        // Presses inside stay inside: the click-away layer beneath must not close the slider.
        .occlude()
        .w(px(SLIDER_WIDTH))
        .p(px(SLIDER_PAD))
        .pb(px(10.))
        .rounded(px(16.))
        .bg(theme.raised)
        .border_1()
        .border_color(if ultra { Hsla { a: 0.34, ..tone } } else { theme.border_strong })
        .shadow_lg()
        .flex()
        .flex_col()
        .gap(px(14.))
        .child(head)
        .child(div().flex().flex_col().child(track).child(names))
        .tag("effort-slider")
}

/// The room each stop's name has beneath the track.
const STOP_LABEL: f32 = 46.;

fn menu_view(menu: MenuData, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let mut card = div()
        .id("menu")
        .occlude()
        .min_w(px(260.))
        .max_w(px(420.))
        .py(px(6.))
        .rounded(px(12.))
        .bg(theme.raised)
        .border_1()
        .border_color(theme.border_strong)
        .shadow_lg()
        .flex()
        .flex_col();
    let mut index: usize = 0;
    for (number, group) in menu.groups.into_iter().enumerate() {
        if number > 0 {
            card = card.child(div().my(px(6.)).h(px(1.)).bg(theme.border));
        }
        if let Some(title) = group.title {
            card = card.child(div().px(px(14.)).pt(px(4.)).pb(px(2.)).typography(Type::Label).text_color(theme.text_dim).child(title));
        }
        for item in group.items {
            let ultra = matches!(item.choice, Choice::Effort(Some(Reasoning::Ultra)));
            let action: Box<dyn gpui::Action> = match item.choice {
                Choice::Model { connection, model } => Box::new(ChooseModel { connection, model }),
                Choice::Effort(effort) => Box::new(ChooseEffort { effort }),
            };
            card = card.child(
                div()
                    .id(("menu-item", index))
                    .mx(px(6.))
                    .px(px(8.))
                    .h(px(30.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .hover(|row| row.bg(theme.hover))
                    .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
                    .child(div().w(px(14.)).flex_none().when(item.selected, |mark| mark.child(Icon::Check.view(px(13.), theme.accent_text))))
                    .child(if ultra {
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(UltraLabel::new("menu-ultra", item.label, if item.selected { Type::UiStrong } else { Type::Ui }).decode_on_arrival(false))
                            .child(UltraMark::new("menu-ultra-mark", px(10.)))
                    } else {
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .typography(if item.selected { Type::UiStrong } else { Type::Ui })
                            .text_color(theme.text)
                            .child(item.label)
                    })
                    .when_some(item.detail, |row, detail| {
                        row.child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(detail))
                    }),
            );
            index += 1;
        }
    }
    card
}

/// The mode ⇧Tab moves to: Build → Plan → Ultra Plan → Build.
pub fn next_mode(mode: AgentMode) -> AgentMode {
    match mode {
        AgentMode::Build => AgentMode::Plan,
        AgentMode::Plan => AgentMode::UltraPlan,
        AgentMode::UltraPlan => AgentMode::Build,
    }
}

/// What clicking Plan does: from Plan it goes on to Ultra Plan; from anywhere else, to Plan.
pub fn plan_button_target(mode: AgentMode) -> AgentMode {
    match mode {
        AgentMode::Plan => AgentMode::UltraPlan,
        _ => AgentMode::Plan,
    }
}

/// The toggle's options, in points: wide enough for each word and its icon, so the pill can
/// slide between known places without measuring text.
const BUILD_WIDTH: f32 = 66.;
const PLAN_WIDTH: f32 = 60.;
const ULTRA_PLAN_WIDTH: f32 = 96.;
const TRACK_PAD: f32 = 2.;
const OPTION_HEIGHT: f32 = 24.;
/// How long the pill takes to slide, and the icon's nudge.
const SLIDE: std::time::Duration = std::time::Duration::from_millis(220);
const NUDGE: std::time::Duration = std::time::Duration::from_millis(520);

/// Where the pill sits for `mode`: its left edge and width inside the track.
fn pill_place(mode: AgentMode) -> (f32, f32) {
    match mode {
        AgentMode::Build => (TRACK_PAD, BUILD_WIDTH),
        AgentMode::Plan => (TRACK_PAD + BUILD_WIDTH, PLAN_WIDTH),
        AgentMode::UltraPlan => (TRACK_PAD + BUILD_WIDTH, ULTRA_PLAN_WIDTH),
    }
}

/// The toggle's memory between frames: the mode it last showed, the one before, and a count of
/// switches, so each switch replays the slide and the nudge once.
struct Shown {
    mode: AgentMode,
    from: AgentMode,
    switches: usize,
}

/// Build | Plan, with a pill that slides to the mode in force. Switching into Build or Plan plays
/// a one-shot nudge on the icon that just became active (the hammer taps, the brain swells) and
/// a soft wash behind it; Ultra Plan is exempt, since its own entrance (the mark booting into its
/// orbit, the label decoding) is the louder moment.
#[derive(IntoElement)]
struct ModeToggle {
    mode: AgentMode,
    locked: bool,
    key: SharedString,
}

impl RenderOnce for ModeToggle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let mode = self.mode;
        let shown = window.use_keyed_state(SharedString::from(format!("mode-toggle-{}", self.key)), cx, |_, _| Shown { mode, from: mode, switches: 0 });
        let (from, switches) = shown.update(cx, |shown, _| {
            if shown.mode != mode {
                shown.from = shown.mode;
                shown.mode = mode;
                shown.switches += 1;
            }
            (shown.from, shown.switches)
        });
        let planning = mode.planning();
        let ultra = mode == AgentMode::UltraPlan;
        let tooltip = if self.locked {
            "Wait for the current run"
        } else {
            match mode {
                AgentMode::Plan => "Click Plan again for Ultra Plan (⇧Tab)",
                AgentMode::UltraPlan => "Click to go back to Plan (⇧Tab)",
                AgentMode::Build => "Switch mode (⇧Tab)",
            }
        };

        // The pill: plain for Build, the accent's wash for Plan, the prism's for Ultra Plan.
        let (pill_ground, pill_line) = match mode {
            AgentMode::Build => (theme.surface, None),
            AgentMode::Plan => (theme.accent_wash, None),
            AgentMode::UltraPlan => (Hsla { a: 0.14, ..theme.ultra }, Some(Hsla { a: 0.34, ..theme.ultra })),
        };
        let (to_left, to_width) = pill_place(mode);
        let (from_left, from_width) = pill_place(from);
        let pill = div()
            .absolute()
            .top(px(TRACK_PAD))
            .h(px(OPTION_HEIGHT))
            .rounded_full()
            .bg(pill_ground)
            .when_some(pill_line, |pill, line| pill.border_1().border_color(line))
            .when(self.locked, |pill| pill.opacity(0.55));
        let pill = if switches == 0 {
            pill.left(px(to_left)).w(px(to_width)).into_any_element()
        } else {
            pill.with_animation(
                ("mode-pill", switches),
                gpui::Animation::new(SLIDE).with_easing(crate::motion::spring),
                move |pill, t| {
                    pill.left(px(from_left + (to_left - from_left) * t)).w(px(from_width + (to_width - from_width) * t))
                },
            )
            .into_any_element()
        };

        // Only a switch the user just made nudges, and only into Build or Plan.
        let nudge = (switches > 0 && !self.locked).then_some(mode).filter(|mode| *mode != AgentMode::UltraPlan);
        let ink = |active: bool| if active { theme.text } else { theme.text_dim };

        let build_icon = Icon::Hammer.view(px(12.), ink(mode == AgentMode::Build));
        let build_icon = if nudge == Some(AgentMode::Build) {
            build_icon
                .with_animation(("mode-nudge", switches), gpui::Animation::new(NUDGE), |icon, t| {
                    icon.with_transformation(gpui::Transformation::rotate(gpui::radians(hammer_tap(t))))
                })
                .into_any_element()
        } else {
            build_icon.into_any_element()
        };
        let plan_icon = if ultra {
            UltraMark::new("mode-ultra-mark", px(12.)).animated(!self.locked).into_any_element()
        } else {
            let brain = Icon::Brain.view(px(12.), if planning { theme.accent_text } else { theme.text_dim });
            if nudge == Some(AgentMode::Plan) {
                brain
                    .with_animation(("mode-nudge", switches), gpui::Animation::new(NUDGE), |icon, t| {
                        let scale = brain_swell(t);
                        icon.with_transformation(gpui::Transformation::scale(gpui::size(scale, scale)))
                    })
                    .into_any_element()
            } else {
                brain.into_any_element()
            }
        };

        let option = |id: &'static str, width: f32, target: AgentMode| {
            div()
                .id(id)
                .relative()
                .flex_none()
                .w(px(width))
                .h(px(OPTION_HEIGHT))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(5.))
                .when(!self.locked, |option| option.on_click(dispatch(SetAgentMode { mode: target })))
        };
        // A soft wash blooms behind the option as the pill arrives.
        let wash = |colour: Hsla| {
            div().absolute().inset_0().rounded_full().bg(colour).with_animation(
                ("mode-wash", switches),
                gpui::Animation::new(NUDGE),
                |wash, t| wash.opacity(0.75 * (1. - t)),
            )
        };

        let build = option("mode-build", BUILD_WIDTH, AgentMode::Build)
            .when(nudge == Some(AgentMode::Build), |option| option.child(wash(Hsla { a: 0.18, ..theme.text })))
            .child(build_icon)
            .child(div().typography(Type::Small).text_color(ink(mode == AgentMode::Build)).child("Build"))
            .when(!self.locked && mode != AgentMode::Build, |option| option.hover(|option| option.bg(theme.hover)))
            .tag("mode-build");
        let plan = option("mode-plan", if ultra { ULTRA_PLAN_WIDTH } else { PLAN_WIDTH }, plan_button_target(mode))
            .when(nudge == Some(AgentMode::Plan), |option| option.child(wash(Hsla { a: 0.26, ..theme.accent })))
            .child(plan_icon)
            .child(if ultra {
                UltraLabel::new("mode-ultra-label", "Ultra Plan", Type::Small).enabled(!self.locked).into_any_element()
            } else {
                div()
                    .typography(Type::Small)
                    .text_color(if planning { theme.accent_text } else { theme.text_dim })
                    .child("Plan")
                    .into_any_element()
            })
            .when(!self.locked && !planning, |option| option.hover(|option| option.bg(theme.hover)))
            .tag("mode-plan");

        div()
            .id("mode-toggle")
            .flex_none()
            .relative()
            .h(px(OPTION_HEIGHT + 2. * TRACK_PAD))
            .px(px(TRACK_PAD))
            .flex()
            .items_center()
            .rounded_full()
            .bg(theme.sidebar)
            .border_1()
            .border_color(theme.border)
            .tooltip(tip(tooltip))
            .child(pill)
            .child(build)
            .child(plan)
            .when(self.locked, |toggle| toggle.opacity(0.7))
            .tag("mode-toggle")
    }
}

/// The hammer's tap, in radians, `t` (0–1) through: it lifts, strikes through rest, and recoils
/// once.
fn hammer_tap(t: f32) -> f32 {
    let keys = [(0., 0.), (0.3, -13.), (0.5, 6.), (0.68, -2.5), (1., 0.)];
    keyframes(&keys, t).to_radians()
}

/// The brain's swell, as a scale, `t` (0–1) through: it swells twice like a thought settling.
fn brain_swell(t: f32) -> f32 {
    keyframes(&[(0., 1.), (0.35, 1.15), (0.6, 0.97), (0.8, 1.06), (1., 1.)], t)
}

/// Eased steps between `(time, value)` keys.
pub fn keyframes(keys: &[(f32, f32)], t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    for pair in keys.windows(2) {
        let ((t0, v0), (t1, v1)) = (pair[0], pair[1]);
        if t <= t1 {
            let k = if t1 > t0 { (t - t0) / (t1 - t0) } else { 1. };
            let k = k * k * (3. - 2. * k);
            return v0 + (v1 - v0) * k;
        }
    }
    keys.last().map_or(0., |key| key.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slider_finds_the_nearest_stop() {
        let width = 292.;
        assert_eq!(stop_x(0, 7, width), 15.);
        assert_eq!(stop_x(6, 7, width), width - 15.);
        for index in 0..7 {
            assert_eq!(nearest_stop(stop_x(index, 7, width), 7, width), index);
            assert_eq!(nearest_stop(stop_x(index, 7, width) + 10., 7, width), index);
        }
        assert_eq!(nearest_stop(-40., 7, width), 0);
        assert_eq!(nearest_stop(width + 40., 7, width), 6);
    }

    #[test]
    fn shift_tab_cycles_all_three_and_plan_toggles_ultra() {
        assert_eq!(next_mode(AgentMode::Build), AgentMode::Plan);
        assert_eq!(next_mode(AgentMode::Plan), AgentMode::UltraPlan);
        assert_eq!(next_mode(AgentMode::UltraPlan), AgentMode::Build);
        assert_eq!(plan_button_target(AgentMode::Build), AgentMode::Plan);
        assert_eq!(plan_button_target(AgentMode::Plan), AgentMode::UltraPlan);
        assert_eq!(plan_button_target(AgentMode::UltraPlan), AgentMode::Plan);
    }

    #[test]
    fn the_nudges_end_at_rest() {
        assert_eq!(hammer_tap(0.), 0.);
        assert!(hammer_tap(1.).abs() < 1e-6);
        assert!(hammer_tap(0.3) < 0.);
        assert_eq!(brain_swell(0.), 1.);
        assert!((brain_swell(1.) - 1.).abs() < 1e-6);
        assert!(brain_swell(0.35) > 1.1);
    }
}
