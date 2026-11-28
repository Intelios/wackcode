//! The composer: where the user writes to the agent. One composer, the same everywhere.
//!
//! One card. Its controls read as WackCode 2's did: attach, then what you ask with (the model,
//! with its optional provider logo, and the effort's pill, which opens the slider from Default to
//! Ultra), then how (the Build | Plan track) beside Send. Send is the one filled control; while
//! the agent works it is Stop, with Queue beside it once something is written. Round Send runs a
//! ring of how full the model's context is: hovering it opens WackCode 2's context popover. Only
//! hovering: a click on the ring is Send's or Stop's, so reaching for the popover never stops a
//! run. In Ultra Plan the card wears the prism's edge.
//!
//! On the card's top edge, one piece with it, a tab of segments, as version 2's pills were: the
//! run's time ("Working for 8s", the duck paddling; never what it is doing, which the transcript
//! shows), the todo list's count and its task in hand, a Team's crew at work, commands still
//! running, the project's changed files (opening Changes), and ⌘B while the agent waits on
//! something it could leave running. The todo list and the commands open as popovers rising from
//! their segment, over the transcript, so opening one never moves the page; in a chat the tab's
//! room is always kept, so a run starting never moves it either. It never runs past the card: short
//! of room it gives up words, then whole segments, in the order `Fit::fitting` gives (measured
//! against the card's width as last drawn). Messages written while the agent
//! works wait above the card as faint bubbles, at the foot of the chat, with Steer now, Edit and
//! Remove. An open question comes first: it stacks above the card, and while it is open the tab
//! only reports.
//!
//! A chat with its Team on splits the model's pill: the lead (the chat's model) and, sliding in
//! beside it, the crew, which opens its own picker. The model picker's foot switches Team on or
//! off. A crew whose model is gone dims its half and holds Send, saying why: a crew is never
//! swapped for another model.
//!
//! The mode toggle is a recessed track with a pill that slides to the mode in force. Clicking
//! Plan again switches to Ultra Plan and back; ⇧Tab cycles all three. It is locked while the
//! agent works: a run keeps the mode it started with, unless the agent itself asks to plan.
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
use wack_core::{AgentMode, Reasoning, context::Kind};

use crate::{
    actions::{
        AttachPictures, ChooseCrew, ChooseCrewEffort, ChooseEffort, ChooseModel, CloseMenu, OpenCrewMenu, SetTeam, RemoveAttachment, Compact, MoveToBackground, OpenEffortMenu, OpenModelMenu, Send, SetAgentMode, ShowTask,
        SlideEffort, Stop, StopTask, ToggleTasks, ToggleTodos, Unqueue, SteerQueued, RemoveQueued, InsertFile, RemoveFile,
    },
    attachments::{FileAttachment, FileSource},
    brand::Brand,
    chat::ChangeCount,
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
    /// The model provider's logo, when Settings › Appearance › Model logos is on and the model is
    /// recognised; `None` draws nothing beside the model name.
    pub brand: Option<Brand>,
    pub effort: Option<SharedString>,
    /// The effort is Ultra: its word is drawn in the prism.
    pub ultra_effort: bool,
    /// The chat's Team, while it is on: the crew's half of the model's pill.
    pub crew: Option<CrewPill>,
    /// A Team's workers on this message, while any of them works: the tab's crew segment stays up for
    /// them even while the lead waits.
    pub crew_at_work: Option<CrewAtWork>,
    /// Build, Plan or Ultra Plan.
    pub mode: AgentMode,
    /// Whose composer this is (a chat's id, or the new chat's), so switching chats never plays
    /// the toggle's slide.
    pub key: SharedString,
    /// How full the model's context is, for the ring round Send and its popover.
    pub meter: Option<Meter>,
    /// The context popover is open whatever the pointer does: for scenes.
    pub context_open: bool,
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
    /// While a run is going: how long it has run ("Working for 8s", `ChatSession::activity`).
    pub activity: Option<SharedString>,
    /// The todo list is open, rising from its segment of the tab.
    pub todos_open: bool,
    /// Commands the agent started in the background: running ones first, then a few that ended.
    pub tasks: Vec<TaskLine>,
    pub tasks_open: bool,
    /// The project's changed files, for the tab on the card's edge.
    pub changes: Option<ChangeCount>,
    /// The agent is waiting on a command or sub-agents in the foreground, which ⌘B would move.
    pub foreground: bool,
    /// Pictures attached to the message being written, shown in a tray above the field.
    pub attachments: Vec<crate::attachments::Attachment>,
    /// Text files attached to the message (picked, dropped, pasted, terminal).
    pub files: Vec<FileAttachment>,
    /// Why the chat's own model cannot run (removed or switched off): Send and Compact wait until
    /// another model is picked.
    pub model_missing: Option<SharedString>,
    /// Why the message's pictures can't go to this model, when they can't: Send waits on it.
    pub pictures_refused: Option<SharedString>,
    /// Why the message's attached text files can't send (over count or size cap): Send waits on it.
    pub files_refused: Option<SharedString>,
    /// Comments drafted on a canvas, which go with the next message: how many, and the canvas
    /// the chip opens.
    pub comments: Option<(usize, String)>,
    /// When a message last left a composer: for a moment after, it lifts and a glint crosses it.
    pub sent_at: Option<Instant>,
}

/// A background command as its list, rising from the tab, shows it.
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
    /// The model picker's foot: the Team switch, for a Code chat's composer.
    pub team: Option<TeamRow>,
}

/// How a Team's workers stand on the user's latest message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CrewAtWork {
    pub working: usize,
    pub queued: usize,
    pub done: usize,
}

impl CrewAtWork {
    /// "2 working · 1 queued · 3 done", leaving out what is none.
    pub fn words(&self) -> String {
        let parts = [(self.working, "working"), (self.queued, "queued"), (self.done, "done")];
        parts.iter().filter(|(count, _)| *count > 0).map(|(count, word)| format!("{count} {word}")).collect::<Vec<_>>().join(" · ")
    }
}

/// A Team's crew as its half of the model's pill shows it.
#[derive(Clone, Debug)]
pub struct CrewPill {
    pub model: SharedString,
    pub connection: SharedString,
    /// The crew's effort, when it has one of its own.
    pub effort: Option<SharedString>,
    /// Why the Team can't work as it stands: its half dims and says so.
    pub problem: Option<SharedString>,
    /// The crew's model is gone: Send waits until another is chosen or Team is off.
    pub missing: bool,
}

/// The Team switch at the model picker's foot.
#[derive(Clone, Debug)]
pub struct TeamRow {
    pub on: bool,
    /// A run is going: the Team stays as it started.
    pub locked: bool,
    /// Why Team can't be switched on (sub-agents are off), which disables the switch.
    pub unavailable: Option<SharedString>,
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
    /// The model provider's logo, when enabled and recognised; `None` draws nothing.
    pub brand: Option<Brand>,
    pub selected: bool,
    pub choice: Choice,
}

#[derive(Clone, Debug)]
pub enum Choice {
    Model { connection: String, model: String },
    Effort(Option<Reasoning>),
    /// A model for the Team's crew.
    Crew { connection: String, model: String },
    /// The crew's effort.
    CrewEffort(Option<Reasoning>),
}

#[derive(Clone, Debug)]
pub struct Meter {
    /// Tokens the next request would send.
    pub tokens: u64,
    /// The model's context window, when known.
    pub window: Option<u64>,
    /// Getting full: the ring is drawn in the warning colour.
    pub high: bool,
    /// The popover offers Compact.
    pub can_compact: bool,
    /// Part by part, as the last run left it; empty before one has ended in this window.
    pub parts: Vec<wack_core::context::Part>,
    /// The share of the chat's prompt tokens the provider served from its cache, once a reply
    /// has said (`ChatSession::cache_rate`).
    pub cache: Option<f32>,
    /// Every token the chat and its sub-agents have used (`ChatSession::session_tokens`).
    pub spent: u64,
}

impl Meter {
    /// How full, 0 to 1, when the window is known.
    pub fn share(&self) -> Option<f32> {
        self.window.filter(|window| *window > 0).map(|window| (self.tokens as f32 / window as f32).min(1.))
    }
}

