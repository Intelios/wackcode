//! The Tasks pane: the commands the agent started in this chat, and the output of the one picked.
//!
//! The pane only looks: the commands belong to the chat's [`Tasks`], so closing the pane never
//! stops one, and Stop here stops a command for the user (the agent is told). Output is the end
//! of what the command printed, as plain text with terminal colour codes removed: these commands
//! write to a pipe, not a terminal, so there is no screen to draw.

use gpui::{Context, FocusHandle, Hsla, SharedString, Window, div, prelude::*, px};
use wack_core::tasks::{Status, StoppedBy, TaskInfo, Tasks, elapsed};

use crate::{
    actions::StopTask,
    icon::Icon,
    tags::Tag,
    theme::Theme,
    tooltip::tip,
    typography::{Type, Typography},
};

/// Lines of output the pane shows.
const OUTPUT_LINES: usize = 400;
const LIST_WIDTH: f32 = 220.;

pub struct TasksPane {
    tasks: Tasks,
    /// The task whose output shows; the newest when none was picked.
    pub selected: Option<String>,
    pub focus: FocusHandle,
}

impl TasksPane {
    pub fn new(tasks: Tasks, cx: &mut Context<Self>) -> Self {
        Self { tasks, selected: None, focus: cx.focus_handle() }
    }

    pub fn select(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected = Some(id);
        cx.notify();
    }

    /// How many are running, for the tab's dot.
    pub fn running(&self) -> usize {
        self.tasks.list().iter().filter(|info| info.in_background() && info.status.running()).count()
    }
}

/// The colour that says how a task stands: running in the accent, then success or danger. A
/// stop is neither.
pub fn status_colour(status: &Status, theme: &Theme) -> Hsla {
    match status {
        Status::Running => theme.accent_text,
        Status::Finished => theme.success,
        Status::Failed { .. } | Status::Stopped { by: StoppedBy::Chatty } => theme.danger,
        Status::Stopped { .. } => theme.text_faint,
    }
}

/// "running · 2m10s", "exit 1 · 42s", "stopped".
pub fn status_words(info: &TaskInfo, now: u64) -> String {
    let took = elapsed(std::time::Duration::from_millis(info.ended_at.unwrap_or(now).saturating_sub(info.started_at)));
    match &info.status {
        Status::Running => format!("running · {took}"),
        Status::Finished => format!("done · {took}"),
        Status::Failed { code: Some(code), .. } => format!("exit {code} · {took}"),
        Status::Failed { .. } => format!("killed · {took}"),
        Status::Stopped { by: StoppedBy::Quit } => "stopped on quit".into(),
        Status::Stopped { by: StoppedBy::Expired } => "time limit".into(),
        Status::Stopped { by: StoppedBy::Chatty } => "too noisy".into(),
        Status::Stopped { .. } => format!("stopped · {took}"),
    }
}

/// Text without terminal escape sequences (colours, cursor moves) or carriage-return redraws.
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                // CSI: parameters, then one final letter.
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ESC \.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && chars.peek() == Some(&'\\')) {
                            chars.next_if_eq(&'\\');
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            }
            continue;
        }
        out.push(c);
    }
    // A progress bar redraws its line with \r: keep what it says last.
    out.lines().map(|line| line.rsplit('\r').find(|part| !part.is_empty()).unwrap_or("")).collect::<Vec<_>>().join("\n")
}

impl Render for TasksPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let now = wack_core::tasks::now_millis();
        let list: Vec<_> = self.tasks.list().into_iter().filter(|info| info.in_background()).collect();
        let selected = self
            .selected
            .as_ref()
            .and_then(|id| list.iter().find(|info| &info.id == id))
            .or_else(|| list.iter().rev().find(|info| info.status.running()))
            .or_else(|| list.last())
            .cloned();

        if list.is_empty() {
            return div()
                .track_focus(&self.focus)
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .child(Icon::Terminal.view(px(18.), theme.text_faint))
                .child(div().typography(Type::Ui).text_color(theme.text_dim).child("No background commands yet"))
                .child(
                    div()
                        .typography(Type::Small)
                        .text_color(theme.text_faint)
                        .child("Servers, watchers and long builds the agent starts show here."),
                )
                .tag("tasks-pane")
                .into_any_element();
        }

        let mut rows = div().id("tasks-list").flex_none().w(px(LIST_WIDTH)).h_full().overflow_y_scroll().border_r_1().border_color(theme.border).py(px(4.));
        // Running first, then the rest newest first.
        let ordered = list.iter().rev().filter(|info| info.status.running()).chain(list.iter().rev().filter(|info| !info.status.running()));
        for info in ordered {
            let picked = selected.as_ref().is_some_and(|selected| selected.id == info.id);
            let id = info.id.clone();
            let pane = cx.entity();
            rows = rows.child(
                div()
                    .id(SharedString::from(format!("task-row-{}", info.id)))
                    .mx(px(4.))
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(picked, |row| row.bg(theme.hover))
                    .hover(|row| row.bg(theme.hover))
                    .on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.select(id.clone(), cx)))
                    .child(div().flex_none().size(px(6.)).rounded_full().bg(status_colour(&info.status, &theme)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().truncate().typography(Type::Ui).text_color(theme.text_soft).child(info.label().to_string()))
                            .child(div().truncate().typography(Type::Small).text_color(theme.text_faint).child(status_words(info, now))),
                    ),
            );
        }

        let output = selected.as_ref().map(|info| {
            let text = self.tasks.get(&info.id).map(|task| plain(&task.output())).unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();
            let shown = lines[lines.len().saturating_sub(OUTPUT_LINES)..].join("\n");
            (info.clone(), shown)
        });

        let detail = match output {
            None => div().flex_1(),
            Some((info, shown)) => {
                let running = info.status.running();
                let id = SharedString::from(info.id.clone());
                let header = div()
                    .flex_none()
                    .h(px(32.))
                    .px(px(12.))
                    .border_b_1()
                    .border_color(theme.border)
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().flex_1().min_w_0().truncate().typography(Type::Mono).text_color(theme.text_soft).child(format!("$ {}", info.command.trim())))
                    .child(div().flex_none().typography(Type::Small).text_color(status_colour(&info.status, &theme)).child(status_words(&info, now)))
                    .when(running, |header| {
                        header.child(
                            div()
                                .id("stop-task-pane")
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
                                .tooltip(tip("Stop this command; the agent is told"))
                                .on_click(move |_, window, cx| window.dispatch_action(Box::new(StopTask { id: id.clone() }), cx))
                                .child(Icon::Stop.view(px(10.), theme.danger))
                                .child(div().typography(Type::Small).text_color(theme.danger).child("Stop"))
                                .tag("stop-task-pane"),
                        )
                    });
                let body = if shown.trim().is_empty() {
                    div().p(px(12.)).typography(Type::Small).text_color(theme.text_faint).child(if running { "No output yet." } else { "It printed nothing." })
                } else {
                    div().p(px(12.)).typography(Type::Mono).text_color(theme.text_body).whitespace_normal().child(shown)
                };
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(div().id("task-output").flex_1().min_h_0().overflow_y_scroll().child(body))
            }
        };

        div().track_focus(&self.focus).size_full().flex().child(rows).child(detail).tag("tasks-pane").into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_drops_escapes_and_keeps_the_last_redraw() {
        assert_eq!(plain("\u{1b}[32mok\u{1b}[0m done"), "ok done");
        assert_eq!(plain("\u{1b}]0;title\u{7}hi"), "hi");
        assert_eq!(plain("10%\r50%\r100%\nnext"), "100%\nnext");
    }
}
