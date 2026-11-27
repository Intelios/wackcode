//! Pictures in the transcript: the user's attachments on their bubble, a tool's under its step,
//! and the ones a reply names (`![what](img:3f2a9c01b4d7)`) under the reply; and the lightbox a
//! click opens.
//!
//! A [`Picture`] is plain data (the base64 the model was sent, which every message keeps), so the
//! transcript's items stay cheap to build. Decoding happens once per picture, off the main
//! thread, into a shared cache ([`Decoded`]): a strip draws a quiet placeholder until its
//! thumbnail is ready, then the window redraws. The cache keeps the most recent pictures only.

use std::{
    collections::{HashMap, VecDeque},
    hash::{Hash as _, Hasher as _},
    sync::Arc,
};

use gpui::{App, Global, ObjectFit, RenderImage, SharedString, div, img, prelude::*, px};
use wack_core::llm::{Block, Image};

use crate::{
    actions::OpenPicture,
    tags::Tag,
    theme::Theme,
    tooltip::tip,
    typography::{Type, Typography},
};

/// A strip's thumbnails: their height, and how many show before "+N".
const STRIP_HEIGHT: f32 = 72.;
const STRIP_MOST: usize = 6;
/// A thumbnail's long edge in pixels (drawn at a third of it, sharp on Retina).
const THUMB_EDGE: u32 = 288;
/// How many decoded pictures the cache keeps, of each size.
const KEPT: usize = 96;

#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    /// A hash of the data: the cache's key, and the element ids'.
    pub key: u64,
    /// The saved original's id, which a reply names it by.
    pub id: Option<String>,
    pub label: Option<SharedString>,
    pub data: Arc<String>,
}

impl Picture {
    pub fn new(data: &str, label: Option<&str>, id: Option<&str>) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        data.hash(&mut hasher);
        Self { key: hasher.finish(), id: id.map(str::to_string), label: label.map(|label| label.to_string().into()), data: Arc::new(data.to_string()) }
    }

    pub fn of_image(image: &Image) -> Self {
        Self::new(&image.data, image.label.as_deref(), image.id.as_deref())
    }

    /// The user's attached pictures in a message.
    pub fn of_message(content: &[Block]) -> Vec<Self> {
        content
            .iter()
            .filter_map(|block| match block {
                Block::Image { data, label, id, .. } => Some(Self::new(data, label.as_deref(), id.as_deref())),
                _ => None,
            })
            .collect()
    }

    /// Whether `reference` names this picture: `img:` and the start of its id, or the path a
    /// file was read from (its label), which is how a model names a file it just showed.
    pub fn answers_to(&self, reference: &str) -> bool {
        if let Some(prefix) = reference.strip_prefix("img:") {
            return prefix.len() >= 6 && self.id.as_deref().is_some_and(|id| id.starts_with(prefix));
        }
        let path = reference.trim_start_matches("./");
        self.label.as_deref().is_some_and(|label| {
            let label = label.trim_start_matches("./");
            !path.is_empty() && (label == path || label.ends_with(&format!("/{path}")) || path.ends_with(&format!("/{label}")))
        })
    }
}

/// Decoded pictures, thumbnails and full size apart. `None` marks one being decoded, or one that
/// could not be.
#[derive(Default)]
struct Decoded {
    thumbs: HashMap<u64, Option<Arc<RenderImage>>>,
    full: HashMap<u64, Option<Arc<RenderImage>>>,
    order: VecDeque<(u64, bool)>,
}

impl Global for Decoded {}

/// The picture's thumbnail, or `None` while it is decoded (the window redraws when it is).
pub fn thumb(picture: &Picture, cx: &mut App) -> Option<Arc<RenderImage>> {
    decoded(picture, false, cx)
}

/// The picture at the size the model was sent it.
pub fn full(picture: &Picture, cx: &mut App) -> Option<Arc<RenderImage>> {
    decoded(picture, true, cx)
}