#[derive(Clone, Debug)]
pub struct Waiting {
    pub text: SharedString,
    /// Names of text files attached to the queued or steering message.
    pub files: Vec<SharedString>,
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
                .cursor_pointer().on_click(dispatch(crate::actions::UnarchiveChat { id: chat, ask: false }))
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
        // Pictures or files alone make a message; a missing model, pictures this model can't take,
        // overflowing files, or a crew that is gone hold Send back. So do comments drafted on a canvas.
        let crew_missing = data.crew.as_ref().filter(|crew| crew.missing).and_then(|crew| crew.problem.clone());
        let has_text = (has_text || !data.attachments.is_empty() || !data.files.is_empty() || data.comments.is_some())
            && data.model_missing.is_none()
            && data.pictures_refused.is_none()
            && data.files_refused.is_none()
            && crew_missing.is_none();

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
        let asking = data.question.is_some() || data.access.is_some();

        // What you ask with first (the model, its effort), then how (Build or Plan) beside Send.
        let mut controls = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(
                div()
                    .id("attach")
                    .flex_none()
                    .size(px(28.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|button| button.bg(theme.hover))
                    .tooltip(tip("Attach pictures or text files — or paste or drop them anywhere on the chat"))
                    .on_click(dispatch(AttachPictures))
                    .child(Icon::Paperclip.view(px(15.), theme.text_dim))
                    .tag("attach"),
            )
            // In a narrow composer the model's name gives way first; the send button never does.
            .child({
                let missing = data.model_missing.clone();
                picker(
                    "model",
                    pill("model")
                        .min_w(px(MODEL_MIN))
                        .gap(px(6.))
                        .text_color(if missing.is_some() { theme.warning } else { theme.text_soft })
                        .hover(|pill| pill.bg(theme.hover).text_color(if missing.is_some() { theme.warning } else { theme.text }))
                        .when_some(missing.clone(), |pill, why| {
                            pill.tooltip(tip(why)).child(Icon::Alert.view(px(12.), theme.warning))
                        })
                        .when(missing.is_none(), |pill| {
                            pill.when_some(data.brand, |pill, brand| pill.child(brand.view(px(14.), &theme)))
                        })
                        .child(div().min_w_0().truncate().typography(Type::Ui).child(data.model.clone()))
                        .child(Icon::ChevronDown.view(px(12.), theme.text_faint)),
                    menu_for(Menu::Model).map(|menu| ModelPicker { menu }.into_any_element()),
                    true,
                    viewport,
                )
            });
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
        // The lead's model and effort, then the crew's half: what each side runs on reads as one.
        if let Some(crew) = data.crew.clone() {
            let popover = menu_for(Menu::Crew).map(|menu| CrewPicker { menu, model: crew.model.clone() }.into_any_element());
            controls = controls.child(crew_half(crew, popover, data.key.clone(), viewport, cx.reduce_motion(), &theme));
        }
        controls = controls
            .child(div().flex_1())
            .when(!data.chat_area, |controls| {
                controls.child(div().flex_none().mr(px(4.)).child(ModeToggle { mode: data.mode, locked: data.working, key: data.key.clone() }))
            })
            // While the agent works, what is written waits for it: Queue says so.
            .when(data.working && has_text, |controls| {
                controls.child(
                    div()
                        .id("queue")
                        .flex_none()
                        .h(px(30.))
                        .px(px(13.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .bg(theme.accent_fill())
                        .hover(|button| button.bg(theme.accent_fill_hover()))
                        .tooltip(tip("Send once the agent finishes (↩). ⌘↩ steers it now"))
                        .cursor_pointer().on_click(dispatch(Send))
                        .child(Icon::ArrowUp.view(px(13.), theme.on_accent))
                        .child(div().typography(Type::UiStrong).text_color(theme.on_accent).child("Queue"))
                        .tag("queue"),
                )
            })
            .child(send_ring(&data, has_text, viewport, window, cx, &theme));

        let running_tasks = data.tasks.iter().filter(|task| task.status.running()).count();
        let card = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(CARD_GAP))
            .pl(px(16.))
            .pr(px(9.))
            .pt(px(CARD_TOP))
            .pb(px(8.))
            .rounded(px(CARD_RADIUS))
            .bg(theme.surface)
            .when(!ultra_plan, |card| card.border_1().border_color(theme.border))
            .when_some(data.comments.clone(), |card, (count, canvas)| card.child(comments_chip(count, canvas, data.model_missing.is_none(), &theme)))
            .when(!data.attachments.is_empty() || !data.files.is_empty(), |card| card.child(tray(&data.attachments, &data.files, &theme)))
            .when_some(data.model_missing.clone(), |card, missing| {
                card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .typography(Type::Small)
                        .text_color(theme.warning)
                        .child(Icon::Alert.view(px(12.), theme.warning))
                        .child(missing)
                        .tag("model-missing"),
                )
            })
            .when_some(crew_missing.filter(|_| data.model_missing.is_none()), |card, missing| {
                card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .typography(Type::Small)
                        .text_color(theme.warning)
                        .child(Icon::Alert.view(px(12.), theme.warning))
                        .child(missing)
                        .tag("crew-missing"),
                )
            })
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
            .when_some(data.files_refused.clone(), |card, refused| {
                card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .typography(Type::Small)
                        .text_color(theme.warning)
                        .child(Icon::Alert.view(px(12.), theme.warning))
                        .child(refused)
                        .tag("files-refused"),
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
        let card = div().rounded(px(CARD_RADIUS)).child(card);
        let card = if ultra_plan { prism_edge(CARD_RADIUS, card, &theme).into_any_element() } else { card.into_any_element() };
        // A message leaving: the card lifts and settles while a glint crosses it.
        let moment = data.sent_at.and_then(|at| crate::motion::send_moment(at.elapsed(), cx.reduce_motion()));
        if moment.is_some() {
            window.request_animation_frame();
        }
        // The tab sits on the card's top edge, drawn over the edge so the two read as one piece.
        // In a chat its room is kept even while it is empty, so it coming and going never moves
        // the transcript.
        // The card's width as last drawn: the tab gives up words to fit inside it.
        let card_width = window.use_keyed_state("composer-card-width", cx, |_, _| 0f32);
        let room = Some(*card_width.read(cx) - 2. * TAB_INSET).filter(|room| *room > 0.);
        let measure = {
            let text_system = window.text_system().clone();
            let style = Type::Small.style();
            move |text: &str| {
                let run = gpui::TextRun { len: text.len(), font: gpui::font(style.family), color: theme.text, background_color: None, underline: None, strikethrough: None };
                f32::from(text_system.shape_line(SharedString::from(text.to_string()), px(style.size), &[run], None).width())
            }
        };
        let tab = tab(&data, running_tasks, asking, viewport, room, &measure, &theme);
        let card = div()
            .relative()
            .when(tab.is_some() || !data.roomy, |card| card.mt(px(TAB_HEIGHT - 1.)))
            .when_some(moment, |card, moment| card.top(px(-moment.lift)))
            .child(card)
            .child(
                gpui::canvas(
                    move |bounds, _, cx| {
                        let width = f32::from(bounds.size.width);
                        if (*card_width.read(cx) - width).abs() > 0.5 {
                            card_width.update(cx, |shown, cx| {
                                *shown = width;
                                cx.notify();
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .children(moment.map(|moment| send_glint(moment, &theme)))
            .children(tab.map(|tab| div().absolute().left(px(TAB_INSET)).top(px(1. - TAB_HEIGHT)).child(tab)))
            // The `/` menu rises from the card's top edge, over whatever is above it.
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
        // What was written while the agent worked waits at the foot of the chat, as faint
        // bubbles that firm up when the agent takes them.
        if !data.waiting.is_empty() {
            let steering = data.waiting.iter().filter(|waiting| waiting.steering).count();
            let mut bubbles = div().id("waiting").flex_shrink(1.).min_h(px(40.)).max_h(px(WAITING_MAX_HEIGHT)).overflow_y_scroll().flex().flex_col().gap(px(6.)).pb(px(4.));
            for (index, waiting) in data.waiting.into_iter().enumerate() {
                let queued = (!waiting.steering).then(|| index - steering);
                bubbles = bubbles.child(div().flex_none().child(waiting_bubble(index, queued, waiting, data.working, &theme)));
            }
            column = column.child(bubbles.tag("waiting"));
        }
        if let Some(access) = data.access {
            column = column.child(div().flex_none().pb(px(4.)).child(crate::computer_card::ComputerCard::new(access)));
        }
        if let Some(question) = data.question {
            column = column.child(
                div().min_h(px(QUESTION_MIN_HEIGHT)).flex_shrink(1.).flex().flex_col().pb(px(4.)).child(crate::question::QuestionCard::new(question)),
            );
        }
        column.child(div().flex_none().child(card)).into_any_element()
    }
}

/// How much of each segment the tab shows: everything while there is room, then less, in the
/// order [`Fit::fitting`] gives things up, so the tab never runs past the card.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fit {
    /// The longest the todo in hand runs; 0 leaves it out.
    todo_words: f32,
    /// "⌘B Background", or "⌘B" alone.
    background_words: bool,
    /// "2 working · 1 queued", or "2 working" alone.
    crew_all: bool,
    /// "Working for 8s", or "8s" alone.
    activity_words: bool,
    /// The changed files' lines added and removed.
    change_lines: bool,
    background: bool,
    crew: bool,
    changes: bool,
}

impl Fit {
    const FULL: Fit = Fit { todo_words: TODO_WORDS, background_words: true, change_lines: true, activity_words: true, crew_all: true, background: true, crew: true, changes: true };

    /// The fullest tab no wider than `room`, given how wide a fit draws (`width`): the todo in
    /// hand shortens first, then ⌘B's word, the crew's queue, "Working for", the changed lines,
    /// and last whole segments (⌘B, the crew, the changes). The time, the todo count and the
    /// running commands always stay.
    fn fitting(room: Option<f32>, width: impl Fn(Fit) -> f32) -> Fit {
        let mut fit = Fit::FULL;
        let Some(room) = room else {
            return fit;
        };
        if width(fit) <= room {
            return fit;
        }
        // The todo's words give up exactly what is over, while enough of them stays to read.
        let over = width(fit) - room;
        let without = width(Fit { todo_words: 0., ..fit });
        // What the words draw, less the gap before them.
        let drawn = width(fit) - without - TAB_GAP;
        if drawn - over >= TODO_WORDS_LEAST {
            return Fit { todo_words: drawn - over, ..fit };
        }
        fit.todo_words = 0.;
        let steps: [fn(&mut Fit); 7] = [
            |fit| fit.background_words = false,
            |fit| fit.crew_all = false,
            |fit| fit.activity_words = false,
            |fit| fit.change_lines = false,
            |fit| fit.background = false,
            |fit| fit.crew = false,
            |fit| fit.changes = false,
        ];
        for step in steps {
            if width(fit) <= room {
                break;
            }
            step(&mut fit);
        }
        fit
    }
}

/// The tab on the card's top edge: a segment for each thing worth saying (see the module's
/// header), joined by hairlines, no wider than `room` (the card's width less the tab's insets,
/// once known; `measure` gives a small text's width). `None` when there is nothing to say.
#[allow(clippy::too_many_arguments)]
fn tab(data: &ComposerData, running: usize, asking: bool, viewport: gpui::Size<Pixels>, room: Option<f32>, measure: &dyn Fn(&str) -> f32, theme: &Theme) -> Option<gpui::AnyElement> {
    let opens = !asking;
    let activity_time = |time: &SharedString, fit: Fit| -> SharedString {
        match time.strip_prefix("Working for ") {
            Some(short) if !fit.activity_words => short.to_string().into(),
            _ => time.clone(),
        }
    };
    let crew_words = |crew: &CrewAtWork, fit: Fit| -> String {
        let words = crew.words();
        if fit.crew_all { words } else { words.split(" · ").next().unwrap_or_default().to_string() }
    };
    let todo_task = data.todos.as_ref().and_then(|todos| crate::todo_panel::in_hand(&todos.view));
    let changes = data.changes.filter(|changes| changes.files > 0);
    // What each segment draws, by the sizes below: its padding, icons, the gaps between, words.
    const PAD: f32 = 16.;
    const GAP: f32 = TAB_GAP;
    const RULE: f32 = 5.;
    let width = |fit: Fit| -> f32 {
        let mut widths: Vec<f32> = Vec::new();
        if let Some(time) = &data.activity {
            widths.push(PAD + 13. + GAP + measure(&activity_time(time, fit)));
        }
        if let Some(crew) = data.crew_at_work.filter(|_| fit.crew) {
            widths.push(PAD + 12. + GAP + measure(&crew_words(&crew, fit)));
        }
        if let Some(todos) = &data.todos {
            let mut todo = PAD + 12. + GAP + measure(&format!("{}/{}", todos.view.done, todos.view.total));
            if let Some(task) = &todo_task
                && fit.todo_words > 0.
            {
                todo += GAP + measure(task).min(fit.todo_words);
            }
            if opens {
                todo += GAP + 10.;
            }
            widths.push(todo);
        }
        if running > 0 {
            widths.push(PAD + 12. + GAP + measure(&format!("{running} running")) + if opens { GAP + 10. } else { 0. });
        }
        if let Some(changes) = changes.filter(|_| fit.changes) {
            let noun = if changes.files == 1 { "file" } else { "files" };
            let mut change = PAD + 12. + GAP + measure(&format!("{} {noun}", changes.files));
            if fit.change_lines {
                if changes.added > 0 {
                    change += GAP + measure(&format!("+{}", changes.added));
                }
                if changes.removed > 0 {
                    change += GAP + measure(&format!("−{}", changes.removed));
                }
            }
            widths.push(change);
        }
        if data.working && data.foreground && fit.background {
            widths.push(PAD + measure("⌘B") + if fit.background_words { GAP + measure("Background") } else { 0. });
        }
        // The tab's own padding and edges, and a hairline between segments.
        let rules = widths.len().saturating_sub(1) as f32 * RULE;
        widths.iter().sum::<f32>() + rules + 10.
    };
    let fit = Fit::fitting(room, width);
    let segment = |id: &'static str| div().id(id).flex_none().h_full().px(px(8.)).rounded(px(8.)).flex().items_center().gap(px(TAB_GAP));
    let words = |text: SharedString, colour: Hsla| div().flex_none().typography(Type::Small).text_color(colour).child(text);
    // Closed it points up, where the list rises; open, down, where it goes back.
    let chevron = |open: bool| {
        let down = Icon::ChevronDown.view(px(10.), theme.text_faint);
        if open { down } else { down.with_transformation(gpui::Transformation::rotate(gpui::radians(std::f32::consts::PI))) }
    };
    let lead_idle = data.activity.is_none();
    let mut segments: Vec<gpui::AnyElement> = Vec::new();

    if let Some(time) = data.activity.clone() {
        segments.push(
            segment("activity")
                .child(crate::motion::paddling(div().flex_none().child(Icon::Duck.view(px(13.), theme.accent_text))))
                .child(words(activity_time(&time, fit), theme.text_soft))
                .tag("activity")
                .into_any_element(),
        );
    }
    if let Some(crew) = data.crew_at_work.filter(|_| fit.crew) {
        let duck = Icon::Duck.view(px(12.), if crew.working > 0 { theme.accent_text } else { theme.text_faint });
        segments.push(
            segment("crew-pill")
                .cursor_pointer()
                .hover(|segment| segment.bg(theme.hover))
                // The lead's run ended with its crew still out: it wakes as their reports come in.
                .tooltip(tip(if lead_idle { "The lead picks up as reports come in. Show the team (⌘⇧A)" } else { "Show the team (⌘⇧A)" }))
                .on_click(dispatch(crate::actions::ShowTeam))
                // One duck paddles at a time: the lead's while it works, else the crew's.
                .child(if lead_idle && crew.working > 0 { crate::motion::paddling(div().child(duck)).into_any_element() } else { duck.into_any_element() })
                .child(words(crew_words(&crew, fit).into(), theme.text_soft))
                .tag("crew-pill")
                .into_any_element(),
        );
    }
    if let Some(todos) = data.todos.clone() {
        let open = data.todos_open && opens;
        let view = &todos.view;
        let live = todos.busy && view.active;
        let mark = if view.active {
            div()
                .flex_none()
                .size(px(11.))
                .rounded_full()
                .border_1()
                .border_color(if live { theme.accent_text } else { theme.text_faint })
                .when(live, |mark| mark.child(div().m(px(2.)).size(px(5.)).rounded_full().bg(theme.accent_text)))
                .into_any_element()
        } else {
            Icon::Check.view(px(12.), theme.success).into_any_element()
        };
        let button = segment("todos-pill")
            .when(open, |segment| segment.bg(theme.hover))
            .when(opens, |segment| segment.cursor_pointer().hover(|segment| segment.bg(theme.hover)).on_click(dispatch(ToggleTodos)))
            .tooltip(tip("The agent's todo list"))
            .child(mark)
            .child(words(format!("{}/{}", view.done, view.total).into(), theme.text_soft))
            .when(fit.todo_words > 0., |segment| {
                segment.children(todo_task.clone().map(|task| div().flex_none().max_w(px(fit.todo_words)).truncate().typography(Type::Small).text_color(theme.text_dim).child(task)))
            })
            .when(opens, |segment| segment.child(chevron(open)))
            .tag("todos-pill");
        let list = open.then(|| todos_popover(todos, theme));
        segments.push(rising(button, "todos", Box::new(ToggleTodos), list, viewport));
    }
    if running > 0 {
        let open = data.tasks_open && opens;
        let button = segment("tasks-pill")
            .when(open, |segment| segment.bg(theme.hover))
            .when(opens, |segment| segment.cursor_pointer().hover(|segment| segment.bg(theme.hover)).on_click(dispatch(ToggleTasks)))
            .tooltip(tip("Commands the agent left running"))
            .child(Icon::Terminal.view(px(12.), theme.accent_text))
            .child(words(format!("{running} running").into(), theme.text_soft))
            .when(opens, |segment| segment.child(chevron(open)))
            .tag("tasks-pill");
        let list = open.then(|| tasks_popover(&data.tasks, running, theme));
        segments.push(rising(button, "tasks", Box::new(ToggleTasks), list, viewport));
    }
    if let Some(changes) = changes.filter(|_| fit.changes) {
        let noun = if changes.files == 1 { "file" } else { "files" };
        segments.push(
            segment("changes-pill")
                .cursor_pointer()
                .hover(|segment| segment.bg(theme.hover))
                .tooltip(tip("Show the changes (⌘⇧C)"))
                .on_click(dispatch(crate::actions::ToggleChanges))
                .child(Icon::Branch.view(px(12.), theme.accent_text))
                .child(words(format!("{} {noun}", changes.files).into(), theme.text_soft))
                .when(fit.change_lines && changes.added > 0, |segment| segment.child(words(format!("+{}", changes.added).into(), theme.diff_added)))
                .when(fit.change_lines && changes.removed > 0, |segment| segment.child(words(format!("−{}", changes.removed).into(), theme.diff_removed)))
                .tag("changes-pill")
                .into_any_element(),
        );
    }
    if data.working && data.foreground && fit.background {
        segments.push(
            segment("background-pill")
                .cursor_pointer()
                .hover(|segment| segment.bg(theme.hover))
                .tooltip(tip("Keep it working in the background and let the agent carry on"))
                .cursor_pointer().on_click(dispatch(MoveToBackground))
                .child(words("⌘B".into(), theme.text_faint))
                .when(fit.background_words, |segment| segment.child(words("Background".into(), theme.text_dim)))
                .tag("background-pill")
                .into_any_element(),
        );
    }
    if segments.is_empty() {
        return None;
    }
    let mut tab = div()
        .h(px(TAB_HEIGHT))
        .px(px(4.))
        .py(px(3.))
        .flex()
        .items_center()
        .gap(px(2.))
        .bg(theme.surface)
        .rounded_tl(px(12.))
        .rounded_tr(px(12.))
        .border_t_1()
        .border_l_1()
        .border_r_1()
        .border_color(theme.border);
    for (index, segment) in segments.into_iter().enumerate() {
        if index > 0 {
            tab = tab.child(div().flex_none().w(px(1.)).h(px(12.)).bg(theme.border));
        }
        tab = tab.child(segment);
    }
    Some(tab.tag("composer-tab").into_any_element())
}

/// The longest a todo's words run on the tab before they truncate, and the least worth showing
/// when the tab is short of room.
const TODO_WORDS: f32 = 220.;
const TODO_WORDS_LEAST: f32 = 60.;
/// The space between the things in a segment of the tab.
const TAB_GAP: f32 = 6.;

/// A segment of the tab with its list, when open, rising from it over everything; a click
/// anywhere else closes the list (`close`), as the pickers' menus close.
fn rising(segment: impl IntoElement, name: &'static str, close: Box<dyn gpui::Action>, list: Option<gpui::AnyElement>, viewport: gpui::Size<Pixels>) -> gpui::AnyElement {
    div()
        .relative()
        .h_full()
        .flex_none()
        .child(segment)
        .when_some(list, |wrap, list| {
            let backdrop = anchored().position_mode(AnchoredPositionMode::Window).position(point(px(0.), px(0.))).child(
                div().id(SharedString::from(format!("{name}-backdrop"))).w(viewport.width).h(viewport.height).on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(close.boxed_clone(), cx);
                }),
            );
            let card = div().absolute().top_0().left_0().child(
                anchored()
                    .anchor(gpui::Anchor::BottomLeft)
                    .offset(point(px(-4.), px(-PICKER_GAP)))
                    .snap_to_window_with_margin(px(8.))
                    .child(crate::motion::enter(ElementId::Name(format!("arrive-{name}-list").into()), div().child(list))),
            );
            wrap.child(deferred(backdrop).with_priority(1)).child(deferred(card).with_priority(2))
        })
        .into_any_element()
}

/// The frame of a list rising from the tab: raised over the transcript, as the pickers are.
fn list_frame(id: &'static str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        // Presses inside stay inside: the click-away layer beneath must not close the list.
        .occlude()
        .w(px(LIST_WIDTH))
        .px(px(14.))
        .py(px(12.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .rounded(px(16.))
        .bg(theme.raised)
        .border_1()
        .border_color(theme.border_strong)
        .shadow_lg()
}

const LIST_WIDTH: f32 = 460.;

fn todos_popover(todos: crate::todo_panel::TodoData, theme: &Theme) -> gpui::AnyElement {
    list_frame("todos-list", theme).child(crate::todo_panel::todo_panel(todos, theme)).into_any_element()
}

/// The commands the agent left running, each with its newest output, Output and Stop; then a
/// few that ended. Its foot opens the Tasks pane, where every command's whole output is.
fn tasks_popover(tasks: &[TaskLine], running: usize, theme: &Theme) -> gpui::AnyElement {
    let head = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(Icon::Terminal.view(px(13.), theme.accent_text))
        .child(div().typography(Type::UiStrong).text_color(theme.text_soft).child("Background commands"))
        .child(div().typography(Type::Small).text_color(theme.text_faint).child(format!("{running} running")))
        .child(div().flex_1())
        .child(
            div()
                .id("tasks-all")
                .typography(Type::Small)
                .text_color(theme.text_dim)
                .cursor_pointer()
                .hover(|word| word.text_color(theme.accent_text))
                .on_click(dispatch(crate::actions::ToggleTasksPane))
                .child("Tasks pane")
                .tag("tasks-all"),
        );
    let rows = div()
        .id("tasks")
        .max_h(px(TASKS_MAX_HEIGHT))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(2.))
        .children(tasks.iter().cloned().map(|task| task_row(task, theme)));
    list_frame("tasks-list", theme).child(head).child(rows).tag("tasks-list").into_any_element()
}

/// What the ring round Send keeps between frames: whether the pointer is over the ring or its
/// popover, and which of the popover's parts are open.
#[derive(Default)]
struct ContextHover {
    over_ring: bool,
    over_card: bool,
    open: Vec<Kind>,
}

/// Send (or Stop while the agent works), with the context's ring round it. Hovering the ring
/// opens the context popover; a click is Send's or Stop's alone, so reaching for the popover can
/// never send or stop anything.
fn send_ring(data: &ComposerData, has_text: bool, viewport: gpui::Size<Pixels>, window: &mut Window, cx: &mut App, theme: &Theme) -> impl IntoElement + use<> {
    let button = if data.working {
        div()
            .id("stop")
            .flex_none()
            .size(px(SEND))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border_strong)
            .hover(|button| button.bg(theme.hover))
            .tooltip(tip("Stop (⎋)"))
            .cursor_pointer().on_click(dispatch(Stop))
            .child(Icon::Stop.view(px(12.), theme.text))
            .tag("stop")
    } else {
        div()
            .id("send")
            .flex_none()
            .size(px(SEND))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .when(has_text, |button| button.bg(theme.accent_fill()).hover(|button| button.bg(theme.accent_fill_hover())))
            .when(!has_text, |button| button.bg(theme.hover))
            .cursor_pointer().on_click(dispatch(Send))
            .child(Icon::ArrowUp.view(px(15.), if has_text { theme.on_accent } else { theme.text_faint }))
            .tag("send")
    };
    let Some(meter) = data.meter.clone() else {
        return div().flex_none().size(px(RING)).flex().items_center().justify_center().child(button).into_any_element();
    };
    let hover = window.use_keyed_state(ElementId::Name(format!("context-{}", data.key).into()), cx, |_, _| ContextHover::default());
    let (open, parts_open) = {
        let state = hover.read(cx);
        (state.over_ring || state.over_card || data.context_open, state.open.clone())
    };
    let colour = if meter.high { theme.warning } else { theme.accent };
    let over_ring = hover.clone();
    let mut cluster = div()
        .id("send-ring")
        .relative()
        .flex_none()
        .size(px(RING))
        .flex()
        .items_center()
        .justify_center()
        .on_hover(move |hovered, _, cx| {
            let hovered = *hovered;
            over_ring.update(cx, |state, cx| {
                if state.over_ring != hovered {
                    state.over_ring = hovered;
                    cx.notify();
                }
            });
        })
        .child(div().absolute().top_0().left_0().child(crate::ring::ring(meter.share().unwrap_or(0.), RING, RING_WIDTH, theme.border, colour)))
        .child(button);
    if open {
        let over_card = hover.clone();
        let card = context_card(&meter, data.working, data.model_missing.is_some(), &parts_open, hover.clone(), theme);
        // The popover's foot reaches down to the ring, so the pointer never crosses a gap that
        // would close it on the way up.
        let popover = div().absolute().top_0().right_0().child(
            anchored()
                .anchor(gpui::Anchor::BottomRight)
                .offset(point(px(10.), px(0.)))
                .snap_to_window_with_margin(px(8.))
                .child(
                    div()
                        .id("context-popover")
                        // A press on the popover stays there: it never reaches the chat beneath.
                        .occlude()
                        .pb(px(PICKER_GAP + 2.))
                        .on_hover(move |hovered, _, cx| {
                            let hovered = *hovered;
                            over_card.update(cx, |state, cx| {
                                if state.over_card != hovered {
                                    state.over_card = hovered;
                                    cx.notify();
                                }
                            });
                        })
                        .child(crate::motion::enter(ElementId::Name(format!("arrive-context-{}", data.key).into()), div().child(card))),
                ),
        );
        let _ = viewport;
        cluster = cluster.child(deferred(popover).with_priority(2));
    }
    cluster.tag("context-ring").into_any_element()
}

/// The context popover, ported from WackCode 2: how full, how long until the agent compacts on
/// its own, a bar of every part, and each part with its share, opening to its pieces.
fn context_card(meter: &Meter, working: bool, model_missing: bool, open: &[Kind], hover: Entity<ContextHover>, theme: &Theme) -> impl IntoElement + use<> {
    let count = crate::work::count;
    let window = meter.window.filter(|window| *window > 0);
    let percent = meter.share().map(|share| share * 100.);
    let colour = if meter.high { theme.warning } else { theme.accent };
    let threshold = window.map(|window| window * wack_core::context::COMPACT_AT_PERCENT / 100);
    let used_words = match window {
        Some(window) => format!("{} of {} tokens", count(meter.tokens), count(window)),
        None => format!("{} tokens", count(meter.tokens)),
    };
    let left_words: Option<(String, Hsla)> = match threshold {
        _ if model_missing => Some(("No model configured".into(), theme.warning)),
        Some(at) if meter.tokens >= at => Some(("Auto-compaction is due".into(), theme.warning)),
        Some(at) => Some((format!("{} until auto-compact", count(at - meter.tokens)), theme.accent_text)),
        None => Some(("This model's window isn't known".into(), theme.text_faint)),
    };
    let hero = div()
        .flex()
        .items_center()
        .gap(px(14.))
        .child(
            div()
                .relative()
                .flex_none()
                .size(px(HERO_RING))
                .child(crate::ring::ring(meter.share().unwrap_or(0.), HERO_RING, 4., theme.border, colour))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .typography(Type::Small)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(percent.map(|percent| format!("{}%", percent.round())).unwrap_or_else(|| "—".into())),
                ),
        )
        .child(
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(div().typography(Type::UiStrong).text_color(theme.text).child("Context window"))
                .child(div().typography(Type::Small).text_color(theme.text_dim).child(used_words))
                .children(left_words.map(|(words, colour)| div().typography(Type::Small).text_color(colour).child(words))),
        );
    let part_colour = |kind: Kind| -> Hsla {
        match kind {
            Kind::Instructions => theme.parts[0],
            Kind::Tools => theme.parts[1],
            Kind::Project => theme.parts[2],
            Kind::Skills => theme.parts[3],
            Kind::Memory => theme.parts[4],
            Kind::Summary => theme.parts[5],
            Kind::Messages => theme.accent,
            Kind::ToolResults => theme.parts[6],
        }
    };
    let whole = window.unwrap_or(meter.tokens.max(1)) as f32;
    let mut bar = div().mt(px(14.)).h(px(8.)).w_full().rounded_full().overflow_hidden().flex().bg(theme.hover);
    if meter.parts.is_empty() {
        bar = bar.child(div().h_full().w(gpui::relative(meter.share().unwrap_or(0.))).bg(colour));
    } else {
        for part in &meter.parts {
            bar = bar.child(div().h_full().w(gpui::relative((part.tokens as f32 / whole).min(1.))).bg(part_colour(part.kind)));
        }
    }
    let share_words = |tokens: u64| -> String {
        match window {
            Some(window) => {
                let percent = tokens as f32 / window as f32 * 100.;
                if percent > 0. && percent < 0.05 { "<0.1%".into() } else { format!("{percent:.1}%") }
            }
            None => String::new(),
        }
    };
    let mut rows = div().mt(px(10.)).flex().flex_col();
    if meter.parts.is_empty() {
        rows = rows.child(
            div().px(px(4.)).py(px(4.)).typography(Type::Small).text_color(theme.text_faint).child("What each part holds arrives after the next reply."),
        );
    }
    for (index, part) in meter.parts.iter().enumerate() {
        let kind = part.kind;
        let opens = part.pieces.len() > 1;
        let is_open = opens && open.contains(&kind);
        let toggle = hover.clone();
        rows = rows.child(
            div()
                .id(("context-part", index))
                .h(px(26.))
                .px(px(6.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .when(opens, |row| {
                    row.cursor_pointer().hover(|row| row.bg(theme.hover)).on_click(move |_, _, cx| {
                        toggle.update(cx, |state, cx| {
                            match state.open.iter().position(|open| *open == kind) {
                                Some(at) => {
                                    state.open.remove(at);
                                }
                                None => state.open.push(kind),
                            }
                            cx.notify();
                        });
                    })
                })
                .child(div().flex_none().size(px(8.)).rounded_full().bg(part_colour(kind)))
                .child(div().flex_1().min_w_0().truncate().typography(Type::Small).text_color(theme.text_soft).child(kind.label()))
                .child(div().flex_none().typography(Type::Small).text_color(theme.text).child(count(part.tokens)))
                .child(div().flex_none().w(px(42.)).flex().justify_end().typography(Type::Small).text_color(theme.text_faint).child(share_words(part.tokens)))
                .child(div().flex_none().w(px(10.)).children(opens.then(|| if is_open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(10.), theme.text_faint))))
                .tag("context-part"),
        );
        if is_open {
            let mut pieces = div().pl(px(22.)).pr(px(22.)).pb(px(4.)).flex().flex_col();
            for (name, tokens) in &part.pieces {
                pieces = pieces.child(
                    div()
                        .h(px(20.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_1().min_w_0().truncate().typography(Type::Small).text_color(theme.text_dim).child(name.clone()))
                        .child(div().flex_none().typography(Type::Small).text_color(theme.text_dim).child(count(*tokens))),
                );
            }
            rows = rows.child(pieces);
        }
    }
    let (foot_words, foot_colour) = if working {
        ("Compacting waits for the agent to finish.", theme.text_faint)
    } else if model_missing {
        ("This chat's model is no longer configured.", theme.warning)
    } else {
        ("Compacting summarises older turns to free room.", theme.text_faint)
    };
    // A figure for the whole chat on a tile of its own, its label small over it, as version 2's
    // were; a dash until there is one.
    let stat = |label: &'static str, value: Option<String>, tag: &'static str| {
        div()
            .flex_1()
            .min_w_0()
            .px(px(10.))
            .py(px(7.))
            .rounded(px(10.))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(1.))
            .child(div().truncate().typography(Type::Small).text_color(theme.text_faint).child(label))
            .child(
                div()
                    .typography(Type::UiStrong)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(if value.is_some() { theme.text } else { theme.text_faint })
                    .child(value.unwrap_or_else(|| "—".into())),
            )
            .tag(tag)
    };
    // What the chat has cost so far, sub-agents included, and how much of what it sent the
    // provider read from its cache: cheap and fast when high. Side by side under a hairline.
    let stats = div()
        .mt(px(10.))
        .pt(px(10.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .gap(px(6.))
        .child(stat("Session tokens", (meter.spent > 0).then(|| count(meter.spent)), "context-spent"))
        .child(stat("Prompt cache hits", meter.cache.map(|rate| format!("{:.1}%", rate * 100.)), "context-cache"));
    let foot = div()
        .mt(px(10.))
        .pt(px(10.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .typography(Type::Small)
                .text_color(foot_colour)
                .child(foot_words),
        )
        .when(meter.can_compact && !working && !model_missing, |foot| {
            foot.child(
                div()
                    .id("compact")
                    .flex_none()
                    .h(px(28.))
                    .px(px(12.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .bg(theme.accent_wash)
                    .border_1()
                    .border_color(theme.accent_line)
                    .cursor_pointer()
                    .hover(|button| button.bg(theme.hover))
                    .typography(Type::Small)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.accent_text)
                    .on_click(dispatch(Compact))
                    .child("Compact")
                    .tag("compact"),
            )
        });
    div()
        .w(px(CONTEXT_WIDTH))
        .p(px(16.))
        .rounded(px(18.))
        .bg(theme.raised)
        .border_1()
        .border_color(theme.border)
        .shadow_lg()
        .child(hero)
        .child(bar)
        .child(rows)
        .child(stats)
        .child(foot)
        .tag("context-popover")
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
/// A thumbnail's side in the tray.
const THUMB: f32 = 56.;

/// Comments drafted on a canvas, waiting for the next message: a click shows them on the
/// canvas, and Send now sends them alone.
fn comments_chip(count: usize, canvas: String, can_send: bool, theme: &Theme) -> impl IntoElement {
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
                .border_1()
                .border_color(theme.border)
                .typography(Type::Small)
                .when(can_send, |send| {
                    send.cursor_pointer()
                        .hover(|send| send.bg(theme.hover))
                        .text_color(theme.text_soft)
                        .tooltip(tip("Send the comments with no message"))
                        .on_click(dispatch(crate::actions::Send))
                })
                .when(!can_send, |send| send.opacity(0.45).text_color(theme.text_dim))
                .child("Send now")
                .tag("composer-send-comments"),
        )
}

/// What is attached to the message: each picture with its remove button, and each text file
/// (picked, dropped, pasted, or from the Terminal) as a chip.
fn tray(attachments: &[crate::attachments::Attachment], files: &[FileAttachment], theme: &Theme) -> impl IntoElement {
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
            .child(remove_button(("remove-attachment", index), RemoveAttachment { index }, "remove-attachment", theme))
    });
    let file_chips = files.iter().enumerate().map(|(index, file)| {
        let lines = file.lines();
        let line_words = if lines == 1 { "1 line".to_string() } else { format!("{lines} lines") };
        let terminal = file.source == FileSource::Terminal;
        let icon = if terminal { Icon::Terminal.view(px(16.), theme.accent_text) } else { Icon::File.view(px(16.), theme.text_dim) };
        div()
            .id(("attached-file", index))
            .group("attached-file")
            .relative()
            .flex_none()
            .h(px(THUMB))
            .pl(px(12.))
            .pr(px(14.))
            .rounded(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border)
            .tooltip(tip(format!("{} · {line_words} · {}", file.name, format_bytes(file.bytes()))))
            .child(icon)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(div().max_w(px(220.)).truncate().typography(Type::Mono).text_color(theme.text).child(file.name.clone()))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .typography(Type::Small)
                            .child(div().text_color(theme.text_faint).child(line_words))
                            .when(file.source == FileSource::Paste, |meta| {
                                meta.child(
                                    div()
                                        .id(("insert-file", index))
                                        .text_color(theme.accent_text)
                                        .cursor_pointer()
                                        .hover(|button| button.underline())
                                        .on_click(dispatch(InsertFile { index }))
                                        .child("Insert as text")
                                        .tag("insert-file"),
                                )
                            }),
                    ),
            )
            .child(remove_button(("remove-file", index), RemoveFile { index }, "remove-file", theme))
            .tag(if terminal { "terminal-file-chip" } else { "file-chip" })
    });
    div().flex().flex_wrap().gap(px(8.)).pt(px(2.)).children(thumbs).children(file_chips).tag("attachments")
}

fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{} KB", (bytes + 512) / 1024)
    }
}

/// The small round × on an attachment's corner.
fn remove_button(id: (&'static str, usize), action: impl gpui::Action, tag: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .id(id)
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
        .on_click(dispatch(action))
        .child(Icon::Close.view(px(10.), theme.text_soft))
        .tag(tag)
}

/// A message written while the agent works, waiting at the foot of the chat: faint until the
/// agent takes it. A queued one (`queued` is its place in the queue) can be sent as steering now,
/// taken back to edit, or removed; steering is already on its way.
fn waiting_bubble(index: usize, queued: Option<usize>, waiting: Waiting, working: bool, theme: &Theme) -> impl IntoElement {
    let words = if waiting.steering { "Steering · interrupting the agent now" } else { "Queued · sends when the agent finishes" };
    let action = |id: &'static str, words: &'static str, action: Box<dyn gpui::Action>| {
        div()
            .id((id, index))
            .flex_none()
            .typography(Type::Small)
            .text_color(theme.text_dim)
            .cursor_pointer()
            .hover(|button| button.text_color(theme.text))
            .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            .child(words)
            .tag(id)
    };
    let actions = queued.map(|queue| {
        div()
            .flex()
            .gap(px(12.))
            .invisible()
            .group_hover("waiting-bubble", |actions| actions.visible())
            .when(working, |actions| actions.child(action("steer-queued", "Steer now", Box::new(SteerQueued { index: queue }))))
            .child(action("unqueue", "Edit", Box::new(Unqueue { index: queue })))
            .child(action("remove-queued", "Remove", Box::new(RemoveQueued { index: queue })))
    });
    div().w_full().flex().justify_end().pl(px(64.)).child(
        div()
            .id(("waiting", index))
            .group("waiting-bubble")
            .max_w(gpui::relative(0.82))
            .px(px(14.))
            .py(px(8.))
            .rounded(px(16.))
            .bg(Hsla { a: theme.surface.a * 0.55, ..theme.surface })
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .typography(Type::Small)
                            .text_color(if waiting.steering { theme.accent_text } else { theme.text_faint })
                            .child(words),
                    )
                    .children(actions),
            )
            .when(!waiting.files.is_empty(), |bubble| {
                bubble.child(div().flex().flex_wrap().gap(px(6.)).children(waiting.files.into_iter().map(|name| {
                    div()
                        .h(px(22.))
                        .px(px(8.))
                        .rounded(px(7.))
                        .bg(theme.raised)
                        .border_1()
                        .border_color(theme.border)
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(Icon::File.view(px(11.), theme.text_dim))
                        .child(div().max_w(px(180.)).truncate().typography(Type::Small).text_color(theme.text_soft).child(name))
                })))
            })
            .when(!waiting.text.is_empty(), |bubble| {
                bubble.child(div().typography(Type::Ui).text_color(theme.text_dim).line_clamp(3).child(waiting.text))
            })
            .tag("waiting-bubble"),
    )
}

/// A round control in the composer's bar.
fn pill(tag: &'static str) -> gpui::Stateful<gpui::Div> {
    div().id(tag).h(px(28.)).px(px(10.)).rounded_full().flex().items_center().gap(px(4.))
}

/// A background command above the card: how it stands, its last line, its output and Stop.
fn task_row(task: TaskLine, theme: &Theme) -> impl IntoElement {
    let running = task.status.running();
    let colour = crate::panes::tasks::status_colour(&task.status, theme);
    let (show, stop) = (task.id.clone(), task.id.clone());
    let top = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(div().flex_none().size(px(6.)).rounded_full().bg(colour))
        .child(div().flex_1().min_w_0().truncate().typography(Type::Ui).text_color(if running { theme.text } else { theme.text_dim }).child(task.label))
        .child(div().flex_none().typography(Type::Small).text_color(if running { theme.text_faint } else { colour }).child(task.words))
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
        });
    div()
        .id(SharedString::from(format!("task-{}", task.id)))
        .px(px(8.))
        .py(px(6.))
        .rounded(px(10.))
        .hover(|row| row.bg(theme.hover))
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(top)
        // What it printed last, so progress shows without opening it.
        .when(!task.last.trim().is_empty(), |row| {
            row.child(div().pl(px(16.)).truncate().typography(Type::Mono).text_color(theme.text_faint).child(SharedString::from(task.last.trim().to_string())))
        })
}

