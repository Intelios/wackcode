//! A chat's panes as plain data: which panes are open, where, and how the user sized them.
//!
//! Each chat has two docks: one to the right of the transcript, one along the bottom. Every pane
//! opens in the right dock, as version 2's side panel did, so a terminal sits beside the chat; the
//! bottom dock holds only what the user drags there. A dock holds a tree whose leaves are tab stacks and whose branches split their space in a row or a
//! column, by weight.
//!
//! Invariants every operation keeps (the tests check them after every random step):
//! - A split has at least two children, positive weights, and no child split on its own axis
//!   (those are flattened into it).
//! - A stack has at least one pane, and `active` is one of them.
//! - A dock with nothing in it is `None`.
//! - Pane ids are unique across the layout and never reused, so state kept for a pane elsewhere
//!   can never be handed to a different one.
//! - The sizes the user chose (`right_width`, `canvas_width`, `bottom_height`) are only ever set by the user;
//!   the solver reads them and never writes them back, so a window made smaller and larger again
//!   gets the same panes back.

use serde::{Deserialize, Serialize};

pub type PaneId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaneKind {
    Changes,
    Terminal,
    Build,
    Run,
    /// The commands the agent runs in the background.
    Tasks,
    /// The chat's sub-agents and their work.
    Agents,
    /// The chat's browser tabs.
    Browser,
    /// The chat's canvases: boards of the agent's mockups.
    Canvas,
}

