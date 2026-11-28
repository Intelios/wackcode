//! The Canvas pane: the chat's canvases, each a board of the agent's artboards to pan, zoom,
//! compare, play, comment on and tweak.
//!
//! - The pane only views and steers: canvases belong to the chat's `Canvases` (`wack-canvas`), and
//!   their pictures to its store, so closing the pane, moving it or switching chats loses nothing.
//! - The board is drawn here, in GPUI, from pictures of the artboards (the owner's call): panning
//!   and zooming never wait on Chromium, and glass, labels and pills draw over it freely.
//! - Only an artboard being played is a live page (`play`): a canvas page laid out at the
//!   artboard's size, scaled to the camera's zoom and placed exactly over its picture once the
//!   camera has arrived. The camera holds still while it plays, a pane resize fits and rescales
//!   it, and it is shown only while wholly inside the pane (a page is a native view, drawn over
//!   anything GPUI does not mask). The pane owns the page: Done, Esc, a press on the board, a
//!   revision (which reopens it) or the pane going away closes it.
//! - Tweaks (`tweaks.rs`) are the agent's levers on the selected or played artboard, moved with
//!   no model call. Play fits beside their panel, so the live page never covers it.
//! - Comments (`comments.rs`) are pinned with the Comment tool to elements found in a picture's
//!   element map, with no page opened; they go to the agent with the user's next message.
//! - Present shows pictures, not live pages: it draws over the whole window, and pages sit
//!   beneath GPUI's overlay plane.
//! - A picture shown is the newest the store has, which may be a revision behind while the next
//!   is drawn; the label says so. At a distance the board draws the small copy, so a large picture
//!   never shimmers.
//! - The camera follows the agent's changes until the user moves it; after that, an arrival pill
//!   offers to go and look. Glides and arrival rings follow Reduce Motion.
//! - What the pane caches (the canvases, the pictures it found) is refreshed only when the chat's
//!   canvases signal (`refresh`), never read from disk while drawing a frame.

pub mod board;
mod comments;
mod tweaks;

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
    actions::{CanvasActualSize, CanvasComment, CanvasEscape, CanvasFit, CanvasMove, CanvasPlay, CanvasPresent, CanvasZoomIn, CanvasZoomOut, CanvasZoomSelection, WriteInComposer},
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
const PLAY_RADIUS: f32 = 6.;
/// Room kept under a played artboard for its pill.
const PLAY_BAND: f32 = 56.;
/// About how wide the zoom pill draws (− 100% + Fit, Comment, Present).
const ZOOM_PILL: f32 = 234.;
/// The room a presented artboard keeps from the window's edges.
const PRESENT_MARGIN: f32 = 64.;
/// Below this pane width the pills say less.
const NARROW: f32 = 480.;
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
    /// What its report says needs fixing: console errors, then overflow or a stuck page. The
    /// same rule as the transcript card's "to fix" (`wack_canvas::tools::outcome`).
    problems: Vec<String>,
    /// The badge's words: "2 errors", "Overflows", "Stuck".
    badge: Option<String>,
    /// The picture's element map, which the Comment tool points into.
    map: Option<Rc<Vec<wack_canvas::Element>>>,
}

struct Glide {
    from: Camera,
    to: Camera,
    since: Instant,
}