/// About three waiting messages; more scroll.
const WAITING_MAX_HEIGHT: f32 = 176.;
/// About five commands; more scroll.
const TASKS_MAX_HEIGHT: f32 = 280.;
/// The tab on the card's top edge: its height, and how far in from the card's left it sits.
const TAB_HEIGHT: f32 = 30.;
const TAB_INSET: f32 = 16.;
/// The ring round Send, its stroke, and the button inside it.
const RING: f32 = 36.;
const RING_WIDTH: f32 = 2.5;
const SEND: f32 = 28.;
/// The context popover's width, and the ring at its head.
const CONTEXT_WIDTH: f32 = 320.;
const HERO_RING: f32 = 48.;
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
        "crew" => Box::new(OpenCrewMenu),
        _ => Box::new(OpenEffortMenu),
    };
    let button = button.cursor_pointer().on_click(move |_, window, cx| window.dispatch_action(open.boxed_clone(), cx)).tag(tag);
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
                Choice::Model { .. } | Choice::Crew { .. } | Choice::CrewEffort(_) => None,
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
            .cursor_pointer()
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
                    .cursor_pointer().on_click(move |_, window, cx| window.dispatch_action(Box::new(SlideEffort { effort }), cx))
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
/// providers (each connection, with how many models it offers); choosing one slides the pair
/// across to its models, with a back button to the list. It opens on the provider of the model
/// in use.
///
/// Both panels are drawn whole while they slide, and the card's height eases from one list's to
/// the other's, so the next list reads from the first frame. (A panel that faded in from nothing
/// under a card that had already jumped to its new size read as lag.) The slide is timed from the
/// click, not from the next frame drawn, and a click mid-slide turns it round from where it is.
/// Each panel scrolls on its own past [`PICKER_LIST_MAX`]; row heights are fixed, which is what
/// lets the card know each list's height before it lays them out.
#[derive(IntoElement)]
struct ModelPicker {
    menu: MenuData,
}

