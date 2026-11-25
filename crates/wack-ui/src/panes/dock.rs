//! Draws a chat's docks from a [`Solution`]: each stack with its tabs, the lines between them,
//! the handles that resize them, and the drop zones shown while a tab is dragged.
//!
//! Everything is placed absolutely from the solver's rectangles, so what is drawn is exactly what
//! the property tests checked. Views never change the layout themselves: they dispatch actions
//! (`ActivatePane`, `MovePane`…) and drag values ([`Resizing`], [`DraggedTab`]) that the shell
//! applies to the chat, so a gallery scene draws the same docks and simply ignores them.
//!
//! The docks sit on `theme.dock`, a step back from the window like the sidebar: the transcript
//! is the lit page between the sidebar and the tools.

use gpui::{
    AnyElement, App, Context, CursorStyle, IntoElement, Pixels, Point, SharedString, Window, div, prelude::*, px,
};

use super::{
    ChatPanes, Layout, PaneModel, PaneId, Stack, Target,
    changes,
    layout::{Axis, Dock, Edge, PaneKind},
    solve::{Rect, Solution, StackPlace},
};
use crate::{
    actions::{ActivatePane, ClosePaneId, EqualizeSplit, MovePane, NewTerminalWith, ResetDockSize},
    icon::Icon,
    motion,
    tags::Tag,
    theme::Theme,
    typography::{Type, Typography},
};

/// The key context of a stack, where ⌘W closes the pane in front rather than the chat tab.
pub const PANE_CONTEXT: &str = "Pane";
pub const HEADER_HEIGHT: f32 = 34.;
/// How far either side of a line its resize handle reaches.
const HANDLE_REACH: f32 = 3.;
/// The share of a stack, from each edge, that drops a tab beside it rather than into it.
const EDGE_SHARE: f32 = 0.25;

/// What a resize handle drags. The shell turns the mouse position into a size.
#[derive(Clone, Debug, PartialEq)]
pub enum Resizing {
    Dock(Dock),
    /// The boundary between children `index` and `index + 1` of the split at `path`, which
    /// together span `total` points from `start` (window x for a row, content y for a column).
    Split { dock: Dock, path: Vec<usize>, index: usize, axis: Axis, start: f32, total: f32 },
}

/// Drawn under the pointer while a resize handle is dragged: nothing, the panes move instead.
pub struct ResizeGhost;

impl Render for ResizeGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// A tab being dragged to another stack, beside one, or into a dock.
#[derive(Clone, Debug)]
pub struct DraggedTab {
    pub pane: PaneId,
    pub title: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx);
        div()
            .h(px(28.))
            .px(px(12.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border_strong)
            .shadow_md()
            .typography(Type::UiStrong)
            .text_color(theme.text)
            .child(self.title.clone())
    }
}

/// Where in a stack a dragged tab would land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    /// Into the stack, as a tab.
    Center,
    Edge(Edge),
}

/// The zone of `rect` (a stack, in content coordinates) under `x`, `y`; `None` outside it. Over
/// the tab bar is always the stack itself.
pub fn zone(rect: Rect, x: f32, y: f32) -> Option<Zone> {
    if x < rect.x || x >= rect.right() || y < rect.y || y >= rect.bottom() {
        return None;
    }
    let body = Rect { y: rect.y + HEADER_HEIGHT, height: (rect.height - HEADER_HEIGHT).max(1.), ..rect };
    if y < body.y {
        return Some(Zone::Center);
    }
    let across = (x - body.x) / body.width.max(1.);
    let down = (y - body.y) / body.height;
    let nearest = [(Edge::Left, across), (Edge::Right, 1. - across), (Edge::Top, down), (Edge::Bottom, 1. - down)]
        .into_iter()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|(_, distance)| *distance < EDGE_SHARE);
    Some(nearest.map_or(Zone::Center, |(edge, _)| Zone::Edge(edge)))
}

/// The part of a stack a zone lights up: the half a new stack would take, or all of it.
pub fn zone_rect(rect: Rect, zone: Zone) -> Rect {
    let body = Rect { y: rect.y + HEADER_HEIGHT, height: (rect.height - HEADER_HEIGHT).max(0.), ..rect };
    match zone {
        Zone::Center => body,
        Zone::Edge(Edge::Left) => Rect { width: body.width / 2., ..body },
        Zone::Edge(Edge::Right) => Rect { x: body.x + body.width / 2., width: body.width / 2., ..body },
        Zone::Edge(Edge::Top) => Rect { height: body.height / 2., ..body },
        Zone::Edge(Edge::Bottom) => Rect { y: body.y + body.height / 2., height: body.height / 2., ..body },
    }
}