fn decoded(picture: &Picture, full: bool, cx: &mut App) -> Option<Arc<RenderImage>> {
    let cache = cx.default_global::<Decoded>();
    let map = if full { &cache.full } else { &cache.thumbs };
    if let Some(found) = map.get(&picture.key) {
        return found.clone();
    }
    let map = if full { &mut cache.full } else { &mut cache.thumbs };
    map.insert(picture.key, None);
    cache.order.push_back((picture.key, full));
    while cache.order.len() > KEPT * 2 {
        if let Some((key, was_full)) = cache.order.pop_front() {
            if was_full { cache.full.remove(&key) } else { cache.thumbs.remove(&key) };
        }
    }
    let (key, data) = (picture.key, picture.data.clone());
    let decoding = cx.background_spawn(async move {
        let bytes = wack_core::images::decode(&data).ok()?;
        let picture = image::load_from_memory(&bytes).ok()?;
        Some(Arc::new(crate::attachments::render_image(&picture, if full { u32::MAX } else { THUMB_EDGE })))
    });
    cx.spawn(async move |cx| {
        let made = decoding.await;
        cx.update(|cx| {
            let cache = cx.default_global::<Decoded>();
            let map = if full { &mut cache.full } else { &mut cache.thumbs };
            if map.contains_key(&key) {
                map.insert(key, made);
            }
            cx.refresh_windows();
        });
    })
    .detach();
    None
}

/// A row of thumbnails, each opening the lightbox. `align_end` lines them up on the right, under
/// the user's bubble.
#[derive(IntoElement)]
pub struct Strip {
    pub pictures: Vec<Picture>,
    pub key: SharedString,
    pub align_end: bool,
}

impl RenderOnce for Strip {
    fn render(self, _: &mut gpui::Window, cx: &mut App) -> impl IntoElement {
        strip(&self.pictures, &self.key, self.align_end, cx)
    }
}

fn strip(pictures: &[Picture], key: &str, align_end: bool, cx: &mut App) -> gpui::AnyElement {
    let theme = Theme::get(cx).clone();
    let shown: Vec<_> = pictures.iter().take(STRIP_MOST).map(|picture| (picture.clone(), thumb(picture, cx))).collect();
    let more = pictures.len().saturating_sub(STRIP_MOST);
    let all: Arc<Vec<Picture>> = Arc::new(pictures.to_vec());
    div()
        .flex()
        .flex_wrap()
        .gap(px(6.))
        .when(align_end, |row| row.justify_end())
        .children(shown.into_iter().enumerate().map(|(index, (picture, decoded))| {
            let label = picture.label.clone().unwrap_or_else(|| "Picture".into());
            let all = all.clone();
            let frame = div()
                .id(SharedString::from(format!("{key}-picture-{index}")))
                .flex_none()
                .h(px(STRIP_HEIGHT))
                .rounded(px(10.))
                .overflow_hidden()
                .border_1()
                .border_color(theme.border)
                .bg(theme.raised)
                .cursor_pointer()
                .tooltip(tip(label))
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(OpenPicture { pictures: all.clone(), index }), cx);
                });
            match decoded {
                Some(decoded) => {
                    let size = decoded.size(0);
                    let width = STRIP_HEIGHT * size.width.0 as f32 / size.height.0.max(1) as f32;
                    frame.w(px(width.clamp(STRIP_HEIGHT * 0.5, STRIP_HEIGHT * 2.2))).child(img(decoded).size_full().rounded(px(10.)).object_fit(ObjectFit::Cover))
                }
                None => frame.w(px(STRIP_HEIGHT * 1.4)),
            }
        }))
        .when(more > 0, |row| {
            let all = all.clone();
            row.child(
                div()
                    .id(SharedString::from(format!("{key}-pictures-more")))
                    .flex_none()
                    .h(px(STRIP_HEIGHT))
                    .px(px(14.))
                    .rounded(px(10.))
                    .flex()
                    .items_center()
                    .border_1()
                    .border_color(theme.border)
                    .cursor_pointer()
                    .hover(|more| more.bg(theme.hover))
                    .typography(Type::Small)
                    .text_color(theme.text_dim)
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(OpenPicture { pictures: all.clone(), index: STRIP_MOST }), cx);
                    })
                    .child(format!("+{more}")),
            )
        })
        .tag("pictures")
        .into_any_element()
}

/// The lightbox: which pictures, and which one is showing. How far it is zoomed is the view's
/// own ([`Zoom`]), and goes when the lightbox closes or steps to another picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Lightbox {
    pub pictures: Arc<Vec<Picture>>,
    pub index: usize,
}