/// An artboard being played: its live page placed over its picture, at the camera's zoom.
struct Playing {
    canvas: String,
    artboard: String,
    /// The page, once open; until then the picture shows with "Opening…".
    tab: Option<std::sync::Arc<wack_browser::Tab>>,
    /// The zoom the page is scaled to, and the pane size it was fitted in: the page is shown
    /// only while the camera is exactly there, so it never sits off its picture.
    zoom: f32,
    size: (f32, f32),
    failed: Option<String>,
    /// Bumped on every (re)open, so a late answer for an older page is closed, not shown.
    turn: u64,
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
    /// A zoom for that fit to end at instead, about the pane's middle (gallery scenes).
    fit_zoom: Option<f32>,
    playing: Option<Playing>,
    play_turns: u64,
    /// The selected artboard's export menu is open.
    export_menu: bool,
    /// What an export is doing, or how it ended: shown in the selection pill.
    export_note: Option<(String, bool)>,
    /// Presenting: the artboard shown, as an index into the board's order.
    presenting: Option<usize>,
    /// The Tweaks panel is open; it shows the played or selected artboard's tweaks.
    tweaks_open: bool,
    tweak_drag: Option<tweaks::TweakDrag>,
    /// A number mid-drag: (artboard, tweak, value), shown but not saved yet.
    tweak_preview: Option<(String, String, serde_json::Value)>,
    tweak_tracks: tweaks::Tracks,
    /// The revision the last tweak made, so a played page that already has it is not reopened.
    tweak_rev: Option<(String, String, u64)>,
    tweak_error: Option<String>,
    /// The Comment tool is on.
    commenting: bool,
    /// The element under the pointer while it is: (artboard, element).
    hover: Option<(String, wack_canvas::Element)>,
    region: Option<comments::Region>,
    editing: Option<comments::Editing>,
    /// Every canvas's comments, read when the canvases signal.
    comment_list: HashMap<String, Vec<wack_canvas::comments::Comment>>,
    /// Pins resolved while on screen, fading: (canvas, comment) and since when.
    fading: HashMap<(String, String), Instant>,
    /// The canvas call the model is writing now: the artboard and how much has come.
    writing: Option<crate::conversation::Writing>,
    /// Bumped as the camera moves, so only the last move of a burst is saved.
    camera_moves: u64,
    /// The cameras last kept on disk, by canvas.
    saved_cameras: HashMap<String, Camera>,
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
            fit_zoom: None,
            playing: None,
            play_turns: 0,
            export_menu: false,
            export_note: None,
            presenting: None,
            tweaks_open: false,
            tweak_drag: None,
            tweak_preview: None,
            tweak_tracks: Default::default(),
            tweak_rev: None,
            tweak_error: None,
            commenting: false,
            hover: None,
            region: None,
            editing: None,
            comment_list: HashMap::new(),
            fading: HashMap::new(),
            writing: None,
            camera_moves: 0,
            saved_cameras: HashMap::new(),
        };
        pane.reload(false, cx);
        pane
    }

    /// Brings a canvas to the front and goes to some of its artboards (all when `artboards` is
    /// empty), selecting the first. The camera then stays where the user asked to look.
    pub fn show(&mut self, canvas: &str, artboards: &[String], cx: &mut Context<Self>) {
        if !self.list.iter().any(|c| c.id == canvas) {
            self.reload(false, cx);
        }
        let Some(found) = self.list.iter().find(|c| c.id == canvas) else { return };
        let ids: Vec<String> = artboards.iter().filter(|id| found.artboards.iter().any(|a| &&a.id == id)).cloned().collect();
        self.current = Some(canvas.to_string());
        self.selected = ids.first().cloned();
        self.switcher = false;
        self.unseen.clear();
        self.following = ids.is_empty();
        if self.bounds.get().is_none() {
            // Not laid out yet: the first paint fits the canvas.
            self.fit_pending = Some(canvas.to_string());
        } else if ids.is_empty() {
            self.fit_all(cx);
        } else {
            self.go_to(&ids, cx);
        }
        self.ask_for_pictures(cx);
        cx.notify();
    }

    /// Sets the pane up for a gallery scene: an artboard selected, and the first fit's zoom.
    pub fn stage(&mut self, selected: Option<&str>, zoom: Option<f32>) {
        self.selected = selected.map(str::to_string);
        self.fit_zoom = zoom;
    }

    /// Sets the pane up for a gallery scene of Play: the artboard playing, its page still
    /// opening (scenes have no engine). The first paint fits it.
    pub fn stage_play(&mut self, id: &str) {
        let Some(canvas) = self.current.clone() else { return };
        self.selected = Some(id.to_string());
        self.playing = Some(Playing { canvas, artboard: id.to_string(), tab: None, zoom: 0., size: (0., 0.), failed: None, turn: 0 });
    }

    /// What the model is drawing now, from the chat's live turn.
    pub fn set_writing(&mut self, writing: Option<crate::conversation::Writing>, cx: &mut Context<Self>) {
        if self.writing != writing {
            self.writing = writing;
            cx.notify();
        }
    }

    /// Opens the switcher for a gallery scene.
    pub fn stage_switcher(&mut self) {
        self.switcher = true;
    }

    /// Opens the Tweaks panel for a gallery scene.
    pub fn stage_tweaks(&mut self) {
        self.tweaks_open = true;
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
                    problems: report.as_ref().map(problems_of).unwrap_or_default(),
                    badge: report.as_ref().and_then(badge_of),
                    map: report.as_ref().filter(|r| !r.elements.is_empty()).map(|r| Rc::new(r.elements.clone())),
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
        // The user's own tweak is no arrival, and a played page has it already (`move_tweak`),
        // so it neither reopens nor offers to go and look.
        if let Some((canvas, id, at)) = &self.tweak_rev {
            changed.retain(|(c, a, _)| !(c == canvas && a == id && revs.get(&(c.clone(), a.clone())) == Some(at)));
        }
        self.list = list;
        self.revs = revs;
        if let Some(current) = self.current.clone()
            && !self.restore_camera(&current)
        {
            self.fit_pending = Some(current);
        }
        // The played artboard changed: its page opens again on the new revision.
        let replay = self.playing.as_ref().is_some_and(|p| changed.iter().any(|(canvas, id, _)| canvas == &p.canvas && id == &p.artboard));
        let gone = self.playing.as_ref().is_some_and(|p| !self.list.iter().any(|c| c.id == p.canvas && c.artboards.iter().any(|a| a.id == p.artboard)));
        if gone {
            self.stop_playing(cx);
        } else if replay && let Some(id) = self.playing.as_ref().map(|p| p.artboard.clone()) {
            self.play(&id, cx);
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
                    self.follow(&ids, cx);
                } else {
                    for id in ids {
                        if !self.unseen.contains(&id) {
                            self.unseen.push(id);
                        }
                    }
                }
            }
        }
        self.refresh_comments();
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

    /// A canvas's camera: the one in hand, else where it was left last time (`view.json`).
    /// False when it has none yet, and the first paint fits it.
    fn restore_camera(&mut self, canvas: &str) -> bool {
        if self.cameras.contains_key(canvas) {
            return true;
        }
        let Some([x, y, zoom]) = self.canvases.camera(canvas) else { return false };
        let camera = Camera { x, y, zoom: zoom.clamp(board::MIN_ZOOM, board::MAX_ZOOM) };
        self.cameras.insert(canvas.to_string(), camera);
        self.saved_cameras.insert(canvas.to_string(), camera);
        true
    }

    /// Keeps where the camera came to rest, once it has rested a moment: a pan or a pinch
    /// moves it every frame, and only the last of a burst is written.
    fn keep_camera(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.current.clone() else { return };
        let Some(camera) = self.cameras.get(&id).copied() else { return };
        if self.glide.is_some() || self.saved_cameras.get(&id) == Some(&camera) {
            return;
        }
        self.saved_cameras.insert(id.clone(), camera);
        self.camera_moves += 1;
        let turn = self.camera_moves;
        let canvases = self.canvases.clone();
        cx.spawn(async move |pane, cx| {
            cx.background_executor().timer(Duration::from_millis(600)).await;
            let latest = pane.update(cx, |pane, _| pane.camera_moves == turn).unwrap_or(false);
            if latest {
                let _ = canvases.save_camera(&id, [camera.x, camera.y, camera.zoom]);
            }
        })
        .detach();
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
            let to = fit_below_pills(bounds, width, height, 1.);
            self.glide_to(to, cx);
        }
    }

    /// Brings the agent's changes into view without taking the user closer than they were: a
    /// change already in view only glints, and one out of view is fitted at no more than the
    /// current zoom. A canvas still waiting for its first fit is left to it.
    fn follow(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let Some(current) = self.current.clone() else { return };
        if self.fit_pending.as_ref() == Some(&current) || !self.cameras.contains_key(&current) {
            return;
        }
        let (width, height) = self.size();
        let Some(bounds) = self.board().bounds_of(ids) else { return };
        let camera = self.camera();
        let shown = camera.to_screen(bounds);
        let in_view = shown.x >= 0. && shown.y >= EDGE + PILL_HEIGHT && shown.right() <= width && shown.bottom() <= height;
        if !in_view {
            self.glide_to(fit_below_pills(bounds, width, height, camera.zoom.min(1.)), cx);
        }
    }

    /// Plays an artboard: fits it in the pane (never above 100%), then opens its live page
    /// there. A second Play of the same artboard leaves it playing.
    pub fn play(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(canvas) = self.current.clone() else { return };
        if !self.canvases.available() {
            return;
        }
        let (width, height) = self.size();
        let Some(bounds) = self.board().frame(id).map(|frame| frame.rect) else { return };
        if let Some(old) = self.playing.take().and_then(|p| p.tab) {
            self.canvases.close_live(old.id);
        }
        self.play_turns += 1;
        let turn = self.play_turns;
        self.commenting = false;
        self.editing = None;
        self.hover = None;
        self.selected = Some(id.to_string());
        self.following = false;
        self.drag = None;
        self.playing = Some(Playing { canvas: canvas.clone(), artboard: id.to_string(), tab: None, zoom: 1., size: (width, height), failed: None, turn });
        // Fitted beside the Tweaks panel when it shows this artboard's tweaks.
        let to = fit_below_pills(bounds, (width - self.panel_room()).max(1.), (height - PLAY_BAND).max(1.), 1.);
        if let Some(playing) = self.playing.as_mut() {
            playing.zoom = to.zoom;
        }
        self.glide_to(to, cx);
        let canvases = self.canvases.clone();
        let artboard = id.to_string();
        let pane = cx.entity().downgrade();
        let display = f64::from(to.zoom);
        let opened = Tokio::spawn(cx, async move { canvases.open_live(&canvas, &artboard, display).await });
        cx.spawn(async move |_, cx| {
            let result = opened.await.unwrap_or_else(|_| Err("The page could not be opened.".into()));
            let _ = pane.update(cx, |pane, cx| {
                match (pane.playing.as_mut().filter(|p| p.turn == turn), result) {
                    (Some(playing), Ok((tab, _))) => playing.tab = Some(tab),
                    (Some(playing), Err(error)) => playing.failed = Some(error),
                    // Play ended or moved on while the page opened.
                    (None, Ok((tab, _))) => pane.canvases.close_live(tab.id),
                    (None, Err(_)) => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Saves an artboard as `format` where the user picks, starting in Downloads.
    fn export(&mut self, id: &str, format: wack_canvas::export::Format, cx: &mut Context<Self>) {
        self.export_menu = false;
        let Some(canvas) = self.canvas().cloned() else { return };
        let Some(artboard) = canvas.artboards.iter().find(|a| a.id == id).cloned() else { return };
        let name = wack_canvas::export::file_name(&canvas, &artboard, format);
        let downloads = std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Downloads")).unwrap_or_else(|| PathBuf::from("/"));
        let chosen = cx.prompt_for_new_path(&downloads, Some(&name));
        let canvases = self.canvases.clone();
        let (canvas_id, id) = (canvas.id.clone(), id.to_string());
        cx.spawn(async move |pane, cx| {
            let Ok(Ok(Some(path))) = chosen.await else { return };
            let file = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
            let _ = pane.update(cx, |pane, cx| {
                pane.export_note = Some((format!("Saving {file}…"), false));
                cx.notify();
            });
            let exported = Tokio::spawn(cx, async move { canvases.export(&canvas_id, &id, format).await }).await;
            let note = match exported.unwrap_or_else(|_| Err("The export stopped.".into())) {
                Ok(bytes) => match std::fs::write(&path, bytes) {
                    Ok(()) => (format!("Saved {file}"), false),
                    Err(error) => (format!("Could not save {file}: {error}."), true),
                },
                Err(error) => (error, true),
            };
            let _ = pane.update(cx, |pane, cx| {
                pane.export_note = Some(note);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Presents the canvas from the selected artboard (or the first): each one fitted to the
    /// whole window, from its picture. Play is how an artboard is used live.
    pub fn present(&mut self, cx: &mut Context<Self>) {
        self.stop_playing(cx);
        let board = self.board();
        if board.frames.is_empty() {
            return;
        }
        let start = self.selected.as_ref().and_then(|id| board.frames.iter().position(|f| &f.id == id)).unwrap_or(0);
        self.presenting = Some(start);
        self.export_menu = false;
        self.commenting = false;
        self.editing = None;
        self.selected = Some(board.frames[start].id.clone());
        cx.notify();
    }

    fn step_presenting(&mut self, by: i32, cx: &mut Context<Self>) {
        let board = self.board();
        let Some(index) = self.presenting else { return };
        let next = (index as i64 + i64::from(by)).clamp(0, board.frames.len().saturating_sub(1) as i64) as usize;
        self.presenting = Some(next);
        self.selected = board.frames.get(next).map(|f| f.id.clone());
        cx.notify();
    }

    /// Copies the artboard's own markup (a variant's with its CSS), as the agent wrote it.
    fn copy_html(&mut self, id: &str, cx: &mut Context<Self>) {
        self.export_menu = false;
        let Some(canvas) = self.canvas() else { return };
        let Some(artboard) = canvas.artboards.iter().find(|a| a.id == id) else { return };
        let html = wack_canvas::model::resolve_html(canvas, artboard);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(html));
        self.export_note = Some(("Copied the HTML".into(), false));
        cx.notify();
    }

    /// Ends Play: the page closes and the picture shows again, the camera where it was.
    pub fn stop_playing(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.playing.take().and_then(|p| p.tab) {
            self.canvases.close_live(tab.id);
        }
        cx.notify();
    }

    /// The pane changed size while playing: fit again and rescale the page to match.
    fn refit_playing(&mut self, cx: &mut Context<Self>) {
        let Some(playing) = self.playing.as_ref() else { return };
        let (width, height) = self.size();
        let Some(bounds) = self.board().frame(&playing.artboard).map(|frame| frame.rect) else { return };
        let to = fit_below_pills(bounds, (width - self.panel_room()).max(1.), (height - PLAY_BAND).max(1.), 1.);
        let (canvas, artboard, tab) = (playing.canvas.clone(), playing.artboard.clone(), playing.tab.clone());
        if let Some(playing) = self.playing.as_mut() {
            playing.zoom = to.zoom;
            playing.size = (width, height);
        }
        self.set_camera(to);
        if let Some(tab) = tab {
            let canvases = self.canvases.clone();
            let display = f64::from(to.zoom);
            Tokio::spawn(cx, async move { canvases.rescale_live(&tab, &canvas, &artboard, display).await }).detach();
        }
        cx.notify();
    }

    fn fit_all(&mut self, cx: &mut Context<Self>) {
        self.stop_playing(cx);
        let (width, height) = self.size();
        if let Some(bounds) = self.board().bounds() {
            self.following = true;
            self.unseen.clear();
            self.glide_to(fit_below_pills(bounds, width, height, 1.), cx);
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
        // While playing the camera holds still, so the page stays on its picture.
        if self.playing.is_some() {
            return;
        }
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
        if self.playing.is_some() {
            return;
        }
        let (x, y) = self.local(event.position);
        let next = self.camera().zoom_at(x, y, 1. + event.delta);
        self.moved_by_hand();
        self.set_camera(next);
        cx.notify();
    }

    fn on_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.switcher = false;
        // A press on the board around a played artboard ends Play.
        if self.playing.is_some() {
            self.stop_playing(cx);
            return;
        }
        let (x, y) = self.local(event.position);
        if self.comment_down(x, y, cx) {
            return;
        }
        let (wx, wy) = self.camera().to_world(x, y);
        let on = self.board().at(wx, wy).map(|frame| frame.id.clone());
        self.drag = Some(Drag { start: (x, y), camera: self.camera(), moved: false, on });
        cx.notify();
    }

    fn on_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.tweak_drag.is_some() {
            self.drag_tweak(event.position.x, event.pressed_button != Some(MouseButton::Left), cx);
            return;
        }
        let (x, y) = self.local(event.position);
        if self.region.is_some() || (self.commenting && self.drag.is_none()) {
            self.comment_move(x, y, cx);
            return;
        }
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

    fn on_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.drag_tweak(event.position.x, true, cx) || self.comment_up(window, cx) {
            return;
        }
        let Some(drag) = self.drag.take() else { return };
        if !drag.moved {
            if self.selected != drag.on {
                self.export_menu = false;
                self.export_note = None;
            }
            self.selected = drag.on.clone();
            // A double-click plays.
            if event.click_count == 2
                && let Some(id) = drag.on
            {
                self.play(&id, cx);
            }
        }
        cx.notify();
    }

    fn zoom_by(&mut self, inwards: bool, cx: &mut Context<Self>) {
        self.stop_playing(cx);
        let (width, height) = self.size();
        let camera = self.camera();
        let to = camera.zoom_to(width / 2., height / 2., board::step(camera.zoom, inwards));
        self.moved_by_hand();
        self.glide_to(to, cx);
    }

    fn zoom_to_selection(&mut self, cx: &mut Context<Self>) {
        self.stop_playing(cx);
        if let Some(id) = self.selected.clone() {
            let (width, height) = self.size();
            if let Some(bounds) = self.board().bounds_of(&[id]) {
                self.moved_by_hand();
                self.glide_to(fit_below_pills(bounds, width, height, 2.), cx);
            }
        }
    }

    fn actual_size(&mut self, cx: &mut Context<Self>) {
        self.stop_playing(cx);
        let (width, height) = self.size();
        let to = self.camera().zoom_to(width / 2., height / 2., 1.);
        self.moved_by_hand();
        self.glide_to(to, cx);
    }

    fn step_selection(&mut self, dx: i32, dy: i32, cx: &mut Context<Self>) {
        if self.presenting.is_some() {
            self.step_presenting(if dx + dy >= 0 { 1 } else { -1 }, cx);
            return;
        }
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
        if self.editing.is_some() {
            self.close_editor(cx);
            return;
        }
        if self.presenting.take().is_some() {
            // Back on the board at the artboard last shown.
            if let Some(id) = self.selected.clone() {
                self.go_to(&[id], cx);
            }
            cx.notify();
            return;
        }
        if self.export_menu {
            self.export_menu = false;
            cx.notify();
            return;
        }
        if self.tweak_target().is_some() {
            self.toggle_tweaks(cx);
            return;
        }
        if self.commenting {
            self.toggle_commenting(cx);
            return;
        }
        if self.playing.is_some() {
            self.stop_playing(cx);
            return;
        }
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
        if !self.restore_camera(&id) {
            self.fit_pending = Some(id);
        }
        self.ask_for_pictures(cx);
        cx.notify();
    }
}

/// A fit that keeps clear of the pills along the pane's top, so no label hides under them.
fn fit_below_pills(bounds: Rect, width: f32, height: f32, max_zoom: f32) -> Camera {
    let band = EDGE + PILL_HEIGHT - board::FIT_PADDING / 2.;
    let camera = board::fit(bounds, width, (height - band).max(1.), max_zoom);
    Camera { y: camera.y + band, ..camera }
}

fn problems_of(report: &wack_canvas::Report) -> Vec<String> {
    let mut problems: Vec<String> = report.errors().cloned().collect();
    if report.stuck {
        problems.push("Its page never settled: a script may be stuck in a loop.".into());
    }
    if report.overflow_y > 0 {
        problems.push(format!("Content overflows the bottom by {} px.", report.overflow_y));
    }
    if report.overflow_x > 0 {
        problems.push(format!("Content overflows the right edge by {} px.", report.overflow_x));
    }
    problems
}

fn badge_of(report: &wack_canvas::Report) -> Option<String> {
    match report.errors().count() {
        0 if report.stuck => Some("Stuck".into()),
        0 if report.overflow_x > 0 || report.overflow_y > 0 => Some("Overflows".into()),
        0 => None,
        1 => Some("1 error".into()),
        count => Some(format!("{count} errors")),
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
                let mut camera = fit_below_pills(bounds, width, height, 1.);
                if let Some(zoom) = self.fit_zoom.take() {
                    camera = camera.zoom_at(width / 2., height / 2., zoom / camera.zoom);
                }
                self.cameras.insert(id, camera);
            }
        }
        // The pane changed size under a played artboard: fit again, rescale its page.
        if self.playing.as_ref().is_some_and(|p| self.bounds.get().is_some() && ((p.size.0 - width).abs() > 0.5 || (p.size.1 - height).abs() > 0.5)) {
            self.refit_playing(cx);
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
        self.fading.retain(|_, since| since.elapsed() < comments::FADE);
        if !self.fading.is_empty() && !still {
            animating = true;
        }
        if animating {
            window.request_animation_frame();
        } else {
            self.keep_camera(cx);
        }

        let camera = self.camera();
        let board = self.board();
        let canvas = self.canvas().cloned();
        let view = camera.view(width, height);
        let far = camera.zoom < FAR_ZOOM;
        let bounds_cell = self.bounds.clone();

        let mut layer = div().absolute().inset_0();
        if let Some(canvas) = &canvas {
            // A group's name sits a fixed height above its row's artboard names, which keep one
            // size at any zoom; far out there is no room between rows, so only the names show.
            for group in board.groups.iter().filter(|_| !far) {
                let row_top = (group.y + board::GROUP_BAND + board::LABEL_BAND) * camera.zoom + camera.y;
                let (x, y) = (group.x * camera.zoom + camera.x, row_top - 44.);
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
                    // No wider than the artboard and most of the gap after it, so names never run together.
                    .max_w(px((screen.width + (board::ARTBOARD_GAP - 12.) * camera.zoom).max(24.)))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .overflow_hidden()
                    .child(
                        div()
                            .min_w_0()
                            .flex_shrink(1.)
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
                            .when(!artboard.tweak_values.is_empty(), |label| {
                                label.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(4.))
                                        .typography(Type::Small)
                                        .text_color(theme.accent_text)
                                        .whitespace_nowrap()
                                        .child(div().size(px(6.)).rounded_full().bg(theme.accent_line))
                                        .child("Tweaked"),
                                )
                            })
                            .when_some(shown.badge.clone(), |label, badge| {
                                label.child(
                                    div()
                                        .px(px(6.))
                                        .rounded_full()
                                        .bg(Hsla { a: 0.14, ..theme.danger })
                                        .typography(Type::Small)
                                        .text_color(theme.danger)
                                        .whitespace_nowrap()
                                        .child(badge),
                                )
                            })
                    });
                layer = layer.child(label);
            }
        }

        // Comments: their pins, the element under the pointer, the area being dragged.
        if let Some(canvas) = &canvas
            && self.presenting.is_none()
        {
            let mut marks = Vec::new();
            self.comment_marks(canvas, &board, camera, &theme, still, &cx.entity(), &mut marks);
            layer = layer.children(marks);
        }

        // Play: the board dims, and the played artboard's live page sits exactly on its picture
        // once the camera has arrived at the zoom the page is scaled to.
        let mut play_pill = None;
        if let (Some(playing), Some(canvas)) = (&self.playing, &canvas)
            && let Some(frame) = board.frame(&playing.artboard)
        {
            let screen = camera.to_screen(frame.rect);
            let arrived = self.glide.is_none() && (camera.zoom - playing.zoom).abs() < 1e-4;
            layer = layer.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(Hsla { a: 0.72, ..theme.window }),
            );
            let shown = self.shown.get(&(canvas.id.clone(), playing.artboard.clone())).cloned().unwrap_or_default();
            let mut stage = div()
                .absolute()
                .left(px(screen.x))
                .top(px(screen.y))
                .w(px(screen.width.max(1.)))
                .h(px(screen.height.max(1.)))
                .rounded(px(PLAY_RADIUS))
                .overflow_hidden()
                .bg(theme.raised)
                .shadow_lg()
                .when_some(shown.full.or(shown.thumb), |stage, path| stage.child(img(path).size_full().object_fit(ObjectFit::Fill)));
            // A pane too narrow to play beside the Tweaks panel keeps the page behind its
            // picture while the panel would cover it.
            let covered = self.panel_left().is_some_and(|left| screen.right() > left);
            if let (Some(tab), true, false) = (playing.tab.as_ref(), arrived, covered) {
                let (id, focus) = (tab.id, self.focus.clone());
                stage = stage.child(
                    gpui::canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| {
                            // Never over the pane's edge: a page is a native view, drawn over
                            // anything GPUI does not mask.
                            let mask = window.content_mask().bounds;
                            if contains(mask, bounds) {
                                let _ = crate::browser::show(window, id, &focus, bounds, PLAY_RADIUS);
                            }
                        },
                    )
                    .absolute()
                    .inset_0(),
                );
            }
            layer = layer.child(stage);
            let artboard = canvas.artboards.iter().find(|a| a.id == playing.artboard);
            let has_tweaks = artboard.is_some_and(|a| !a.tweaks.is_empty());
            let room = self.panel_room();
            let status = match (&playing.failed, &playing.tab) {
                (Some(error), _) => error.clone(),
                (None, None) => "Opening the live page…".into(),
                (None, Some(_)) if covered => "Close Tweaks to use it".into(),
                (None, Some(_)) => format!("Live · {}%", (playing.zoom * 100.).round()),
            };
            let pane = cx.entity();
            play_pill = Some(
                div()
                    .absolute()
                    .bottom(px(EDGE))
                    .left_0()
                    .right(px(room))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .h(px(PILL_HEIGHT + 4.))
                            .pl(px(14.))
                            .pr(px(4.))
                            .max_w(px((width - room - 2. * EDGE).max(160.)))
                            .rounded_full()
                            .bg(theme.raised)
                            .border_1()
                            .border_color(theme.border)
                            .shadow_sm()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(Icon::Play.view(px(12.), if playing.failed.is_some() { theme.danger } else { theme.accent_text }))
                            .child(div().min_w_0().flex_shrink(1.).typography(Type::UiStrong).text_color(theme.text).whitespace_nowrap().text_ellipsis().child(artboard.map(|a| a.name.clone()).unwrap_or_default()))
                            .child(div().flex_none().typography(Type::Small).text_color(if playing.failed.is_some() { theme.danger } else { theme.text_dim }).whitespace_nowrap().child(status))
                            .when(has_tweaks, |row| {
                                let pane = pane.clone();
                                row.child(
                                    circle("canvas-play-tweaks", Icon::Sliders, "Tweaks", &theme)
                                        .when(self.tweaks_open, |circle| circle.bg(theme.accent_wash))
                                        .on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.toggle_tweaks(cx)))
                                        .tag("canvas-play-tweaks"),
                                )
                            })
                            .child(
                                div()
                                    .id("canvas-play-done")
                                    .flex_none()
                                    .h(px(PILL_HEIGHT - 4.))
                                    .px(px(12.))
                                    .rounded_full()
                                    .bg(theme.hover)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(|done| done.bg(theme.accent_wash))
                                    .typography(Type::Ui)
                                    .text_color(theme.text)
                                    .tooltip(tip("Back to the board (Esc)"))
                                    .on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.stop_playing(cx)))
                                    .child("Done")
                                    .tag("canvas-play-done"),
                            ),
                    )
                    .tag("canvas-play"),
            );
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
            .on_action(cx.listener(|pane, _: &CanvasFit, _, cx| if pane.typing() { cx.propagate() } else { pane.fit_all(cx) }))
            .on_action(cx.listener(|pane, _: &CanvasZoomIn, _, cx| if pane.typing() { cx.propagate() } else { pane.zoom_by(true, cx) }))
            .on_action(cx.listener(|pane, _: &CanvasZoomOut, _, cx| if pane.typing() { cx.propagate() } else { pane.zoom_by(false, cx) }))
            .on_action(cx.listener(|pane, _: &CanvasZoomSelection, _, cx| if pane.typing() { cx.propagate() } else { pane.zoom_to_selection(cx) }))
            .on_action(cx.listener(|pane, _: &CanvasActualSize, _, cx| if pane.typing() { cx.propagate() } else { pane.actual_size(cx) }))
            .on_action(cx.listener(|pane, _: &CanvasEscape, _, cx| pane.escape(cx)))
            .on_action(cx.listener(|pane, _: &CanvasPresent, _, cx| if pane.typing() { cx.propagate() } else { pane.present(cx) }))
            .on_action(cx.listener(|pane, _: &CanvasPlay, _, cx| {
                if pane.typing() {
                    cx.propagate();
                } else if let Some(id) = pane.selected.clone() {
                    pane.play(&id, cx);
                }
            }))
            .on_action(cx.listener(|pane, action: &CanvasMove, _, cx| if pane.typing() { cx.propagate() } else { pane.step_selection(action.dx, action.dy, cx) }))
            .on_action(cx.listener(|pane, _: &CanvasComment, _, cx| if pane.typing() { cx.propagate() } else { pane.toggle_commenting(cx) }))
            .when(self.commenting, |root| root.cursor(gpui::CursorStyle::Crosshair))
            .child(layer);

        let Some(canvas) = canvas else {
            return root.child(empty(&theme, self.canvases.available())).tag("canvas-pane").into_any_element();
        };

        // Top left: which canvas, and the others.
        let many = self.list.len() > 1;
        // The switcher also lists the canvas's open comments.
        let open_here = self.open_comments(&canvas.id);
        let openable = many || open_here > 0;
        let title = div()
            .id("canvas-switcher")
            .absolute()
            .top(px(EDGE))
            .left(px(EDGE))
            // Room is kept for the zoom pill on the right; a narrow pane drops the count.
            .max_w(px((width - ZOOM_PILL - 3. * EDGE).max(96.)))
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
            .when(openable, |pill| {
                let pane = pane.clone();
                pill.cursor_pointer().hover(|pill| pill.bg(theme.hover)).on_click(move |_, _, cx| {
                    pane.update(cx, |pane, cx| {
                        pane.switcher = !pane.switcher;
                        cx.notify();
                    })
                })
            })
            .child(div().min_w_0().flex_shrink(1.).typography(Type::UiStrong).text_color(theme.text).whitespace_nowrap().text_ellipsis().child(canvas.title.clone()))
            .when(width >= NARROW, |pill| {
                pill.child(div().flex_none().typography(Type::Small).text_color(theme.text_dim).whitespace_nowrap().child(artboards(canvas.artboards.len())))
            })
            .when(open_here > 0, |pill| {
                pill.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .typography(Type::Small)
                        .text_color(theme.accent_text)
                        .child(Icon::Comment.view(px(11.), theme.accent_text))
                        .child(open_here.to_string())
                        .tag("canvas-comment-count"),
                )
            })
            .when(openable, |pill| pill.child(Icon::ChevronDown.view(px(12.), theme.text_dim)))
            .tag("canvas-switcher");
        let switcher = (self.switcher && openable).then(|| {
            let current = self.current.clone();
            let comment_rows = self.comment_rows(&canvas, &theme, &pane);
            deferred(
                div()
                    .id("canvas-switcher-menu")
                    .absolute()
                    .top(px(EDGE + PILL_HEIGHT + 6.))
                    .left(px(EDGE))
                    .min_w(px(220.))
                    .max_w(px((width - 2. * EDGE).clamp(220., 360.)))
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
                    }).filter(|_| many))
                    .when(!comment_rows.is_empty(), |menu| {
                        menu.when(many, |menu| menu.child(div().h(px(1.)).mx(px(6.)).my(px(4.)).bg(theme.border)))
                            .child(div().px(px(10.)).pt(px(6.)).pb(px(2.)).typography(Type::Small).text_color(theme.text_dim).child(format!("Comments on “{}”", canvas.title)))
                            .children(comment_rows)
                    }),
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
            .child(circle("canvas-zoom-out", Icon::Minus, "Zoom out  ⌘−", &theme).cursor_pointer().on_click({
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
            .child(
                circle("canvas-comment-tool", Icon::Comment, "Comment  C", &theme)
                    .when(self.commenting, |circle| circle.bg(theme.accent_wash))
                    .on_click({
                        let pane = pane.clone();
                        move |_, _, cx| pane.update(cx, |pane, cx| pane.toggle_commenting(cx))
                    })
                    .tag("canvas-comment-tool"),
            )
            .child(circle("canvas-present", Icon::Eye, "Present  ⌘↩", &theme).on_click({
                let pane = pane.clone();
                move |_, _, cx| pane.update(cx, |pane, cx| pane.present(cx))
            }))
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
            let errors = shown.problems.clone();
            let ask = (!errors.is_empty()).then(|| {
                let text = format!(
                    "The artboard {} on canvas “{}” (canvas: \"{}\") has problems. Please fix them:\n{}",
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
                        .child(div().min_w_0().flex_shrink(1.).typography(Type::Ui).text_color(theme.text).whitespace_nowrap().text_ellipsis().child(facts.join(" · ")))
                        .children(ask)
                        .when(self.canvases.available(), |row| {
                            let (pane, id) = (pane.clone(), artboard.id.clone());
                            row.child(
                                div()
                                    .id("canvas-play-button")
                                    .flex_none()
                                    .h(px(24.))
                                    .pl(px(8.))
                                    .pr(px(10.))
                                    .rounded_full()
                                    .bg(theme.accent_wash)
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .cursor_pointer()
                                    .hover(|play| play.bg(theme.hover))
                                    .typography(Type::Small)
                                    .text_color(theme.accent_text)
                                    .tooltip(tip("Use it as a live page (↩)"))
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                    .on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.play(&id, cx)))
                                    .child(Icon::Play.view(px(10.), theme.accent_text))
                                    .child("Play")
                                    .tag("canvas-play-button"),
                            )
                        })
                        .when(!artboard.tweaks.is_empty(), |row| {
                            let pane = pane.clone();
                            row.child(
                                circle("canvas-tweaks-button", Icon::Sliders, "Tweaks", &theme)
                                    .when(self.tweaks_open, |circle| circle.bg(theme.accent_wash))
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                    .on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.toggle_tweaks(cx)))
                                    .tag("canvas-tweaks-button"),
                            )
                        })
                        .child({
                            let pane = pane.clone();
                            circle("canvas-export", Icon::Download, "Export, copy or mention", &theme)
                                .when(self.export_menu, |circle| circle.bg(theme.hover))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(move |_, _, cx| {
                                    pane.update(cx, |pane, cx| {
                                        pane.export_menu = !pane.export_menu;
                                        cx.notify();
                                    })
                                })
                                .tag("canvas-export")
                        }),
                )
                .when_some(artboard.notes.clone(), |hint, notes| hint.child(div().typography(Type::Small).text_color(theme.text_dim).child(notes)))
                .when_some(self.export_note.clone(), |hint, (note, failed)| {
                    hint.child(div().typography(Type::Small).text_color(if failed { theme.danger } else { theme.success }).child(note))
                })
                .when(self.export_menu, |hint| {
                    use wack_canvas::export::Format;
                    type Act = Box<dyn Fn(&mut CanvasPane, &mut Window, &mut Context<CanvasPane>)>;
                    let row = |id: &'static str, label: String, act: Act| {
                        let pane = pane.clone();
                        div()
                            .id(id)
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .hover(|row| row.bg(theme.hover))
                            .typography(Type::Ui)
                            .text_color(theme.text)
                            .on_click(move |_, window, cx| pane.update(cx, |pane, cx| act(pane, window, cx)))
                            .child(label)
                            .tag(id)
                    };
                    let id = artboard.id.clone();
                    let mention = format!("the artboard “{}” ({}) on the canvas “{}” (canvas: \"{}\")", artboard.name, artboard.id, canvas.title, canvas.id);
                    let mut menu = div()
                        .id("canvas-export-menu")
                        .absolute()
                        .bottom_full()
                        .left_0()
                        .mb(px(6.))
                        .min_w(px(200.))
                        .p(px(4.))
                        .rounded(px(12.))
                        .bg(theme.raised)
                        .border_1()
                        .border_color(theme.border)
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
                    for (tag, format) in [("canvas-export-png", Format::Png), ("canvas-export-html", Format::Html), ("canvas-export-pdf", Format::Pdf)] {
                        let id = id.clone();
                        menu = menu.child(row(tag, format!("Save as {}…", format.label()), Box::new(move |pane, _, cx| pane.export(&id, format, cx))));
                    }
                    let copy = id.clone();
                    menu = menu
                        .child(div().h(px(1.)).mx(px(6.)).my(px(4.)).bg(theme.border))
                        .child(row("canvas-copy-html", "Copy HTML".into(), Box::new(move |pane, _, cx| pane.copy_html(&copy, cx))))
                        .child(row(
                            "canvas-mention",
                            "Mention in chat".into(),
                            Box::new(move |pane, window, cx| {
                                pane.export_menu = false;
                                window.dispatch_action(Box::new(WriteInComposer { text: mention.clone() }), cx);
                                cx.notify();
                            }),
                        ));
                    hint.child(deferred(menu.tag("canvas-export-menu")))
                })
                .tag("canvas-selection")
        });

        // Present: one artboard at a time over the whole window, from its picture.
        let presented = self.presenting.and_then(|index| board.frames.get(index).map(|frame| (index, frame.clone()))).map(|(index, frame)| {
            let viewport = window.viewport_size();
            let (view_width, view_height) = (f32::from(viewport.width), f32::from(viewport.height));
            let origin = self.bounds.get().map_or(gpui::point(px(0.), px(0.)), |bounds| bounds.origin);
            let shown = self.shown.get(&(canvas.id.clone(), frame.id.clone())).cloned().unwrap_or_default();
            let (width, height) = (frame.rect.width.max(1.), frame.rect.height.max(1.));
            // Up to twice its size: the pictures are drawn at 2x.
            let fit = ((view_width - 2. * PRESENT_MARGIN) / width).min((view_height - 2.6 * PRESENT_MARGIN) / height).clamp(0.05, 2.);
            let name = canvas.artboards.iter().find(|a| a.id == frame.id).map(|a| a.name.clone()).unwrap_or_default();
            let count = board.frames.len();
            let step = |by: i32| {
                let pane = pane.clone();
                move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut gpui::App| pane.update(cx, |pane, cx| pane.step_presenting(by, cx))
            };
            let close = {
                let pane = pane.clone();
                move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut gpui::App| pane.update(cx, |pane, cx| pane.escape(cx))
            };
            let picture = div()
                .w(px(width * fit))
                .h(px(height * fit))
                .rounded(px(10.))
                .overflow_hidden()
                .bg(theme.raised)
                .shadow_lg()
                .when_some(shown.full.or(shown.thumb), |picture, path| picture.child(img(path).size_full().object_fit(ObjectFit::Fill)));
            deferred(
                div()
                    .id("canvas-present")
                    .absolute()
                    .left(-origin.x)
                    .top(-origin.y)
                    .w(viewport.width)
                    .h(viewport.height)
                    .bg(Hsla { a: 1., ..theme.window })
                    .occlude()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(16.))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                    .child(crate::motion::enter(SharedString::from(format!("canvas-present-{}", frame.id)), picture))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(circle("canvas-present-previous", Icon::ArrowLeft, "Previous  ←", &theme).bg(theme.raised).cursor_pointer().on_click(step(-1)))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .max_w(px(480.))
                                    .child(div().max_w(px(480.)).truncate().typography(Type::UiStrong).text_color(theme.text).child(name))
                                    .child(div().typography(Type::Small).text_color(theme.text_dim).child(format!("{} of {count} · ← → to step · esc to close", index + 1))),
                            )
                            .child(circle("canvas-present-next", Icon::ArrowRight, "Next  →", &theme).bg(theme.raised).cursor_pointer().on_click(step(1)))
                            .child(circle("canvas-present-close", Icon::Close, "Back to the board  esc", &theme).bg(theme.raised).cursor_pointer().on_click(close)),
                    )
                    .tag("canvas-present"),
            )
            .with_priority(20)
        });

        let tweaks_panel = self.tweak_target().map(|artboard| self.tweaks_panel(&canvas, artboard, height, &theme, pane.clone()));
        let editor = self.comment_editor(&canvas, &board, camera, (width, height), &theme, &pane);
        // What the model is writing now, under the top pills (and clear of the Tweaks panel).
        let writing = self.writing.as_ref().filter(|_| self.playing.is_none()).map(|writing| {
            let right = if self.tweak_target().is_some() { tweaks::PANEL_WIDTH + 2. * EDGE } else { 0. };
            div()
                .absolute()
                .top(px(EDGE + PILL_HEIGHT + 8.))
                .left_0()
                .right(px(right))
                .flex()
                .justify_center()
                .child(
                    div()
                        .h(px(PILL_HEIGHT))
                        .max_w(px((width - right - 2. * EDGE).max(160.)))
                        .px(px(14.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .bg(theme.raised)
                        .border_1()
                        .border_color(theme.accent_line)
                        .shadow_sm()
                        .child(Icon::Board.view(px(12.), theme.accent_text))
                        .child(div().flex_none().typography(Type::UiStrong).text_color(theme.text).child("Drawing"))
                        .child(div().min_w_0().flex_shrink(1.).typography(Type::Small).text_color(theme.text_dim).whitespace_nowrap().text_ellipsis().child(crate::work::writing_words(writing)))
                        .tag("canvas-writing"),
                )
        });
        let comment_hint = self.comment_hint(&theme, &canvas.id);
        let playing = self.playing.is_some();
        let commenting = self.commenting;
        root.child(title)
            .children(switcher)
            .child(zoom_pill)
            .children(arrivals.filter(|_| !playing))
            .children(hint.filter(|_| !playing && !commenting))
            .children(comment_hint)
            .children(writing)
            .children(editor)
            .children(play_pill)
            .children(tweaks_panel)
            .children(presented)
            .tag("canvas-pane")
            .into_any_element()
    }
}

impl Drop for CanvasPane {
    /// A played page is the pane's own: it closes with the pane.
    fn drop(&mut self) {
        if let Some(tab) = self.playing.take().and_then(|p| p.tab) {
            self.canvases.close_live(tab.id);
        }
    }
}

/// `inner` lies inside `outer`, give or take a point of rounding.
fn contains(outer: Bounds<Pixels>, inner: Bounds<Pixels>) -> bool {
    let slack = px(1.);
    inner.origin.x + slack >= outer.origin.x
        && inner.origin.y + slack >= outer.origin.y
        && inner.origin.x + inner.size.width <= outer.origin.x + outer.size.width + slack
        && inner.origin.y + inner.size.height <= outer.origin.y + outer.size.height + slack
}

fn artboards(count: usize) -> String {
    if count == 1 { "1 artboard".into() } else { format!("{count} artboards") }
}

fn circle(id: impl Into<gpui::ElementId>, icon: Icon, tooltip: impl Into<SharedString>, theme: &Theme) -> gpui::Stateful<gpui::Div> {
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
