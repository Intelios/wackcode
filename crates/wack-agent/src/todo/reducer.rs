//! The todo list's rules: `(list, action, input) → (list, what happened)`.
//!
//! Ported from `@juicesharp/rpiv-todo` v2.11.0 (MIT) by way of version 2. Validation is in-band:
//! a refused change never alters the list, and the reason travels in [`Op::Error`].
//!
//! - Status is a four-state machine: pending ⇄ in_progress → completed, and anything → deleted.
//!   `completed` never goes back; `deleted` is a tombstone and terminal.
//! - `blockedBy` edges may not point at a missing or deleted task, at the task itself, or close
//!   a cycle.
//! - An update that changes nothing says so, or a model re-issues it in a loop.

use std::collections::{HashMap, HashSet};

use super::{Input, Status, Task, TodoList};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Create,
    Update,
    List,
    Get,
    Delete,
    Clear,
}

impl Action {
    pub const NAMES: [&str; 6] = ["create", "update", "list", "get", "delete", "clear"];

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "create" => Self::Create,
            "update" => Self::Update,
            "list" => Self::List,
            "get" => Self::Get,
            "delete" => Self::Delete,
            "clear" => Self::Clear,
            _ => return None,
        })
    }
}

/// What a call did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Create { id: u64 },
    Update { id: u64, from: Status, to: Status, changed: bool },
    Delete { id: u64, subject: String },
    List { status: Option<Status>, include_deleted: bool },
    Get { task: Task },
    Clear { count: usize },
    Error(String),
}

pub fn transition_valid(from: Status, to: Status) -> bool {
    use Status::{Completed, Deleted, InProgress, Pending};
    from == to
        || matches!(
            (from, to),
            (Pending, InProgress | Completed | Deleted) | (InProgress, Pending | Completed | Deleted) | (Completed, Deleted)
        )
}

/// Whether giving `task` the dependencies `blocked_by` would close a cycle.
pub fn detect_cycle(tasks: &[Task], task: u64, blocked_by: &[u64]) -> bool {
    let mut edges: HashMap<u64, Vec<u64>> = HashMap::new();
    for entry in tasks {
        let mut dependencies = entry.blocked_by.clone();
        if entry.id == task {
            for dependency in blocked_by {
                if !dependencies.contains(dependency) {
                    dependencies.push(*dependency);
                }
            }
        }
        edges.insert(entry.id, dependencies);
    }
    fn cycles(node: u64, edges: &HashMap<u64, Vec<u64>>, visiting: &mut HashSet<u64>, visited: &mut HashSet<u64>) -> bool {
        if visiting.contains(&node) {
            return true;
        }
        if !visited.insert(node) {
            return false;
        }
        visiting.insert(node);
        let found = edges.get(&node).is_some_and(|next| next.iter().any(|next| cycles(*next, edges, visiting, visited)));
        visiting.remove(&node);
        found
    }
    let (mut visiting, mut visited) = (HashSet::new(), HashSet::new());
    tasks.iter().any(|entry| cycles(entry.id, &edges, &mut visiting, &mut visited))
}

/// The tasks each task blocks: `blockedBy` turned around.
pub fn blocks(tasks: &[Task]) -> HashMap<u64, Vec<u64>> {
    let mut blocks: HashMap<u64, Vec<u64>> = HashMap::new();
    for task in tasks {
        for dependency in &task.blocked_by {
            blocks.entry(*dependency).or_default().push(task.id);
        }
    }
    blocks
}

fn error(message: impl Into<String>) -> Op {
    Op::Error(message.into())
}