impl Lightbox {
    pub fn new(pictures: Arc<Vec<Picture>>, index: usize) -> Self {
        let index = index.min(pictures.len().saturating_sub(1));
        Self { pictures, index }
    }

    /// Moves `by` pictures, stopping at either end.
    pub fn step(&mut self, by: i32) {
        let last = self.pictures.len().saturating_sub(1) as i64;
        self.index = (self.index as i64 + i64::from(by)).clamp(0, last) as usize;
    }
}

/// The key context of the open lightbox, whose keys step and zoom.
pub const LIGHTBOX_CONTEXT: &str = "Lightbox";

/// Where the picture sits: how far zoomed (1 fits the stage) and which point of it (each axis
/// from 0 to 1) is at the stage's centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub zoom: f32,
    pub center: (f32, f32),
}

impl Default for View {
    fn default() -> Self {
        Self { zoom: 1., center: (0.5, 0.5) }
    }
}

impl View {
    pub fn fitted(&self) -> bool {
        self.zoom <= 1.001
    }
}

/// The lightbox's stage (the window above its bar) and the picture fitted to it, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub stage: (f32, f32),
    pub fitted: (f32, f32),
    /// The furthest it zooms: until one of the picture's pixels is [`MOST_POINTS`] points.
    pub most: f32,
}

/// The room a fitted picture keeps from the stage's edges.
const FIT_MARGIN: f32 = 40.;
/// A small picture is enlarged to fit, but to no more than this many points a pixel.
const MOST_FITTED: f32 = 2.;
const MOST_POINTS: f32 = 6.;
/// A click on a fitted picture zooms this far, about where it landed; ⌘= and ⌘− step by this.
const CLICK_ZOOM: f32 = 2.5;
const STEP_ZOOM: f32 = 1.5;
/// How far a press moves before it is a drag, not a click.
const DRAG_SLOP: f32 = 3.;
/// The room under the stage for the lightbox's bar.
const BAR_ROOM: f32 = 76.;
/// A zoom a click or key asks for glides there over this long; a pinch or scroll follows the hand.
const GLIDE: std::time::Duration = std::time::Duration::from_millis(240);

impl Frame {
    /// A picture of `pixels` on a stage of `stage` points: fitted inside the stage's margin,
    /// enlarged when small (screenshots come at the size a model reads, which is small on a
    /// large window), never past [`MOST_FITTED`] points a pixel.
    pub fn new(pixels: (f32, f32), stage: (f32, f32)) -> Self {
        let pixels = (pixels.0.max(1.), pixels.1.max(1.));
        let room = ((stage.0 - 2. * FIT_MARGIN).max(80.), (stage.1 - 2. * FIT_MARGIN).max(80.));
        let scale = (room.0 / pixels.0).min(room.1 / pixels.1).min(MOST_FITTED);
        Self { stage, fitted: (pixels.0 * scale, pixels.1 * scale), most: (MOST_POINTS / scale).max(1.) }
    }

    pub fn drawn(&self, zoom: f32) -> (f32, f32) {
        (self.fitted.0 * zoom, self.fitted.1 * zoom)
    }

    /// `view` held in bounds: zoomed between fitting and its most, and moved no further than
    /// keeps the stage covered (centred on an axis the picture doesn't fill).
    pub fn settle(&self, view: View) -> View {
        let zoom = view.zoom.clamp(1., self.most);
        let (width, height) = self.drawn(zoom);
        let axis = |center: f32, drawn: f32, stage: f32| {
            if drawn <= stage {
                0.5
            } else {
                let half = stage / 2. / drawn;
                center.clamp(half, 1. - half)
            }
        };
        View { zoom, center: (axis(view.center.0, width, self.stage.0), axis(view.center.1, height, self.stage.1)) }
    }

    /// Where the picture's top-left corner draws on the stage.
    pub fn origin(&self, view: View) -> (f32, f32) {
        let (width, height) = self.drawn(view.zoom);
        (self.stage.0 / 2. - view.center.0 * width, self.stage.1 / 2. - view.center.1 * height)
    }

