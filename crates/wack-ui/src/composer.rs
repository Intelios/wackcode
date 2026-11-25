//! The composer: where the user writes to the agent.
//!
//! One card of round controls: the mode track, the model's pill, the effort's own pill (which
//! opens WackCode 2's slider, Default through Ultra, staying open as it moves), the context meter
//! and the send button, the one filled control (the stop button while the agent works). In Ultra
//! Plan the card wears the prism's edge.
//!
//! Above the card, a row of status pills: what the run is doing and for how long (with ⌘B to move
//! the command it waits on to the background), the todo list, the commands running in the
//! background, the waiting messages. The list, the commands and the messages open above their
//! pills and fold back into them (`ChatSession::todos_open`, `tasks_open`, `waiting_open`). The
//! commands' pill shows even between runs: a server keeps running after the turn. An open question comes first: it stacks
//! above the pills, and while it is open the pills only report, with the list and the messages
//! folded into them.
//!
//! The mode toggle (Build | Plan) sits first in the controls, a recessed track with a pill that
//! slides to the mode in force. Clicking Plan again switches to Ultra Plan and back; ⇧Tab cycles
//! all three. It is locked while the agent works: a run keeps the mode it started with, unless
//! the agent itself asks to plan.
//!
//! A `/word` raises the `/` menu above the card (`crate::slash_menu`), as wide as the card; a
//! bare `/command` that takes something after it says what under the field.
//!
//! Renders from plain data ([`ComposerData`]) and reports clicks as actions (`crate::actions`),
//! which the shell handles; in a gallery scene nothing handles them.

use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    AnchoredPositionMode, AnimationExt, App, Background, Bounds, BoxShadow, ElementId, Entity, Focusable, Hsla, MouseButton, Pixels,
    SharedString, Window, anchored, deferred, div, point, prelude::*, px,
};
use wack_core::{AgentMode, Reasoning};

use crate::{
    actions::{
        AttachFiles, AttachPictures, ChooseEffort, ChooseModel, CloseMenu, RemoveAttachment, Compact, MoveToBackground, OpenEffortMenu, OpenModelMenu, Send, SetAgentMode, ShowTask,
        SlideEffort, Stop, StopTask, ToggleAgentsPane, ToggleTasks, ToggleTodos, ToggleWaiting, Unqueue,
    },
    app::Menu,
    icon::Icon,
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    tooltip::tip,
    typography::{Type, Typography},
    ultra::{UltraField, UltraLabel, UltraMark, prism_edge},
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
    /// A Chat area composer: no mode track, since a Chat chat is always in Build.
    pub chat_area: bool,
    /// The chat is archived (its id): in the composer's place, a line saying so and Unarchive.
    pub archived: Option<SharedString>,
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
    /// The `/` menu, when a `/word` is being typed.
    pub slash: Option<crate::slash_menu::SlashData>,
    /// Under a bare `/command`: what it takes after it.
    pub hint: Option<SharedString>,
    /// The question the agent is waiting on, drawn above the waiting messages.
    pub question: Option<crate::question::QuestionData>,
    /// The computer-use card: the agent waits for the user to let it use an app.
    pub access: Option<crate::computer_card::CardData>,
    /// The agent's todo list, at the top of the column.
    pub todos: Option<crate::todo_panel::TodoData>,
    /// While a run is going: how long, and what it is doing now (`ChatSession::activity`).
    pub activity: Option<(SharedString, Option<SharedString>)>,
    /// The todo list and the waiting messages are open above their pills.
    pub todos_open: bool,
    pub waiting_open: bool,
    /// Commands the agent started in the background: running ones first, then a few that ended.
    pub tasks: Vec<TaskLine>,
    pub tasks_open: bool,
    /// The agent is waiting on a command or sub-agents in the foreground, which ⌘B would move.
    pub foreground: bool,
    /// Sub-agents queued or working.
    pub agents: usize,
    /// Pictures attached to the message being written, shown in a tray above the field.
    pub attachments: Vec<crate::attachments::Attachment>,
    /// Why the message's pictures can't go to this model, when they can't: Send waits on it.
    pub pictures_refused: Option<SharedString>,
    /// Comments drafted on a canvas, which go with the next message: how many, and the canvas
    /// the chip opens.
    pub comments: Option<(usize, String)>,
    /// When a message last left a composer: for a moment after, it lifts and a glint crosses it.
    pub sent_at: Option<Instant>,
}

