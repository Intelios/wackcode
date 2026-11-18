//! The Canvas pane: the chat's canvases, each a board of the agent's artboards to pan, zoom,
//! compare, play, comment on and tweak.
//!
//! - The pane only views and steers: canvases belong to the chat's `Canvases` (`wack-canvas`), and
//!   their pictures to its store, so closing the pane, moving it or switching chats loses nothing.
//! - The board is drawn here, in GPUI, from pictures of the artboards (the owner's call): panning
//!   and zooming never wait on Chromium, and glass, labels and pills draw over it freely. Only an
//!   artboard being played is a live page (`play.rs`), placed over the pane like the Browser pane's.
//! - A picture shown is the newest the store has, which may be a revision behind while the next
//!   is drawn; the label says so. At a distance the board draws the small copy, so a large picture
//!   never shimmers.
//! - The camera follows the agent's changes until the user moves it; after that, an arrival pill
//!   offers to go and look. Glides and arrival rings follow Reduce Motion.
//! - What the pane caches (the canvases, the pictures it found) is refreshed only when the chat's
//!   canvases signal (`refresh`), never read from disk while drawing a frame.

pub mod board;

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    Bounds, Context, FocusHandle, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, PinchEvent, Pixels, ScrollDelta,
    ScrollWheelEvent, SharedString, Window, deferred, div, img, prelude::*, px,
};
use gpui_tokio::Tokio;
use wack_canvas::{Canvas, Canvases};

use self::board::{Board, Camera, FAR_ZOOM, Rect};
use crate::{
    actions::{CanvasActualSize, CanvasEscape, CanvasFit, CanvasMove, CanvasZoomIn, CanvasZoomOut, CanvasZoomSelection, WriteInComposer},
    icon::Icon,
    tags::Tag,
    theme::Theme,
    tooltip::tip,
    typography::{Type, Typography},
};

/// The key context the pane's own keys bind in.
pub const CANVAS_CONTEXT: &str = "CanvasPane";
const CIRCLE: f32 = 26.;
const PILL_HEIGHT: f32 = 32.;
const EDGE: f32 = 12.;
/// A press that moves less than this is a click, not a pan.
const CLICK_SLOP: f32 = 4.;
const GLIDE: Duration = Duration::from_millis(420);
const ARRIVAL: Duration = Duration::from_millis(1400);
/// On-screen width, in pixels, below which the small copy of a picture is drawn.
const THUMB_BELOW: f32 = 560.;

/// What the pane knows of one artboard's picture.
#[derive(Clone, Debug, Default)]
struct Shown {
    full: Option<PathBuf>,
    thumb: Option<PathBuf>,
    /// The revision the picture shows.
    rev: u64,
    /// Its drawn height, for an auto-height artboard.
    height: Option<f32>,
    errors: Vec<String>,
}

struct Glide {
    from: Camera,
    to: Camera,
    since: Instant,
}

struct Drag {
    /// Where the press was, in pane points.
    start: (f32, f32),
    camera: Camera,
    moved: bool,
    /// The artboard under the press, if any.
    on: Option<String>,
}

pub struct CanvasPane {
    canvases: Canvases,
    pub focus: FocusHandle,
    list: Vec<Canvas>,
    shown: HashMap<(String, String), Shown>,
    /// Revisions seen at the last refresh, to tell what is new.
    revs: HashMap<(String, String), u64>,
    current: Option<String>,
    cameras: HashMap<String, Camera>,
    selected: Option<String>,
    /// The camera goes to the agent's changes until the user moves it.
    following: bool,
    /// Changes the camera did not go to.
    unseen: Vec<String>,
    glide: Option<Glide>,
    arrivals: HashMap<String, (Instant, bool)>,
    drag: Option<Drag>,
    /// Drawings asked for, so a revision is asked for once.
    asked: HashSet<(String, String, u64)>,
    /// The pane's bounds in the window, from the last paint.
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    switcher: bool,
    /// The canvas to fit once the pane's size is known.
    fit_pending: Option<String>,
}

impl CanvasPane {
    pub fn new(canvases: Canvases, cx: &mut Context<Self>) -> Self {
        let mut pane = Self {
            canvases,
            focus: cx.focus_handle(),
            list: Vec::new(),
            shown: HashMap::new(),
            revs: HashMap::new(),
            current: None,
            cameras: HashMap::new(),
            selected: None,
            following: true,
            unseen: Vec::new(),
            glide: None,
            arrivals: HashMap::new(),
            drag: None,
            asked: HashSet::new(),
            bounds: Rc::default(),
            switcher: false,
            fit_pending: None,
        };
        pane.reload(false, cx);
        pane
    }

