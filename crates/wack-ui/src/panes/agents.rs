//! The Agents pane: the chat's sub-agents, and the work of the one picked.
//!
//! - The pane only looks and talks: the agents belong to the chat's registry, so closing the
//!   pane never stops one. Stop here stops an agent for the user, and the field at the bottom sends
//!   it a message (one working reads it before its next step; one that ended works another round,
//!   whose report reaches the chat quietly).
//! - An agent's work is its own session file, read back through the same pure pipeline as the
//!   chat (`conversation.rs`, then `work.rs`), so it reads exactly like a chat. The file is read
//!   again only when it has grown, so a pane left open costs nothing while nothing happens.
//! - Every agent's work shows live as it saves each step: the pane redraws when the registry
//!   says one changed (`ChatPanes::refresh_agents`).
//! - An agent started from a copy of the conversation shows from its directive on, not the
//!   copied chat.
//! - In a Team chat a Team pill leads the strip and is what the pane shows until an agent is
//!   picked: the lead's and the crew's models, what each spent on the request, the lead's todo
//!   list and the request's workers. Its chat side ([`TeamView`]) is read from the app as the pane
//!   draws ([`TeamSource`]), so it is never stale and nothing pushes it.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
};

use gpui::{AnyElement, App, Context, Entity, FocusHandle, Hsla, ScrollHandle, SharedString, Subscription, Window, div, prelude::*, px, relative};
use gpui_tokio::Tokio;
use wack_core::{
    HistoryEntry, Message,
    agents::{AgentInfo, Agents, COPY_DIRECTIVE, Status, Tier},
};

use crate::{
    actions::StopAgent,
    conversation::Conversation,
    icon::Icon,
    tags::Tag,
    text_field::{TextField, TextFieldEvent},
    theme::Theme,
    tooltip::tip,
    transcript::{self, AgentState, Handlers, Item},
    typography::{Type, Typography},
    work::{self, ReplyCache},
};

/// The strip of agents scrolls past this, so many agents never crowd out the work.
const STRIP_HEIGHT: f32 = 96.;
const PILL_WIDTH: f32 = 220.;
/// The sentence that opens a copy's directive, where its own work starts.
fn copy_mark() -> &'static str {
    COPY_DIRECTIVE.split_once(". ").map_or(COPY_DIRECTIVE, |(first, _)| first)
}

/// What a Team overview shows of the chat beside its agents.
#[derive(Clone, Debug, Default)]
pub struct TeamView {
    /// The lead's model, and the crew's.
    pub lead: SharedString,
    pub crew: SharedString,
    /// Every token the lead's own turns used since the user's latest message.
    pub lead_tokens: u64,
    /// The lead's todo list: the plan it works to.
    pub todos: Vec<wack_core::todo::Task>,
}

/// Reads a chat's [`TeamView`] while its Team is on; `None` while it is off.
pub type TeamSource = Rc<dyn Fn(&App) -> Option<TeamView>>;

pub struct AgentsPane {
    agents: Agents,
    cwd: PathBuf,
    /// The agent whose work shows; the newest when none was picked.
    pub selected: Option<String>,
    /// The chat's Team, when it has one to show.
    team: Option<TeamSource>,
    /// The Team overview was picked over any agent.
    pub on_team: bool,
    team_scroll: ScrollHandle,
    pub focus: FocusHandle,
    /// Each agent's work as last read, with the size of its file then.
    read: HashMap<String, (u64, Conversation)>,
    replies: HashMap<String, ReplyCache>,
    /// Rows the user opened, and work logs they folded.
    open: HashSet<String>,
    folded: HashSet<String>,
    field: Entity<TextField>,
    /// What the last message sent came to: a word, or why it couldn't go.
    sent: Option<(SharedString, bool)>,
    /// The work's scroll: it follows new steps while the reader is at the bottom, and starts at
    /// the bottom (the latest step, or the report) when an agent is picked.
    scroll: ScrollHandle,
    /// Where each opened thought is scrolled, and whether it follows its newest words.
    follows: crate::transcript::Follows,
    _field: Subscription,
}