/// A background command as its list above the composer shows it.
#[derive(Clone, Debug)]
pub struct TaskLine {
    pub id: SharedString,
    pub label: SharedString,
    /// "running · 2m10s", "exit 1 · 42s".
    pub words: SharedString,
    pub status: wack_core::tasks::Status,
    /// The last line it printed.
    pub last: SharedString,
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

/// An archived chat's foot, in the composer's place: it can be read, not written to, until it
/// comes back to the list.
fn archived_footer(chat: String, theme: &Theme) -> impl IntoElement {
    div()
        .w_full()
        .h(px(56.))
        .px(px(18.))
        .rounded(px(28.))
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(12.))
        .child(Icon::Archive.view(px(15.), theme.text_dim))
        .child(div().flex_1().typography(Type::Ui).text_color(theme.text_soft).child("This chat is archived."))
        .child(
            div()
                .id("unarchive-footer")
                .h(px(30.))
                .px(px(14.))
                .rounded_full()
                .flex()
                .items_center()
                .bg(theme.accent_fill())
                .hover(|button| button.bg(theme.accent_fill_hover()))
                .typography(Type::UiStrong)
                .text_color(theme.on_accent)
                .on_click(dispatch(crate::actions::UnarchiveChat { id: chat, ask: false }))
                .child("Unarchive")
                .tag("unarchive-footer"),
        )
        .tag("archived-footer")
}

fn dispatch(action: impl gpui::Action) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static {
    let action = action.boxed_clone();
    move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)
}

