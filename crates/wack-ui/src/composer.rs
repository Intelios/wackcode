//! The composer: where the user writes to the agent.
//!
//! One card, no toolbar of pills: the model and effort are plain words you click, the context
//! meter is a number, and the one filled control is the send button (the stop button while the
//! agent works). Messages waiting for the run sit just above the card, so the user sees what goes
//! next.
//!
//! The mode toggle (Build | Plan) sits first in the controls, a recessed track with a pill that
//! slides to the mode in force. Clicking Plan again switches to Ultra Plan and back; ⇧Tab cycles
//! all three. It is locked while the agent works: a run keeps the mode it started with, unless
//! the agent itself asks to plan.
//!
//! Renders from plain data ([`ComposerData`]) and reports clicks as actions (`crate::actions`),
//! which the shell handles; in a gallery scene nothing handles them.

use gpui::{
    AnchoredPositionMode, AnimationExt, App, Entity, Focusable, Hsla, MouseButton, SharedString, Window, anchored, deferred, div,
    point, prelude::*, px,
};
use wack_core::{AgentMode, Reasoning};

use crate::{
    actions::{ChooseEffort, ChooseModel, CloseMenu, Compact, OpenEffortMenu, OpenModelMenu, Send, SetAgentMode, Stop, Unqueue},
    app::Menu,
    icon::Icon,
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    tooltip::tip,
    typography::{Type, Typography},
    ultra::{UltraLabel, UltraMark},
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
    /// "14% context", or a token count when the model's window is unknown.
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
                .bg(if has_text { theme.accent } else { theme.hover })
                .when(has_text, |button| button.hover(|button| button.bg(theme.accent_hover)))
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

        let mut controls = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(Icon::Plus.view(px(16.), theme.text_dim))
            .child(ModeToggle { mode: data.mode, locked: data.working })
            // In a narrow composer the model's name gives way first; the send button never does.
            .child(picker("model", Word::Plain(data.model.clone()), theme.text_soft, OpenModelMenu, menu_for(Menu::Model), viewport, &theme));
        if let Some(effort) = data.effort.clone() {
            let word = if data.ultra_effort {
                Word::Ultra(effort)
            } else {
                Word::Plain(effort)
            };
            controls = controls.child(picker("effort", word, theme.text_dim, OpenEffortMenu, menu_for(Menu::Effort), viewport, &theme));
        }
        controls = controls.child(div().flex_1());
        if let Some(meter) = data.meter {
            controls = controls.child(meter_view(meter, data.working, &theme));
        }
        let controls = controls.child(send);

        let card = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .px(px(16.))
            .pt(px(14.))
            .pb(px(10.))
            .rounded(px(14.))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .child(
                field
                    .min_h(px(if data.roomy { 64. } else { 22. }))
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

        // Inside a capped column the card stays whole and the question gives way first.
        let mut column = div().w_full().min_h_0().flex().flex_col().gap(px(6.));
        // While a question is open it comes first: the todo list folds to its head and the
        // waiting messages to one line, and in a short window both step aside for it.
        let asking = data.question.is_some();
        let short = f32::from(viewport.height) < SHORT_WINDOW;
        if let Some(todos) = data.todos
            && !(asking && short)
        {
            column = column.child(div().flex_none().child(crate::todo_panel::todo_panel(todos, &theme)));
        }
        if let Some(question) = data.question {
            column = column.child(
                div().min_h(px(QUESTION_MIN_HEIGHT)).flex_shrink(1.).flex().flex_col().pb(px(4.)).child(crate::question::QuestionCard::new(question)),
            );
        }
        if asking && !data.waiting.is_empty() {
            if !short {
                let count = data.waiting.len();
                let summary = Waiting {
                    text: if count == 1 { "1 message waiting".into() } else { format!("{count} messages waiting").into() },
                    steering: false,
                };
                column = column.child(div().flex_none().child(waiting_row(usize::MAX, summary, &theme)));
            }
        } else {
            for (index, waiting) in data.waiting.into_iter().enumerate() {
                column = column.child(div().flex_none().child(waiting_row(index, waiting, &theme)));
            }
        }
        column.child(div().flex_none().child(card))
    }
}

/// A message waiting above the card. A queued one can be taken back to edit.
fn waiting_row(index: usize, waiting: Waiting, theme: &Theme) -> impl IntoElement {
    div()
        .id(("waiting", index))
        .group("waiting")
        .mx(px(14.))
        .px(px(12.))
        .py(px(6.))
        .flex()
        .items_center()
        .gap(px(10.))
        .rounded(px(9.))
        .bg(theme.sidebar)
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

/// "14% context", and Compact beside it when there is something to compact.
fn meter_view(meter: Meter, working: bool, theme: &Theme) -> impl IntoElement {
    let colour: Hsla = if meter.high { theme.warning } else { theme.text_faint };
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .typography(Type::Small)
        .child(div().text_color(colour).child(meter.label))
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

/// Below this window height an open question has the column to itself.
const SHORT_WINDOW: f32 = 600.;
/// The least an open question card keeps: its head, a little of its body, its buttons.
const QUESTION_MIN_HEIGHT: f32 = 196.;

/// What a picker shows.
enum Word {
    Plain(SharedString),
    /// Ultra, in the prism.
    Ultra(SharedString),
}

/// A word that opens a menu: the model, the effort. The menu opens upwards from the word, over
/// everything, and a click anywhere else closes it.
fn picker(
    tag: &'static str,
    name: Word,
    colour: Hsla,
    open: impl gpui::Action,
    menu: Option<MenuData>,
    viewport: gpui::Size<gpui::Pixels>,
    theme: &Theme,
) -> impl IntoElement {
    let word = div()
        .id(tag)
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(3.))
        .text_color(colour)
        .hover(|word| word.text_color(theme.text))
        .on_click(dispatch(open))
        .child(match name {
            Word::Plain(name) => div().min_w_0().truncate().typography(Type::Ui).child(name).into_any_element(),
            Word::Ultra(name) => UltraLabel::new("effort-ultra", name, Type::Ui).into_any_element(),
        })
        .child(Icon::ChevronDown.view(px(12.), theme.text_faint))
        .tag(tag);
    // Only the model's name gives way in a narrow composer; the effort is a short word.
    let shrinks = tag == "model";
    div().relative().when(shrinks, |picker| picker.min_w_0()).when(!shrinks, |picker| picker.flex_none()).child(word).when_some(menu, |picker, menu| {
        let backdrop = anchored().position_mode(AnchoredPositionMode::Window).position(point(px(0.), px(0.))).child(
            div().id("menu-backdrop").w(viewport.width).h(viewport.height).on_mouse_down(MouseButton::Left, |_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(Box::new(CloseMenu), cx);
            }),
        );
        // The card rises into place inside a box that holds its position above the word.
        let card = div().absolute().bottom(px(28.)).left(px(-12.)).child(crate::motion::enter(
            gpui::ElementId::Name(format!("arrive-{tag}-menu").into()),
            menu_view(menu, theme),
        ));
        picker.child(deferred(backdrop).with_priority(1)).child(deferred(card).with_priority(2))
    })
}

fn menu_view(menu: MenuData, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let mut card = div()
        .id("menu")
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
}

impl RenderOnce for ModeToggle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let mode = self.mode;
        let shown = window.use_keyed_state("mode-toggle", cx, |_, _| Shown { mode, from: mode, switches: 0 });
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