/// Applies one call. On [`Op::Error`] the list is unchanged.
pub fn apply(list: &mut TodoList, action: Action, input: &Input) -> Op {
    match action {
        Action::Create => {
            let Some(subject) = input.subject.as_ref().filter(|subject| !subject.trim().is_empty()) else {
                return error("subject required for create");
            };
            let blocked_by = input.blocked_by.clone().unwrap_or_default();
            for dependency in &blocked_by {
                match list.task(*dependency) {
                    None => return error(format!("blockedBy: #{dependency} not found")),
                    Some(task) if task.status == Status::Deleted => return error(format!("blockedBy: #{dependency} is deleted")),
                    Some(_) => {}
                }
            }
            let id = list.next_id;
            list.tasks.push(Task {
                id,
                subject: subject.clone(),
                description: input.description.clone().filter(|text| !text.is_empty()),
                active_form: input.active_form.clone().filter(|text| !text.is_empty()),
                status: Status::Pending,
                blocked_by,
            });
            list.next_id += 1;
            Op::Create { id }
        }
        Action::Update => {
            let Some(id) = input.id else {
                return error("id required for update");
            };
            let Some(current) = list.task(id).cloned() else {
                return error(format!("#{id} not found"));
            };
            let add = input.add_blocked_by.clone().unwrap_or_default();
            let remove = input.remove_blocked_by.clone().unwrap_or_default();
            let has_mutation = input.subject.is_some()
                || input.description.is_some()
                || input.active_form.is_some()
                || input.status.is_some()
                || !add.is_empty()
                || !remove.is_empty();
            if !has_mutation {
                return error("update requires at least one mutable field: subject, description, activeForm, status, addBlockedBy, or removeBlockedBy");
            }
            let mut updated = current.clone();
            if let Some(status) = input.status {
                if !transition_valid(current.status, status) {
                    return error(format!("illegal transition {} → {}", current.status.as_str(), status.as_str()));
                }
                updated.status = status;
            }
            updated.blocked_by.retain(|dependency| !remove.contains(dependency));
            if !add.is_empty() {
                for dependency in &add {
                    if *dependency == id {
                        return error(format!("cannot block #{id} on itself"));
                    }
                    match list.task(*dependency) {
                        None => return error(format!("addBlockedBy: #{dependency} not found")),
                        Some(task) if task.status == Status::Deleted => return error(format!("addBlockedBy: #{dependency} is deleted")),
                        Some(_) => {}
                    }
                    if !updated.blocked_by.contains(dependency) {
                        updated.blocked_by.push(*dependency);
                    }
                }
                if detect_cycle(&list.tasks, id, &updated.blocked_by) {
                    return error("addBlockedBy would create a cycle in the blockedBy graph");
                }
            }
            if let Some(subject) = &input.subject {
                updated.subject = subject.clone();
            }
            if let Some(description) = &input.description {
                updated.description = Some(description.clone());
            }
            if let Some(active_form) = &input.active_form {
                updated.active_form = Some(active_form.clone());
            }
            let op = Op::Update { id, from: current.status, to: updated.status, changed: updated != current };
            if let Some(slot) = list.tasks.iter_mut().find(|task| task.id == id) {
                *slot = updated;
            }
            op
        }
        Action::List => Op::List { status: input.status, include_deleted: input.include_deleted == Some(true) },
        Action::Get => {
            let Some(id) = input.id else {
                return error("id required for get");
            };
            match list.task(id) {
                Some(task) => Op::Get { task: task.clone() },
                None => error(format!("#{id} not found")),
            }
        }
        Action::Delete => {
            let Some(id) = input.id else {
                return error("id required for delete");
            };
            let Some(task) = list.tasks.iter_mut().find(|task| task.id == id) else {
                return error(format!("#{id} not found"));
            };
            if task.status == Status::Deleted {
                return error(format!("#{id} is already deleted"));
            }
            task.status = Status::Deleted;
            Op::Delete { id, subject: task.subject.clone() }
        }
        Action::Clear => {
            let count = list.tasks.len();
            *list = TodoList::default();
            Op::Clear { count }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: u64, subject: &str, status: Status, blocked_by: &[u64]) -> Task {
        Task { id, subject: subject.into(), description: None, active_form: None, status, blocked_by: blocked_by.to_vec() }
    }

    fn seed() -> TodoList {
        TodoList { tasks: vec![task(1, "First", Status::Completed, &[]), task(2, "Second", Status::InProgress, &[1])], next_id: 3 }
    }

    #[test]
    fn status_follows_the_four_state_machine_with_a_terminal_tombstone() {
        use Status::{Completed, Deleted, InProgress, Pending};
        assert!(transition_valid(Pending, InProgress));
        assert!(transition_valid(Pending, Completed));
        assert!(transition_valid(InProgress, Pending));
        assert!(!transition_valid(Completed, InProgress));
        assert!(transition_valid(Completed, Deleted));
        assert!(!transition_valid(Deleted, Pending));
        // Setting a status to what it already is passes: the update reports "No change".
        assert!(transition_valid(Completed, Completed));
        assert!([Pending, InProgress, Completed].iter().all(|to| !transition_valid(Deleted, *to)));
    }

    #[test]
    fn create_starts_pending_and_advances_the_id() {
        let mut list = seed();
        let op = apply(&mut list, Action::Create, &Input { subject: Some("Third".into()), active_form: Some("building the third".into()), ..Input::default() });
        assert_eq!(op, Op::Create { id: 3 });
        assert_eq!(list.next_id, 4);
        assert_eq!(list.tasks[2], Task { active_form: Some("building the third".into()), ..task(3, "Third", Status::Pending, &[]) });
    }

    #[test]
    fn a_refused_change_leaves_the_list_alone() {
        let input = |change: &dyn Fn(&mut Input)| {
            let mut input = Input::default();
            change(&mut input);
            input
        };
        let cases: Vec<(Action, Input, &str)> = vec![
            (Action::Create, Input::default(), "subject required for create"),
            (Action::Create, input(&|i| { i.subject = Some("X".into()); i.blocked_by = Some(vec![9]); }), "blockedBy: #9 not found"),
            (Action::Update, Input::default(), "id required for update"),
            (Action::Update, input(&|i| i.id = Some(9)), "#9 not found"),
            (Action::Update, input(&|i| i.id = Some(2)), "update requires at least one mutable field: subject, description, activeForm, status, addBlockedBy, or removeBlockedBy"),
            (Action::Update, input(&|i| { i.id = Some(1); i.status = Some(Status::InProgress); }), "illegal transition completed → in_progress"),
            (Action::Update, input(&|i| { i.id = Some(2); i.add_blocked_by = Some(vec![2]); }), "cannot block #2 on itself"),
            (Action::Update, input(&|i| { i.id = Some(1); i.add_blocked_by = Some(vec![2]); }), "addBlockedBy would create a cycle in the blockedBy graph"),
            (Action::Get, input(&|i| i.id = Some(9)), "#9 not found"),
            (Action::Get, Input::default(), "id required for get"),
        ];
        for (action, input, message) in cases {
            let mut list = seed();
            assert_eq!(apply(&mut list, action, &input), Op::Error(message.into()), "{action:?} {input:?}");
            assert_eq!(list, seed());
        }
    }

    #[test]
    fn an_update_that_changes_nothing_says_so() {
        let mut list = seed();
        let noop = apply(&mut list, Action::Update, &Input { id: Some(2), status: Some(Status::InProgress), ..Input::default() });
        assert_eq!(noop, Op::Update { id: 2, from: Status::InProgress, to: Status::InProgress, changed: false });
        let moved = apply(&mut list, Action::Update, &Input { id: Some(2), status: Some(Status::Completed), ..Input::default() });
        assert_eq!(moved, Op::Update { id: 2, from: Status::InProgress, to: Status::Completed, changed: true });
    }

    #[test]
    fn dependencies_merge_and_come_off() {
        let mut list = seed();
        apply(&mut list, Action::Update, &Input { id: Some(2), remove_blocked_by: Some(vec![1]), ..Input::default() });
        assert!(list.tasks[1].blocked_by.is_empty());
        apply(&mut list, Action::Update, &Input { id: Some(2), add_blocked_by: Some(vec![1, 1]), ..Input::default() });
        assert_eq!(list.tasks[1].blocked_by, [1]);
    }

    #[test]
    fn delete_leaves_a_tombstone_and_clear_starts_over() {
        let mut list = seed();
        let by_id = |id: u64| Input { id: Some(id), ..Input::default() };
        assert_eq!(apply(&mut list, Action::Delete, &by_id(2)), Op::Delete { id: 2, subject: "Second".into() });
        assert_eq!(list.tasks[1].status, Status::Deleted);
        assert_eq!(apply(&mut list, Action::Delete, &by_id(2)), Op::Error("#2 is already deleted".into()));
        assert_eq!(apply(&mut list, Action::Delete, &by_id(9)), Op::Error("#9 not found".into()));
        assert_eq!(apply(&mut list, Action::Delete, &Input::default()), Op::Error("id required for delete".into()));
        assert_eq!(apply(&mut list, Action::Clear, &Input::default()), Op::Clear { count: 2 });
        assert_eq!(list, TodoList::default());
    }

    #[test]
    fn the_graph_turns_around_and_previews_cycles() {
        let tasks = vec![task(1, "A", Status::Pending, &[2]), task(2, "B", Status::Pending, &[])];
        assert_eq!(blocks(&tasks).get(&2), Some(&vec![1]));
        assert!(detect_cycle(&tasks, 2, &[1]));
        assert!(!detect_cycle(&tasks, 2, &[]));
    }
}