impl RenderOnce for Composer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let data = self.data;
        if let Some(chat) = data.archived {
            return archived_footer(chat.to_string(), &theme).into_any_element();
        }
        let viewport = window.viewport_size();
        let menu_for = |kind: Menu| data.menu.clone().filter(|menu| menu.kind == kind);
        let has_text = match &data.input {
            Input::Field(field) => !field.read(cx).text().trim().is_empty(),
            Input::Drawn { text, .. } => text.is_some(),
        };
        // Pictures alone make a message; pictures this model can't take hold Send back.
        // So do comments drafted on a canvas.
        let has_text = (has_text || !data.attachments.is_empty() || data.comments.is_some()) && data.pictures_refused.is_none();

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
            .child(
                div()
                    .id("attach")
                    .flex_none()
                    .size(px(26.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|button| button.bg(theme.hover))
                    .tooltip(tip("Attach pictures — or paste or drop them here"))
                    .on_click(dispatch(AttachPictures))
                    .child(Icon::Plus.view(px(16.), theme.text_dim))
                    .tag("attach"),
            )
            .when(!data.chat_area, |controls| controls.child(ModeToggle { mode: data.mode, locked: data.working, key: data.key.clone() }))
            // In a narrow composer the model's name gives way first; the send button never does.
            .child(picker(
                "model",
                pill("model").min_w(px(MODEL_MIN)).text_color(theme.text_soft).hover(|pill| pill.bg(theme.hover).text_color(theme.text)).child(
                    div().min_w_0().truncate().typography(Type::Ui).child(data.model.clone()),
                ).child(Icon::ChevronDown.view(px(12.), theme.text_faint)),
                menu_for(Menu::Model).map(|menu| ModelPicker { menu }.into_any_element()),
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
                    UltraMark::new("effort-ultra-mark", px(12.)).animated(!data.working).into_any_element()
                } else {
                    Icon::Brain.view(px(13.), theme.accent_text).into_any_element()
                })
                .child(if ultra {
                    UltraLabel::new("effort-ultra", effort, Type::Ui).into_any_element()
                } else {
                    div().typography(Type::Ui).text_color(theme.accent_text).child(effort).into_any_element()
                });
            let slider = menu_for(Menu::Effort).map(|menu| {
                EffortSlider { menu, model: data.model.clone(), key: data.key.clone() }.into_any_element()
            });
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
            .when_some(data.comments.clone(), |card, (count, canvas)| card.child(comments_chip(count, canvas, &theme)))
            .when(!data.attachments.is_empty(), |card| card.child(tray(&data.attachments, &theme)))
            .when_some(data.pictures_refused.clone(), |card, refused| {
                card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .typography(Type::Small)
                        .text_color(theme.warning)
                        .child(Icon::Alert.view(px(12.), theme.warning))
                        .child(refused)
                        .tag("pictures-refused"),
                )
            })
            .child(
                field
                    .min_h(px(field_height))
                    .when_some(data.hint.clone(), |field, hint| {
                        field.child(div().pt(px(2.)).typography(Type::Small).text_color(theme.text_faint).child(hint).tag("command-hint"))
                    })
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
        // Files dragged over from the Finder: the card lights its edge and takes them.
        let card = div()
            .id("composer-drop")
            .rounded(px(CARD_RADIUS))
            .drag_over::<gpui::ExternalPaths>(|card, _, _, cx| {
                let theme = Theme::get(cx);
                card.border_2().border_color(theme.accent_text).bg(theme.accent_wash)
            })
            .on_drop(|paths: &gpui::ExternalPaths, window, cx| {
                window.dispatch_action(Box::new(AttachFiles { paths: paths.paths().to_vec() }), cx);
            })
            .child(card);
        let card = if ultra_plan { prism_edge(CARD_RADIUS, card, &theme).into_any_element() } else { card.into_any_element() };
        // A message leaving: the card lifts and settles while a glint crosses it.
        let moment = data.sent_at.and_then(|at| crate::motion::send_moment(at.elapsed(), cx.reduce_motion()));
        if moment.is_some() {
            window.request_animation_frame();
        }
        // The `/` menu rises from the card's top edge, over whatever is above it.
        let card = div()
            .relative()
            .when_some(moment, |card, moment| card.top(px(-moment.lift)))
            .child(card)
            .children(moment.map(|moment| send_glint(moment, &theme)))
            .when_some(data.slash.clone(), |card, slash| {
            card.child(
                deferred(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom(gpui::relative(1.))
                        .pb(px(PICKER_GAP))
                        .child(crate::motion::enter(ElementId::Name(format!("arrive-slash-{}", data.key).into()), div().child(crate::slash_menu::menu(slash, &theme)))),
                )
                .with_priority(2),
            )
        });

        // Inside a capped column the card stays whole and the question gives way first.
        let mut column = div().w_full().min_h_0().flex().flex_col().gap(px(6.));
        let asking = data.question.is_some() || data.access.is_some();
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
        if let Some(access) = data.access {
            column = column.child(div().flex_none().pb(px(4.)).child(crate::computer_card::ComputerCard::new(access)));
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
        // Moving the command the agent waits on to the background, beside what it is doing.
        let background_pill = (data.working && data.foreground).then(|| {
            status_pill("background-pill", false, true, &theme)
                .tooltip(tip("Keep it working in the background and let the agent carry on"))
                .on_click(dispatch(MoveToBackground))
                .child(div().typography(Type::Small).text_color(theme.text_soft).child("Background"))
                .child(div().typography(Type::Small).text_color(theme.text_faint).child("⌘B"))
                .tag("background-pill")
        });
        let running_tasks = data.tasks.iter().filter(|task| task.status.running()).count();
        let tasks_pill = (running_tasks > 0).then(|| {
            status_pill("tasks-pill", data.tasks_open && !asking, !asking, &theme)
                .when(!asking, |pill| pill.on_click(dispatch(ToggleTasks)))
                .child(Icon::Terminal.view(px(11.), theme.accent_text))
                .child(div().typography(Type::Small).text_color(theme.text_soft).child(format!("{running_tasks} running")))
                .when(!asking, |pill| pill.child(if data.tasks_open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(11.), theme.text_faint)))
                .tag("tasks-pill")
        });
        // Sub-agents at work: the pill opens the Agents pane.
        let agents_pill = (data.agents > 0).then(|| {
            status_pill("agents-pill", false, true, &theme)
                .tooltip(tip("Show the agents' work (⌘⇧A)"))
                .on_click(dispatch(ToggleAgentsPane))
                .child(crate::motion::paddling(div().flex_none().child(Icon::Duck.view(px(11.), theme.accent_text))))
                .child(div().typography(Type::Small).text_color(theme.text_soft).child(if data.agents == 1 { "1 agent".to_string() } else { format!("{} agents", data.agents) }))
                .tag("agents-pill")
        });
        if data.tasks_open && !asking && running_tasks > 0 {
            let mut rows = div().id("tasks").flex_shrink(1.).min_h(px(36.)).max_h(px(WAITING_MAX_HEIGHT)).overflow_y_scroll().flex().flex_col().gap(px(6.));
            for task in data.tasks {
                rows = rows.child(div().flex_none().child(task_row(task, &theme)));
            }
            column = column.child(rows);
        }
        if activity_pill.is_some() || todo_pill.is_some() || waiting_pill.is_some() || tasks_pill.is_some() || agents_pill.is_some() {
            column = column.child(
                div()
                    .flex_none()
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .children(activity_pill)
                    .children(background_pill)
                    .children(todo_pill)
                    .child(div().flex_1())
                    .children(agents_pill)
                    .children(tasks_pill)
                    .children(waiting_pill),
            );
        }
        column.child(div().flex_none().child(card)).into_any_element()
    }
}