/// Where the picker is: which provider's models it shows (`None`: the list of providers), whose
/// models the second panel holds (kept while it slides back out), and how far across it is, from
/// 0 (the providers) to 1 (the models).
struct PickerView {
    provider: Option<usize>,
    models: usize,
    across: crate::motion::Ease,
}

const PICKER_WIDTH: f32 = 280.;
/// One panel's width: the card's, inside its hairline.
const PICKER_PANEL: f32 = PICKER_WIDTH - 2.;
/// How long a slide between the panels takes.
const PICKER_SLIDE: Duration = Duration::from_millis(240);
/// Past this a panel's list scrolls inside it.
const PICKER_LIST_MAX: f32 = 380.;
/// A provider's or a model's row.
const PICKER_ROW: f32 = 32.;
/// The providers panel's "Providers" heading.
const PICKER_PROVIDERS_HEAD: f32 = 28.;
/// The models panel's back row (30 and a 4 gap) and the hairline under it (1 and a 4 gap).
const PICKER_MODELS_HEAD: f32 = 39.;

impl RenderOnce for ModelPicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let still = cx.reduce_motion();
        let groups = self.menu.groups;
        let current = groups.iter().position(|group| group.items.iter().any(|item| item.selected));
        let initial = current.or_else(|| (groups.len() == 1).then_some(0));
        let state = window.use_keyed_state("model-picker", cx, move |_, _| PickerView {
            provider: initial,
            models: initial.unwrap_or(0),
            across: crate::motion::Ease::at(if initial.is_some() { 1. } else { 0. }).lasting(PICKER_SLIDE),
        });
        let (models_of, across) = {
            let view = state.read(cx);
            if view.across.moving() {
                window.request_animation_frame();
            }
            (view.models.min(groups.len().saturating_sub(1)), view.across.value())
        };
        let go = move |to: Option<usize>| {
            let state = state.clone();
            move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
                cx.stop_propagation();
                state.update(cx, |view, _| {
                    view.provider = to;
                    if let Some(index) = to {
                        view.models = index;
                    }
                    view.across.toward(if to.is_some() { 1. } else { 0. }, still);
                });
                window.refresh();
            }
        };
        let row = |id: (&'static str, usize)| {
            div()
                .id(id)
                .flex_none()
                .mx(px(6.))
                .px(px(8.))
                .h(px(PICKER_ROW))
                .rounded(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .hover(|row| row.bg(theme.hover))
        };
        let panel = |id: &'static str| div().id(id).flex_none().w(px(PICKER_PANEL)).max_h(px(PICKER_LIST_MAX)).overflow_y_scroll().flex().flex_col();

        let mut providers = panel("picker-providers").child(
            div()
                .flex_none()
                .h(px(PICKER_PROVIDERS_HEAD))
                .px(px(14.))
                .pb(px(6.))
                .flex()
                .items_end()
                .typography(Type::Label)
                .text_color(theme.text_dim)
                .child("Providers"),
        );
        if groups.is_empty() {
            providers = providers.child(
                div()
                    .px(px(14.))
                    .py(px(6.))
                    .typography(Type::Small)
                    .text_color(theme.text_dim)
                    .child("No enabled connections in Settings › Providers."),
            );
        }
        for (index, group) in groups.iter().enumerate() {
            let holds_current = current == Some(index);
            let name = group.title.clone().unwrap_or_else(|| "Models".into());
            providers = providers.child(
                row(("picker-provider", index))
                    .cursor_pointer().on_click(go(Some(index)))
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

        let card = div()
            .id("menu")
            .occlude()
            .w(px(PICKER_WIDTH))
            .overflow_hidden()
            .py(px(6.))
            .rounded(px(14.))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border_strong)
            .shadow_lg();
        // With no connection there is nowhere to slide to.
        let Some(group) = groups.get(models_of) else {
            return card.child(providers).when_some(self.menu.team, |card, team| card.child(team_row(team, &theme)));
        };

        let back = div()
            .id("picker-back")
            .flex_none()
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
        let mut models = panel("picker-models").child(back).child(div().flex_none().mx(px(12.)).mb(px(4.)).h(px(1.)).bg(theme.border));
        for (number, item) in group.items.iter().enumerate() {
            let action = item.choice.action();
            models = models.child(
                row(("menu-item", models_of * 1000 + number))
                    .when(item.selected, |row| row.bg(theme.accent_wash))
                    .cursor_pointer().on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
                    .when_some(item.brand, |row, brand| row.child(brand.view(px(14.), &theme)))
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

        // The pair of panels in a window as tall as the list in view, sliding across together.
        let height = |content: f32| content.min(PICKER_LIST_MAX);
        let providers_height = height(PICKER_PROVIDERS_HEAD + PICKER_ROW * groups.len() as f32);
        let models_height = height(PICKER_MODELS_HEAD + PICKER_ROW * group.items.len() as f32);
        let track = div()
            .relative()
            .left(px(-PICKER_PANEL * across))
            .w(px(PICKER_PANEL * 2.))
            .flex()
            .items_start()
            .child(providers)
            .child(models);
        card.child(div().w_full().h(px(providers_height + (models_height - providers_height) * across)).overflow_hidden().child(track))
            .when_some(self.menu.team, |card, team| card.child(team_row(team, &theme)))
    }
}

impl Choice {
    fn action(&self) -> Box<dyn gpui::Action> {
        match self.clone() {
            Choice::Model { connection, model } => Box::new(ChooseModel { connection, model }),
            Choice::Effort(effort) => Box::new(ChooseEffort { effort }),
            Choice::Crew { connection, model } => Box::new(ChooseCrew { connection, model }),
            Choice::CrewEffort(effort) => Box::new(ChooseCrewEffort { effort }),
        }
    }
}

/// The model picker's foot: Team's switch, and what it means.
fn team_row(team: TeamRow, theme: &Theme) -> impl IntoElement {
    let enabled = !team.locked && (team.on || team.unavailable.is_none());
    let hint: SharedString = match (&team.unavailable, team.on, team.locked) {
        (_, _, true) => "The Team stays as it is until the agent finishes.".into(),
        (Some(why), false, _) => why.clone(),
        (_, true, _) => "This model leads; the crew does the reading and typing.".into(),
        (_, false, _) => "Lead with this model; a cheaper crew does the typing.".into(),
    };
    div()
        .id("team-row")
        .mt(px(6.))
        .mx(px(6.))
        .pt(px(8.))
        .px(px(8.))
        .pb(px(2.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(10.))
        .when(enabled, |row| row.cursor_pointer().on_click(move |_, window, cx| window.dispatch_action(Box::new(SetTeam { on: !team.on }), cx)))
        .child(crate::motion::paddling(div().flex_none().child(Icon::Duck.view(px(14.), if team.on { theme.accent_text } else { theme.text_dim }))))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().typography(Type::UiStrong).text_color(theme.text).child("Team"))
                .child(div().typography(Type::Small).text_color(if team.unavailable.is_some() && !team.on { theme.warning } else { theme.text_dim }).child(hint)),
        )
        .child(switch(team.on, enabled, theme))
        .tag("team-row")
}

/// A small on/off switch: a capsule whose knob sits right when on.
fn switch(on: bool, enabled: bool, theme: &Theme) -> impl IntoElement {
    let track = if on { theme.accent_fill() } else { Background::from(theme.hover) };
    div()
        .flex_none()
        .w(px(30.))
        .h(px(18.))
        .p(px(2.))
        .rounded_full()
        .bg(track)
        .border_1()
        .border_color(if on { theme.accent_line } else { theme.border_strong })
        .when(!enabled, |track| track.opacity(0.5))
        .flex()
        .when(on, |track| track.justify_end())
        .child(div().size(px(12.)).rounded_full().bg(if on { theme.on_accent } else { theme.text_dim }))
}

/// The crew's half of a Team's model pill: the duckling, the crew's model and its effort. It
/// slides in beside the lead's half when Team goes on. In a narrow composer it gives way before
/// the lead's, so the model in charge stays the one most readable.
fn crew_half(crew: CrewPill, popover: Option<gpui::AnyElement>, key: SharedString, viewport: gpui::Size<Pixels>, still: bool, theme: &Theme) -> impl IntoElement {
    let ink = if crew.problem.is_some() { theme.warning } else { theme.text_soft };
    let button = pill("crew")
        .min_w(px(MODEL_MIN))
        .gap(px(6.))
        .text_color(ink)
        .hover(|pill| pill.bg(theme.hover).text_color(if crew.problem.is_some() { theme.warning } else { theme.text }))
        .tooltip(tip(match &crew.problem {
            Some(problem) => problem.to_string(),
            None => format!("Team crew: {}. Explore and General run on it; Review runs on the lead's model.", crew.model),
        }))
        .child(Icon::Duck.view(px(11.), if crew.problem.is_some() { theme.warning } else { theme.accent_text }))
        .child(div().min_w_0().truncate().typography(Type::Ui).child(crew.model.clone()))
        .when_some(crew.effort.clone(), |pill, effort| pill.child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(effort)))
        .child(Icon::ChevronDown.view(px(12.), theme.text_faint));
    let half = div()
        .flex()
        .items_center()
        .min_w(px(MODEL_MIN))
        .flex_shrink(2.)
        .child(div().flex_none().w(px(1.)).h(px(16.)).bg(theme.border_strong))
        .child(picker("crew", button, popover, true, viewport));
    if still {
        half.into_any_element()
    } else {
        crate::motion::slide_in(ElementId::Name(format!("crew-arrives-{key}").into()), half, -14., 0.).into_any_element()
    }
}