impl AgentsPane {
    pub fn new(agents: Agents, cwd: PathBuf, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| TextField::new(Type::Ui, "Message this agent", cx).multiline(4));
        let subscription = cx.subscribe(&field, |pane, _, event: &TextFieldEvent, cx| {
            if matches!(event, TextFieldEvent::Enter | TextFieldEvent::SendNow) {
                pane.send(cx);
            }
        });
        Self {
            agents,
            cwd,
            selected: None,
            team: None,
            on_team: false,
            team_scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            read: HashMap::new(),
            replies: HashMap::new(),
            open: HashSet::new(),
            folded: HashSet::new(),
            field,
            sent: None,
            scroll: ScrollHandle::new(),
            follows: Default::default(),
            _field: subscription,
        }
    }

    pub fn select(&mut self, id: String, cx: &mut Context<Self>) {
        if self.selected.as_ref() != Some(&id) || self.on_team {
            self.sent = None;
            self.scroll.scroll_to_bottom();
        }
        self.selected = Some(id);
        self.on_team = false;
        cx.notify();
    }

    /// Shows the Team overview, while the chat has a Team.
    pub fn select_team(&mut self, cx: &mut Context<Self>) {
        self.on_team = true;
        cx.notify();
    }

    pub fn set_team_source(&mut self, source: Option<TeamSource>, cx: &mut Context<Self>) {
        self.team = source;
        cx.notify();
    }

    /// How many are working, for the tab's count.
    pub fn working(&self) -> usize {
        self.agents.active().len()
    }

    fn picked(&self, list: &[AgentInfo]) -> Option<AgentInfo> {
        self.selected
            .as_ref()
            .and_then(|id| list.iter().find(|info| &info.id == id))
            .or_else(|| list.iter().rev().find(|info| info.status.active()))
            .or_else(|| list.last())
            .cloned()
    }

    /// Sends the field's text to the agent picked.
    fn send(&mut self, cx: &mut Context<Self>) {
        let text = self.field.read(cx).text().trim().to_string();
        let list = self.agents.list();
        let Some(info) = self.picked(&list) else { return };
        if text.is_empty() {
            return;
        }
        let agents = self.agents.clone();
        let id = info.id.clone();
        let work = Tokio::spawn(cx, async move { agents.message_from_user(&id, &text).await });
        cx.spawn(async move |pane, cx| {
            let result = work.await.unwrap_or_else(|_| Err("The message could not be sent.".into()));
            let _ = pane.update(cx, |pane, cx| {
                match result {
                    Ok(_) => {
                        pane.field.update(cx, |field, cx| field.set_text("", cx));
                        pane.sent = Some(("Sent.".into(), false));
                    }
                    Err(message) => pane.sent = Some((message.into(), true)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The agent's work as transcript items, reading its file again only when it has grown.
    fn items(&mut self, info: &AgentInfo) -> Vec<Item> {
        let size = std::fs::metadata(&info.session).map(|meta| meta.len()).unwrap_or(0);
        let stale = self.read.get(&info.id).is_none_or(|(read, _)| *read != size);
        if stale {
            // Near the bottom (or nothing to scroll yet): keep up with what it does next.
            let (offset, max) = (self.scroll.offset().y, self.scroll.max_offset().y);
            if max - (-offset) < px(48.) {
                self.scroll.scroll_to_bottom();
            }
            let history = wack_core::history(&info.session).unwrap_or_default();
            let mut conversation = Conversation::from_history(&own_work(history));
            // A turn still being written shows as working, as the chat's own does.
            if info.status.active()
                && let Some(crate::conversation::Entry::Turn(turn)) = conversation.entries.last_mut()
            {
                turn.live = true;
            }
            self.read.insert(info.id.clone(), (size, conversation));
        }
        let Some((_, conversation)) = self.read.get(&info.id) else { return Vec::new() };
        let cache = self.replies.entry(info.id.clone()).or_default();
        let mut items = work::items(conversation, &self.cwd, &self.open, wack_core::tasks::now_millis(), cache);
        // Sub-agent views show every work log open; omit the per-turn Changes card.
        items.retain(|item| !matches!(item, Item::Changes(_)));
        // Its work is the point here: logs show open unless folded.
        for item in &mut items {
            if let Item::Work(log) = item {
                log.open = log.live || !self.folded.contains(log.key.as_ref());
            }
        }
        items
    }

    fn toggle(&mut self, key: SharedString, cx: &mut Context<Self>) {
        let key = key.to_string();
        if key.starts_with("turn-") && !key[5..].contains('-') {
            if !self.folded.remove(&key) {
                self.folded.insert(key);
            }
        } else if !self.open.remove(&key) {
            self.open.insert(key);
        }
        cx.notify();
    }
}

/// A copy's history from its directive on, the directive standing as its first message; anyone
/// else's whole.
fn own_work(history: Vec<HistoryEntry>) -> Vec<HistoryEntry> {
    let start = history.iter().rposition(|entry| matches!(entry, HistoryEntry::Message { message, .. } if message.text().contains(copy_mark())));
    let Some(start) = start else { return history };
    let mut own: Vec<HistoryEntry> = history.into_iter().skip(start).collect();
    if let Some(HistoryEntry::Message { message, .. }) = own.first_mut() {
        let text = message.text();
        let directive = text.split_once("Your task:").map(|(_, task)| task.trim().to_string()).unwrap_or(text);
        *message = Message::user(directive);
    }
    own
}

/// How an agent stands, as the row in the work log says it.
fn state(info: &AgentInfo) -> AgentState {
    match &info.status {
        Status::Queued => AgentState::Queued,
        Status::Running if info.background => AgentState::Background,
        Status::Running => AgentState::Working,
        Status::Done => AgentState::Done,
        Status::Failed { .. } => AgentState::Failed,
        Status::Stopped { .. } => AgentState::Stopped,
        Status::Interrupted => AgentState::Interrupted,
    }
}

fn role(info: &AgentInfo) -> String {
    let mut chars = info.role.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn took(info: &AgentInfo, now: u64) -> String {
    work::duration(info.ended_at.unwrap_or(now).saturating_sub(info.started_at))
}

impl Render for AgentsPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let now = wack_core::tasks::now_millis();
        let list = self.agents.list();
        let team = self.team.clone().and_then(|source| source(cx));
        // A Team shows its overview until an agent is picked.
        let overview = team.is_some() && (self.on_team || self.selected.is_none());
        let picked = if overview { None } else { self.picked(&list) };
        if !overview && picked.is_none() {
            return div()
                .track_focus(&self.focus)
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .child(Icon::Duck.view(px(18.), theme.text_faint))
                .child(div().typography(Type::Ui).text_color(theme.text_dim).child("No agents yet"))
                .child(
                    div()
                        .px(px(24.))
                        .text_center()
                        .typography(Type::Small)
                        .text_color(theme.text_faint)
                        .child("When the agent hands work to a sub-agent, its work shows here."),
                )
                .tag("agents-pane")
                .into_any_element();
        }

        // A strip of pills, one per agent in the order they started, so the work below keeps the
        // pane's whole width. A Team's own pill leads it.
        let mut rows = div().id("agents-list").flex_none().max_h(px(STRIP_HEIGHT)).overflow_y_scroll().px(px(10.)).py(px(8.)).border_b_1().border_color(theme.border).flex().flex_wrap().gap(px(6.));
        if team.is_some() {
            let pane = cx.entity();
            let working = list.iter().any(|info| info.status == Status::Running);
            rows = rows.child(
                div()
                    .id("team-pill")
                    .h(px(26.))
                    .px(px(10.))
                    .rounded_full()
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .border_1()
                    .border_color(if overview { theme.border_strong } else { theme.border })
                    .when(overview, |pill| pill.bg(theme.hover))
                    .hover(|pill| pill.bg(theme.hover))
                    .tooltip(tip("The team: its plan, its workers and what each model spent"))
                    .on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.select_team(cx)))
                    .child(Icon::Duck.view(px(11.), if working { theme.accent_text } else { theme.text_dim }))
                    .child(div().typography(Type::Small).text_color(if overview { theme.text } else { theme.text_soft }).child("Team"))
                    .tag("team-pill"),
            );
        }
        for info in &list {
            let picked = picked.as_ref().is_some_and(|picked| picked.id == info.id);
            let (_, colour) = transcript::agent_state_word(state(info), &theme);
            let id = info.id.clone();
            let pane = cx.entity();
            rows = rows.child(
                div()
                    .id(SharedString::from(format!("agent-row-{}", info.id)))
                    .max_w(px(PILL_WIDTH))
                    .h(px(26.))
                    .px(px(10.))
                    .rounded_full()
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .border_1()
                    .border_color(if picked { theme.border_strong } else { theme.border })
                    .when(picked, |pill| pill.bg(theme.hover))
                    .hover(|pill| pill.bg(theme.hover))
                    .tooltip(tip(format!("{} · {} · {}", info.id, role(info), info.description)))
                    .on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.select(id.clone(), cx)))
                    .child(div().flex_none().size(px(6.)).rounded_full().bg(colour))
                    .child(div().flex_none().typography(Type::Small).text_color(if picked { theme.text } else { theme.text_soft }).child(role(info)))
                    .child(div().min_w_0().truncate().typography(Type::Small).text_color(theme.text_dim).child(info.description.clone()))
                    .tag("agent-pill"),
            );
        }

        let Some(selected) = picked else {
            let team = team.unwrap_or_default();
            let this = self.agents.this_message();
            return div()
                .track_focus(&self.focus)
                .size_full()
                .flex()
                .flex_col()
                .child(rows)
                .child(team_overview(&team, &list, &this, now, &self.team_scroll, &theme))
                .tag("agents-pane")
                .into_any_element();
        };
        let working = selected.status.active();
        let running = selected.status == Status::Running;
        let (word, colour) = transcript::agent_state_word(state(&selected), &theme);
        let stop_id = SharedString::from(selected.id.clone());
        let header = div()
            .flex_none()
            .px(px(14.))
            .py(px(10.))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_none()
                            .size(px(24.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(if running { theme.accent_wash } else { theme.hover })
                            .child(if running {
                                crate::motion::paddling(div().child(Icon::Duck.view(px(13.), theme.accent_text))).into_any_element()
                            } else {
                                Icon::Duck.view(px(13.), theme.text_dim).into_any_element()
                            }),
                    )
                    .child(div().flex_1().min_w_0().truncate().typography(Type::UiStrong).text_color(theme.text).child(role(&selected)))
                    .child(div().flex_none().typography(Type::Small).text_color(colour).child(word))
                    .when(working, |line| {
                        line.child(
                            div()
                                .id("stop-agent")
                                .flex_none()
                                .h(px(22.))
                                .px(px(8.))
                                .rounded_full()
                                .bg(Hsla { a: 0.14, ..theme.danger })
                                .cursor_pointer()
                                .hover(|button| button.bg(Hsla { a: 0.22, ..theme.danger }))
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .tooltip(tip("Stop this agent; the chat is told"))
                                .on_click(move |_, window, cx| window.dispatch_action(Box::new(StopAgent { id: stop_id.clone() }), cx))
                                .child(Icon::Stop.view(px(10.), theme.danger))
                                .child(div().typography(Type::Small).text_color(theme.danger).child("Stop"))
                                .tag("stop-agent"),
                        )
                    }),
            )
            // The task wraps rather than cut off: it is what the agent was asked.
            .child(div().typography(Type::Ui).text_color(theme.text_soft).child(selected.description.clone()))
            .child(div().truncate().typography(Type::Small).text_color(theme.text_faint).child(format!(
                "{} · {} · {} tool{} · {} · {}{}",
                selected.id,
                selected.model,
                selected.tool_calls,
                if selected.tool_calls == 1 { "" } else { "s" },
                took(&selected, now),
                if selected.edits { "edits files" } else { "no edits" },
                if selected.round > 1 { format!(" · round {}", selected.round) } else { String::new() }
            )));

        let items = self.items(&selected);
        let pane = cx.entity();
        let handlers = Handlers {
            toggle: std::rc::Rc::new(move |key, _, cx| pane.update(cx, |pane, cx| pane.toggle(key, cx))),
            open_link: std::rc::Rc::new(|url, _, cx| {
                if url.starts_with("https://") || url.starts_with("http://") {
                    cx.open_url(&url);
                }
            }),
            still: cx.reduce_motion(),
            // A sub-agent's messages are its parent's to rewind, never the user's: Copy only.
            prompts: Default::default(),
            clock: true,
            folds: Default::default(),
            follows: self.follows.clone(),
        };
        let mut work = div().px(px(16.)).py(px(14.)).flex().flex_col();
        if items.is_empty() {
            work = work.child(div().typography(Type::Small).text_color(theme.text_faint).child(if working { "Starting…" } else { "Nothing was saved of its work." }));
        }
        for (index, item) in items.iter().enumerate() {
            let previous = index.checked_sub(1).and_then(|index| items.get(index));
            work = work.child(transcript::item(item, previous, &theme, &handlers));
        }

        let note = self.sent.clone();
        let footer = div()
            .flex_none()
            .px(px(12.))
            .py(px(8.))
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .min_h(px(34.))
                    .px(px(12.))
                    .py(px(7.))
                    .rounded(px(17.))
                    .bg(theme.hover)
                    .border_1()
                    .border_color(theme.border)
                    .child(self.field.clone()),
            )
            .child(div().px(px(6.)).typography(Type::Small).text_color(match &note {
                Some((_, true)) => theme.danger,
                _ => theme.text_faint,
            }).child(match note {
                Some((text, _)) => text,
                None if working => "It reads your message before its next step.".into(),
                None => "It works another round with what it learned; the chat hears its report.".into(),
            }));

        let detail = div()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .child(header)
            .child(div().id("agent-work").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.scroll).child(work))
            .child(footer);

        div().track_focus(&self.focus).size_full().flex().flex_col().child(rows).child(detail).tag("agents-pane").into_any_element()
    }
}

