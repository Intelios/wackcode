//! The image backdrop: the user's picture behind the window, and the veil that keeps text off it.
//!
//! The treatment is the app's own: the picture is never blurred or dimmed into mud behind text.
//! Instead every place text lives is an *island* of solid window colour (the title strip, the
//! reading column, the new-chat block), and each island's edge dissolves into the picture
//! through an ordered dither of small square cells, like a print's halftone. The picture shows
//! cleanly in the open space the composition already leaves to the right.
//!
//! Invariants:
//! - Inside an island is solid; farther than `BAND` from every island is clear; in between, a
//!   cell is solid when its coverage beats its Bayer threshold, so the density falls smoothly
//!   with distance and the pattern is stable from frame to frame (no shimmer while resizing).
//! - Cells sit on a grid fixed to the window, so moving an island never makes the pattern crawl.
//!   Islands grow to whole cells, so no sliver of picture shows along an edge.
//! - The picture covers the whole window, not just the area drawn, so it stays put when the
//!   sidebar comes and goes.
//! - The veil is pure geometry ([`cells`]) and tested; [`Veil`] keeps it between frames, so it
//!   is worked out again only when the window's shape changes.

use std::rc::Rc;

use gpui::{Bounds, Hsla, ObjectFit, Pixels, SharedString, Window, canvas, div, fill, img, point, prelude::*, px, size};

use crate::panes::Rect;

/// One dither cell, in points.
pub const CELL: f32 = 3.;
/// How far a dissolve reaches from an island's edge, in points.
pub const BAND: f32 = 96.;

/// The 8×8 Bayer matrix: each cell's threshold, spread so neighbouring thresholds differ most.
const BAYER: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// How far `(x, y)` is from the nearest island; 0 inside one.
fn distance(islands: &[Rect], x: f32, y: f32) -> f32 {
    islands
        .iter()
        .map(|island| {
            let dx = (island.x - x).max(x - island.right()).max(0.);
            let dy = (island.y - y).max(y - island.bottom()).max(0.);
            (dx * dx + dy * dy).sqrt()
        })
        .fold(f32::INFINITY, f32::min)
}

/// The solid parts of the veil over `area`: the islands (clipped to it) and the dither cells
/// around them, with neighbouring cells in a row joined into one rectangle.
pub fn cells(area: Rect, islands: &[Rect]) -> Vec<Rect> {
    // Each island grows to whole cells, so a cell is either inside one or wholly outside: no
    // sliver of picture is left along an edge that falls part-way through a cell.
    let islands: Vec<Rect> = islands
        .iter()
        .map(|island| {
            let x = (island.x / CELL).floor() * CELL;
            let y = (island.y / CELL).floor() * CELL;
            Rect { x, y, width: (island.right() / CELL).ceil() * CELL - x, height: (island.bottom() / CELL).ceil() * CELL - y }
        })
        .collect();
    let islands = islands.as_slice();
    let mut solid: Vec<Rect> = islands
        .iter()
        .filter_map(|island| {
            let x = island.x.max(area.x);
            let y = island.y.max(area.y);
            let right = island.right().min(area.right());
            let bottom = island.bottom().min(area.bottom());
            (right > x && bottom > y).then_some(Rect { x, y, width: right - x, height: bottom - y })
        })
        .collect();
    if solid.is_empty() {
        return solid;
    }
    // Cells on the window's grid, covering the area.
    let first_column = (area.x / CELL).floor() as i64;
    let first_row = (area.y / CELL).floor() as i64;
    let last_column = (area.right() / CELL).ceil() as i64;
    let last_row = (area.bottom() / CELL).ceil() as i64;
    for row in first_row..last_row {
        let top = row as f32 * CELL;
        let mut run: Option<(f32, f32)> = None;
        for column in first_column..last_column {
            let left = column as f32 * CELL;
            let d = distance(islands, left + CELL / 2., top + CELL / 2.);
            let on = d > 0. && d < BAND && {
                // Smoothstep: a soft start and end to the dissolve.
                let t = 1. - d / BAND;
                let coverage = t * t * (3. - 2. * t);
                let threshold = (BAYER[row.rem_euclid(8) as usize][column.rem_euclid(8) as usize] as f32 + 0.5) / 64.;
                coverage > threshold
            };
            match (on, &mut run) {
                (true, Some((_, end))) => *end = left + CELL,
                (true, None) => run = Some((left, left + CELL)),
                (false, Some((start, end))) => {
                    solid.push(Rect { x: *start, y: top, width: *end - *start, height: CELL });
                    run = None;
                }
                (false, None) => {}
            }
        }
        if let Some((start, end)) = run {
            solid.push(Rect { x: start, y: top, width: end - start, height: CELL });
        }
    }
    // Cells at the area's edge are cut to it.
    solid.retain_mut(|rect| {
        let x = rect.x.max(area.x);
        let y = rect.y.max(area.y);
        let right = rect.right().min(area.right());
        let bottom = rect.bottom().min(area.bottom());
        *rect = Rect { x, y, width: right - x, height: bottom - y };
        rect.width > 0. && rect.height > 0.
    });
    solid
}

/// The picture drawn to cover `frame`, keeping `focus` (0–1 across and down) in view.
pub fn cover(frame: Rect, picture: (u32, u32), focus: (f32, f32)) -> Rect {
    let aspect = picture.0.max(1) as f32 / picture.1.max(1) as f32;
    let (width, height) = if frame.width / frame.height.max(1.) > aspect {
        (frame.width, frame.width / aspect)
    } else {
        (frame.height * aspect, frame.height)
    };
    Rect {
        x: frame.x - (width - frame.width) * focus.0.clamp(0., 1.),
        y: frame.y - (height - frame.height) * focus.1.clamp(0., 1.),
        width,
        height,
    }
}