/// The crew's picker: every model of every connection that is on, in one list, then the crew's
/// effort, then a way to switch the Team off.
#[derive(IntoElement)]
struct CrewPicker {
    menu: MenuData,
    /// The crew's model, as its pill names it.
    model: SharedString,
}

impl RenderOnce for CrewPicker {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let mut list = div()
            .flex()
            .flex_col()
            .child(div().px(px(14.)).pt(px(4.)).typography(Type::Label).text_color(theme.text_dim).child("The crew"))
            .child(
                div()
                    .px(px(14.))
                    .pb(px(6.))
                    .typography(Type::Small)
                    .text_color(theme.text_faint)
                    .child("Explore and General run on it. Review runs on the lead's model."),
            );
        let mut efforts = Vec::new();
        for (index, group) in self.menu.groups.iter().enumerate() {
            let crew_models = group.items.iter().any(|item| matches!(item.choice, Choice::Crew { .. }));
            if !crew_models {
                efforts.extend(group.items.iter().cloned());
                continue;
            }
            if let Some(title) = &group.title {
                list = list.child(div().px(px(14.)).pt(px(6.)).pb(px(2.)).typography(Type::Small).text_color(theme.text_faint).child(title.clone()));
            }
            for (number, item) in group.items.iter().enumerate() {
                let action = item.choice.action();
                list = list.child(
                    div()
                        .id(("crew-item", index * 1000 + number))
                        .mx(px(6.))
                        .px(px(8.))
                        .h(px(30.))
                        .rounded(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        .hover(|row| row.bg(theme.hover))
                        .when(item.selected, |row| row.bg(theme.accent_wash))
                        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
                        .when_some(item.brand, |row, brand| row.child(brand.view(px(14.), &theme)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .typography(if item.selected { Type::UiStrong } else { Type::Ui })
                                .text_color(if item.selected { theme.accent_text } else { theme.text })
                                .child(item.label.clone()),
                        )
                        .when_some(item.detail.clone(), |row, detail| row.child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(detail)))
                        .when(item.selected, |row| row.child(Icon::Check.view(px(13.), theme.accent_text))),
                );
            }
        }
        if !efforts.is_empty() {
            let mut chips = div().px(px(12.)).pt(px(4.)).flex().flex_wrap().gap(px(4.));
            for (number, item) in efforts.into_iter().enumerate() {
                let action = item.choice.action();
                chips = chips.child(
                    div()
                        .id(("crew-effort", number))
                        .h(px(24.))
                        .px(px(9.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .border_1()
                        .border_color(if item.selected { theme.accent_line } else { theme.border })
                        .when(item.selected, |chip| chip.bg(theme.accent_wash))
                        .hover(|chip| chip.bg(theme.hover))
                        .typography(Type::Small)
                        .text_color(if item.selected { theme.accent_text } else { theme.text_soft })
                        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
                        .child(item.label.clone()),
                );
            }
            list = list
                .child(div().mx(px(12.)).mt(px(6.)).h(px(1.)).bg(theme.border))
                .child(div().px(px(14.)).pt(px(8.)).typography(Type::Small).text_color(theme.text_faint).child(format!("{}'s reasoning", self.model)))
                .child(chips);
        }
        list = list.child(
            div()
                .id("crew-team-off")
                .mx(px(6.))
                .mt(px(8.))
                .px(px(8.))
                .h(px(30.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .hover(|row| row.bg(theme.hover))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(SetTeam { on: false }), cx))
                .child(Icon::Close.view(px(12.), theme.text_dim))
                .child(div().typography(Type::Ui).text_color(theme.text_soft).child("Switch Team off"))
                .tag("crew-team-off"),
        );
        div()
            .id("crew-menu")
            .occlude()
            .w(px(PICKER_WIDTH))
            .max_h(px(440.))
            .overflow_y_scroll()
            .overflow_x_hidden()
            .py(px(6.))
            .rounded(px(14.))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border_strong)
            .shadow_lg()
            .child(list)
            .tag("crew-menu")
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
                .when(!self.locked, |option| option.cursor_pointer().on_click(dispatch(SetAgentMode { mode: target })))
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

    /// A tab whose todo in hand draws 200 points, beside 400 of everything else at its fullest;
    /// each step after the todo's words takes 40 off.
    fn drawn(fit: Fit) -> f32 {
        let mut width = 400. + if fit.todo_words > 0. { TAB_GAP + fit.todo_words.min(200.) } else { 0. };
        for given_up in [!fit.background_words, !fit.crew_all, !fit.activity_words, !fit.change_lines, !fit.background, !fit.crew, !fit.changes] {
            if given_up {
                width -= 40.;
            }
        }
        width
    }

    #[test]
    fn the_tab_gives_up_words_in_order_until_it_fits_the_card() {
        assert_eq!(Fit::fitting(None, drawn), Fit::FULL, "before the card is measured");
        assert_eq!(Fit::fitting(Some(700.), drawn), Fit::FULL);
        // The todo's words shorten to what is left.
        let fit = Fit::fitting(Some(520.), drawn);
        assert_eq!(fit, Fit { todo_words: 520. - 400. - TAB_GAP, ..Fit::FULL });
        // Too little left for them to read: they go, then ⌘B's word, then the crew's queue.
        let fit = Fit::fitting(Some(330.), drawn);
        assert_eq!(fit.todo_words, 0.);
        assert!(!fit.background_words && !fit.crew_all && fit.activity_words && fit.change_lines);
        assert!(drawn(fit) <= 330.);
        // However narrow, the time, the todo count and the commands stay.
        let fit = Fit::fitting(Some(10.), drawn);
        assert!(!fit.background && !fit.crew && !fit.changes);
    }

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