    /// Whether the stage point `at` falls on the picture.
    pub fn holds(&self, view: View, at: (f32, f32)) -> bool {
        let (x, y) = self.origin(view);
        let (width, height) = self.drawn(view.zoom);
        at.0 >= x && at.0 <= x + width && at.1 >= y && at.1 <= y + height
    }

    /// `view` zoomed by `factor`, keeping the point of the picture under `at` where it is.
    pub fn zoom_at(&self, view: View, factor: f32, at: (f32, f32)) -> View {
        let view = self.settle(view);
        let zoom = (view.zoom * factor).clamp(1., self.most);
        let before = self.drawn(view.zoom);
        let after = self.drawn(zoom);
        let offset = (at.0 - self.stage.0 / 2., at.1 - self.stage.1 / 2.);
        let center = (view.center.0 + offset.0 / before.0 - offset.0 / after.0, view.center.1 + offset.1 / before.1 - offset.1 / after.1);
        self.settle(View { zoom, center })
    }

    /// `view` with the picture moved by `by` points.
    pub fn pan(&self, view: View, by: (f32, f32)) -> View {
        let view = self.settle(view);
        let (width, height) = self.drawn(view.zoom);
        self.settle(View { zoom: view.zoom, center: (view.center.0 - by.0 / width, view.center.1 - by.1 / height) })
    }
}

/// A press on the picture: where it started and where the picture was, so a drag moves it by
/// how far the pointer has come.
#[derive(Clone, Copy, Debug)]
struct Drag {
    start: (f32, f32),
    from: View,
    moved: bool,
}

/// The open lightbox's own state: its focus (so its keys reach it), the picture it is on, and
/// how that picture is zoomed.
pub struct Zoom {
    focus: gpui::FocusHandle,
    focused: bool,
    picture: u64,
    view: View,
    /// A zoom gliding to `view`: from where, since when.
    glide: Option<(View, std::time::Instant)>,
    drag: Option<Drag>,
}

impl Zoom {
    /// The view as drawn now, part way through a glide; true while it is still gliding.
    fn shown(&self, still: bool) -> (View, bool) {
        match self.glide {
            Some((from, since)) if !still && since.elapsed() < GLIDE => {
                let t = since.elapsed().as_secs_f32() / GLIDE.as_secs_f32();
                let t = 1. - (1. - t).powi(3);
                let mix = |a: f32, b: f32| a + (b - a) * t;
                // The zoom glides evenly in scale: each frame grows by the same share.
                let zoom = (from.zoom.ln() + (self.view.zoom.ln() - from.zoom.ln()) * t).exp();
                (View { zoom, center: (mix(from.center.0, self.view.center.0), mix(from.center.1, self.view.center.1)) }, true)
            }
            _ => (self.view, false),
        }
    }

    /// Moves to `view` at once, from wherever a glide had got to.
    fn set(&mut self, view: View) {
        self.glide = None;
        self.view = view;
    }

    /// Glides to `view` from where it shows now.
    fn glide_to(&mut self, view: View, still: bool) {
        let (from, _) = self.shown(still);
        self.glide = (!still).then(|| (from, std::time::Instant::now()));
        self.view = view;
    }

    /// The view a gesture starts from: where it shows now, held in bounds.
    fn current(&self, frame: &Frame, still: bool) -> View {
        frame.settle(self.shown(still).0)
    }
}

/// The lightbox over the whole window: the picture fitted to it, enlarged when small, its name
/// and place below, with round buttons to step, zoom and close. A click on the picture zooms in
/// about it and back; a pinch, ⌘-scroll or ⌘= ⌘− ⌘0 zoom too; zoomed, a drag or scroll moves it.
/// ← → step through the pictures, esc closes, as does a click beside the picture. It draws in
/// GPUI's overlay plane, above browser pages, and arrives with the app's enter motion (still under
/// reduced motion, where zooms jump instead of gliding).
#[derive(IntoElement)]
pub struct LightboxView {
    pub lightbox: Lightbox,
}

