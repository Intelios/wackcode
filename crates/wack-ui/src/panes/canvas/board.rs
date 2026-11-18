//! The board's geometry, as plain data: where each artboard sits, and the camera over them.
//!
//! Ported from version 2 (`src/canvas-layout.ts`, `src/canvas-camera.ts`), whose rules held:
//! - Each group is a row, in the order groups first appear; within a row, artboards run left to
//!   right in creation order. Adding an artboard never moves the ones already there sideways.
//! - An auto-height artboard is drawn at its last measured height, or `AUTO_HEIGHT` until it has
//!   one.
//! - Screen = world × zoom + offset. Zoom stays within `MIN_ZOOM`…`MAX_ZOOM`, and zooming keeps
//!   the point under the pointer still.
//!
//! Everything here is pure and tested; the pane draws what it returns.

use std::collections::HashMap;

use wack_canvas::Canvas;

pub const ARTBOARD_GAP: f32 = 64.;
pub const ROW_GAP: f32 = 120.;
/// Room above a row for its group's name.
pub const GROUP_BAND: f32 = 44.;
/// Room above an artboard for its own name.
pub const LABEL_BAND: f32 = 30.;
pub const AUTO_HEIGHT: f32 = 600.;
pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 4.;
pub const ZOOM_STEP: f32 = 1.25;
/// Below this zoom only the names show, not each artboard's details.
pub const FAR_ZOOM: f32 = 0.25;
/// Space kept around what a fit shows, in screen points.
pub const FIT_PADDING: f32 = 48.;

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

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.right() && y >= self.y && y <= self.bottom()
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right() && other.x < self.right() && self.y < other.bottom() && other.y < self.bottom()
    }

    pub fn union(&self, other: &Rect) -> Rect {
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        Rect { x, y, width: self.right().max(other.right()) - x, height: self.bottom().max(other.bottom()) - y }
    }
}

/// One artboard's frame on the board, in world points (one point per CSS pixel).
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub id: String,
    pub rect: Rect,
}

/// A group's name, over its row.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupLabel {
    pub name: String,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Board {
    pub frames: Vec<Frame>,
    pub groups: Vec<GroupLabel>,
}

impl Board {
    pub fn frame(&self, id: &str) -> Option<&Frame> {
        self.frames.iter().find(|frame| frame.id == id)
    }

    /// Everything on the board, or `None` when it is empty.
    pub fn bounds(&self) -> Option<Rect> {
        let mut frames = self.frames.iter().map(|frame| with_label(frame.rect));
        let first = frames.next()?;
        Some(frames.fold(first, |all, rect| all.union(&rect)))
    }

    /// The bounds of the artboards named.
    pub fn bounds_of(&self, ids: &[String]) -> Option<Rect> {
        let mut frames = self.frames.iter().filter(|frame| ids.contains(&frame.id)).map(|frame| with_label(frame.rect));
        let first = frames.next()?;
        Some(frames.fold(first, |all, rect| all.union(&rect)))
    }

    /// The artboard under a world point: the frame, not its label.
    pub fn at(&self, x: f32, y: f32) -> Option<&Frame> {
        self.frames.iter().rev().find(|frame| frame.rect.contains(x, y))
    }

    /// The nearest artboard from `from` in a direction (`dx`, `dy` each -1, 0 or 1), as the arrow
    /// keys move: in the direction's half-plane, nearest by centre with the cross axis weighed
    /// double.
    pub fn neighbour(&self, from: &str, dx: i32, dy: i32) -> Option<&Frame> {
        let origin = self.frame(from)?.rect;
        let (ox, oy) = (origin.x + origin.width / 2., origin.y + origin.height / 2.);
        self.frames
            .iter()
            .filter(|frame| frame.id != from)
            .filter_map(|frame| {
                let (cx, cy) = (frame.rect.x + frame.rect.width / 2., frame.rect.y + frame.rect.height / 2.);
                let (ax, ay) = (cx - ox, cy - oy);
                let along = ax * dx as f32 + ay * dy as f32;
                if along <= 0. {
                    return None;
                }
                let across = if dx != 0 { ay.abs() } else { ax.abs() };
                // Within a cone around the arrow, so ← never jumps to the row below.
                if across > 2. * along {
                    return None;
                }
                Some((along + 2. * across, frame))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, frame)| frame)
    }
}

