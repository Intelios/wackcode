//! The todo list, pinned above the composer while the agent keeps one. Ported from version 2's
//! `TodoPanel.tsx`.
//!
//! Tasks done in the current run stay in view; when the next run starts they fold behind
//! "+N done" ([`view`]), so the list shows what is left. `#id`s appear only once some task
//! depends on another, since that is the only time they mean anything. While a question card is
//! open the list folds to its one-line head, leaving the room to the question.

use std::collections::HashSet;

use gpui::{Hsla, SharedString, div, prelude::*, px};
use wack_core::todo::{Status, Task, TodoList};

use crate::{
    actions::ShowDoneTodos,
    icon::Icon,
    tags::Tag,
    theme::Theme,
    typography::{Type, Typography},
};

/// The ids of tasks done so far: what folds away when a run starts.
pub fn completed_ids(list: &TodoList) -> HashSet<u64> {
    list.visible().filter(|task| task.status == Status::Completed).map(|task| task.id).collect()
}

/// The list as the panel draws it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TodoView {
    /// The rows listed: every live task but those folded away.
    pub shown: Vec<Task>,
    /// Done tasks folded behind "+N done".
    pub hidden_done: usize,
    pub done: usize,
    pub total: usize,
    /// Some task is not done yet.
    pub active: bool,
    /// Some task depends on another, so ids are worth showing.
    pub show_ids: bool,
}

pub fn view(list: &TodoList, faded: &HashSet<u64>, show_done: bool) -> TodoView {
    let visible: Vec<&Task> = list.visible().collect();
    let done = visible.iter().filter(|task| task.status == Status::Completed).count();
    let folded = |task: &Task| task.status == Status::Completed && faded.contains(&task.id) && !show_done;
    TodoView {
        shown: visible.iter().filter(|task| !folded(task)).map(|task| (*task).clone()).collect(),
        hidden_done: visible.iter().filter(|task| folded(task)).count(),
        done,
        total: visible.len(),
        active: visible.iter().any(|task| task.status != Status::Completed),
        show_ids: visible.iter().any(|task| !task.blocked_by.is_empty()),
    }
}

/// The most rows shown before the list scrolls.
const MAX_ROWS_HEIGHT: f32 = 168.;

#[derive(Clone, Debug)]
pub struct TodoData {
    pub view: TodoView,
    /// The agent is working: the head's dot is lit while tasks remain.
    pub busy: bool,
    /// Only the head, while a question needs the room.
    pub folded: bool,
}

pub fn todo_panel(data: TodoData, theme: &Theme) -> impl IntoElement {
    let view = data.view;
    let live = data.busy && view.active;
    let head = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().size(px(6.)).rounded_full().bg(if live { theme.accent } else { theme.border_strong }))
        .child(div().typography(Type::UiStrong).text_color(theme.text_soft).child("Todos"))
        .child(div().typography(Type::Small).text_color(theme.text_faint).child(format!("{}/{}", view.done, view.total)))
        .when(view.hidden_done > 0 && !data.folded, |head| {
            head.child(
                div()
                    .id("todo-show-done")
                    .typography(Type::Small)
                    .text_color(theme.text_dim)
                    .hover(|word| word.text_color(theme.accent_text))
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowDoneTodos), cx))
                    .child(format!("+{} done", view.hidden_done))
                    .tag("todo-show-done"),
            )
        });

    let rows = (!data.folded).then(|| {
        div()
            .id("todo-rows")
            .max_h(px(MAX_ROWS_HEIGHT))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(3.))
            .children(view.shown.iter().map(|task| row(task, view.show_ids, theme)))
    });

    div()
        .mx(px(14.))
        .px(px(12.))
        .py(px(8.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .rounded(px(10.))
        .bg(theme.sidebar)
        .border_1()
        .border_color(theme.border)
        .child(head)
        .children(rows)
        .tag("todo-panel")
}

fn row(task: &Task, show_ids: bool, theme: &Theme) -> impl IntoElement {
    let done = task.status == Status::Completed;
    let working = task.status == Status::InProgress;
    let mark = div().flex_none().size(px(14.)).flex().items_center().justify_center().child(if done {
        Icon::Check.view(px(13.), theme.success).into_any_element()
    } else {
        let ring: Hsla = if working { theme.accent_text } else { theme.text_faint };
        div()
            .size(px(11.))
            .rounded_full()
            .border_1()
            .border_color(ring)
            .when(working, |mark| mark.child(div().m(px(2.)).size(px(5.)).rounded_full().bg(theme.accent_text)))
            .into_any_element()
    });
    let subject: SharedString = task.subject.clone().into();
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(mark)
        .when(show_ids, |row| row.child(div().flex_none().typography(Type::Mono).text_color(theme.text_faint).child(format!("#{}", task.id))))
        .child(
            div()
                .min_w_0()
                .truncate()
                .typography(Type::Ui)
                .text_color(if done { theme.text_dim } else if working { theme.text } else { theme.text_soft })
                .when(done, |text| text.line_through())
                .child(subject),
        )
        .when_some(task.active_form.clone().filter(|_| working), |row, active| {
            row.child(div().flex_none().typography(Type::Small).italic().text_color(theme.accent_text).child(format!("({active})")))
        })
        .when(!task.blocked_by.is_empty(), |row| {
            let ids = task.blocked_by.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(", ");
            row.child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .child(Icon::Link.view(px(11.), theme.text_faint))
                    .child(div().typography(Type::Small).text_color(theme.text_faint).child(ids)),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: u64, status: Status, blocked_by: Vec<u64>) -> Task {
        Task { id, subject: format!("Task {id}"), description: None, active_form: None, status, blocked_by }
    }

    fn list(tasks: Vec<Task>) -> TodoList {
        let next_id = tasks.len() as u64 + 1;
        TodoList { tasks, next_id }
    }

    #[test]
    fn tasks_done_before_this_run_fold_behind_a_count() {
        let tasks = list(vec![task(1, Status::Completed, vec![]), task(2, Status::InProgress, vec![]), task(3, Status::Pending, vec![])]);
        let faded = completed_ids(&tasks);
        // Task 2 finishes during the run: it stays in view until the next one.
        let later = list(vec![task(1, Status::Completed, vec![]), task(2, Status::Completed, vec![]), task(3, Status::Pending, vec![])]);
        let shown = view(&later, &faded, false);
        assert_eq!(shown.shown.iter().map(|task| task.id).collect::<Vec<_>>(), vec![2, 3]);
        assert_eq!((shown.hidden_done, shown.done, shown.total), (1, 2, 3));
        assert!(shown.active);
        assert_eq!(view(&later, &faded, true).shown.len(), 3, "+N done reveals them");
    }

    #[test]
    fn deleted_tasks_never_show_and_ids_appear_with_dependencies() {
        let tasks = list(vec![task(1, Status::Deleted, vec![]), task(2, Status::Pending, vec![])]);
        let plain = view(&tasks, &HashSet::new(), false);
        assert_eq!(plain.total, 1);
        assert!(!plain.show_ids);
        let linked = list(vec![task(1, Status::Pending, vec![]), task(2, Status::Pending, vec![1])]);
        assert!(view(&linked, &HashSet::new(), false).show_ids);
    }

    #[test]
    fn a_finished_list_is_no_longer_active() {
        let tasks = list(vec![task(1, Status::Completed, vec![])]);
        assert!(!view(&tasks, &HashSet::new(), false).active);
    }
}
