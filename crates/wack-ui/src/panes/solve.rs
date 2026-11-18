//! Turns the window's size and a chat's [`Layout`] into rectangles: the sidebar, the transcript,
//! each dock, and every stack inside them.
//!
//! The rules, which the property tests below check over random layouts and window sizes:
//! - The transcript keeps `TRANSCRIPT_MIN_WIDTH` × `TRANSCRIPT_MIN_HEIGHT` at any window size the
//!   app allows. It is the one thing that must stay readable.
//! - A Canvas in front of the right dock opens as wide as the transcript's minimum allows beside
//!   the sidebar, and keeps a width of its own once dragged (`Layout::canvas_width`).
//! - A dock gets the size the user gave it when that fits. When it does not, the sidebar steps
//!   aside first (only while open, and it comes back once there is room), and only then is the
//!   dock clamped.
//! - The user's sizes are read, never written: this is a pure function of its inputs.
//! - Everything tiles exactly, on whole points: no gaps, no overlaps, nothing outside the window.
//!
//! The bottom dock spans the transcript and the right dock, so a terminal gets the full width.

use super::layout::{Axis, Dock, Layout, Node};
use crate::sidebar::SIDEBAR_WIDTH;

pub const TRANSCRIPT_MIN_WIDTH: f32 = 420.;
pub const TRANSCRIPT_MIN_HEIGHT: f32 = 240.;
pub const DOCK_MIN_WIDTH: f32 = 260.;
pub const DOCK_MIN_HEIGHT: f32 = 140.;
/// A stack inside a split keeps at least this much, while the dock has room for it.
pub const STACK_MIN_WIDTH: f32 = 180.;
pub const STACK_MIN_HEIGHT: f32 = 100.;
/// The smallest window the app allows: the transcript plus a dock with the sidebar aside.
pub const WINDOW_MIN_WIDTH: f32 = 700.;
/// Below the title strip.
pub const CONTENT_MIN_HEIGHT: f32 = 440.;
/// A new bottom dock's share of the height, until the user drags it.
const BOTTOM_SHARE: f32 = 0.36;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn area(&self) -> f32 {
        self.width * self.height
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StackPlace {
    pub dock: Dock,
    /// From the dock's root to the stack.
    pub path: Vec<usize>,
    pub rect: Rect,
}

/// The boundary between two children of a split, which the user can drag.
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    pub dock: Dock,
    /// The split's path.
    pub path: Vec<usize>,
    pub axis: Axis,
    /// Between child `index` and `index + 1`.
    pub index: usize,
    /// Where the boundary runs: zero wide in a row, zero tall in a column.
    pub line: Rect,
    /// The two children's sizes along the axis.
    pub before: f32,
    pub after: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Solution {
    pub sidebar: bool,
    /// The sidebar is open by choice but hidden to make room.
    pub sidebar_stepped_aside: bool,
    pub transcript: Rect,
    pub right: Option<Rect>,
    pub bottom: Option<Rect>,
    pub stacks: Vec<StackPlace>,
    pub dividers: Vec<Divider>,
}

/// The right dock's width before the user drags it: half of what the sidebar leaves.
pub fn balanced_width(width: f32, sidebar: f32) -> f32 {
    ((width - sidebar) / 2.).max(DOCK_MIN_WIDTH)
}

/// The canvas's width before the user drags it: everything the transcript's minimum leaves beside
/// the sidebar, so the board gets the room and nothing steps aside for it.
pub fn canvas_width(width: f32, sidebar: f32) -> f32 {
    (width - sidebar - TRANSCRIPT_MIN_WIDTH).max(DOCK_MIN_WIDTH)
}

/// The widest the right dock can be: what leaves the transcript its minimum with the sidebar
/// aside. Dragging is clamped to this.
pub fn widest_dock(width: f32) -> f32 {
    (width - TRANSCRIPT_MIN_WIDTH).max(DOCK_MIN_WIDTH)
}

pub fn tallest_dock(height: f32) -> f32 {
    (height - TRANSCRIPT_MIN_HEIGHT).max(DOCK_MIN_HEIGHT)
}

/// `width` × `height` is the window below the title strip.
pub fn solve(width: f32, height: f32, sidebar_open: bool, layout: &Layout) -> Solution {
    let (width, height) = (width.max(0.).round(), height.max(0.).round());
    let mut sidebar = if sidebar_open { f32::from(SIDEBAR_WIDTH) } else { 0. };
    let room = |sidebar: f32| (width - sidebar - TRANSCRIPT_MIN_WIDTH).max(0.);

    let right = layout.shown(Dock::Right).map(|_| {
        let wanted = if layout.canvas_in_front() {
            layout.canvas_width.filter(|width| width.is_finite()).unwrap_or_else(|| canvas_width(width, sidebar))
        } else {
            layout.right_width.filter(|width| width.is_finite()).unwrap_or_else(|| balanced_width(width, sidebar))
        };
        wanted.max(DOCK_MIN_WIDTH)
    });
    let needed = right.map_or(0., |wanted| wanted.min(room(0.)).max(DOCK_MIN_WIDTH.min(room(0.))));
    if sidebar > 0. && needed > room(sidebar) {
        sidebar = 0.;
    }
    let right_width = right.map(|wanted| wanted.min(room(sidebar)).max(DOCK_MIN_WIDTH.min(room(sidebar))).round());
    let transcript_width = width - sidebar - right_width.unwrap_or(0.);

    let bottom_height = layout.shown(Dock::Bottom).map(|_| {
        let wanted = layout.bottom_height.filter(|height| height.is_finite()).unwrap_or(height * BOTTOM_SHARE).max(DOCK_MIN_HEIGHT);
        let room = (height - TRANSCRIPT_MIN_HEIGHT).max(0.);
        wanted.min(room).max(DOCK_MIN_HEIGHT.min(room)).round()
    });
    let upper = height - bottom_height.unwrap_or(0.);

    let mut solution = Solution {
        sidebar: sidebar > 0.,
        sidebar_stepped_aside: sidebar_open && sidebar == 0.,
        transcript: Rect { x: sidebar, y: 0., width: transcript_width, height: upper },
        right: right_width.map(|right| Rect { x: sidebar + transcript_width, y: 0., width: right, height: upper }),
        bottom: bottom_height.map(|bottom| Rect { x: sidebar, y: upper, width: width - sidebar, height: bottom }),
        stacks: Vec::new(),
        dividers: Vec::new(),
    };
    for (dock, rect) in [(Dock::Right, solution.right), (Dock::Bottom, solution.bottom)] {
        if let (Some(node), Some(rect)) = (layout.shown(dock), rect) {
            place(node, rect, dock, &mut Vec::new(), &mut solution);
        }
    }
    solution
}

fn place(node: &Node, rect: Rect, dock: Dock, path: &mut Vec<usize>, solution: &mut Solution) {
    match node {
        Node::Stack(_) => solution.stacks.push(StackPlace { dock, path: path.clone(), rect }),
        Node::Split { axis, children } => {
            let (start, total, minimum) = match axis {
                Axis::Row => (rect.x, rect.width, STACK_MIN_WIDTH),
                Axis::Column => (rect.y, rect.height, STACK_MIN_HEIGHT),
            };
            let weights: Vec<f32> = children.iter().map(|child| child.weight).collect();
            let edges = boundaries(total, &weights, minimum);
            for (index, child) in children.iter().enumerate() {
                let (from, to) = (start + edges[index], start + edges[index + 1]);
                let part = match axis {
                    Axis::Row => Rect { x: from, width: to - from, ..rect },
                    Axis::Column => Rect { y: from, height: to - from, ..rect },
                };
                if index + 1 < children.len() {
                    let line = match axis {
                        Axis::Row => Rect { x: to, width: 0., ..rect },
                        Axis::Column => Rect { y: to, height: 0., ..rect },
                    };
                    solution.dividers.push(Divider {
                        dock,
                        path: path.clone(),
                        axis: *axis,
                        index,
                        line,
                        before: to - from,
                        after: edges[index + 2] - edges[index + 1],
                    });
                }
                path.push(index);
                place(&child.node, part, dock, path, solution);
                path.pop();
            }
        }
    }
}

/// Where each child of a split starts, plus the end: `weights.len() + 1` whole-point offsets
/// from 0 to `total`. Children keep `minimum` while there is room for all of them; when there
/// is not, they share equally.
fn boundaries(total: f32, weights: &[f32], minimum: f32) -> Vec<f32> {
    let count = weights.len();
    let mut sizes = vec![total / count as f32; count];
    if total > minimum * count as f32 {
        let mut fixed = vec![false; count];
        loop {
            let spare = total - minimum * fixed.iter().filter(|fixed| **fixed).count() as f32;
            let weight: f32 = weights.iter().zip(&fixed).filter(|(_, fixed)| !**fixed).map(|(weight, _)| weight.max(0.0001)).sum();
            let mut changed = false;
            for index in 0..count {
                if fixed[index] {
                    sizes[index] = minimum;
                    continue;
                }
                sizes[index] = spare * weights[index].max(0.0001) / weight;
                if sizes[index] < minimum {
                    fixed[index] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }
    let mut edges = Vec::with_capacity(count + 1);
    let mut sum = 0.;
    edges.push(0.);
    for size in &sizes[..count - 1] {
        sum += size;
        edges.push(sum.round().clamp(0., total));
    }
    edges.push(total);
    edges
}

#[cfg(test)]
mod tests {
    use super::super::layout::{Edge, PaneKind, Target, tests::{Random, random_layout}};
    use super::*;

    fn overlap(a: &Rect, b: &Rect) -> f32 {
        let width = (a.right().min(b.right()) - a.x.max(b.x)).max(0.);
        let height = (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0.);
        width * height
    }

    fn inside(inner: &Rect, outer: &Rect) -> bool {
        inner.x >= outer.x && inner.y >= outer.y && inner.right() <= outer.right() + 0.001 && inner.bottom() <= outer.bottom() + 0.001
    }

    fn tiles(parts: &[Rect], whole: &Rect) -> Result<(), String> {
        let area: f32 = parts.iter().map(Rect::area).sum();
        if (area - whole.area()).abs() > 0.5 {
            return Err(format!("areas {area} ≠ {}", whole.area()));
        }
        for (index, part) in parts.iter().enumerate() {
            if !inside(part, whole) {
                return Err(format!("{part:?} is outside {whole:?}"));
            }
            if part.width < 0. || part.height < 0. {
                return Err(format!("{part:?} is negative"));
            }
            for other in &parts[index + 1..] {
                if overlap(part, other) > 0. {
                    return Err(format!("{part:?} overlaps {other:?}"));
                }
            }
        }
        Ok(())
    }

    fn with_random_sizes(layout: &mut Layout, random: &mut Random) {
        let size = |random: &mut Random| match random.below(5) {
            0 => None,
            1 => Some(-50.),
            2 => Some(10_000.),
            3 => Some(f32::NAN),
            _ => Some(random.between(0., 1600.)),
        };
        layout.right_width = size(random);
        layout.canvas_width = size(random);
        layout.bottom_height = size(random);
    }

    #[test]
    fn a_canvas_opens_as_wide_as_the_transcript_allows_and_keeps_its_own_width() {
        for width in (700..=2600).step_by(25) {
            let width = width as f32;
            for sidebar_open in [true, false] {
                let mut layout = Layout::default();
                layout.open(PaneKind::Changes);
                layout.right_width = Some(500.);
                layout.open(PaneKind::Canvas);
                let solved = solve(width, 900., sidebar_open, &layout);
                let sidebar = if sidebar_open { f32::from(SIDEBAR_WIDTH) } else { 0. };
                let context = format!("{width}, sidebar {sidebar_open}: {solved:?}");
                if width - sidebar - TRANSCRIPT_MIN_WIDTH >= DOCK_MIN_WIDTH {
                    assert_eq!(solved.sidebar, sidebar_open, "nothing steps aside for a default canvas: {context}");
                    assert_eq!(solved.transcript.width, TRANSCRIPT_MIN_WIDTH, "{context}");
                }
                assert!(solved.transcript.width >= TRANSCRIPT_MIN_WIDTH, "{context}");

                // Dragging sets the canvas's width; Changes in front keeps the user's own.
                layout.set_size(Dock::Right, Some(640.));
                assert_eq!((layout.canvas_width, layout.right_width), (Some(640.), Some(500.)));
                let changes = layout.first_of(PaneKind::Changes).unwrap();
                layout.activate(changes);
                let solved = solve(width, 900., sidebar_open, &layout);
                if width - sidebar - TRANSCRIPT_MIN_WIDTH >= 500. {
                    assert_eq!(solved.right.unwrap().width, 500., "{context}");
                }
            }
        }
    }

    #[test]
    fn every_window_keeps_the_transcript_and_tiles_exactly() {
        for seed in 0..3000 {
            let mut random = Random::new(seed);
            let mut layout = random_layout(&mut random);
            with_random_sizes(&mut layout, &mut random);
            let width = random.between(WINDOW_MIN_WIDTH, 2600.).round();
            let height = random.between(CONTENT_MIN_HEIGHT, 1700.).round();
            let sidebar_open = random.below(2) == 0;
            let before = layout.clone();
            let solved = solve(width, height, sidebar_open, &layout);
            let context = format!("seed {seed}: {width}×{height}, sidebar {sidebar_open}, {layout:?}\n{solved:#?}");

            // Compared as text: a NaN size never equals itself.
            assert_eq!(format!("{layout:?}"), format!("{before:?}"), "the solver never writes the layout");
            assert!(solved.transcript.width >= TRANSCRIPT_MIN_WIDTH, "transcript too narrow: {context}");
            assert!(solved.transcript.height >= TRANSCRIPT_MIN_HEIGHT, "transcript too short: {context}");

            // The window below the title strip, less the sidebar, is the transcript and the docks.
            let sidebar = if solved.sidebar { f32::from(SIDEBAR_WIDTH) } else { 0. };
            let main = Rect { x: sidebar, y: 0., width: width - sidebar, height };
            let mut parts = vec![solved.transcript];
            parts.extend(solved.right);
            parts.extend(solved.bottom);
            tiles(&parts, &main).unwrap_or_else(|problem| panic!("{problem}: {context}"));

            for (dock, rect) in [(Dock::Right, solved.right), (Dock::Bottom, solved.bottom)] {
                assert_eq!(rect.is_some(), layout.shown(dock).is_some(), "{context}");
                let Some(rect) = rect else { continue };
                let stacks: Vec<Rect> = solved.stacks.iter().filter(|place| place.dock == dock).map(|place| place.rect).collect();
                tiles(&stacks, &rect).unwrap_or_else(|problem| panic!("{dock:?} dock, {problem}: {context}"));
                for place in solved.stacks.iter().filter(|place| place.dock == dock) {
                    assert!(layout.stack(dock, &place.path).is_some(), "every place is a stack: {context}");
                    for value in [place.rect.x, place.rect.y, place.rect.width, place.rect.height] {
                        assert_eq!(value, value.round(), "whole points: {context}");
                    }
                }
            }
            // Docks keep their minimum: the window minimum leaves room for them.
            if let Some(right) = solved.right {
                assert!(right.width >= DOCK_MIN_WIDTH, "{context}");
            }
            if let Some(bottom) = solved.bottom {
                assert!(bottom.height >= DOCK_MIN_HEIGHT, "{context}");
            }
            // The sidebar steps aside only for a dock that would not fit beside it.
            if sidebar_open && !solved.sidebar {
                assert!(solved.right.is_some(), "the sidebar hid with no dock to make room for: {context}");
            }
            assert_eq!(solve(width, height, sidebar_open, &layout), solved, "the same inputs, the same answer");
        }
    }

    #[test]
    fn the_sidebar_steps_aside_before_a_dock_is_clamped() {
        let mut layout = Layout::default();
        layout.open(PaneKind::Changes);
        layout.right_width = Some(600.);
        // Room for both: everything as asked.
        let wide = solve(1600., 800., true, &layout);
        assert!(wide.sidebar);
        assert_eq!(wide.right.unwrap().width, 600.);
        // 1200 - 252 - 420 leaves 528: the sidebar goes and the dock keeps its 600.
        let narrower = solve(1200., 800., true, &layout);
        assert!(!narrower.sidebar && narrower.sidebar_stepped_aside);
        assert_eq!(narrower.right.unwrap().width, 600.);
        // At the window's minimum the dock is clamped, and the transcript still has its width.
        let narrowest = solve(WINDOW_MIN_WIDTH, 800., true, &layout);
        assert_eq!(narrowest.transcript.width, TRANSCRIPT_MIN_WIDTH);
        assert_eq!(narrowest.right.unwrap().width, WINDOW_MIN_WIDTH - TRANSCRIPT_MIN_WIDTH);
        // Larger again: the same 600, because the preference was never rewritten.
        assert_eq!(solve(1600., 800., true, &layout), wide);
    }

    #[test]
    fn splits_share_by_weight_and_keep_their_minimum() {
        assert_eq!(boundaries(900., &[1., 2.], 100.), vec![0., 300., 900.]);
        assert_eq!(boundaries(900., &[1., 100.], 180.), vec![0., 180., 900.], "a tiny weight still gets its minimum");
        assert_eq!(boundaries(300., &[1., 1., 1., 1.], 180.), vec![0., 75., 150., 225., 300.], "no room: equal shares");

        let mut layout = Layout::default();
        let a = layout.add(PaneKind::Terminal);
        let b = layout.add(PaneKind::Terminal);
        layout.move_pane(b, Target::Beside(a, Edge::Right));
        let solved = solve(1400., 900., false, &layout);
        assert_eq!(solved.dividers.len(), 1);
        let divider = &solved.dividers[0];
        assert_eq!(divider.before + divider.after, solved.bottom.unwrap().width);
    }

    #[test]
    fn hidden_docks_take_no_room() {
        let mut layout = Layout::default();
        layout.toggle(PaneKind::Terminal);
        layout.toggle(PaneKind::Terminal);
        let solved = solve(1200., 800., true, &layout);
        assert!(solved.bottom.is_none() && solved.stacks.is_empty());
        assert_eq!(solved.transcript.height, 800.);
    }
}