impl RenderOnce for LightboxView {
    fn render(self, window: &mut gpui::Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let Lightbox { pictures, index } = self.lightbox;
        let Some(picture) = pictures.get(index).cloned() else {
            return div().into_any_element();
        };
        let still = cx.reduce_motion();
        let viewport = window.viewport_size();
        let zoom = window.use_keyed_state("lightbox-zoom", cx, |_, cx| Zoom {
            focus: cx.focus_handle(),
            focused: false,
            picture: picture.key,
            view: View::default(),
            glide: None,
            drag: None,
        });
        // Keys go to the lightbox while it is open, not the composer behind it.
        if !zoom.read(cx).focused {
            let focus = zoom.read(cx).focus.clone();
            zoom.update(cx, |zoom, _| zoom.focused = true);
            window.defer(cx, move |window, cx| window.focus(&focus, cx));
        }
        if zoom.read(cx).picture != picture.key {
            zoom.update(cx, |zoom, _| {
                zoom.picture = picture.key;
                zoom.view = View::default();
                zoom.glide = None;
                zoom.drag = None;
            });
        }
        let decoded = full(&picture, cx).or_else(|| thumb(&picture, cx));
        let stage = (f32::from(viewport.width), (f32::from(viewport.height) - BAR_ROOM).max(120.));
        let pixels = decoded.as_ref().map_or((960., 600.), |decoded| {
            let size = decoded.size(0);
            (size.width.0 as f32, size.height.0 as f32)
        });
        let frame = Frame::new(pixels, stage);
        let (shown, gliding) = zoom.read(cx).shown(still);
        let shown = frame.settle(shown);
        if gliding {
            window.request_animation_frame();
        }
        let dragging = zoom.read(cx).drag.is_some_and(|drag| drag.moved);
        let origin = frame.origin(shown);
        let drawn = frame.drawn(shown.zoom);
        let corner = if shown.fitted() { 12. } else { 0. };
        let point = |position: gpui::Point<gpui::Pixels>| (f32::from(position.x), f32::from(position.y));

        let picture_box = match decoded {
            Some(decoded) => div()
                .absolute()
                .left(px(origin.0))
                .top(px(origin.1))
                .w(px(drawn.0))
                .h(px(drawn.1))
                .rounded(px(corner))
                .overflow_hidden()
                .shadow_lg()
                .child(img(decoded).size_full().rounded(px(corner)).object_fit(ObjectFit::Fill))
                .into_any_element(),
            None => div().absolute().left(px(origin.0)).top(px(origin.1)).w(px(drawn.0)).h(px(drawn.1)).rounded(px(12.)).bg(theme.raised).into_any_element(),
        };
        let stage_view = div()
            .id("lightbox-stage")
            .absolute()
            .left_0()
            .top_0()
            .w(px(stage.0))
            .h(px(stage.1))
            .overflow_hidden()
            .cursor(if dragging {
                gpui::CursorStyle::ClosedHand
            } else if shown.fitted() {
                gpui::CursorStyle::PointingHand
            } else {
                gpui::CursorStyle::OpenHand
            })
            .on_mouse_down(gpui::MouseButton::Left, {
                let zoom = zoom.clone();
                move |event, _, cx| {
                    let at = point(event.position);
                    let from = zoom.read(cx).current(&frame, still);
                    // Beside the picture: the press goes on to the lightbox, which closes.
                    if !frame.holds(from, at) {
                        return;
                    }
                    cx.stop_propagation();
                    zoom.update(cx, |zoom, cx| {
                        zoom.set(from);
                        zoom.drag = Some(Drag { start: at, from, moved: false });
                        cx.notify();
                    });
                }
            })
            .on_mouse_move({
                let zoom = zoom.clone();
                move |event, _, cx| {
                    let Some(drag) = zoom.read(cx).drag else {
                        return;
                    };
                    if event.pressed_button != Some(gpui::MouseButton::Left) {
                        zoom.update(cx, |zoom, _| zoom.drag = None);
                        return;
                    }
                    let at = point(event.position);
                    let by = (at.0 - drag.start.0, at.1 - drag.start.1);
                    if !drag.moved && by.0.hypot(by.1) < DRAG_SLOP {
                        return;
                    }
                    zoom.update(cx, |zoom, cx| {
                        zoom.drag = Some(Drag { moved: true, ..drag });
                        zoom.set(frame.pan(drag.from, by));
                        cx.notify();
                    });
                }
            })
            .on_mouse_up(gpui::MouseButton::Left, {
                let zoom = zoom.clone();
                move |event, _, cx| {
                    let Some(drag) = zoom.read(cx).drag else {
                        return;
                    };
                    cx.stop_propagation();
                    zoom.update(cx, |zoom, cx| {
                        zoom.drag = None;
                        // A click, not a drag: in about where it landed, or back out to fit.
                        if !drag.moved {
                            let to = if drag.from.fitted() { frame.zoom_at(drag.from, CLICK_ZOOM, point(event.position)) } else { View::default() };
                            zoom.glide_to(to, still);
                        }
                        cx.notify();
                    });
                }
            })
            .on_scroll_wheel({
                let zoom = zoom.clone();
                move |event, window, cx| {
                    let delta = match event.delta {
                        gpui::ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
                        gpui::ScrollDelta::Lines(delta) => (delta.x * 24., delta.y * 24.),
                    };
                    let from = zoom.read(cx).current(&frame, still);
                    let to = if event.modifiers.platform || event.modifiers.control {
                        frame.zoom_at(from, (delta.1 * 0.0025).exp(), point(event.position))
                    } else {
                        frame.pan(from, delta)
                    };
                    cx.stop_propagation();
                    if to != from {
                        zoom.update(cx, |zoom, cx| {
                            zoom.set(to);
                            cx.notify();
                        });
                        window.refresh();
                    }
                }
            })
            .on_pinch({
                let zoom = zoom.clone();
                move |event, _, cx| {
                    let from = zoom.read(cx).current(&frame, still);
                    let to = frame.zoom_at(from, 1. + event.delta, point(event.position));
                    zoom.update(cx, |zoom, cx| {
                        zoom.set(to);
                        cx.notify();
                    });
                }
            })
            .child(crate::motion::enter(gpui::ElementId::Name(format!("lightbox-{}", picture.key).into()), div().absolute().size_full().child(picture_box)));

        let round = |id: &'static str, icon: crate::icon::Icon, theme: &Theme| {
            div()
                .id(id)
                .flex_none()
                .size(px(34.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.raised)
                .border_1()
                .border_color(theme.border_strong)
                .cursor_pointer()
                .hover(|button| button.bg(theme.hover))
                .child(icon.view(px(14.), theme.text))
        };
        let zoom_by = |by: i32| {
            move |_: &gpui::ClickEvent, window: &mut gpui::Window, cx: &mut App| window.dispatch_action(Box::new(crate::actions::ZoomPicture { by }), cx)
        };
        // − Fit +: the zoom as a pill, its middle saying how far (and fitting again on a click).
        let zoom_pill = div()
            .flex_none()
            .h(px(34.))
            .p(px(3.))
            .flex()
            .items_center()
            .gap(px(2.))
            .rounded_full()
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border_strong)
            .child(
                div()
                    .id("lightbox-zoom-out")
                    .size(px(26.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!shown.fitted(), |button| button.cursor_pointer().hover(|button| button.bg(theme.hover)).on_click(zoom_by(-1)))
                    .child(crate::icon::Icon::Minus.view(px(13.), if shown.fitted() { theme.text_faint } else { theme.text }))
                    .tooltip(tip("Zoom out (⌘−)")),
            )
            .child(
                div()
                    .id("lightbox-fit")
                    .min_w(px(44.))
                    .h(px(26.))
                    .px(px(6.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .typography(Type::Small)
                    .text_color(theme.text_soft)
                    .when(!shown.fitted(), |button| button.cursor_pointer().hover(|button| button.bg(theme.hover)).on_click(zoom_by(0)).tooltip(tip("Fit (⌘0)")))
                    .child(zoom_words(shown.zoom))
                    .tag("lightbox-zoom-level"),
            )
            .child(
                div()
                    .id("lightbox-zoom-in")
                    .size(px(26.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|button| button.bg(theme.hover))
                    .on_click(zoom_by(1))
                    .child(crate::icon::Icon::Plus.view(px(13.), theme.text))
                    .tooltip(tip("Zoom in (⌘=)")),
            );
        let label = picture.label.clone().unwrap_or_else(|| "Picture".into());
        let place = (pictures.len() > 1).then(|| format!("{} of {}", index + 1, pictures.len()));
        let hint = if shown.fitted() { "click or pinch to zoom · esc to close" } else { "drag to move · click to fit · esc to close" };
        let several = pictures.len() > 1;
        let scrim = gpui::Hsla { a: 0.94, ..theme.window };
        let bar = div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(BAR_ROOM))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when(several, |bar| {
                        bar.child(
                            round("lightbox-previous", crate::icon::Icon::ArrowLeft, &theme)
                                .tooltip(tip("Previous (←)"))
                                .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::actions::StepPicture { by: -1 }), cx)),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .max_w(px(480.))
                            .child(div().max_w(px(480.)).truncate().typography(Type::UiStrong).text_color(theme.text).child(label))
                            .child(div().typography(Type::Small).text_color(theme.text_dim).child(match place {
                                Some(place) => format!("{place} · {hint}"),
                                None => hint.to_string(),
                            })),
                    )
                    .when(several, |bar| {
                        bar.child(
                            round("lightbox-next", crate::icon::Icon::ArrowRight, &theme)
                                .tooltip(tip("Next (→)"))
                                .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::actions::StepPicture { by: 1 }), cx)),
                        )
                    })
                    .child(zoom_pill)
                    .child(
                        round("lightbox-close", crate::icon::Icon::Close, &theme)
                            .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::actions::ClosePicture), cx)),
                    ),
            );
        let focus = zoom.read(cx).focus.clone();
        gpui::deferred(
            div()
                .id("lightbox")
                .key_context(LIGHTBOX_CONTEXT)
                .track_focus(&focus)
                .absolute()
                .left_0()
                .top_0()
                .size_full()
                .bg(scrim)
                // Nothing under the lightbox takes a click, a scroll or a hover while it is open.
                .occlude()
                .on_mouse_down(gpui::MouseButton::Left, |_, window, cx| window.dispatch_action(Box::new(crate::actions::ClosePicture), cx))
                .on_action({
                    let zoom = zoom.clone();
                    move |action: &crate::actions::ZoomPicture, _, cx| {
                        let by = action.by;
                        zoom.update(cx, |zoom, cx| {
                            let from = zoom.current(&frame, still);
                            let middle = (frame.stage.0 / 2., frame.stage.1 / 2.);
                            let to = match by {
                                0 => View::default(),
                                by if by > 0 => frame.zoom_at(from, STEP_ZOOM, middle),
                                _ => frame.zoom_at(from, 1. / STEP_ZOOM, middle),
                            };
                            zoom.glide_to(to, still);
                            cx.notify();
                        });
                    }
                })
                .child(stage_view)
                .child(bar)
                .tag("lightbox"),
        )
        .with_priority(20)
        .into_any_element()
    }
}