/// Where a dragged tab goes if dropped at `x`, `y` (content coordinates), and the area to light
/// up; `None` where a drop would change nothing.
pub fn drop_target(layout: &Layout, solution: &Solution, dragged: PaneId, x: f32, y: f32) -> Option<(Target, Rect)> {
    let found = solution.stacks.iter().find_map(|place| {
        let zone = zone(place.rect, x, y)?;
        let stack = layout.stack(place.dock, &place.path)?;
        let other = stack.active_pane().id;
        let target = match zone {
            Zone::Center => Target::With(other),
            Zone::Edge(edge) => Target::Beside(other, edge),
        };
        Some((target, zone_rect(place.rect, zone)))
    });
    let found = found.or_else(|| {
        // Over the transcript: the edge strips open a dock that is not showing.
        let transcript = solution.transcript;
        let right = Rect { x: transcript.right() - (transcript.width * 0.3).min(220.), width: (transcript.width * 0.3).min(220.), ..transcript };
        let bottom = Rect { y: transcript.bottom() - (transcript.height * 0.3).min(180.), height: (transcript.height * 0.3).min(180.), ..transcript };
        let inside = |rect: Rect| x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom();
        if solution.bottom.is_none() && inside(bottom) {
            Some((Target::Dock(Dock::Bottom), bottom))
        } else if solution.right.is_none() && inside(right) {
            Some((Target::Dock(Dock::Right), right))
        } else {
            None
        }
    })?;
    let mut moved = layout.clone();
    moved.move_pane(dragged, found.0);
    (moved != *layout).then_some(found)
}

/// Everything the docks draw, positioned in the stage. The solution is in the solver's space
/// (`shell::solver_space`), `width` × `height`; `left` is where the stage starts in it.
pub struct Frame<'a> {
    pub panes: &'a ChatPanes,
    pub solution: &'a Solution,
    pub left: f32,
    pub width: f32,
    pub height: f32,
    /// The tab being dragged and the mouse in content coordinates, while a tab is dragged.
    pub dragging: Option<(PaneId, Point<Pixels>)>,
}

pub fn docks(frame: &Frame, theme: &Theme, cx: &App) -> Vec<AnyElement> {
    let place = |rect: Rect| div().absolute().left(px(rect.x - frame.left)).top(px(rect.y)).w(px(rect.width)).h(px(rect.height));
    let mut elements = Vec::new();
    // A dock is a step back from the stage, and rounds the stage's corners it reaches. Over a
    // picture it is a tint on the stage's glass.
    let corner = px(crate::shell::PANE_RADIUS);
    let ground = if theme.veil > 0. { gpui::Hsla { a: 0.5, ..theme.dock } } else { theme.dock };
    for rect in [frame.solution.right, frame.solution.bottom].into_iter().flatten() {
        let (right, bottom) = (rect.right() >= frame.width - 0.5, rect.bottom() >= frame.height - 0.5);
        let (top, left) = (rect.y <= 0.5, rect.x <= frame.left + 0.5);
        elements.push(
            place(rect)
                .bg(ground)
                .when(top && right, |dock| dock.rounded_tr(corner))
                .when(bottom && right, |dock| dock.rounded_br(corner))
                .when(bottom && left, |dock| dock.rounded_bl(corner))
                .into_any_element(),
        );
    }
    for stack_place in &frame.solution.stacks {
        if let Some(stack) = frame.panes.layout.stack(stack_place.dock, &stack_place.path) {
            elements.push(place(stack_place.rect).child(stack_view(stack_place, stack, frame.panes, theme, cx)).into_any_element());
        }
    }
    elements.extend(lines(frame, theme));
    if let Some((dragged, mouse)) = frame.dragging
    {
        if let Some((_, rect)) = drop_target(&frame.panes.layout, frame.solution, dragged, f32::from(mouse.x), f32::from(mouse.y)) {
            elements.push(
                place(rect)
                    .p(px(6.))
                    .child(div().size_full().rounded(px(8.)).bg(theme.accent_wash).border_1().border_color(theme.accent_line))
                    .into_any_element(),
            );
        }
        // One catcher over the whole area takes the drop, wherever it lands.
        let (layout, solution) = (frame.panes.layout.clone(), frame.solution.clone());
        elements.push(
            div()
                .id("pane-drop")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_drop(move |tab: &DraggedTab, window, cx| {
                    let mouse = window.mouse_position();
                    let (x, y) = crate::shell::to_solver(f32::from(mouse.x), f32::from(mouse.y));
                    if let Some((target, _)) = drop_target(&layout, &solution, tab.pane, x, y) {
                        window.dispatch_action(Box::new(MovePane { pane: tab.pane, target }), cx);
                    }
                })
                .into_any_element(),
        );
    }
    elements
}