/// The composer card's corners, the space above its field, and between the field and the bar.
const CARD_RADIUS: f32 = 18.;
/// The glint's share of the card it crosses as a message leaves.
const SEND_GLINT_BAND: f32 = 0.36;

/// The send moment over the card: its rim lit in the accent, and a band of light crossing it.
fn send_glint(moment: crate::motion::SendMoment, theme: &Theme) -> impl IntoElement {
    let light = Hsla { a: 0.2 * moment.light, ..theme.accent };
    let clear = Hsla { a: 0., ..light };
    let half = |from: Hsla, to: Hsla| {
        div().flex_1().h_full().bg(gpui::linear_gradient(90., gpui::linear_color_stop(from, 0.), gpui::linear_color_stop(to, 1.)))
    };
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .rounded(px(CARD_RADIUS))
        .border_1()
        .border_color(Hsla { a: theme.accent_line.a.max(0.6) * moment.light, ..theme.accent_line })
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .top_0()
                .h_full()
                .w(gpui::relative(SEND_GLINT_BAND))
                .left(gpui::relative(-SEND_GLINT_BAND + (1. + SEND_GLINT_BAND) * moment.across))
                .flex()
                .child(half(clear, light))
                .child(half(light, clear)),
        )
}
const CARD_TOP: f32 = 14.;
const CARD_GAP: f32 = 10.;
/// The least of the model's pill a narrow composer keeps.
const MODEL_MIN: f32 = 64.;
/// How far above its pill a picker's popover opens.
const PICKER_GAP: f32 = 6.;
/// A status pill's height.
const STATUS_HEIGHT: f32 = 26.;

/// A thumbnail's side in the tray.
const THUMB: f32 = 56.;

/// Comments drafted on a canvas, waiting for the next message: a click shows them on the
/// canvas, and Send now sends them alone.
fn comments_chip(count: usize, canvas: String, theme: &Theme) -> impl IntoElement {
    let words = if count == 1 { "1 comment on the canvas".to_string() } else { format!("{count} comments on the canvas") };
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .id("composer-comments")
                .h(px(28.))
                .pl(px(8.))
                .pr(px(12.))
                .rounded_full()
                .flex()
                .items_center()
                .gap(px(6.))
                .bg(theme.accent_wash)
                .cursor_pointer()
                .hover(|chip| chip.bg(theme.hover))
                .tooltip(tip("They go with your next message. Show them on the canvas"))
                .on_click(dispatch(crate::actions::ShowCanvas { canvas, artboards: Vec::new() }))
                .child(Icon::Comment.view(px(12.), theme.accent_text))
                .child(div().typography(Type::Small).text_color(theme.accent_text).child(words))
                .tag("composer-comments"),
        )
        .child(
            div()
                .id("composer-send-comments")
                .h(px(28.))
                .px(px(12.))
                .rounded_full()
                .flex()
                .items_center()
                .cursor_pointer()
                .border_1()
                .border_color(theme.border)
                .hover(|send| send.bg(theme.hover))
                .typography(Type::Small)
                .text_color(theme.text_soft)
                .tooltip(tip("Send the comments with no message"))
                .on_click(dispatch(crate::actions::Send))
                .child("Send now")
                .tag("composer-send-comments"),
        )
}