impl PaneKind {
    pub fn title(self) -> &'static str {
        match self {
            PaneKind::Changes => "Changes",
            PaneKind::Terminal => "Terminal",
            PaneKind::Build => "Build",
            PaneKind::Run => "Run",
            PaneKind::Tasks => "Tasks",
            PaneKind::Agents => "Agents",
            PaneKind::Browser => "Browser",
            PaneKind::Canvas => "Canvas",
        }
    }

    /// One per chat: opening it again shows the one there is.
    pub fn single(self) -> bool {
        matches!(self, PaneKind::Changes | PaneKind::Build | PaneKind::Run | PaneKind::Tasks | PaneKind::Agents | PaneKind::Browser | PaneKind::Canvas)
    }

    /// Where a new pane of this kind opens: beside the transcript, as a tab of the right dock.
    pub fn home(self) -> Dock {
        Dock::Right
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pane {
    pub id: PaneId,
    pub kind: PaneKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Axis {
    /// Children side by side, left to right.
    Row,
    /// Children one above the other, top to bottom.
    Column,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Dock {
    Right,
    Bottom,
}

/// Which side of a stack a pane is dropped on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    fn axis(self) -> Axis {
        match self {
            Edge::Left | Edge::Right => Axis::Row,
            Edge::Top | Edge::Bottom => Axis::Column,
        }
    }

    fn before(self) -> bool {
        matches!(self, Edge::Left | Edge::Top)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stack {
    pub panes: Vec<Pane>,
    pub active: usize,
}

impl Stack {
    pub fn active_pane(&self) -> Pane {
        self.panes[self.active.min(self.panes.len() - 1)]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Child {
    pub node: Node,
    pub weight: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Node {
    Split { axis: Axis, children: Vec<Child> },
    Stack(Stack),
}

/// Where a moved pane goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Into the stack that holds this pane, as a tab.
    With(PaneId),
    /// Beside the stack that holds this pane.
    Beside(PaneId, Edge),
    /// Into a dock: a tab of its first stack, or the whole dock when it is empty.
    Dock(Dock),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<Node>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<Node>,
    /// The width the user dragged the right dock to; until then it opens balanced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_width: Option<f32>,
    /// The width the user dragged the right dock to while a Canvas was in front of it. A board
    /// wants far more room than a diff, so it keeps its own (version 2 learned this); until the
    /// user drags it, the canvas opens as wide as the transcript allows (`solve`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas_width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom_height: Option<f32>,
    /// A hidden dock keeps its panes, and their state, until it is shown again.
    #[serde(default)]
    pub right_hidden: bool,
    #[serde(default)]
    pub bottom_hidden: bool,
    #[serde(default)]
    next_id: PaneId,
}

impl Layout {
    pub fn dock(&self, dock: Dock) -> Option<&Node> {
        match dock {
            Dock::Right => self.right.as_ref(),
            Dock::Bottom => self.bottom.as_ref(),
        }
    }

    fn dock_mut(&mut self, dock: Dock) -> &mut Option<Node> {
        match dock {
            Dock::Right => &mut self.right,
            Dock::Bottom => &mut self.bottom,
        }
    }

    pub fn hidden(&self, dock: Dock) -> bool {
        match dock {
            Dock::Right => self.right_hidden,
            Dock::Bottom => self.bottom_hidden,
        }
    }

    pub fn set_hidden(&mut self, dock: Dock, hidden: bool) {
        match dock {
            Dock::Right => self.right_hidden = hidden,
            Dock::Bottom => self.bottom_hidden = hidden,
        }
    }

    /// A dock with panes that is not hidden.
    pub fn shown(&self, dock: Dock) -> Option<&Node> {
        self.dock(dock).filter(|_| !self.hidden(dock))
    }

    /// Sets a dock's size; the right dock's is the canvas's own while a Canvas is in front of it.
    pub fn set_size(&mut self, dock: Dock, size: Option<f32>) {
        match dock {
            Dock::Right if self.canvas_in_front() => self.canvas_width = size,
            Dock::Right => self.right_width = size,
            Dock::Bottom => self.bottom_height = size,
        }
    }

    /// The right dock is shown with a Canvas in front of one of its stacks: it takes the canvas's
    /// width.
    pub fn canvas_in_front(&self) -> bool {
        let mut found = false;
        if let Some(node) = self.shown(Dock::Right) {
            visit(node, &mut |stack| found |= stack.active_pane().kind == PaneKind::Canvas);
        }
        found
    }

    /// Every pane, with its dock.
    pub fn panes(&self) -> Vec<(Dock, Pane)> {
        let mut found = Vec::new();
        for dock in [Dock::Right, Dock::Bottom] {
            if let Some(node) = self.dock(dock) {
                visit(node, &mut |stack| found.extend(stack.panes.iter().map(|pane| (dock, *pane))));
            }
        }
        found
    }

    /// The panes on screen: each shown dock's active tabs.
    pub fn visible(&self) -> Vec<Pane> {
        let mut found = Vec::new();
        for dock in [Dock::Right, Dock::Bottom] {
            if let Some(node) = self.shown(dock) {
                visit(node, &mut |stack| found.push(stack.active_pane()));
            }
        }
        found
    }

    pub fn find(&self, id: PaneId) -> Option<(Dock, Vec<usize>)> {
        for dock in [Dock::Right, Dock::Bottom] {
            if let Some(path) = self.dock(dock).and_then(|node| path_to(node, id)) {
                return Some((dock, path));
            }
        }
        None
    }

    pub fn stack(&self, dock: Dock, path: &[usize]) -> Option<&Stack> {
        match node_at(self.dock(dock)?, path)? {
            Node::Stack(stack) => Some(stack),
            Node::Split { .. } => None,
        }
    }

    fn stack_mut(&mut self, dock: Dock, path: &[usize]) -> Option<&mut Stack> {
        match node_at_mut(self.dock_mut(dock).as_mut()?, path)? {
            Node::Stack(stack) => Some(stack),
            Node::Split { .. } => None,
        }
    }

    pub fn first_of(&self, kind: PaneKind) -> Option<PaneId> {
        self.panes().into_iter().find(|(_, pane)| pane.kind == kind).map(|(_, pane)| pane.id)
    }

    fn fresh_id(&mut self) -> PaneId {
        // Past every id in use too, so a layout written by hand cannot hand out a duplicate.
        let highest = self.panes().iter().map(|(_, pane)| pane.id + 1).max().unwrap_or(0);
        let id = self.next_id.max(highest);
        self.next_id = id + 1;
        id
    }

    /// Brings a pane to the front of its stack and shows its dock.
    pub fn activate(&mut self, id: PaneId) {
        let Some((dock, path)) = self.find(id) else {
            return;
        };
        if let Some(stack) = self.stack_mut(dock, &path)
            && let Some(index) = stack.panes.iter().position(|pane| pane.id == id)
        {
            stack.active = index;
        }
        self.set_hidden(dock, false);
    }

    /// Adds a new pane of `kind` in its home dock, in front.
    pub fn add(&mut self, kind: PaneKind) -> PaneId {
        let pane = Pane { id: self.fresh_id(), kind };
        self.insert(pane, Target::Dock(kind.home()));
        self.set_hidden(kind.home(), false);
        pane.id
    }

    /// Shows a pane of `kind`, adding one when there is none.
    pub fn open(&mut self, kind: PaneKind) -> PaneId {
        match self.first_of(kind) {
            Some(id) => {
                self.activate(id);
                id
            }
            None => self.add(kind),
        }
    }

    /// ⌃` and friends: shows the kind's pane, or hides its dock when it is already in front.
    /// Hiding keeps the pane (a terminal keeps its shell); only `close` ends it.
    pub fn toggle(&mut self, kind: PaneKind) -> Option<PaneId> {
        if let Some(id) = self.first_of(kind)
            && let Some((dock, _)) = self.find(id)
            && !self.hidden(dock)
            && self.visible().iter().any(|pane| pane.kind == kind)
        {
            self.set_hidden(dock, true);
            return None;
        }
        Some(self.open(kind))
    }

    /// Takes a pane out. Returns false when there was no such pane.
    pub fn close(&mut self, id: PaneId) -> bool {
        for dock in [Dock::Right, Dock::Bottom] {
            let slot = self.dock_mut(dock);
            if let Some(node) = slot.take() {
                let (rest, removed) = remove(node, id);
                *slot = rest;
                if removed.is_some() {
                    if slot.is_none() {
                        self.set_hidden(dock, false);
                    }
                    return true;
                }
            }
        }
        false
    }

    /// Moves a pane, to another stack or beside one or into a dock. Moving a pane beside the
    /// stack it is alone in changes nothing.
    pub fn move_pane(&mut self, id: PaneId, target: Target) {
        let Some((dock, path)) = self.find(id) else {
            return;
        };
        let Some(stack) = self.stack(dock, &path) else {
            return;
        };
        let alone = stack.panes.len() == 1;
        // Another pane of the same stack, to stand in when a pane is moved beside its own stack.
        let neighbour = stack.panes.iter().find(|pane| pane.id != id).map(|pane| pane.id);
        let target = match target {
            Target::With(other) => {
                let Some((other_dock, other_path)) = self.find(other) else {
                    return;
                };
                if other_dock == dock && other_path == path {
                    self.activate(id);
                    return;
                }
                target
            }
            Target::Beside(other, edge) => {
                let Some((other_dock, other_path)) = self.find(other) else {
                    return;
                };
                let same_stack = other_dock == dock && other_path == path;
                if same_stack && alone {
                    return;
                }
                if same_stack && other == id {
                    match neighbour {
                        Some(neighbour) => Target::Beside(neighbour, edge),
                        None => return,
                    }
                } else {
                    target
                }
            }
            Target::Dock(_) => target,
        };
        let Some(pane) = self.panes().into_iter().find(|(_, pane)| pane.id == id).map(|(_, pane)| pane) else {
            return;
        };
        let slot = self.dock_mut(dock);
        if let Some(node) = slot.take() {
            *slot = remove(node, id).0;
        }
        self.insert(pane, target);
    }

    fn insert(&mut self, pane: Pane, target: Target) {
        match target {
            Target::Dock(dock) => {
                let slot = self.dock_mut(dock);
                match slot.as_mut() {
                    None => *slot = Some(Node::Stack(Stack { panes: vec![pane], active: 0 })),
                    Some(node) => {
                        let stack = first_stack(node);
                        stack.panes.push(pane);
                        stack.active = stack.panes.len() - 1;
                    }
                }
                self.set_hidden(dock, false);
            }
            Target::With(other) => {
                let Some((dock, path)) = self.find(other) else {
                    return self.insert(pane, Target::Dock(pane.kind.home()));
                };
                if let Some(stack) = self.stack_mut(dock, &path) {
                    stack.panes.push(pane);
                    stack.active = stack.panes.len() - 1;
                }
                self.set_hidden(dock, false);
            }
            Target::Beside(other, edge) => {
                let Some((dock, path)) = self.find(other) else {
                    return self.insert(pane, Target::Dock(pane.kind.home()));
                };
                let slot = self.dock_mut(dock);
                if let Some(node) = slot.take() {
                    *slot = Some(flatten(beside(node, &path, pane, edge)));
                }
                self.set_hidden(dock, false);
            }
        }
    }

    /// The divider between children `index` and `index + 1` of the split at `path` was dragged
    /// so that they now measure `before` and `after`: their weights take that ratio, and their
    /// total stays the same so the rest of the split does not move.
    pub fn resize_split(&mut self, dock: Dock, path: &[usize], index: usize, before: f32, after: f32) {
        let Some(Node::Split { children, .. }) = self.dock_mut(dock).as_mut().and_then(|node| node_at_mut(node, path)) else {
            return;
        };
        if index + 1 >= children.len() || before <= 0. || after <= 0. {
            return;
        }
        let total = children[index].weight + children[index + 1].weight;
        children[index].weight = total * before / (before + after);
        children[index + 1].weight = total * after / (before + after);
    }

    /// Checks the invariants above. For tests, and for layouts read back from disk.
    pub fn is_sound(&self) -> bool {
        let mut seen = std::collections::HashSet::new();
        for dock in [Dock::Right, Dock::Bottom] {
            if let Some(node) = self.dock(dock)
                && !sound(node, None, &mut seen)
            {
                return false;
            }
        }
        true
    }

    /// A layout read from disk, or an empty one if it is damaged: a broken layout must never
    /// stop a chat from opening.
    pub fn from_saved(value: Option<&serde_json::Value>) -> Self {
        value
            .and_then(|value| serde_json::from_value::<Layout>(value.clone()).ok())
            .filter(Layout::is_sound)
            .unwrap_or_default()
    }
}

fn sound(node: &Node, parent_axis: Option<Axis>, seen: &mut std::collections::HashSet<PaneId>) -> bool {
    match node {
        Node::Stack(stack) => {
            !stack.panes.is_empty() && stack.active < stack.panes.len() && stack.panes.iter().all(|pane| seen.insert(pane.id))
        }
        Node::Split { axis, children } => {
            children.len() >= 2
                && Some(*axis) != parent_axis
                && children.iter().all(|child| child.weight > 0. && child.weight.is_finite() && sound(&child.node, Some(*axis), seen))
        }
    }
}

fn visit(node: &Node, f: &mut impl FnMut(&Stack)) {
    match node {
        Node::Stack(stack) => f(stack),
        Node::Split { children, .. } => children.iter().for_each(|child| visit(&child.node, f)),
    }
}

fn path_to(node: &Node, id: PaneId) -> Option<Vec<usize>> {
    match node {
        Node::Stack(stack) => stack.panes.iter().any(|pane| pane.id == id).then(Vec::new),
        Node::Split { children, .. } => children.iter().enumerate().find_map(|(index, child)| {
            path_to(&child.node, id).map(|mut path| {
                path.insert(0, index);
                path
            })
        }),
    }
}

pub fn node_at<'a>(node: &'a Node, path: &[usize]) -> Option<&'a Node> {
    match (path.split_first(), node) {
        (None, _) => Some(node),
        (Some((index, rest)), Node::Split { children, .. }) => node_at(&children.get(*index)?.node, rest),
        (Some(_), Node::Stack(_)) => None,
    }
}

fn node_at_mut<'a>(node: &'a mut Node, path: &[usize]) -> Option<&'a mut Node> {
    match (path.split_first(), node) {
        (None, node) => Some(node),
        (Some((index, rest)), Node::Split { children, .. }) => node_at_mut(&mut children.get_mut(*index)?.node, rest),
        (Some(_), Node::Stack(_)) => None,
    }
}

fn first_stack(node: &mut Node) -> &mut Stack {
    match node {
        Node::Stack(stack) => stack,
        Node::Split { children, .. } => first_stack(&mut children[0].node),
    }
}

/// `node` without the pane, tidied: empty stacks and one-child splits disappear.
fn remove(node: Node, id: PaneId) -> (Option<Node>, Option<Pane>) {
    match node {
        Node::Stack(mut stack) => match stack.panes.iter().position(|pane| pane.id == id) {
            None => (Some(Node::Stack(stack)), None),
            Some(index) => {
                let pane = stack.panes.remove(index);
                if stack.panes.is_empty() {
                    return (None, Some(pane));
                }
                if stack.active > index || stack.active >= stack.panes.len() {
                    stack.active = stack.active.saturating_sub(1);
                }
                (Some(Node::Stack(stack)), Some(pane))
            }
        },
        Node::Split { axis, children } => {
            let mut removed = None;
            let mut kept = Vec::new();
            for child in children {
                if removed.is_some() {
                    kept.push(child);
                    continue;
                }
                let (rest, pane) = remove(child.node, id);
                removed = pane;
                if let Some(node) = rest {
                    kept.push(Child { node, weight: child.weight });
                }
            }
            (collapse(axis, kept), removed)
        }
    }
}

fn collapse(axis: Axis, mut children: Vec<Child>) -> Option<Node> {
    match children.len() {
        0 => None,
        1 => children.pop().map(|child| child.node),
        _ => Some(flatten(Node::Split { axis, children })),
    }
}

/// Merges children that split on their parent's axis into the parent, keeping their share.
fn flatten(node: Node) -> Node {
    match node {
        Node::Stack(_) => node,
        Node::Split { axis, children } => {
            let mut flat = Vec::new();
            for child in children {
                match flatten(child.node) {
                    Node::Split { axis: inner, children: grandchildren } if inner == axis => {
                        let sum: f32 = grandchildren.iter().map(|grandchild| grandchild.weight).sum();
                        for grandchild in grandchildren {
                            flat.push(Child { weight: child.weight * grandchild.weight / sum, node: grandchild.node });
                        }
                    }
                    node => flat.push(Child { node, weight: child.weight }),
                }
            }
            Node::Split { axis, children: flat }
        }
    }
}

/// Puts `pane` in a new stack on `edge` of the stack at `path`.
fn beside(node: Node, path: &[usize], pane: Pane, edge: Edge) -> Node {
    let fresh = Node::Stack(Stack { panes: vec![pane], active: 0 });
    match (path.split_first(), node) {
        // The stack itself: it becomes a split of two.
        (None, node) => {
            let (first, second) = if edge.before() { (fresh, node) } else { (node, fresh) };
            Node::Split {
                axis: edge.axis(),
                children: vec![Child { node: first, weight: 1. }, Child { node: second, weight: 1. }],
            }
        }
        // The stack's parent splits the same way: the new stack joins it, halving the target's
        // share so nothing else moves.
        (Some((&index, [])), Node::Split { axis, mut children }) if axis == edge.axis() && index < children.len() => {
            let half = children[index].weight / 2.;
            children[index].weight = half;
            let at = if edge.before() { index } else { index + 1 };
            children.insert(at, Child { node: fresh, weight: half });
            Node::Split { axis, children }
        }
        (Some((&index, rest)), Node::Split { axis, mut children }) if index < children.len() => {
            let child = children.remove(index);
            children.insert(index, Child { node: beside(child.node, rest, pane, edge), weight: child.weight });
            Node::Split { axis, children }
        }
        (_, node) => node,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A small deterministic generator, so property tests need no extra crate and fail the same
    /// way every time.
    pub struct Random(u64);

    impl Random {
        pub fn new(seed: u64) -> Self {
            Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
        }

        pub fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        pub fn below(&mut self, limit: usize) -> usize {
            (self.next() % limit.max(1) as u64) as usize
        }

        pub fn between(&mut self, low: f32, high: f32) -> f32 {
            low + (self.next() % 10_000) as f32 / 10_000. * (high - low)
        }
    }

    /// A layout built by random steps through the public operations.
    pub fn random_layout(random: &mut Random) -> Layout {
        let mut layout = Layout::default();
        for _ in 0..random.below(14) {
            step(&mut layout, random);
        }
        layout
    }

    fn step(layout: &mut Layout, random: &mut Random) {
        let ids: Vec<PaneId> = layout.panes().iter().map(|(_, pane)| pane.id).collect();
        let any = |random: &mut Random| ids.get(random.below(ids.len())).copied();
        let edge = |random: &mut Random| [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom][random.below(4)];
        match random.below(9) {
            0 => {
                layout.add(PaneKind::Terminal);
            }
            1 => {
                layout.open([PaneKind::Changes, PaneKind::Build, PaneKind::Run, PaneKind::Browser, PaneKind::Canvas][random.below(5)]);
            }
            2 => {
                layout.toggle([PaneKind::Changes, PaneKind::Terminal, PaneKind::Build, PaneKind::Run, PaneKind::Browser, PaneKind::Canvas][random.below(6)]);
            }
            3 => {
                if let Some(id) = any(random) {
                    layout.close(id);
                }
            }
            4 | 5 => {
                if let (Some(id), Some(other)) = (any(random), any(random)) {
                    layout.move_pane(id, Target::Beside(other, edge(random)));
                }
            }
            6 => {
                if let (Some(id), Some(other)) = (any(random), any(random)) {
                    layout.move_pane(id, Target::With(other));
                }
            }
            7 => {
                if let Some(id) = any(random) {
                    layout.move_pane(id, Target::Dock([Dock::Right, Dock::Bottom][random.below(2)]));
                }
            }
            _ => {
                if let Some(id) = any(random) {
                    layout.activate(id);
                }
            }
        }
    }

    #[test]
    fn random_steps_keep_the_invariants() {
        for seed in 0..400 {
            let mut random = Random::new(seed);
            let mut layout = Layout::default();
            for number in 0..60 {
                let before = layout.panes().len();
                step(&mut layout, &mut random);
                assert!(layout.is_sound(), "seed {seed}, step {number}: {layout:#?}");
                assert!(layout.panes().len().abs_diff(before) <= 1, "a step adds or removes at most one pane");
            }
        }
    }

    #[test]
    fn moves_keep_every_pane() {
        for seed in 0..200 {
            let mut random = Random::new(seed);
            let mut layout = random_layout(&mut random);
            let mut before: Vec<PaneId> = layout.panes().iter().map(|(_, pane)| pane.id).collect();
            before.sort();
            for _ in 0..20 {
                let ids: Vec<PaneId> = layout.panes().iter().map(|(_, pane)| pane.id).collect();
                if ids.is_empty() {
                    break;
                }
                let (id, other) = (ids[random.below(ids.len())], ids[random.below(ids.len())]);
                let target = match random.below(3) {
                    0 => Target::With(other),
                    1 => Target::Beside(other, [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom][random.below(4)]),
                    _ => Target::Dock([Dock::Right, Dock::Bottom][random.below(2)]),
                };
                layout.move_pane(id, target);
                let mut after: Vec<PaneId> = layout.panes().iter().map(|(_, pane)| pane.id).collect();
                after.sort();
                assert_eq!(before, after, "seed {seed}: a move lost or duplicated a pane");
            }
        }
    }

    #[test]
    fn changes_is_single_and_terminals_are_not() {
        let mut layout = Layout::default();
        let changes = layout.open(PaneKind::Changes);
        assert_eq!(layout.open(PaneKind::Changes), changes);
        let first = layout.add(PaneKind::Terminal);
        let second = layout.add(PaneKind::Terminal);
        assert_ne!(first, second);
        assert_eq!(layout.find(changes).map(|(dock, _)| dock), Some(Dock::Right));
        assert_eq!(layout.find(second).map(|(dock, _)| dock), Some(Dock::Right));
        // Terminals join Changes as tabs of the right dock; the newest is in front.
        assert_eq!(layout.stack(Dock::Right, &[]).map(|stack| stack.panes.len()), Some(3));
        assert_eq!(layout.stack(Dock::Right, &[]).map(|stack| stack.active_pane().id), Some(second));
        assert!(layout.bottom.is_none());
    }

    #[test]
    fn toggling_hides_without_closing() {
        let mut layout = Layout::default();
        let terminal = layout.toggle(PaneKind::Terminal).unwrap();
        assert!(layout.visible().iter().any(|pane| pane.id == terminal));
        assert_eq!(layout.toggle(PaneKind::Terminal), None);
        assert!(layout.right_hidden);
        assert!(layout.find(terminal).is_some(), "hidden, not closed");
        assert_eq!(layout.toggle(PaneKind::Terminal), Some(terminal), "the same terminal comes back");
        assert!(!layout.right_hidden);
    }

    #[test]
    fn splitting_and_closing_tidy_up() {
        let mut layout = Layout::default();
        let a = layout.add(PaneKind::Terminal);
        let b = layout.add(PaneKind::Terminal);
        layout.move_pane(b, Target::Beside(a, Edge::Right));
        assert!(matches!(layout.right, Some(Node::Split { axis: Axis::Row, ref children }) if children.len() == 2));
        let c = layout.add(PaneKind::Terminal);
        layout.move_pane(c, Target::Beside(b, Edge::Right));
        // A third column joins the row rather than nesting a new one.
        assert!(matches!(layout.right, Some(Node::Split { axis: Axis::Row, ref children }) if children.len() == 3));
        layout.close(b);
        layout.close(c);
        assert!(matches!(layout.right, Some(Node::Stack(_))), "a split of one is just its child");
        layout.close(a);
        assert!(layout.right.is_none());
    }

    #[test]
    fn a_damaged_saved_layout_opens_empty() {
        let saved = serde_json::json!({ "right": { "stack": { "panes": [], "active": 0 } } });
        assert_eq!(Layout::from_saved(Some(&saved)), Layout::default());
        assert_eq!(Layout::from_saved(Some(&serde_json::json!("nonsense"))), Layout::default());

        let mut layout = Layout::default();
        layout.open(PaneKind::Changes);
        layout.right_width = Some(480.);
        let saved = serde_json::to_value(&layout).unwrap();
        assert_eq!(Layout::from_saved(Some(&saved)), layout, "a sound layout round-trips");
    }

    #[test]
    fn ids_are_never_reused() {
        let mut layout = Layout::default();
        let first = layout.add(PaneKind::Terminal);
        layout.close(first);
        assert_ne!(layout.add(PaneKind::Terminal), first);
    }

    #[test]
    fn resizing_a_split_keeps_its_total() {
        let mut layout = Layout::default();
        let a = layout.add(PaneKind::Terminal);
        let b = layout.add(PaneKind::Terminal);
        layout.move_pane(b, Target::Beside(a, Edge::Right));
        layout.resize_split(Dock::Right, &[], 0, 300., 100.);
        let Some(Node::Split { children, .. }) = &layout.right else { panic!() };
        assert!((children[0].weight - 1.5).abs() < 1e-4 && (children[1].weight - 0.5).abs() < 1e-4);
    }

    #[test]
    fn build_and_run_are_singletons_in_right_dock() {
        let mut layout = Layout::default();
        let build = layout.open(PaneKind::Build);
        let run = layout.open(PaneKind::Run);
        assert_ne!(build, run);
        assert_eq!(layout.open(PaneKind::Build), build);
        assert_eq!(layout.open(PaneKind::Run), run);
        assert_eq!(layout.find(build).map(|(dock, _)| dock), Some(Dock::Right));
        assert_eq!(layout.find(run).map(|(dock, _)| dock), Some(Dock::Right));

        let saved = serde_json::to_value(&layout).unwrap();
        assert_eq!(Layout::from_saved(Some(&saved)), layout);
    }
}