fn with_label(rect: Rect) -> Rect {
    Rect { y: rect.y - LABEL_BAND, height: rect.height + LABEL_BAND, ..rect }
}

/// Lays a canvas out: groups as rows, artboards in creation order. `measured` gives auto-height
/// artboards their drawn height.
pub fn layout(canvas: &Canvas, measured: &HashMap<String, f32>) -> Board {
    let mut rows: Vec<(Option<String>, Vec<&wack_canvas::Artboard>)> = Vec::new();
    for artboard in &canvas.artboards {
        match rows.iter_mut().find(|(group, _)| *group == artboard.group) {
            Some((_, members)) => members.push(artboard),
            None => rows.push((artboard.group.clone(), vec![artboard])),
        }
    }
    let mut board = Board::default();
    let mut y = 0.;
    for (group, members) in rows {
        if let Some(name) = group {
            board.groups.push(GroupLabel { name, x: 0., y });
            y += GROUP_BAND;
        }
        y += LABEL_BAND;
        let mut x = 0.;
        let mut tallest: f32 = 0.;
        for artboard in members {
            let height = artboard.height.map(|h| h as f32).or_else(|| measured.get(&artboard.id).copied()).unwrap_or(AUTO_HEIGHT);
            let rect = Rect { x, y, width: artboard.width as f32, height };
            board.frames.push(Frame { id: artboard.id.clone(), rect });
            x += rect.width + ARTBOARD_GAP;
            tallest = tallest.max(height);
        }
        y += tallest + ROW_GAP;
    }
    board
}

/// Where the world is drawn: screen = world × zoom + (x, y), in the pane's own points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { x: FIT_PADDING, y: FIT_PADDING, zoom: 1. }
    }
}

impl Camera {
    pub fn to_screen(&self, rect: Rect) -> Rect {
        Rect { x: rect.x * self.zoom + self.x, y: rect.y * self.zoom + self.y, width: rect.width * self.zoom, height: rect.height * self.zoom }
    }

    pub fn to_world(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.x) / self.zoom, (y - self.y) / self.zoom)
    }

    /// Zooms by `factor` keeping the screen point (`px`, `py`) still.
    pub fn zoom_at(&self, px: f32, py: f32, factor: f32) -> Camera {
        let zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let (wx, wy) = self.to_world(px, py);
        Camera { x: px - wx * zoom, y: py - wy * zoom, zoom }
    }

    /// Sets the zoom, keeping the screen point (`px`, `py`) still.
    pub fn zoom_to(&self, px: f32, py: f32, zoom: f32) -> Camera {
        self.zoom_at(px, py, zoom / self.zoom)
    }

    pub fn pan(&self, dx: f32, dy: f32) -> Camera {
        Camera { x: self.x + dx, y: self.y + dy, ..*self }
    }

    /// The world rectangle the pane shows.
    pub fn view(&self, width: f32, height: f32) -> Rect {
        let (x, y) = self.to_world(0., 0.);
        Rect { x, y, width: width / self.zoom, height: height / self.zoom }
    }
}

/// The camera that shows `bounds` whole in a `width` × `height` pane, never past `max_zoom`.
pub fn fit(bounds: Rect, width: f32, height: f32, max_zoom: f32) -> Camera {
    let room_w = (width - 2. * FIT_PADDING).max(1.);
    let room_h = (height - 2. * FIT_PADDING).max(1.);
    let zoom = (room_w / bounds.width.max(1.)).min(room_h / bounds.height.max(1.)).min(max_zoom).clamp(MIN_ZOOM, MAX_ZOOM);
    Camera {
        x: (width - bounds.width * zoom) / 2. - bounds.x * zoom,
        y: (height - bounds.height * zoom) / 2. - bounds.y * zoom,
        zoom,
    }
}

/// The step after `zoom` in the direction of `inwards`, on the 1.25× ladder.
pub fn step(zoom: f32, inwards: bool) -> f32 {
    if inwards { (zoom * ZOOM_STEP).min(MAX_ZOOM) } else { (zoom / ZOOM_STEP).max(MIN_ZOOM) }
}