/// The pictures attached to the message, each with its remove button.
fn tray(attachments: &[crate::attachments::Attachment], theme: &Theme) -> impl IntoElement {
    let thumbs = attachments.iter().enumerate().map(|(index, attachment)| {
        let (width, height) = attachment.size;
        div()
            .id(("attachment", index))
            .relative()
            .flex_none()
            .size(px(THUMB))
            .tooltip(tip(format!("{} · {width} × {height}", attachment.name)))
            .child(
                gpui::img(attachment.thumb.clone())
                    .size(px(THUMB))
                    .rounded(px(12.))
                    .object_fit(gpui::ObjectFit::Cover)
                    .border_1()
                    .border_color(theme.border),
            )
            .child(
                div()
                    .id(("remove-attachment", index))
                    .absolute()
                    .top(px(-6.))
                    .right(px(-6.))
                    .size(px(20.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.raised)
                    .border_1()
                    .border_color(theme.border_strong)
                    .cursor_pointer()
                    .hover(|button| button.bg(theme.hover))
                    .on_click(dispatch(RemoveAttachment { index }))
                    .child(Icon::Close.view(px(10.), theme.text_soft))
                    .tag("remove-attachment"),
            )
    });
    div().flex().flex_wrap().gap(px(8.)).pt(px(2.)).children(thumbs).tag("attachments")
}

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

/// A background command above the card: how it stands, its last line, its output and Stop.
fn task_row(task: TaskLine, theme: &Theme) -> impl IntoElement {
    let running = task.status.running();
    let colour = crate::panes::tasks::status_colour(&task.status, theme);
    let (show, stop) = (task.id.clone(), task.id.clone());
    div()
        .id(SharedString::from(format!("task-{}", task.id)))
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
        .child(div().flex_none().size(px(6.)).rounded_full().bg(colour))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_baseline()
                .gap(px(8.))
                .child(div().flex_none().max_w(gpui::relative(0.5)).truncate().typography(Type::Ui).text_color(theme.text_soft).child(task.label))
                .child(div().flex_none().typography(Type::Small).text_color(if running { theme.text_faint } else { colour }).child(task.words))
                .child(div().flex_1().min_w_0().truncate().typography(Type::Mono).text_color(theme.text_faint).child(task.last)),
        )
        .child(
            div()
                .id(SharedString::from(format!("show-task-{}", task.id)))
                .flex_none()
                .typography(Type::Small)
                .text_color(theme.text_dim)
                .cursor_pointer()
                .hover(|button| button.text_color(theme.text))
                .on_click(move |_, window, cx| window.dispatch_action(Box::new(ShowTask { id: show.clone() }), cx))
                .child("Output")
                .tag("task-output"),
        )
        .when(running, |row| {
            row.child(
                div()
                    .id(SharedString::from(format!("stop-task-{}", task.id)))
                    .flex_none()
                    .size(px(22.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(Hsla { a: 0.14, ..theme.danger })
                    .cursor_pointer()
                    .hover(|button| button.bg(Hsla { a: 0.24, ..theme.danger }))
                    .tooltip(tip("Stop it; the agent is told"))
                    .on_click(move |_, window, cx| window.dispatch_action(Box::new(StopTask { id: stop.clone() }), cx))
                    .child(Icon::Stop.view(px(9.), theme.danger))
                    .tag("task-stop"),
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

/// How long the knob takes to travel between stops, and the thumb to pop as it lands.
const TRAVEL: Duration = Duration::from_millis(260);
const POP: Duration = Duration::from_millis(320);

/// The knob's travel, `t` (0–1) through: it crosses most of the gap quickly and settles with a
/// slight overshoot, version 2's `cubic-bezier(.34, 1.3, .64, 1)`.
fn slide_ease(t: f32) -> f32 {
    let back = t.clamp(0., 1.) - 1.;
    1. + 1.9 * back * back * back + 0.9 * back * back
}

/// The thumb's pop, `t` (0–1) through: it swells past full size and settles back, version 2's
/// `.72 → 1.14 → 1`.
fn thumb_pop(t: f32) -> f32 {
    keyframes(&[(0., 0.72), (0.55, 1.14), (1., 1.)], t)
}

/// A value that eases to each new target over [`TRAVEL`], for the slider's knob and fill. Unlike
/// [`crate::motion::Ease`] it keeps version 2's slide: quick off the mark, a slight overshoot,
/// then settled. The holder calls [`Slide::toward`] as it renders and asks for another frame
/// while [`Slide::moving`].
#[derive(Clone, Copy, Debug)]
struct Slide {
    from: f32,
    to: f32,
    since: Option<Instant>,
}

impl Slide {
    fn at(value: f32) -> Self {
        Self { from: value, to: value, since: None }
    }

    fn value(&self, now: Instant) -> f32 {
        match self.since {
            Some(since) => {
                let t = now.saturating_duration_since(since).as_secs_f32() / TRAVEL.as_secs_f32();
                self.from + (self.to - self.from) * slide_ease(t)
            }
            None => self.to,
        }
    }

    fn moving(&self, now: Instant) -> bool {
        self.since.is_some_and(|since| now.saturating_duration_since(since) < TRAVEL)
    }

    /// Heads for `to` from wherever the value is now; `still` jumps straight there.
    fn toward(&mut self, to: f32, still: bool) {
        if to == self.to {
            return;
        }
        let now = Instant::now();
        *self = Self { from: self.value(now), to, since: (!still).then_some(now) };
    }
}

/// The slider's memory between frames: where the knob and the fill are easing, and when the thumb
/// last popped.
struct SliderMotion {
    knob: Slide,
    fill: Slide,
    pop: Option<Instant>,
}

impl SliderMotion {
    fn new(knob: f32, fill: f32, still: bool) -> Self {
        Self { knob: Slide::at(knob), fill: Slide::at(fill), pop: (!still).then(Instant::now) }
    }

    /// Heads for the stop at `knob` and its fill from wherever they are; `still` jumps.
    fn toward(&mut self, knob: f32, fill: f32, still: bool) {
        if self.knob.to != knob {
            self.pop = (!still).then(Instant::now);
        }
        self.knob.toward(knob, still);
        self.fill.toward(fill, still);
    }

    fn moving(&self, now: Instant) -> bool {
        self.knob.moving(now) || self.fill.moving(now) || self.pop.is_some_and(|pop| now.saturating_duration_since(pop) < POP)
    }

    fn pop_scale(&self, now: Instant) -> f32 {
        match self.pop {
            Some(pop) => {
                let t = now.saturating_duration_since(pop).as_secs_f32() / POP.as_secs_f32();
                if t >= 1. { 1. } else { thumb_pop(t) }
            }
            None => 1.,
        }
    }

    /// How far the head's name has risen into place, 0–1, after the last move.
    fn name_rise(&self, now: Instant) -> f32 {
        match self.knob.since {
            Some(since) => crate::motion::spring(now.saturating_duration_since(since).as_secs_f32() / TRAVEL.as_secs_f32()),
            None => 1.,
        }
    }
}

/// WackCode 2's effort slider, from the effort menu's choices (Default, then Low through Ultra):
/// a head naming the effort and the model, a track with a stop for each choice and a knob, and the
/// stops' names beneath. Clicking or dragging along the track, or clicking a name, moves the
/// effort at once and leaves the slider open (`SlideEffort`).
///
/// The knob and the fill ease between stops rather than jumping, the thumb pops as it lands, and
/// the head's name rises into place; at Ultra the fill becomes the prism's glyph field. All of
/// them are version 2's, restored.
#[derive(IntoElement)]
struct EffortSlider {
    menu: MenuData,
    model: SharedString,
    /// Whose composer this is, so the motion's state is this slider's own.
    key: SharedString,
}

impl RenderOnce for EffortSlider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let items: Vec<MenuItem> = self.menu.groups.into_iter().flat_map(|group| group.items).collect();
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
        // Off is no reasoning: the track stays unlit there, wherever it sits.
        let on = choices.get(current).copied().flatten().is_some_and(|effort| effort != Reasoning::Off);
        let width = SLIDER_WIDTH - 2. * SLIDER_PAD;
        let (tone, wash) = if ultra { (theme.ultra, Hsla { a: 0.14, ..theme.ultra }) } else { (theme.accent, theme.accent_wash) };
        let selected = &items[current.min(count.saturating_sub(1))];

        // The knob's place and the fill's, eased from wherever they were: a press or a drag moves
        // the effort at once, and the bar catches up.
        let still = cx.reduce_motion();
        let knob_target = stop_x(current, count, width);
        let fill_target = if on { knob_target + TRACK_HEIGHT / 2. } else { 0. };
        let motion = window.use_keyed_state(
            ElementId::Name(format!("effort-motion-{}", self.key).into()),
            cx,
            move |_, _| SliderMotion::new(knob_target, fill_target, still),
        );
        let now = Instant::now();
        let (knob, fill, pop, rise) = motion.update(cx, |motion, _| {
            motion.toward(knob_target, fill_target, still);
            (motion.knob.value(now), motion.fill.value(now), motion.pop_scale(now), motion.name_rise(now))
        });
        if motion.read(cx).moving(now) {
            window.request_animation_frame();
        }

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
                        UltraMark::new("slider-ultra-mark", px(14.)).animated(true).into_any_element()
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
                    // The name rises into place at each stop, as version 2's did.
                    .child(
                        div()
                            .relative()
                            .top(px((1. - rise) * 3.))
                            .opacity(0.25 + 0.75 * rise)
                            .when(ultra, |name| name.child(UltraLabel::new("slider-ultra", selected.label.clone(), Type::UiStrong)))
                            .when(!ultra, |name| {
                                name.child(
                                    div()
                                        .typography(Type::UiStrong)
                                        .text_color(if on { theme.accent_text } else { theme.text })
                                        .child(selected.label.clone()),
                                )
                            }),
                    )
                    .child(div().truncate().typography(Type::Small).text_color(theme.text_dim).child(self.model.clone())),
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
        // At Ultra the fill is a well the prism's glyph field writes into, as version 2 drew it.
        let fill_ground: Background = if ultra { Hsla { a: 0.13, ..theme.ultra }.into() } else { theme.accent_fill() };
        let mut track = div()
            .id("effort-track")
            .relative()
            .h(px(TRACK_HEIGHT))
            .rounded_full()
            .bg(theme.hover)
            .child(crate::slide::follow("effort-track", drawn, pick.clone()).absolute().left_0().top_0().size_full())
            .when(fill > 0.01, |track| {
                track.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(fill))
                        .rounded_full()
                        .overflow_hidden()
                        .bg(fill_ground)
                        .when(ultra, |fill| fill.border_1().border_color(Hsla { a: 0.2, ..theme.ultra }))
                        .when(ultra, |fill| {
                            fill.child(UltraField::new(
                                ElementId::Name(format!("effort-field-{}", self.key).into()),
                                px(width),
                                px(TRACK_HEIGHT),
                            ))
                        }),
                )
            });
        // The field's density carries the stops at Ultra, so the dots step aside there.
        if !ultra {
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
        }
        let knob_size = KNOB * pop;
        let knob = div()
            .absolute()
            .top(px((TRACK_HEIGHT - knob_size) / 2.))
            .left(px(knob - knob_size / 2.))
            .size(px(knob_size))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(if ultra { theme.ultra_glow } else { theme.raised })
            .border_1()
            .border_color(if ultra { Hsla { a: 0.55, ..theme.ultra } } else { theme.border_strong });
        let knob = if ultra {
            knob
                .shadow(vec![BoxShadow {
                    color: Hsla { a: 0.5, ..theme.ultra_spectrum[1] },
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(14.),
                    spread_radius: px(2.),
                    inset: false,
                }])
                .child(UltraMark::new("effort-knob-mark", px(11. * pop)).ink(theme.on_ultra))
        } else {
            knob.shadow_sm()
        };
        let track = track
            .child(knob)
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
}

/// The room each stop's name has beneath the track.
const STOP_LABEL: f32 = 46.;

/// The model picker, after version 2's: two panels side by side in one card. The first lists the
/// providers (each connection, with how many models it offers); choosing one slides across to
/// its models, with a back button to the list. It opens on the provider of the model in use.
#[derive(IntoElement)]
struct ModelPicker {
    menu: MenuData,
}

/// Which provider the picker shows (`None`: the list of providers), and how many times it has
/// slid, so each slide plays afresh.
struct PickerView {
    provider: Option<usize>,
    slides: usize,
    /// The way the last slide went: towards a provider's models, or back.
    forward: bool,
}

const PICKER_WIDTH: f32 = 280.;
const PICKER_SLIDE: f32 = 36.;

impl RenderOnce for ModelPicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let still = cx.reduce_motion();
        let groups = self.menu.groups;
        let current = groups.iter().position(|group| group.items.iter().any(|item| item.selected));
        let state = window.use_keyed_state("model-picker", cx, move |_, _| PickerView { provider: current.or(Some(0)), slides: 0, forward: true });
        let (shown, slides, forward) = {
            let view = state.read(cx);
            (view.provider.filter(|&index| index < groups.len()), view.slides, view.forward)
        };
        let go = move |to: Option<usize>| {
            let state = state.clone();
            move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
                cx.stop_propagation();
                state.update(cx, |view, _| {
                    view.forward = to.is_some();
                    view.provider = to;
                    view.slides += 1;
                });
                window.refresh();
            }
        };
        let row = |id: (&'static str, usize)| {
            div().id(id).mx(px(6.)).px(px(8.)).h(px(32.)).rounded(px(8.)).flex().items_center().gap(px(8.)).cursor_pointer().hover(|row| row.bg(theme.hover))
        };
        let panel = match shown {
            None => {
                let mut list = div()
                    .flex()
                    .flex_col()
                    .child(div().px(px(14.)).pt(px(4.)).pb(px(6.)).typography(Type::Label).text_color(theme.text_dim).child("Providers"));
                for (index, group) in groups.iter().enumerate() {
                    let holds_current = current == Some(index);
                    let name = group.title.clone().unwrap_or_else(|| "Models".into());
                    list = list.child(
                        row(("picker-provider", index))
                            .on_click(go(Some(index)))
                            .child(Icon::Key.view(px(13.), if holds_current { theme.accent_text } else { theme.text_dim }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .typography(if holds_current { Type::UiStrong } else { Type::Ui })
                                    .text_color(theme.text)
                                    .child(name),
                            )
                            .child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(group.items.len().to_string()))
                            .child(Icon::ChevronRight.view(px(12.), theme.text_faint)),
                    );
                }
                list.into_any_element()
            }
            Some(index) => {
                let group = &groups[index];
                let back = div()
                    .id("picker-back")
                    .mx(px(6.))
                    .mb(px(4.))
                    .px(px(6.))
                    .h(px(30.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .hover(|row| row.bg(theme.hover))
                    .on_click(go(None))
                    .child(Icon::ChevronRight.view(px(12.), theme.text_dim).with_transformation(gpui::Transformation::rotate(gpui::radians(std::f32::consts::PI))))
                    .child(div().flex_1().min_w_0().truncate().typography(Type::UiStrong).text_color(theme.text).child(group.title.clone().unwrap_or_else(|| "Models".into())))
                    .tag("picker-back");
                let mut list = div().flex().flex_col().child(back).child(div().mx(px(12.)).mb(px(4.)).h(px(1.)).bg(theme.border));
                for (number, item) in group.items.iter().enumerate() {
                    let action: Box<dyn gpui::Action> = match item.choice.clone() {
                        Choice::Model { connection, model } => Box::new(ChooseModel { connection, model }),
                        Choice::Effort(effort) => Box::new(ChooseEffort { effort }),
                    };
                    list = list.child(
                        row(("menu-item", index * 1000 + number))
                            .when(item.selected, |row| row.bg(theme.accent_wash))
                            .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .typography(if item.selected { Type::UiStrong } else { Type::Ui })
                                    .text_color(if item.selected { theme.accent_text } else { theme.text })
                                    .child(item.label.clone()),
                            )
                            .when_some(item.detail.clone(), |row, detail| {
                                row.child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(detail))
                            })
                            .when(item.selected, |row| row.child(Icon::Check.view(px(13.), theme.accent_text))),
                    );
                }
                list.into_any_element()
            }
        };
        // Each slide comes in from the side it was asked for: a provider's models from the right,
        // the list from the left.
        let from = if forward { PICKER_SLIDE } else { -PICKER_SLIDE };
        let panel = div().w_full().child(panel);
        let panel = if slides == 0 || still {
            panel.into_any_element()
        } else {
            crate::motion::slide_in(ElementId::NamedInteger("picker-slide".into(), slides as u64), panel, from, 0.).into_any_element()
        };
        div()
            .id("menu")
            .occlude()
            .w(px(PICKER_WIDTH))
            .max_h(px(420.))
            .overflow_y_scroll()
            .overflow_x_hidden()
            .py(px(6.))
            .rounded(px(14.))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border_strong)
            .shadow_lg()
            .child(panel)
    }
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
    fn the_knob_crosses_the_gap_quickly_and_settles_with_a_slight_overshoot() {
        assert_eq!(slide_ease(0.), 0.);
        assert_eq!(slide_ease(1.), 1.);
        assert!(slide_ease(0.25) > 0.6, "most of the way there early: {}", slide_ease(0.25));
        let peak = (0..=100).map(|step| slide_ease(step as f32 / 100.)).fold(0., f32::max);
        assert!(peak > 1.02 && peak < 1.05, "a slight overshoot: {peak}");
        assert!(slide_ease(0.7) > 1.);
    }

    #[test]
    fn the_thumb_pops_past_full_size_and_settles() {
        assert_eq!(thumb_pop(0.), 0.72);
        assert_eq!(thumb_pop(1.), 1.);
        assert!(thumb_pop(0.55) > 1.1);
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