fn stack_view(place: &StackPlace, stack: &Stack, panes: &ChatPanes, theme: &Theme, cx: &App) -> AnyElement {
    let active = stack.active_pane();
    let tabs = stack.panes.iter().map(|pane| {
        let selected = pane.id == active.id;
        let title = panes.title(*pane, cx);
        let badge = panes.badge(*pane, cx);
        let status_dot = panes.command_status(*pane, cx).and_then(|status| match status {
            wack_core::CommandStatus::Running => Some(theme.accent_text),
            wack_core::CommandStatus::Finished => Some(theme.success),
            wack_core::CommandStatus::Failed { .. } => Some(theme.danger),
            _ => None,
        });
        let group: SharedString = format!("pane-tab-{}", pane.id).into();
        let (id, dragged) = (pane.id, DraggedTab { pane: pane.id, title: title.clone() });
        div()
            .id(("pane-tab", pane.id as usize))
            .group(group.clone())
            .relative()
            .flex_none()
            .max_w(px(200.))
            .h_full()
            .pl(px(12.))
            .pr(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .on_click(move |_, window, cx| window.dispatch_action(Box::new(ActivatePane { pane: id }), cx))
            .on_drag(dragged, |tab, _, _, cx| cx.new(|_| tab.clone()))
            .when_some(status_dot, |tab, dot| {
                tab.child(div().flex_none().size(px(6.)).rounded_full().bg(dot))
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .typography(if selected { Type::UiStrong } else { Type::Ui })
                    .text_color(if selected { theme.text } else { theme.text_dim })
                    .when(!selected, |title| title.group_hover(group.clone(), |title| title.text_color(theme.text_soft)))
                    .child(title),
            )
            .when_some(badge, |tab, badge| {
                tab.child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(badge))
            })
            .child(
                div()
                    .id(("close-pane", pane.id as usize))
                    .flex_none()
                    .size(px(18.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .invisible()
                    .group_hover(group.clone(), |close| close.visible())
                    .hover(|close| close.bg(theme.hover))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(ClosePaneId { pane: id }), cx);
                    })
                    .child(Icon::Close.view(px(11.), theme.text_dim))
                    .tag("close-pane"),
            )
            // The tab in front is marked by a short rule under its words.
            .when(selected, |tab| {
                tab.child(div().absolute().bottom_0().left(px(12.)).right(px(24.)).h(px(2.)).rounded(px(1.)).bg(theme.accent_line))
            })
            .tag(match pane.kind {
                PaneKind::Changes => "pane-tab-changes",
                PaneKind::Terminal => "pane-tab-terminal",
                PaneKind::Build => "pane-tab-build",
                PaneKind::Run => "pane-tab-run",
                PaneKind::Tasks => "pane-tab-tasks",
                PaneKind::Agents => "pane-tab-agents",
                PaneKind::Browser => "pane-tab-browser",
                PaneKind::Canvas => "pane-tab-canvas",
            })
    });
    let add_terminal = (active.kind == PaneKind::Terminal).then(|| {
        div()
            .id("new-terminal")
            .flex_none()
            .size(px(22.))
            .mr(px(8.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .justify_center()
            .hover(|button| button.bg(theme.hover))
            .on_click(move |_, window, cx| window.dispatch_action(Box::new(NewTerminalWith { pane: active.id }), cx))
            .child(Icon::Plus.view(px(13.), theme.text_dim))
            .tag("new-terminal")
    });
    let header = div()
        .flex_none()
        .h(px(HEADER_HEIGHT))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(theme.border)
        .child(div().flex_1().min_w_0().h_full().flex().overflow_hidden().children(tabs))
        .children(add_terminal);
    let content = match panes.model(active.id) {
        Some(PaneModel::Changes(model)) => changes::view(model, place.rect.width, theme, cx),
        Some(PaneModel::Terminal(model)) => model.clone().into_any_element(),
        Some(PaneModel::Build(model)) | Some(PaneModel::Run(model)) => model.clone().into_any_element(),
        Some(PaneModel::Tasks(model)) => model.clone().into_any_element(),
        Some(PaneModel::Agents(model)) => model.clone().into_any_element(),
        Some(PaneModel::Browser(model)) => model.clone().into_any_element(),
        Some(PaneModel::Canvas(model)) => model.clone().into_any_element(),
        None => div().into_any_element(),
    };
    // The pane in front arrives from the side its dock opens on.
    let (from_right, from_below) = match place.dock {
        Dock::Right => (28., 0.),
        Dock::Bottom => (0., 20.),
    };
    let content = motion::slide_in(("pane-arrive", active.id as usize), div().size_full().child(content), from_right, from_below);
    div()
        .size_full()
        .flex()
        .flex_col()
        .key_context(PANE_CONTEXT)
        .child(header)
        .child(div().flex_1().min_h_0().overflow_hidden().child(content))
        .into_any_element()
}

/// The hairlines between the transcript and the docks and between stacks, with their handles.
fn lines(frame: &Frame, theme: &Theme) -> Vec<AnyElement> {
    let mut elements = Vec::new();
    let left = frame.left;
    let mut line = |rect: Rect, axis: Axis, resizing: Resizing, reset: Box<dyn gpui::Action>, tag: &'static str| {
        let (hairline, handle) = match axis {
            Axis::Row => (
                Rect { width: 1., ..rect },
                Rect { x: rect.x - HANDLE_REACH, width: 2. * HANDLE_REACH + 1., ..rect },
            ),
            Axis::Column => (
                Rect { height: 1., ..rect },
                Rect { y: rect.y - HANDLE_REACH, height: 2. * HANDLE_REACH + 1., ..rect },
            ),
        };
        elements.push(
            div()
                .absolute()
                .left(px(hairline.x - left))
                .top(px(hairline.y))
                .w(px(hairline.width))
                .h(px(hairline.height))
                .bg(theme.border)
                .into_any_element(),
        );
        let reset = std::rc::Rc::new(reset);
        elements.push(
            div()
                .id(SharedString::from(format!("{tag}-{}-{}", hairline.x, hairline.y)))
                .absolute()
                .left(px(handle.x - left))
                .top(px(handle.y))
                .w(px(handle.width))
                .h(px(handle.height))
                .cursor(match axis {
                    Axis::Row => CursorStyle::ResizeLeftRight,
                    Axis::Column => CursorStyle::ResizeUpDown,
                })
                .on_drag(resizing, |_, _, _, cx| cx.new(|_| ResizeGhost))
                .on_click(move |event, window, cx| {
                    if event.click_count() == 2 {
                        window.dispatch_action(reset.boxed_clone(), cx);
                    }
                })
                .tag(tag)
                .into_any_element(),
        );
    };
    if let Some(right) = frame.solution.right {
        line(right, Axis::Row, Resizing::Dock(Dock::Right), Box::new(ResetDockSize { dock: Dock::Right }), "right-dock-edge");
    }
    if let Some(bottom) = frame.solution.bottom {
        line(bottom, Axis::Column, Resizing::Dock(Dock::Bottom), Box::new(ResetDockSize { dock: Dock::Bottom }), "bottom-dock-edge");
    }
    for divider in &frame.solution.dividers {
        let start = match divider.axis {
            Axis::Row => divider.line.x - divider.before,
            Axis::Column => divider.line.y - divider.before,
        };
        let resizing = Resizing::Split {
            dock: divider.dock,
            path: divider.path.clone(),
            index: divider.index,
            axis: divider.axis,
            start,
            total: divider.before + divider.after,
        };
        let reset = EqualizeSplit { dock: divider.dock, path: divider.path.clone(), index: divider.index };
        line(divider.line, divider.axis, resizing, Box::new(reset), "divider");
    }
    elements
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panes::solve;

    #[test]
    fn zones_split_a_stack_by_its_edges() {
        let rect = Rect { x: 100., y: 0., width: 400., height: 334. };
        assert_eq!(zone(rect, 99., 50.), None);
        assert_eq!(zone(rect, 300., 10.), Some(Zone::Center), "over the tabs");
        assert_eq!(zone(rect, 300., 184.), Some(Zone::Center));
        assert_eq!(zone(rect, 110., 184.), Some(Zone::Edge(Edge::Left)));
        assert_eq!(zone(rect, 490., 184.), Some(Zone::Edge(Edge::Right)));
        assert_eq!(zone(rect, 300., 40.), Some(Zone::Edge(Edge::Top)));
        assert_eq!(zone(rect, 300., 330.), Some(Zone::Edge(Edge::Bottom)));
        assert_eq!(zone_rect(rect, Zone::Edge(Edge::Right)).x, 300.);
    }

    #[test]
    fn a_drop_that_changes_nothing_is_not_offered() {
        let mut layout = Layout::default();
        let terminal = layout.add(PaneKind::Terminal);
        let solution = solve(1400., 900., false, &layout);
        let stack = solution.stacks[0].rect;
        // Its own stack, alone: no target in the middle or at an edge.
        assert!(drop_target(&layout, &solution, terminal, stack.x + stack.width / 2., stack.y + 120.).is_none());
        assert!(drop_target(&layout, &solution, terminal, stack.x + 5., stack.y + 120.).is_none());
        // Over the transcript's foot, it would open the bottom dock.
        let transcript = solution.transcript;
        let (target, _) = drop_target(&layout, &solution, terminal, transcript.x + 50., transcript.bottom() - 10.).unwrap();
        assert_eq!(target, Target::Dock(Dock::Bottom));
    }
}