/// Moves `from` towards `to` by `t` (0…1), for a camera glide.
pub fn between(from: Camera, to: Camera, t: f32) -> Camera {
    if t <= 0. {
        return from;
    }
    if t >= 1. {
        return to;
    }
    // Zoom moves geometrically, so a glide from far out feels even.
    let zoom = (from.zoom.ln() + (to.zoom.ln() - from.zoom.ln()) * t).exp();
    Camera { x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t, zoom }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn canvas() -> Canvas {
        wack_canvas::model::plan(&[], &json!({ "title": "T", "artboards": [
            { "name": "A", "group": "One", "width": 400, "height": 300, "html": "<p/>" },
            { "name": "B", "group": "Two", "width": 200, "html": "<p/>" },
            { "name": "C", "group": "One", "width": 500, "height": 200, "html": "<p/>" }
        ] }))
        .unwrap()
        .canvas
    }

    #[test]
    fn groups_are_rows_in_first_appearance_order() {
        let board = layout(&canvas(), &HashMap::new());
        let a = board.frame("a").unwrap().rect;
        let c = board.frame("c").unwrap().rect;
        let b = board.frame("b").unwrap().rect;
        assert_eq!((a.x, a.y), (0., GROUP_BAND + LABEL_BAND));
        assert_eq!((c.x, c.y), (400. + ARTBOARD_GAP, a.y), "C joins its group's row");
        assert_eq!(b.y, a.y + 300. + ROW_GAP + GROUP_BAND + LABEL_BAND, "the next row is under the tallest");
        assert_eq!(b.height, AUTO_HEIGHT);
        let measured = HashMap::from([("b".to_string(), 900.)]);
        assert_eq!(layout(&canvas(), &measured).frame("b").unwrap().rect.height, 900.);
        assert_eq!(board.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["One", "Two"]);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let camera = Camera { x: 30., y: -10., zoom: 0.5 };
        let before = camera.to_world(200., 150.);
        let zoomed = camera.zoom_at(200., 150., 3.);
        let after = zoomed.to_world(200., 150.);
        assert!((before.0 - after.0).abs() < 0.001 && (before.1 - after.1).abs() < 0.001);
        assert_eq!(camera.zoom_at(0., 0., 1000.).zoom, MAX_ZOOM);
        assert_eq!(camera.zoom_at(0., 0., 0.0001).zoom, MIN_ZOOM);
    }

    #[test]
    fn a_fit_shows_everything_and_never_zooms_past_its_limit() {
        let bounds = Rect { x: 100., y: 50., width: 2000., height: 1000. };
        let camera = fit(bounds, 1000., 800., 1.);
        let shown = camera.to_screen(bounds);
        assert!(shown.x >= FIT_PADDING - 0.5 && shown.right() <= 1000. - FIT_PADDING + 0.5, "{shown:?}");
        assert!(shown.y >= 0. && shown.bottom() <= 800.);
        assert_eq!(fit(Rect { x: 0., y: 0., width: 10., height: 10. }, 1000., 800., 1.).zoom, 1.);
    }

    #[test]
    fn arrows_move_to_the_nearest_neighbour() {
        let board = layout(&canvas(), &HashMap::new());
        assert_eq!(board.neighbour("a", 1, 0).unwrap().id, "c");
        assert_eq!(board.neighbour("a", 0, 1).unwrap().id, "b");
        assert!(board.neighbour("a", -1, 0).is_none());
        assert_eq!(board.at(10., GROUP_BAND + LABEL_BAND + 10.).unwrap().id, "a");
        assert!(board.at(-5., -5.).is_none());
    }

    #[test]
    fn a_glide_ends_where_it_was_going() {
        let (from, to) = (Camera { x: 0., y: 0., zoom: 0.1 }, Camera { x: 100., y: 50., zoom: 2. });
        assert_eq!(between(from, to, 0.), from);
        let end = between(from, to, 1.);
        assert!((end.zoom - 2.).abs() < 1e-5 && end.x == 100.);
    }
}