    /// The chat's canvases changed, or a picture was drawn.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.reload(true, cx);
        cx.notify();
    }

    fn reload(&mut self, announce: bool, cx: &mut Context<Self>) {
        let list = self.canvases.list();
        let mut changed: Vec<(String, String, bool)> = Vec::new();
        let mut revs = HashMap::new();
        for canvas in &list {
            for artboard in &canvas.artboards {
                let key = (canvas.id.clone(), artboard.id.clone());
                match self.revs.get(&key) {
                    None => changed.push((canvas.id.clone(), artboard.id.clone(), true)),
                    Some(rev) if *rev != artboard.rev => changed.push((canvas.id.clone(), artboard.id.clone(), false)),
                    _ => {}
                }
                revs.insert(key.clone(), artboard.rev);
                let picture = self.canvases.picture(&canvas.id, &artboard.id);
                let report = self.canvases.report(&canvas.id, &artboard.id);
                let shown = Shown {
                    full: picture.as_ref().map(|(path, _)| path.clone()),
                    thumb: self.canvases.thumbnail(&canvas.id, &artboard.id),
                    rev: picture.map_or(0, |(_, rev)| rev),
                    height: report.as_ref().map(|r| r.height as f32),
                    errors: report.map(|r| r.console.into_iter().filter(|line| line.starts_with("error")).collect()).unwrap_or_default(),
                };
                self.shown.insert(key, shown);
            }
        }
        // A new canvas the agent started comes to the front.
        let newest_new = list.iter().rev().find(|c| !self.list.iter().any(|old| old.id == c.id)).map(|c| c.id.clone());
        if self.current.as_ref().is_none_or(|id| !list.iter().any(|c| &c.id == id)) {
            self.current = list.last().map(|c| c.id.clone());
        }
        if announce && let Some(id) = newest_new {
            self.current = Some(id);
        }
        self.list = list;
        self.revs = revs;
        if let Some(current) = self.current.clone()
            && !self.cameras.contains_key(&current)
        {
            self.fit_pending = Some(current);
        }
        if announce {
            let here: Vec<(String, bool)> =
                changed.iter().filter(|(canvas, _, _)| Some(canvas) == self.current.as_ref()).map(|(_, id, new)| (id.clone(), *new)).collect();
            let now = Instant::now();
            for (id, new) in &here {
                self.arrivals.insert(id.clone(), (now, *new));
            }
            let ids: Vec<String> = here.into_iter().map(|(id, _)| id).collect();
            if !ids.is_empty() {
                if self.following {
                    self.go_to(&ids, cx);
                } else {
                    for id in ids {
                        if !self.unseen.contains(&id) {
                            self.unseen.push(id);
                        }
                    }
                }
            }
        }
        self.ask_for_pictures(cx);
    }

    /// Draws the current canvas's artboards that have no picture of their revision.
    fn ask_for_pictures(&mut self, cx: &mut Context<Self>) {
        let Some(canvas) = self.canvas().cloned() else { return };
        if !self.canvases.available() {
            return;
        }
        let wanted: Vec<String> = canvas
            .artboards
            .iter()
            .filter(|a| self.shown.get(&(canvas.id.clone(), a.id.clone())).is_none_or(|s| s.rev != a.rev))
            .filter(|a| !self.canvases.is_drawing(&canvas.id, &a.id))
            .filter(|a| self.asked.insert((canvas.id.clone(), a.id.clone(), a.rev)))
            .map(|a| a.id.clone())
            .collect();
        if wanted.is_empty() {
            return;
        }
        let canvases = self.canvases.clone();
        let id = canvas.id.clone();
        Tokio::spawn(cx, async move {
            canvases.draw(&id, &wanted).await;
        })
        .detach();
    }

    fn canvas(&self) -> Option<&Canvas> {
        let current = self.current.as_ref()?;
        self.list.iter().find(|c| &c.id == current)
    }

    fn board(&self) -> Board {
        let Some(canvas) = self.canvas() else { return Board::default() };
        let measured: HashMap<String, f32> = canvas
            .artboards
            .iter()
            .filter_map(|a| Some((a.id.clone(), self.shown.get(&(canvas.id.clone(), a.id.clone()))?.height?)))
            .collect();
        board::layout(canvas, &measured)
    }

    fn camera(&self) -> Camera {
        self.current.as_ref().and_then(|id| self.cameras.get(id)).copied().unwrap_or_default()
    }

    fn size(&self) -> (f32, f32) {
        self.bounds.get().map_or((800., 600.), |b| (f32::from(b.size.width), f32::from(b.size.height)))
    }

    fn set_camera(&mut self, camera: Camera) {
        if let Some(id) = self.current.clone() {
            self.cameras.insert(id, camera);
        }
        self.glide = None;
    }

    /// Moves the camera, gliding unless Reduce Motion is on.
    fn glide_to(&mut self, to: Camera, cx: &mut Context<Self>) {
        if cx.reduce_motion() {
            self.set_camera(to);
        } else {
            self.glide = Some(Glide { from: self.camera(), to, since: Instant::now() });
        }
        cx.notify();
    }

    fn go_to(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let (width, height) = self.size();
        if let Some(bounds) = self.board().bounds_of(ids) {
            let to = board::fit(bounds, width, height, 1.);
            self.glide_to(to, cx);
        }
    }

    fn fit_all(&mut self, cx: &mut Context<Self>) {
        let (width, height) = self.size();
        if let Some(bounds) = self.board().bounds() {
            self.following = true;
            self.unseen.clear();
            self.glide_to(board::fit(bounds, width, height, 1.), cx);
        }
    }

    /// The user moved the camera: it stops following the agent.
    fn moved_by_hand(&mut self) {
        self.following = false;
        self.glide = None;
    }

    fn local(&self, position: gpui::Point<Pixels>) -> (f32, f32) {
        let origin = self.bounds.get().map_or(gpui::point(px(0.), px(0.)), |b| b.origin);
        (f32::from(position.x - origin.x), f32::from(position.y - origin.y))
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = self.local(event.position);
        let delta = match event.delta {
            ScrollDelta::Pixels(point) => (f32::from(point.x), f32::from(point.y)),
            ScrollDelta::Lines(point) => (point.x * 24., point.y * 24.),
        };
        let camera = self.camera();
        let next = if event.modifiers.platform || event.modifiers.control {
            camera.zoom_at(x, y, (delta.1 * 0.0025).exp())
        } else {
            camera.pan(delta.0, delta.1)
        };
        self.moved_by_hand();
        self.set_camera(next);
        cx.notify();
    }

    fn on_pinch(&mut self, event: &PinchEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = self.local(event.position);
        let next = self.camera().zoom_at(x, y, 1. + event.delta);
        self.moved_by_hand();
        self.set_camera(next);
        cx.notify();
    }

    fn on_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.switcher = false;
        let (x, y) = self.local(event.position);
        let (wx, wy) = self.camera().to_world(x, y);
        let on = self.board().at(wx, wy).map(|frame| frame.id.clone());
        self.drag = Some(Drag { start: (x, y), camera: self.camera(), moved: false, on });
        cx.notify();
    }

    fn on_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = self.local(event.position);
        let Some(drag) = self.drag.as_mut() else { return };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }
        let (dx, dy) = (x - drag.start.0, y - drag.start.1);
        if !drag.moved && dx.hypot(dy) < CLICK_SLOP {
            return;
        }
        drag.moved = true;
        let next = drag.camera.pan(dx, dy);
        self.moved_by_hand();
        self.set_camera(next);
        cx.notify();
    }

    fn on_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else { return };
        if !drag.moved {
            self.selected = drag.on;
        }
        cx.notify();
    }

    fn zoom_by(&mut self, inwards: bool, cx: &mut Context<Self>) {
        let (width, height) = self.size();
        let camera = self.camera();
        let to = camera.zoom_to(width / 2., height / 2., board::step(camera.zoom, inwards));
        self.moved_by_hand();
        self.glide_to(to, cx);
    }

    fn zoom_to_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected.clone() {
            let (width, height) = self.size();
            if let Some(bounds) = self.board().bounds_of(&[id]) {
                self.moved_by_hand();
                self.glide_to(board::fit(bounds, width, height, 2.), cx);
            }
        }
    }

    fn actual_size(&mut self, cx: &mut Context<Self>) {
        let (width, height) = self.size();
        let to = self.camera().zoom_to(width / 2., height / 2., 1.);
        self.moved_by_hand();
        self.glide_to(to, cx);
    }

    fn step_selection(&mut self, dx: i32, dy: i32, cx: &mut Context<Self>) {
        let board = self.board();
        let next = match &self.selected {
            Some(from) => board.neighbour(from, dx, dy).map(|frame| frame.id.clone()),
            None => board.frames.first().map(|frame| frame.id.clone()),
        };
        if let Some(next) = next {
            self.selected = Some(next.clone());
            // Into view, at the zoom it is at.
            let (width, height) = self.size();
            let camera = self.camera();
            if let Some(frame) = board.frame(&next) {
                let screen = camera.to_screen(frame.rect);
                let inside = screen.x >= 0. && screen.y >= 0. && screen.right() <= width && screen.bottom() <= height;
                if !inside {
                    let to = Camera { x: width / 2. - (frame.rect.x + frame.rect.width / 2.) * camera.zoom, y: height / 2. - (frame.rect.y + frame.rect.height / 2.) * camera.zoom, ..camera };
                    self.moved_by_hand();
                    self.glide_to(to, cx);
                }
            }
            cx.notify();
        }
    }

    fn escape(&mut self, cx: &mut Context<Self>) {
        if self.switcher {
            self.switcher = false;
        } else {
            self.selected = None;
        }
        cx.notify();
    }

    fn choose(&mut self, id: String, cx: &mut Context<Self>) {
        self.current = Some(id.clone());
        self.selected = None;
        self.switcher = false;
        self.unseen.clear();
        if !self.cameras.contains_key(&id) {
            self.fit_pending = Some(id);
        }
        self.ask_for_pictures(cx);
        cx.notify();
    }
}

fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

impl Render for CanvasPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let still = cx.reduce_motion();
        let (width, height) = self.size();

        // A fit waiting for the pane's size, and a glide under way.
        if let Some(id) = self.fit_pending.clone()
            && self.bounds.get().is_some()
            && Some(&id) == self.current.as_ref()
        {
            self.fit_pending = None;
            if let Some(bounds) = self.board().bounds() {
                self.cameras.insert(id, board::fit(bounds, width, height, 1.));
            }
        }
        let mut animating = false;
        if let Some(glide) = &self.glide {
            let t = glide.since.elapsed().as_secs_f32() / GLIDE.as_secs_f32();
            let camera = board::between(glide.from, glide.to, ease_out(t));
            if let Some(id) = self.current.clone() {
                self.cameras.insert(id, camera);
            }
            if t >= 1. {
                self.glide = None;
            } else {
                animating = true;
            }
        }
        self.arrivals.retain(|_, (since, _)| since.elapsed() < ARRIVAL);
        if !self.arrivals.is_empty() && !still {
            animating = true;
        }
        if animating {
            window.request_animation_frame();
        }

        let camera = self.camera();
        let board = self.board();
        let canvas = self.canvas().cloned();
        let view = camera.view(width, height);
        let far = camera.zoom < FAR_ZOOM;
        let bounds_cell = self.bounds.clone();

        let mut layer = div().absolute().inset_0();
        if let Some(canvas) = &canvas {
            for group in &board.groups {
                let (x, y) = (group.x * camera.zoom + camera.x, group.y * camera.zoom + camera.y);
                if y < -40. || y > height + 40. {
                    continue;
                }
                layer = layer.child(
                    div().absolute().left(px(x)).top(px(y)).typography(Type::Label).text_color(theme.text_soft).whitespace_nowrap().child(group.name.clone()),
                );
            }
            for frame in &board.frames {
                // Only what is near the view is drawn.
                let near = Rect { x: view.x - 200., y: view.y - 200., width: view.width + 400., height: view.height + 400. };
                if !frame.rect.intersects(&near) {
                    continue;
                }
                let Some(artboard) = canvas.artboards.iter().find(|a| a.id == frame.id) else { continue };
                let shown = self.shown.get(&(canvas.id.clone(), frame.id.clone())).cloned().unwrap_or_default();
                let drawing = self.canvases.is_drawing(&canvas.id, &frame.id) || shown.rev != artboard.rev;
                let screen = camera.to_screen(frame.rect);
                let selected = self.selected.as_deref() == Some(frame.id.as_str());
                let picture = if screen.width * window.scale_factor() < THUMB_BELOW { shown.thumb.clone().or(shown.full.clone()) } else { shown.full.clone().or(shown.thumb.clone()) };
                let arrival = (!still).then(|| self.arrivals.get(&frame.id)).flatten().map(|(since, new)| {
                    let t = since.elapsed().as_secs_f32() / ARRIVAL.as_secs_f32();
                    (1. - t.clamp(0., 1.), *new)
                });

                let mut card = div()
                    .absolute()
                    .left(px(screen.x))
                    .top(px(screen.y))
                    .w(px(screen.width.max(1.)))
                    .h(px(screen.height.max(1.)))
                    .rounded(px((4. * camera.zoom).clamp(1., 6.)))
                    .overflow_hidden()
                    .bg(theme.raised)
                    .border_1()
                    .border_color(theme.border_strong)
                    .shadow_md();
                card = match picture {
                    Some(path) => card.child(img(path).size_full().object_fit(ObjectFit::Fill)),
                    None => card.flex().items_center().justify_center().when(!far, |card| {
                        card.typography(Type::Small).text_color(theme.text_faint).child(if self.canvases.available() { "Drawing…" } else { "No picture yet" })
                    }),
                };
                layer = layer.child(card);
                if selected || arrival.is_some() {
                    let (ring, alpha) = match arrival {
                        Some((left, _)) if !selected => (theme.accent, left),
                        _ => (theme.accent_line, 1.),
                    };
                    layer = layer.child(
                        div()
                            .absolute()
                            .left(px(screen.x - 3.))
                            .top(px(screen.y - 3.))
                            .w(px(screen.width + 6.))
                            .h(px(screen.height + 6.))
                            .rounded(px(8.))
                            .border_2()
                            .border_color(Hsla { a: ring.a * alpha, ..ring }),
                    );
                }
                // The name, at one size however far out the board is.
                let label = div()
                    .absolute()
                    .left(px(screen.x))
                    .top(px(screen.y - 22.))
                    .max_w(px(screen.width.max(80.)))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .overflow_hidden()
                    .child(
                        div()
                            .typography(Type::Small)
                            .text_color(if selected { theme.text } else { theme.text_soft })
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(artboard.name.clone()),
                    )
                    .when(!far, |label| {
                        let size = match artboard.height {
                            Some(h) => format!("{} × {h}", artboard.width),
                            None => format!("{} × {}", artboard.width, shown.height.map_or("auto".to_string(), |h| format!("{h:.0}"))),
                        };
                        label
                            .child(div().typography(Type::Small).text_color(theme.text_faint).whitespace_nowrap().child(size))
                            .when(drawing, |label| label.child(div().typography(Type::Small).text_color(theme.text_dim).whitespace_nowrap().child("Drawing…")))
                            .when(!shown.errors.is_empty(), |label| {
                                let count = shown.errors.len();
                                label.child(
                                    div()
                                        .px(px(6.))
                                        .rounded_full()
                                        .bg(Hsla { a: 0.14, ..theme.danger })
                                        .typography(Type::Small)
                                        .text_color(theme.danger)
                                        .whitespace_nowrap()
                                        .child(if count == 1 { "1 error".to_string() } else { format!("{count} errors") }),
                                )
                            })
                    });
                layer = layer.child(label);
            }
        }

        let pane = cx.entity();
        let root = div()
            .id("canvas-pane")
            .track_focus(&self.focus)
            .key_context(CANVAS_CONTEXT)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.window)
            .child(
                gpui::canvas(
                    move |bounds, _, _| {
                        bounds_cell.set(Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_pinch(cx.listener(Self::on_pinch))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_up))
            .on_action(cx.listener(|pane, _: &CanvasFit, _, cx| pane.fit_all(cx)))
            .on_action(cx.listener(|pane, _: &CanvasZoomIn, _, cx| pane.zoom_by(true, cx)))
            .on_action(cx.listener(|pane, _: &CanvasZoomOut, _, cx| pane.zoom_by(false, cx)))
            .on_action(cx.listener(|pane, _: &CanvasZoomSelection, _, cx| pane.zoom_to_selection(cx)))
            .on_action(cx.listener(|pane, _: &CanvasActualSize, _, cx| pane.actual_size(cx)))
            .on_action(cx.listener(|pane, _: &CanvasEscape, _, cx| pane.escape(cx)))
            .on_action(cx.listener(|pane, action: &CanvasMove, _, cx| pane.step_selection(action.dx, action.dy, cx)))
            .child(layer);

        let Some(canvas) = canvas else {
            return root.child(empty(&theme, self.canvases.available())).tag("canvas-pane").into_any_element();
        };

        // Top left: which canvas, and the others.
        let many = self.list.len() > 1;
        let title = div()
            .id("canvas-switcher")
            .absolute()
            .top(px(EDGE))
            .left(px(EDGE))
            .max_w(px((width * 0.5).max(160.)))
            .h(px(PILL_HEIGHT))
            .px(px(14.))
            .rounded_full()
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border)
            .shadow_sm()
            .flex()
            .items_center()
            .gap(px(8.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .when(many, |pill| {
                let pane = pane.clone();
                pill.cursor_pointer().hover(|pill| pill.bg(theme.hover)).on_click(move |_, _, cx| {
                    pane.update(cx, |pane, cx| {
                        pane.switcher = !pane.switcher;
                        cx.notify();
                    })
                })
            })
            .child(div().typography(Type::UiStrong).text_color(theme.text).whitespace_nowrap().text_ellipsis().child(canvas.title.clone()))
            .child(div().typography(Type::Small).text_color(theme.text_dim).whitespace_nowrap().child(artboards(canvas.artboards.len())))
            .when(many, |pill| pill.child(Icon::ChevronDown.view(px(12.), theme.text_dim)))
            .tag("canvas-switcher");
        let switcher = (self.switcher && many).then(|| {
            let current = self.current.clone();
            deferred(
                div()
                    .id("canvas-switcher-menu")
                    .absolute()
                    .top(px(EDGE + PILL_HEIGHT + 6.))
                    .left(px(EDGE))
                    .min_w(px(220.))
                    .p(px(4.))
                    .rounded(px(12.))
                    .bg(theme.raised)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .children(self.list.iter().rev().map(|other| {
                        let picked = current.as_ref() == Some(&other.id);
                        let (pane, id) = (pane.clone(), other.id.clone());
                        div()
                            .id(SharedString::from(format!("canvas-choice-{}", other.id)))
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .hover(|row| row.bg(theme.hover))
                            .when(picked, |row| row.bg(theme.accent_wash))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(12.))
                            .on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.choose(id.clone(), cx)))
                            .child(div().typography(Type::Ui).text_color(if picked { theme.accent_text } else { theme.text }).child(other.title.clone()))
                            .child(div().typography(Type::Small).text_color(theme.text_dim).child(artboards(other.artboards.len())))
                    })),
            )
        });

        // Top right: the zoom.
        let zoom_pill = div()
            .absolute()
            .top(px(EDGE))
            .right(px(EDGE))
            .h(px(PILL_HEIGHT))
            .px(px(3.))
            .rounded_full()
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border)
            .shadow_sm()
            .flex()
            .items_center()
            .gap(px(2.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(circle("canvas-zoom-out", Icon::Minus, "Zoom out  ⌘−", &theme).on_click({
                let pane = pane.clone();
                move |_, _, cx| pane.update(cx, |pane, cx| pane.zoom_by(false, cx))
            }))
            .child(
                div()
                    .id("canvas-zoom-actual")
                    .min_w(px(44.))
                    .h(px(CIRCLE))
                    .px(px(4.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|zoom| zoom.bg(theme.hover))
                    .tooltip(tip("Actual size  ⌘0"))
                    .typography(Type::Small)
                    .text_color(theme.text_soft)
                    .on_click({
                        let pane = pane.clone();
                        move |_, _, cx| pane.update(cx, |pane, cx| pane.actual_size(cx))
                    })
                    .child(format!("{:.0}%", camera.zoom * 100.)),
            )
            .child(circle("canvas-zoom-in", Icon::Plus, "Zoom in  ⌘+", &theme).on_click({
                let pane = pane.clone();
                move |_, _, cx| pane.update(cx, |pane, cx| pane.zoom_by(true, cx))
            }))
            .child(
                div()
                    .id("canvas-fit")
                    .h(px(CIRCLE))
                    .px(px(10.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .hover(|fit| fit.bg(theme.hover))
                    .tooltip(tip("Fit the board  ⇧1"))
                    .typography(Type::Small)
                    .text_color(theme.text_soft)
                    .on_click({
                        let pane = pane.clone();
                        move |_, _, cx| pane.update(cx, |pane, cx| pane.fit_all(cx))
                    })
                    .child("Fit"),
            )
            .tag("canvas-zoom");

        // Bottom: what is new elsewhere, and the selected artboard.
        let arrivals = (!self.unseen.is_empty()).then(|| {
            let count = self.unseen.len();
            let pane = pane.clone();
            div()
                .id("canvas-arrivals")
                .absolute()
                .top(px(EDGE + PILL_HEIGHT + 10.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("canvas-arrivals-pill")
                        .h(px(28.))
                        .px(px(14.))
                        .rounded_full()
                        .bg(theme.accent)
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .hover(|pill| pill.bg(theme.accent_hover))
                        .typography(Type::UiStrong)
                        .text_color(theme.on_accent)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, _, cx| {
                            pane.update(cx, |pane, cx| {
                                let ids = std::mem::take(&mut pane.unseen);
                                pane.following = true;
                                pane.go_to(&ids, cx);
                            })
                        })
                        .child(if count == 1 { "1 new or revised artboard".to_string() } else { format!("{count} new or revised artboards") }),
                )
                .tag("canvas-arrivals")
        });
        let hint = self.selected.as_ref().and_then(|id| canvas.artboards.iter().find(|a| &a.id == id)).map(|artboard| {
            let shown = self.shown.get(&(canvas.id.clone(), artboard.id.clone())).cloned().unwrap_or_default();
            let mut facts = vec![artboard.name.clone()];
            facts.push(match artboard.height {
                Some(h) => format!("{} × {h}", artboard.width),
                None => format!("{} wide", artboard.width),
            });
            if let Some(group) = &artboard.group {
                facts.push(group.clone());
            }
            let errors = shown.errors.clone();
            let ask = (!errors.is_empty()).then(|| {
                let text = format!(
                    "The artboard {} on canvas “{}” (canvas: \"{}\") shows errors. Please fix them:\n{}",
                    artboard.id,
                    canvas.title,
                    canvas.id,
                    errors.iter().map(|e| format!("- {e}")).collect::<Vec<_>>().join("\n")
                );
                div()
                    .id("canvas-ask-fix")
                    .flex_none()
                    .h(px(24.))
                    .px(px(10.))
                    .rounded_full()
                    .bg(Hsla { a: 0.14, ..theme.danger })
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .typography(Type::Small)
                    .text_color(theme.danger)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| window.dispatch_action(Box::new(WriteInComposer { text: text.clone() }), cx))
                    .child("Ask to fix")
                    .tag("canvas-ask-fix")
            });
            div()
                .absolute()
                .bottom(px(EDGE))
                .left(px(EDGE))
                .max_w(px((width - 2. * EDGE).max(120.)))
                .px(px(14.))
                .py(px(8.))
                .rounded(px(16.))
                .bg(theme.raised)
                .border_1()
                .border_color(theme.border)
                .shadow_sm()
                .flex()
                .flex_col()
                .gap(px(2.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(div().typography(Type::Ui).text_color(theme.text).whitespace_nowrap().text_ellipsis().child(facts.join(" · ")))
                        .children(ask),
                )
                .when_some(artboard.notes.clone(), |hint, notes| hint.child(div().typography(Type::Small).text_color(theme.text_dim).child(notes)))
                .tag("canvas-selection")
        });

        root.child(title)
            .children(switcher)
            .child(zoom_pill)
            .children(arrivals)
            .children(hint)
            .tag("canvas-pane")
            .into_any_element()
    }
}

fn artboards(count: usize) -> String {
    if count == 1 { "1 artboard".into() } else { format!("{count} artboards") }
}

fn circle(id: &'static str, icon: Icon, tooltip: &'static str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(CIRCLE))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|circle| circle.bg(theme.hover))
        .tooltip(tip(tooltip))
        .child(icon.view(px(13.), theme.text_soft))
}

fn empty(theme: &Theme, available: bool) -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .px(px(32.))
        .child(div().typography(Type::Heading).text_color(theme.text).child("No canvas yet"))
        .child(
            div().max_w(px(360.)).text_center().typography(Type::Body).text_color(theme.text_dim).child(if available {
                "Ask the agent to explore a design, and its artboards appear here, side by side."
            } else {
                "Canvases need the browser engine, which this build of WackCode does not have."
            }),
        )
        .tag("canvas-empty")
}