/// What the shell needs to draw a backdrop.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub path: SharedString,
    pub width: u32,
    pub height: u32,
    pub focus: (f32, f32),
    /// 0–1: how far the picture is toned towards the window's colour.
    pub veil: f32,
}

/// The veil's cells, worked out again only when the area or the islands change: a window that
/// is not being resized costs nothing per frame.
#[derive(Default)]
pub struct Veil {
    key: Option<(Rect, Vec<Rect>)>,
    cells: Rc<Vec<Rect>>,
}

impl Veil {
    pub fn cells(&mut self, area: Rect, islands: Vec<Rect>) -> Rc<Vec<Rect>> {
        if self.key.as_ref() != Some(&(area, islands.clone())) {
            self.cells = Rc::new(cells(area, &islands));
            self.key = Some((area, islands));
        }
        self.cells.clone()
    }
}

/// The backdrop over `area` of the window, both in window coordinates: the picture covering the
/// whole `window` (so it stays put when the sidebar comes and goes), its tone, and the veil's
/// `solid` cells from [`Veil::cells`]. `ground` is the window's colour.
pub fn view(picture: Picture, window: Rect, area: Rect, solid: Rc<Vec<Rect>>, ground: Hsla) -> impl IntoElement {
    let drawn = cover(window, (picture.width, picture.height), picture.focus);
    div()
        .absolute()
        .left_0()
        .top_0()
        .w(px(area.width))
        .h(px(area.height))
        .overflow_hidden()
        .bg(ground)
        .child(
            img(std::path::PathBuf::from(picture.path.as_ref()))
                .absolute()
                .left(px(drawn.x - area.x))
                .top(px(drawn.y - area.y))
                .w(px(drawn.width))
                .h(px(drawn.height))
                .object_fit(ObjectFit::Fill),
        )
        .when(picture.veil > 0., |backdrop| {
            backdrop.child(div().absolute().left_0().top_0().size_full().bg(Hsla { a: picture.veil.clamp(0., 0.9), ..ground }))
        })
        .child(
            canvas(
                |_, _, _| {},
                move |bounds: Bounds<Pixels>, _, window: &mut Window, _| {
                    for rect in solid.iter() {
                        let origin = point(bounds.origin.x + px(rect.x - area.x), bounds.origin.y + px(rect.y - area.y));
                        window.paint_quad(fill(Bounds::new(origin, size(px(rect.width), px(rect.height))), ground));
                    }
                },
            )
            .absolute()
            .left_0()
            .top_0()
            .size_full(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covered(solid: &[Rect], x: f32, y: f32) -> bool {
        solid.iter().any(|rect| x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom())
    }

    fn density(solid: &[Rect], x: f32, y: f32, span: f32) -> f32 {
        let mut on = 0;
        let mut all = 0;
        let mut sample_y = y;
        while sample_y < y + span {
            let mut sample_x = x;
            while sample_x < x + span {
                all += 1;
                if covered(solid, sample_x + 0.5, sample_y + 0.5) {
                    on += 1;
                }
                sample_x += CELL;
            }
            sample_y += CELL;
        }
        on as f32 / all as f32
    }

    #[test]
    fn islands_are_solid_and_fade_into_the_picture() {
        let area = Rect { x: 0., y: 0., width: 1200., height: 800. };
        let column = Rect { x: 0., y: 0., width: 600., height: 800. };
        let solid = cells(area, &[column]);
        assert!(covered(&solid, 300., 400.), "inside the island");
        assert!(!covered(&solid, 600. + BAND + 10., 400.), "past the band is clear");
        let near = density(&solid, 606., 300., 24.);
        let middle = density(&solid, 600. + BAND / 2. - 12., 300., 24.);
        let far = density(&solid, 600. + BAND - 30., 300., 24.);
        assert!(near > middle && middle > far, "density falls with distance: {near} {middle} {far}");
        assert!(near > 0.8 && far < 0.2);
        // An edge part-way through a cell leaves no sliver of picture beside it.
        let ragged = cells(area, &[Rect { width: 601.5, ..column }]);
        assert!((0..800).step_by(3).all(|y| covered(&ragged, 601.6, y as f32 + 0.5)));
        for rect in &solid {
            assert!(rect.x >= area.x && rect.right() <= area.right() + 0.001 && rect.y >= 0. && rect.bottom() <= 800.001);
        }
    }

    #[test]
    fn the_pattern_stays_put_when_an_island_moves() {
        let area = Rect { x: 0., y: 0., width: 900., height: 300. };
        let a = cells(area, &[Rect { x: 0., y: 0., width: 400., height: 300. }]);
        let b = cells(area, &[Rect { x: 0., y: 0., width: 401., height: 300. }]);
        // Cells far from both edges agree: the grid belongs to the window, not the island.
        for x in [300., 520., 560.] {
            for y in [30., 150.] {
                let near_edge = |solid: &Vec<Rect>| covered(solid, x, y);
                if (x - 400.).abs() > 12. {
                    assert_eq!(near_edge(&a), near_edge(&b), "at {x},{y}");
                }
            }
        }
        assert!(cells(area, &[]).is_empty(), "no islands, no veil");
    }

    #[test]
    fn the_picture_covers_and_keeps_its_focus() {
        let frame = Rect { x: 0., y: 0., width: 1000., height: 500. };
        let tall = cover(frame, (1000, 1000), (0.5, 0.));
        assert_eq!((tall.width, tall.height, tall.y), (1000., 1000., 0.));
        let bottom = cover(frame, (1000, 1000), (0.5, 1.));
        assert_eq!(bottom.y, -500.);
        let wide = cover(frame, (4000, 1000), (0., 0.5));
        assert_eq!((wide.width, wide.height, wide.x), (2000., 500., 0.));
    }
}