/// Tokens a Team spent on the request: the lead's model (its own turns and the workers it put
/// on its model) and the crew's.
fn team_spend(team: &TeamView, this: &[AgentInfo]) -> (u64, u64) {
    let on = |lead: bool| this.iter().filter(|info| (info.tier == Some(Tier::Lead)) == lead).map(|info| work::spent(&info.spent)).sum::<u64>();
    (team.lead_tokens + on(true), on(false))
}

/// A Team at a glance: who leads and who types, what each spent, the plan, and the request's
/// workers as the work log draws them.
fn team_overview(team: &TeamView, list: &[AgentInfo], this: &[AgentInfo], now: u64, scroll: &ScrollHandle, theme: &Theme) -> AnyElement {
    let queued = list.iter().filter(|info| info.status == Status::Queued).count();
    let working = list.iter().filter(|info| info.status == Status::Running).count();
    let done = this.iter().filter(|info| info.status == Status::Done).count();
    let counts = crate::composer::CrewAtWork { working, queued, done }.words();
    let head = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_none()
                .size(px(24.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(if working > 0 { theme.accent_wash } else { theme.hover })
                .child(if working > 0 {
                    crate::motion::paddling(div().child(Icon::Duck.view(px(13.), theme.accent_text))).into_any_element()
                } else {
                    Icon::Duck.view(px(13.), theme.text_dim).into_any_element()
                }),
        )
        .child(div().flex_1().typography(Type::UiStrong).text_color(theme.text).child("Team"))
        .child(div().flex_none().typography(Type::Small).text_color(theme.text_dim).child(if counts.is_empty() { "No workers yet".to_string() } else { counts }));
    let slot = |label: &'static str, model: SharedString| {
        div()
            .flex_1()
            .min_w_0()
            .px(px(12.))
            .py(px(7.))
            .rounded(px(12.))
            .bg(theme.hover)
            .flex()
            .flex_col()
            .child(div().typography(Type::Small).text_color(theme.text_faint).child(label))
            .child(div().truncate().typography(Type::Ui).text_color(theme.text).child(model))
    };
    let models = div().flex().gap(px(8.)).child(slot("Lead", team.lead.clone())).child(slot("Crew", team.crew.clone())).tag("team-models");

    let (lead, crew) = team_spend(team, this);
    let spend = (lead + crew > 0).then(|| {
        let share = lead as f32 / (lead + crew) as f32;
        let key = |colour: Hsla, words: String| {
            div().flex().items_center().gap(px(6.)).child(div().flex_none().size(px(7.)).rounded_full().bg(colour)).child(div().typography(Type::Small).text_color(theme.text_dim).child(words))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .h(px(6.))
                    .w_full()
                    .rounded_full()
                    .overflow_hidden()
                    .flex()
                    .gap(px(2.))
                    .child(div().flex_none().h_full().w(relative(share)).bg(theme.text_soft))
                    .child(div().flex_1().h_full().bg(theme.border_strong)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(14.))
                    .child(key(theme.text_soft, format!("Lead's model {}", work::tokens(lead))))
                    .child(key(theme.border_strong, format!("Crew {}", work::tokens(crew))))
                    .child(div().flex_1())
                    .child(div().typography(Type::Small).text_color(theme.text_faint).child("on this request")),
            )
            .tag("team-spend")
    });

    let section = |title: &'static str, count: Option<String>| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().typography(Type::UiStrong).text_color(theme.text_soft).child(title))
            .children(count.map(|count| div().typography(Type::Small).text_color(theme.text_faint).child(count)))
    };
    let plan = (!team.todos.is_empty()).then(|| {
        let finished = team.todos.iter().filter(|task| task.status == wack_core::todo::Status::Completed).count();
        let ids = team.todos.iter().any(|task| !task.blocked_by.is_empty());
        div()
            .px(px(14.))
            .pt(px(14.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(section("Plan", Some(format!("{finished}/{}", team.todos.len()))))
            .children(team.todos.iter().map(|task| crate::todo_panel::row(task, ids, theme)))
            .tag("team-plan")
    });

    let earlier = list.len().saturating_sub(this.len());
    let roster = div()
        .px(px(8.))
        .pt(px(14.))
        .pb(px(14.))
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(div().px(px(6.)).pb(px(4.)).child(section("On this request", (!this.is_empty()).then(|| this.len().to_string()))))
        .when(this.is_empty(), |roster| {
            roster.child(div().px(px(6.)).typography(Type::Small).text_color(theme.text_faint).child(
                "No workers yet. The lead sizes the work first: a small change it makes itself, and wide reading or separable pieces go to the crew.",
            ))
        })
        .children(this.iter().map(|info| {
            let key = SharedString::from(format!("team-{}", info.id));
            transcript::agent_row(&key, &work::agent_line(info, now), theme)
        }))
        .when(earlier > 0, |roster| {
            roster.child(div().px(px(6.)).pt(px(6.)).typography(Type::Small).text_color(theme.text_faint).child(format!(
                "{earlier} earlier {}, in the strip above.",
                if earlier == 1 { "agent" } else { "agents" }
            )))
        })
        .tag("team-roster");

    div()
        .id("team-overview")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .child(div().px(px(14.)).py(px(12.)).border_b_1().border_color(theme.border).flex().flex_col().gap(px(10.)).child(head).child(models).children(spend))
        .children(plan)
        .child(roster)
        .tag("team-overview")
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wack_core::Role;

    #[test]
    fn the_lead_s_model_counts_its_own_turns_and_its_workers() {
        let worker = |tier: Option<Tier>, input: u64| AgentInfo {
            tier,
            spent: wack_core::Usage { input, output: 10, cache_read: 0, cache_write: 0 },
            ..crate::fixtures::team_agents(1_000_000, false).remove(0)
        };
        let team = TeamView { lead_tokens: 1_000, ..TeamView::default() };
        let this = [worker(Some(Tier::Lead), 90), worker(Some(Tier::Crew), 490), worker(None, 190)];
        assert_eq!(team_spend(&team, &this), (1_100, 700), "a worker saved before tiers counts as crew");
    }

    #[test]
    fn a_copy_shows_from_its_directive_on() {
        let message = |role: Role, text: &str| {
            let mut message = Message::user(text);
            message.role = role;
            HistoryEntry::Message { message, usage: None, at: None, command: None, thought_ms: None }
        };
        let history = vec![
            message(Role::User, "The parent's question"),
            message(Role::Assistant, "The parent's answer"),
            message(Role::User, &format!("{COPY_DIRECTIVE}\n\nRename it.")),
            message(Role::Assistant, "Renamed."),
        ];
        let own = own_work(history);
        let texts: Vec<String> = own
            .iter()
            .map(|entry| match entry {
                HistoryEntry::Message { message, .. } => message.text(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(texts, ["Rename it.", "Renamed."]);
        let whole = own_work(vec![message(Role::User, "Brief"), message(Role::Assistant, "Report")]);
        assert_eq!(whole.len(), 2);
    }
}