/// The zoom as the lightbox's pill says it: "Fit", "2.5×", "4×".
fn zoom_words(zoom: f32) -> String {
    if zoom <= 1.001 {
        return "Fit".into();
    }
    let rounded = (zoom * 10.).round() / 10.;
    if rounded.fract() == 0. { format!("{rounded:.0}×") } else { format!("{rounded:.1}×") }
}

/// A quiet line for a reply's picture reference nothing in the conversation answers to.
pub fn missing(reference: &str, theme: &Theme) -> impl IntoElement {
    div().typography(Type::Small).text_color(theme.text_faint).child(format!("Picture {reference} isn't in this chat."))
}

/// The picture references (`img:…`) a reply's Markdown shows as pictures, in order, each once.
pub fn referenced(blocks: &[crate::markdown::Block]) -> Vec<String> {
    fn walk(blocks: &[crate::markdown::Block], out: &mut Vec<String>) {
        use crate::markdown::Block;
        for block in blocks {
            let spans: Vec<&crate::markdown::Span> = match block {
                Block::Paragraph(spans) | Block::Heading(_, spans) => spans.iter().collect(),
                Block::Table { header, rows } => header.iter().flatten().chain(rows.iter().flatten().flatten()).collect(),
                Block::List { items, .. } => {
                    for item in items {
                        walk(&item.blocks, out);
                    }
                    Vec::new()
                }
                Block::Quote(inner) => {
                    walk(inner, out);
                    Vec::new()
                }
                _ => Vec::new(),
            };
            for span in spans {
                if let Some(reference) = &span.picture
                    && !out.contains(reference)
                {
                    out.push(reference.clone());
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(blocks, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_answers_to_its_reference_only() {
        let picture = Picture::new("AAAA", Some("shot"), Some("3f2a9c01b4d7e5f6"));
        assert!(picture.answers_to("img:3f2a9c01b4d7"));
        assert!(!picture.answers_to("img:3f2"));
        assert!(!picture.answers_to("img:000000000000"));
        let read = Picture::new("BBBB", Some("assets/banner.png"), Some("0123456789abcdef"));
        assert!(read.answers_to("banner.png") && read.answers_to("./assets/banner.png") && read.answers_to("/repo/assets/banner.png"));
        assert!(!read.answers_to("other.png") && !read.answers_to("anner.png"));
        assert_eq!(picture.key, Picture::new("AAAA", None, None).key, "the key follows the data");
    }

    #[test]
    fn the_lightbox_steps_within_its_pictures() {
        let pictures = Arc::new(vec![Picture::new("A", None, None), Picture::new("B", None, None)]);
        let mut lightbox = Lightbox::new(pictures, 9);
        assert_eq!(lightbox.index, 1);
        lightbox.step(1);
        assert_eq!(lightbox.index, 1);
        lightbox.step(-5);
        assert_eq!(lightbox.index, 0);
    }

    #[test]
    fn a_small_screenshot_is_enlarged_to_fit_the_window() {
        // A computer-use screenshot, the size a model reads, in a 1440×900 window.
        let frame = Frame::new((1372., 891.), (1440., 824.));
        assert!((frame.fitted.1 - (824. - 2. * FIT_MARGIN)).abs() < 0.5, "{frame:?}");
        assert!(frame.fitted.0 > 1100., "fills the window, not half of it: {frame:?}");
        // A tiny picture grows, but only so far.
        let icon = Frame::new((32., 32.), (1440., 824.));
        assert_eq!(icon.fitted, (32. * MOST_FITTED, 32. * MOST_FITTED));
        assert!(frame.most > 4. && icon.most >= 1.);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer_and_the_stage_covered() {
        let frame = Frame::new((1000., 500.), (1080., 580.));
        let fitted = View::default();
        assert_eq!(frame.settle(fitted), fitted);
        // A click near the top-left corner zooms in about it.
        let at = (frame.origin(fitted).0 + 100., frame.origin(fitted).1 + 50.);
        let zoomed = frame.zoom_at(fitted, CLICK_ZOOM, at);
        assert_eq!(zoomed.zoom, CLICK_ZOOM);
        let under = |view: View| {
            let (x, y) = frame.origin(view);
            let (width, height) = frame.drawn(view.zoom);
            ((at.0 - x) / width, (at.1 - y) / height)
        };
        let (before, after) = (under(fitted), under(zoomed));
        assert!((before.0 - after.0).abs() < 0.001 && (before.1 - after.1).abs() < 0.001, "{before:?} {after:?}");
        // At the very corner it can't be held there without baring the stage: the edge stops it.
        let cornered = frame.zoom_at(fitted, CLICK_ZOOM, frame.origin(fitted));
        assert_eq!(frame.origin(cornered), (0., 0.));
        // Dragging far right stops at the picture's left edge.
        let moved = frame.pan(zoomed, (10_000., 0.));
        assert_eq!(frame.origin(moved).0, 0.);
        // Zooming out past fitting settles at fitting, centred.
        assert_eq!(frame.zoom_at(moved, 0.01, (0., 0.)), View::default());
        // Never past the most.
        assert_eq!(frame.zoom_at(fitted, 1_000., at).zoom, frame.most);
    }

    #[test]
    fn the_zoom_reads_as_fit_or_a_multiple() {
        assert_eq!(zoom_words(1.), "Fit");
        assert_eq!(zoom_words(2.5), "2.5×");
        assert_eq!(zoom_words(3.98), "4×");
    }

    #[test]
    fn a_reply_names_its_pictures_once_in_order() {
        let blocks = crate::markdown::parse("Before ![the page](img:aaaaaaaaaaaa) and\n\n- ![x](img:bbbbbbbbbbbb)\n\n![again](img:aaaaaaaaaaaa) ![web](https://example.com/a.png) ![file](banner.png)");
        assert_eq!(referenced(&blocks), ["img:aaaaaaaaaaaa", "img:bbbbbbbbbbbb", "banner.png"]);
    }
}
