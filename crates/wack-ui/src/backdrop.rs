//! The image backdrop, after WackCode 2 and Zeron.
//!
//! - **Behind everything** ([`behind`]): on every screen the picture fills the window, blurred
//!   and dimmed towards the window's colour exactly as the user chose. The sidebar, the stage
//!   and the side panel are thin glass panes over it (`glass.rs`), painted from a copy of the
//!   picture with its colour boosted ([`vibrant`]) and blurred exactly as the user chose, no more:
//!   the glass tints, it does not soften of its own accord. Only the window's frame around the
//!   panes shows the picture untinted.
//! - **The hero** ([`hero`]): on the new-chat and welcome screens the whole stage shows the
//!   picture clear, as it is or through a texture, undimmed and uncut: the owner wants to see it
//!   there, and a patch of glass behind the heading read as a dim. The screen's smaller words sit
//!   on frosted pills instead (`shell::on_ground`). When the first message is sent the hero fades
//!   away and the glass is left (the shell eases it, `motion::Ease`).
//!
//! Invariants:
//! - Both layers are fitted to the whole window with the same focus ([`cover`]), so the clear
//!   picture lies exactly over the soft one, and neither moves when the sidebar comes and goes.
//! - The hero stays inside the stage. The frame, the panes and the title strip's tray keep the
//!   blurred, dimmed picture the theme's readability sums assume (`Theme::glass`).
//! - A texture is worked out once per picture, texture, and light or dark, off the UI thread,
//!   and kept ([`artwork`]). Until it is ready the hero is simply not drawn. The plain picture is
//!   kept larger than a textured one ([`PLAIN_EDGE`]), since it is shown clear across a Retina
//!   stage.
//! - Behind a conversation the picture is drawn already blurred ([`baked`]): two small copies,
//!   plain and vibrant, made once off the UI thread for the blur chosen and the window's size. The
//!   GPU blurs live only until they are ready, so a scrolling chat costs it no blur at all. Until
//!   then a heavy blur starts from the softened copy made on import, so the GPU only blurs the
//!   rest of the way ([`SOFT_COPY_BLUR`]).
//!
//! The textures and the hero's mask are adapted from Zeron (MIT, github.com/zeronsh/zeron,
//! `crates/ui/src/new_thread_background_effects.rs` and `new_thread_background_mask.rs`).

use std::{path::PathBuf, sync::Arc};

use gpui::{
    App, Bounds, Corners, Global, Hsla, ImageSource, ObjectFit, Pixels, RenderImage, SharedString, Window, canvas,
    div, img, point, prelude::*, px, size,
};
use wack_core::Texture;

use crate::panes::Rect;

/// The long side of the copy textures are drawn from.
const ARTWORK_EDGE: u32 = 2048;
/// The long side of the plain picture the hero shows: enough for a full-screen Retina window.
pub const PLAIN_EDGE: u32 = 3840;
/// How many textures and decoded pictures are kept.
const KEPT: usize = 4;

/// What the shell needs to draw a backdrop.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub path: SharedString,
    /// The softened copy, for behind a conversation.
    pub soft_path: SharedString,
    pub width: u32,
    pub height: u32,
    pub focus: (f32, f32),
    /// How much the picture is magnified over a fit that covers the window, 1 to 4.
    pub zoom: f32,
    /// 0–1: how much of the window's colour the user lays over the picture, drawn exactly.
    pub dim: f32,
    /// How soft the picture is, in points.
    pub blur: f32,
    pub texture: Texture,
    /// How dark and how light the softened copy gets, 0–255 (`wack_core::appearance::shade`).
    pub shade: [u8; 2],
}

impl Picture {
    /// The picture as the theme needs it, to keep text readable over it.
    pub fn ground(&self) -> crate::theme::Ground {
        crate::theme::Ground { dim: self.dim, shade: self.shade }
    }
}

/// The picture drawn to cover `frame`, magnified `zoom` times over a fit that just covers it, with
/// `focus` (0–1 across and down) saying which part of what overflows shows: 0 flush with the left
/// or top edge, 1 with the right or bottom. Being a share of the overflow, a focus set in one
/// window's shape still fills another.
pub fn cover(frame: Rect, picture: (u32, u32), focus: (f32, f32), zoom: f32) -> Rect {
    let aspect = picture.0.max(1) as f32 / picture.1.max(1) as f32;
    let zoom = zoom.max(1.);
    let (width, height) = if frame.width / frame.height.max(1.) > aspect {
        (frame.width * zoom, frame.width / aspect * zoom)
    } else {
        (frame.height * aspect * zoom, frame.height * zoom)
    };
    Rect {
        x: frame.x - (width - frame.width) * focus.0.clamp(0., 1.),
        y: frame.y - (height - frame.height) * focus.1.clamp(0., 1.),
        width,
        height,
    }
}

/// `focus` after dragging the picture `delta` points across `frame` (a window, or a miniature of
/// one): the picture follows the pointer, and an axis with nothing overflowing stays put.
pub fn pan(frame: Rect, picture: (u32, u32), focus: (f32, f32), zoom: f32, delta: (f32, f32)) -> (f32, f32) {
    let drawn = cover(frame, picture, focus, zoom);
    let axis = |focus: f32, delta: f32, overflow: f32| if overflow < 0.5 { focus } else { (focus - delta / overflow).clamp(0., 1.) };
    (axis(focus.0, delta.0, drawn.width - frame.width), axis(focus.1, delta.1, drawn.height - frame.height))
}

/// `focus` at a new `zoom`, keeping whatever was at the middle of `frame` at its middle.
pub fn zoom_focus(frame: Rect, picture: (u32, u32), focus: (f32, f32), from: f32, to: f32) -> (f32, f32) {
    let before = cover(frame, picture, focus, from);
    let after = cover(frame, picture, focus, to);
    let axis = |focus: f32, was: f32, now: f32, visible: f32| {
        let overflow = now - visible;
        if overflow < 0.5 {
            return 0.5;
        }
        // The share of the picture at the frame's middle, carried across the zoom.
        let middle = (focus * (was - visible) + visible / 2.) / was;
        ((middle * now - visible / 2.) / overflow).clamp(0., 1.)
    };
    (axis(focus.0, before.width, after.width, frame.width), axis(focus.1, before.height, after.height, frame.height))
}

/// About how soft the copy made on import already is, in points across a laptop's window.
pub const SOFT_COPY_BLUR: f32 = 14.;

/// Which copy of the picture to draw for a blur of `blur` points, and how much more the GPU must
/// blur it: blurs add in squares, so the softened copy covers its share of a heavy one. The glass
/// panes follow the same plan, adding no softness of their own: with Blur at None the picture
/// shows through them as sharp as it is, only tinted, as it did behind WackCode 2's chats.
pub fn blur_plan(blur: f32) -> (bool, f32) {
    if blur >= SOFT_COPY_BLUR {
        (true, (blur * blur - SOFT_COPY_BLUR * SOFT_COPY_BLUR).sqrt())
    } else {
        (false, blur.max(0.))
    }
}

/// The picture across the whole `window`, blurred and dimmed as the user chose: `dim` of
/// `ground` (the window's colour) over it. Drawn from the `baked` copy when there is one;
/// otherwise the GPU blurs it, starting from the softened copy for a heavy blur.
pub fn behind(picture: &Picture, window: Rect, ground: Hsla, dim: f32, baked: Option<Arc<RenderImage>>) -> impl IntoElement + use<> {
    let drawn = cover(window, (picture.width, picture.height), picture.focus, picture.zoom);
    let (soft, blur) = blur_plan(picture.blur);
    let blur = if baked.is_some() { 0. } else { blur };
    let source: ImageSource = match baked {
        Some(baked) => baked.into(),
        None => PathBuf::from(if soft { picture.soft_path.as_ref() } else { picture.path.as_ref() }).into(),
    };
    div()
        .absolute()
        .left_0()
        .top_0()
        .w(px(window.width))
        .h(px(window.height))
        .overflow_hidden()
        .child(
            img(source)
                .absolute()
                .left(px(drawn.x))
                .top(px(drawn.y))
                .w(px(drawn.width))
                .h(px(drawn.height))
                .object_fit(ObjectFit::Fill),
        )
        .when(blur > 0.5, |behind| {
            behind.child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window: &mut Window, _| {
                        window.paint_layer(bounds, |window| window.paint_backdrop_blur(bounds, Corners::default(), px(blur)));
                    },
                )
                .absolute()
                .left_0()
                .top_0()
                .size_full(),
            )
        })
        .child(div().absolute().left_0().top_0().size_full().bg(Hsla { a: dim.clamp(0., 1.), ..ground }))
}

/// The clear picture filling its parent (the stage), fitted to the whole `window` like
/// [`behind`], with `radius`-rounded corners.
pub fn hero(artwork: Arc<RenderImage>, picture: &Picture, window: Rect, radius: f32) -> impl IntoElement + use<> {
    let fitted = cover(window, (picture.width, picture.height), picture.focus, picture.zoom);
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window: &mut Window, _| {
            let fitted = Bounds::new(point(px(fitted.x), px(fitted.y)), size(px(fitted.width), px(fitted.height)));
            let _ = window.paint_image_fitted(bounds, fitted, Corners::all(px(radius)), artwork, 0, false);
        },
    )
    .absolute()
    .left_0()
    .top_0()
    .size_full()
}

/// A picture decoded once, small enough to texture quickly.
struct Source {
    width: u32,
    height: u32,
    luma: Vec<u8>,
    colours: Vec<[u8; 4]>,
}

/// A textured picture's key: its path, texture, and whether it is for a light window.
type ArtworkKey = (SharedString, Texture, bool);

#[derive(Default)]
struct Artworks {
    /// Decoded pictures, by path.
    sources: Vec<(SharedString, Arc<Source>)>,
    /// Textured pictures by path, texture and light; `None` while one is being made.
    images: Vec<(ArtworkKey, Option<Arc<RenderImage>>)>,
}

impl Global for Artworks {}

/// The picture as the hero draws it, or `None` while it is being made (the windows redraw
/// when it is ready).
pub fn artwork(picture: &Picture, light: bool, cx: &mut App) -> Option<Arc<RenderImage>> {
    // Dither and the plain picture look the same in light and dark.
    let light = light && !matches!(picture.texture, Texture::None | Texture::Dither);
    let key = (picture.path.clone(), picture.texture, light);
    let artworks = cx.default_global::<Artworks>();
    if let Some((_, image)) = artworks.images.iter().find(|(cached, _)| *cached == key) {
        return image.clone();
    }
    artworks.images.push((key.clone(), None));
    if artworks.images.len() > KEPT {
        artworks.images.remove(0);
    }
    let source = artworks.sources.iter().find(|(path, _)| *path == key.0).map(|(_, source)| source.clone());
    let (path, texture) = (key.0.clone(), key.1);
    cx.spawn(async move |cx| {
        let made = cx
            .background_executor()
            .spawn(async move {
                if texture == Texture::None {
                    return Some((None, Arc::new(plain(path.as_ref())?)));
                }
                let source = match source {
                    Some(source) => source,
                    None => Arc::new(Source::open(path.as_ref())?),
                };
                let image = Arc::new(source.render(texture, light));
                Some((Some(source), image))
            })
            .await;
        cx.update(|cx| {
            let artworks = cx.default_global::<Artworks>();
            let Some((source, image)) = made else {
                // A picture that cannot be read leaves no hero, and is not retried until it changes.
                return;
            };
            if let Some(source) = source
                && !artworks.sources.iter().any(|(path, _)| *path == key.0)
            {
                artworks.sources.push((key.0.clone(), source));
                if artworks.sources.len() > KEPT {
                    artworks.sources.remove(0);
                }
            }
            if let Some(entry) = artworks.images.iter_mut().find(|(cached, _)| *cached == key) {
                entry.1 = Some(image);
            }
            cx.refresh_windows();
        });
    })
    .detach();
    None
}

/// How much the glass's copy of the picture boosts its colour, as WackCode 2's glass did with
/// `saturate(1.55)`: laying the window's colour over a picture greys it, and this gives it back.
pub const VIBRANCY: f32 = 1.55;

/// The glass panes' copies of pictures, by the copy's path (the clear one's ends in `#clear`);
/// `None` while one is being made.
#[derive(Default)]
struct Vibrant {
    images: Vec<(SharedString, Option<Arc<RenderImage>>)>,
}

impl Global for Vibrant {}

/// A copy of the picture with its colour boosted by [`VIBRANCY`], for the glass panes to lay under
/// themselves (`glass::Under`): the softened copy made on import (`soft`), or the clear picture
/// brought down to [`PLAIN_EDGE`]; `None` while it is being made (the windows redraw when it
/// is ready). The files on disk are never changed.
pub fn vibrant(picture: &Picture, soft: bool, cx: &mut App) -> Option<Arc<RenderImage>> {
    let key: SharedString = if soft { picture.soft_path.clone() } else { format!("{}#clear", picture.path).into() };
    let vibrant = cx.default_global::<Vibrant>();
    if let Some((_, image)) = vibrant.images.iter().find(|(path, _)| *path == key) {
        return image.clone();
    }
    vibrant.images.push((key.clone(), None));
    if vibrant.images.len() > KEPT {
        vibrant.images.remove(0);
    }
    let path = if soft { picture.soft_path.clone() } else { picture.path.clone() };
    cx.spawn(async move |cx| {
        let made = cx.background_executor().spawn(async move { saturated(path.as_ref(), !soft) }).await;
        cx.update(|cx| {
            // A copy that cannot be read leaves the panes blurring live, and is not retried.
            let Some(image) = made else { return };
            if let Some(entry) = cx.default_global::<Vibrant>().images.iter_mut().find(|(path, _)| *path == key) {
                entry.1 = Some(Arc::new(image));
            }
            cx.refresh_windows();
        });
    })
    .detach();
    None
}

/// The picture at `path`, its colour boosted by [`VIBRANCY`], in the BGRA order GPUI draws. A
/// `clear` copy is brought down to [`PLAIN_EDGE`], enough for a full-screen Retina window.
fn saturated(path: &str, clear: bool) -> Option<RenderImage> {
    let picture = image::open(path).ok()?;
    let picture = if clear && picture.width().max(picture.height()) > PLAIN_EDGE { picture.thumbnail(PLAIN_EDGE, PLAIN_EDGE) } else { picture };
    let mut pixels = picture.to_rgba8();
    for pixel in pixels.pixels_mut() {
        let [r, g, b, a] = pixel.0;
        let [r, g, b] = boost([r, g, b], VIBRANCY);
        pixel.0 = [b, g, r, a];
    }
    Some(RenderImage::new([image::Frame::new(pixels)]))
}

/// `colour` moved `amount` times as far from its own grey, keeping its luma, so it gains colour
/// without getting lighter or darker: the theme's readability sums still hold over it.
fn boost([r, g, b]: [u8; 3], amount: f32) -> [u8; 3] {
    let luma = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
    let channel = |value: u8| (luma + (value as f32 - luma) * amount).round().clamp(0., 255.) as u8;
    [channel(r), channel(g), channel(b)]
}

/// The picture already blurred as the user chose, made once off the UI thread: a copy for behind
/// everything ([`behind`]) and a vibrant one for the glass (`glass::Under`). The window draws these
/// as plain images. A live blur instead costs the GPU a pass over the framebuffer for the window
/// and for each pane, every frame: on an M1 that held a scrolling chat well under 60 frames a
/// second, though the picture never changes while it scrolls.
///
/// A copy is small, since it is blurred ([`BAKE_SIGMA`] of its own pixels at most), and its blur
/// depends on how wide the picture is drawn, so its width moves in steps of [`BAKE_STEP`]: resizing
/// the window makes a new copy only now and then, and a copy within [`BAKE_SLACK`] of the blur
/// wanted stands in while the exact one is made.
#[derive(Clone)]
pub struct Baked {
    pub plain: Arc<RenderImage>,
    pub vibrant: Arc<RenderImage>,
    /// The copies' width, and their blur in their own pixels.
    width: u32,
    sigma: f32,
}

impl Baked {
    /// How soft the copies look drawn `drawn` points wide, in points.
    fn blur_at(&self, drawn: f32) -> f32 {
        self.sigma * drawn / self.width as f32
    }
}

/// A baked copy's blur in its own pixels: soft enough that drawing it larger shows no steps.
const BAKE_SIGMA: f32 = 4.;
/// The widest a baked copy is made, for the lightest blurs.
const BAKE_EDGE: u32 = 2048;
/// Each width a baked copy can have is this much wider than the one before.
const BAKE_STEP: f32 = 1.08;
/// How far a ready copy's blur may be from the one wanted, as a share, to stand in for it.
const BAKE_SLACK: f32 = 0.2;

/// A baked copy's key: the picture, the copy's width, and its blur in quarter pixels.
type BakeKey = (SharedString, u32, u32);

/// Baked copies by key; `None` while one is being made. Beside them, each picture decoded once at
/// no more than [`BAKE_EDGE`], so a new copy (the window resized, the blur changed) starts from
/// that rather than from the file.
#[derive(Default)]
struct BakedCopies {
    copies: Vec<(BakeKey, Option<Baked>)>,
    sources: Vec<(SharedString, Arc<image::DynamicImage>)>,
}

impl Global for BakedCopies {}

/// What to bake for `picture` drawn `drawn` points wide: the copy's width and blur, or `None` when
/// the picture is not blurred at all.
fn bake_plan(picture: &Picture, drawn: f32) -> Option<(u32, f32)> {
    if picture.blur < 0.5 || drawn < 1. || picture.width == 0 || picture.height == 0 {
        return None;
    }
    let ideal = BAKE_SIGMA * drawn / picture.blur;
    let stepped = BAKE_STEP.powf((ideal.ln() / BAKE_STEP.ln()).round());
    // The long side no more than BAKE_EDGE, and never more than the picture has.
    let widest = (BAKE_EDGE as f32 * picture.width as f32 / picture.width.max(picture.height) as f32) as u32;
    let width = (stepped.round() as u32).clamp(16, widest.min(picture.width).max(16));
    Some((width, picture.blur * width as f32 / drawn))
}

/// The picture baked for drawing `drawn` points wide, or `None` when it needs no blur or no copy
/// close enough is ready yet (the windows redraw when it is): blur live meanwhile.
pub fn baked(picture: &Picture, drawn: f32, cx: &mut App) -> Option<Baked> {
    let (width, sigma) = bake_plan(picture, drawn)?;
    let key: BakeKey = (picture.path.clone(), width, (sigma * 4.).round() as u32);
    let copies = cx.default_global::<BakedCopies>();
    if let Some((_, Some(copy))) = copies.copies.iter().find(|(cached, _)| *cached == key) {
        return Some(copy.clone());
    }
    let near = copies
        .copies
        .iter()
        .filter(|((path, _, _), _)| *path == picture.path)
        .filter_map(|(_, copy)| copy.as_ref())
        .map(|copy| (copy, (copy.blur_at(drawn) / picture.blur - 1.).abs()))
        .filter(|(_, off)| *off <= BAKE_SLACK)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(copy, _)| copy.clone());
    if copies.copies.iter().any(|(cached, _)| *cached == key) {
        return near;
    }
    copies.copies.push((key.clone(), None));
    if copies.copies.len() > KEPT {
        copies.copies.remove(0);
    }
    let source = copies.sources.iter().find(|(path, _)| *path == picture.path).map(|(_, source)| source.clone());
    let path = picture.path.clone();
    let height = ((width as f32 * picture.height as f32 / picture.width as f32).round() as u32).max(1);
    cx.spawn(async move |cx| {
        let made = cx
            .background_executor()
            .spawn(async move {
                let source = match source {
                    Some(source) => source,
                    None => Arc::new(image::open(path.as_ref()).ok()?.thumbnail(BAKE_EDGE, BAKE_EDGE)),
                };
                let baked = bake(&source, width, height, sigma);
                Some((path, source, baked))
            })
            .await;
        cx.update(|cx| {
            // A picture that cannot be read leaves the panes blurring live, and is not retried.
            let Some((path, source, made)) = made else { return };
            let copies = cx.default_global::<BakedCopies>();
            if !copies.sources.iter().any(|(kept, _)| *kept == path) {
                copies.sources.push((path, source));
                if copies.sources.len() > 2 {
                    copies.sources.remove(0);
                }
            }
            if let Some(entry) = copies.copies.iter_mut().find(|(cached, _)| *cached == key) {
                entry.1 = Some(made);
            }
            cx.refresh_windows();
        });
    })
    .detach();
    near
}

/// `source` brought down to `width` by `height` and blurred `sigma` of those pixels, as it is and
/// with its colour boosted by [`VIBRANCY`] first, in the BGRA order GPUI draws.
fn bake(source: &image::DynamicImage, width: u32, height: u32, sigma: f32) -> Baked {
    let small = source.thumbnail_exact(width, height).to_rgba8();
    let mut boosted = small.clone();
    for pixel in boosted.pixels_mut() {
        let [r, g, b, a] = pixel.0;
        let [r, g, b] = boost([r, g, b], VIBRANCY);
        pixel.0 = [r, g, b, a];
    }
    let finish = |pixels: &image::RgbaImage| {
        let mut blurred = image::imageops::fast_blur(pixels, sigma);
        for pixel in blurred.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        Arc::new(RenderImage::new([image::Frame::new(blurred)]))
    };
    Baked { plain: finish(&small), vibrant: finish(&boosted), width, sigma }
}

/// The picture as it is, up to [`PLAIN_EDGE`] on its long side, in the BGRA order GPUI draws.
fn plain(path: &str) -> Option<RenderImage> {
    let picture = image::open(path).ok()?;
    let picture = if picture.width().max(picture.height()) > PLAIN_EDGE { picture.thumbnail(PLAIN_EDGE, PLAIN_EDGE) } else { picture };
    let mut pixels = picture.to_rgba8();
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(RenderImage::new([image::Frame::new(pixels)]))
}

impl Source {
    fn open(path: &str) -> Option<Source> {
        let proxy = image::open(path).ok()?.thumbnail(ARTWORK_EDGE, ARTWORK_EDGE);
        let luma = proxy.to_luma8();
        Some(Source {
            width: luma.width(),
            height: luma.height(),
            luma: luma.into_raw(),
            colours: proxy.to_rgba8().pixels().map(|pixel| pixel.0).collect(),
        })
    }

    fn colour(&self, x: u32, y: u32) -> [u8; 4] {
        self.colours[(y.min(self.height - 1) * self.width + x.min(self.width - 1)) as usize]
    }

    fn luma(&self, x: u32, y: u32) -> u8 {
        self.luma[(y.min(self.height - 1) * self.width + x.min(self.width - 1)) as usize]
    }

    /// The picture through `texture`, in the BGRA order GPUI draws.
    fn render(&self, texture: Texture, light: bool) -> RenderImage {
        let pixels = match texture {
            Texture::None => image::RgbaImage::from_fn(self.width, self.height, |x, y| {
                let [r, g, b, a] = self.colour(x, y);
                image::Rgba([b, g, r, a])
            }),
            Texture::Dither => self.dither(),
            Texture::Ascii => self.ascii(light),
            Texture::Halftone => self.halftone(light),
            Texture::Scanlines => self.scanlines(light),
        };
        RenderImage::new([image::Frame::new(pixels)])
    }

    /// Every third row dimmed (lightened on a light ground), like a cathode-ray screen.
    fn scanlines(&self, light: bool) -> image::RgbaImage {
        image::RgbaImage::from_fn(self.width, self.height, |x, y| {
            let [r, g, b, a] = self.colour(x, y);
            let gain = if y % 3 == 0 { 0.52 } else { 1. };
            let channel = |value: u8| {
                if light { (value as f32 + (255. - value as f32) * (1. - gain)) as u8 } else { (value as f32 * gain) as u8 }
            };
            image::Rgba([channel(b), channel(g), channel(r), a])
        })
    }

    /// Small bitmap glyphs, denser where the picture is brighter (darker, on a light ground),
    /// over the picture itself.
    fn ascii(&self, light: bool) -> image::RgbaImage {
        // Five columns by seven rows, with a column and a row of spacing.
        const GLYPHS: [[u8; 7]; 10] = [
            [0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 4, 0],
            [0, 4, 0, 0, 4, 0, 0],
            [0, 0, 0, 14, 0, 0, 0],
            [0, 0, 14, 0, 14, 0, 0],
            [0, 4, 4, 31, 4, 4, 0],
            [0, 21, 14, 31, 14, 21, 0],
            [10, 10, 31, 10, 31, 10, 10],
            [17, 2, 4, 4, 8, 16, 17],
            [14, 17, 23, 21, 23, 16, 14],
        ];
        image::RgbaImage::from_fn(self.width, self.height, |x, y| {
            let (cell_x, cell_y) = (x / 6 * 6 + 3, y / 8 * 8 + 4);
            let luma = self.luma(cell_x, cell_y);
            let ink_density = if light { 255 - luma } else { luma };
            let glyph = ((ink_density as f32 / 255.).sqrt() * 9.) as usize;
            let ink = x % 6 < 5 && y % 8 < 7 && GLYPHS[glyph][y as usize % 8] & (1 << (4 - x % 6)) != 0;
            let [r, g, b, a] = self.colour(x, y);
            let [cr, cg, cb, _] = self.colour(cell_x, cell_y);
            let paper = if light { 255. } else { 0. };
            let mix = |base: u8, glyph: u8| (base as f32 * 0.6 + if ink { glyph as f32 * 0.4 } else { paper * 0.4 }) as u8;
            image::Rgba([mix(b, cb), mix(g, cg), mix(r, cr), a])
        })
    }

    /// Dots on a four-pixel grid, larger where the picture is brighter.
    fn halftone(&self, light: bool) -> image::RgbaImage {
        let paper = if light { 255 } else { 0 };
        let mut pixels = image::RgbaImage::from_pixel(self.width, self.height, image::Rgba([paper, paper, paper, 255]));
        for y in (0..self.height).step_by(4) {
            for x in (0..self.width).step_by(4) {
                let luma = self.luma(x, y);
                let luma = if light { 255 - luma } else { luma };
                let radius = 2. * (0.3 + 0.7 * (luma as f32 / 255.).sqrt());
                let [r, g, b, a] = self.colour(x + 2, y + 2);
                for dy in 0..4.min(self.height - y) {
                    for dx in 0..4.min(self.width - x) {
                        let distance = ((dx as f32 - 1.5).powi(2) + (dy as f32 - 1.5).powi(2)).sqrt();
                        let coverage = (radius + 0.5 - distance).clamp(0., 1.) * a as f32 / 255.;
                        let [sr, sg, sb, sa] = self.colour(x + dx, y + dy);
                        let blend = |source: u8, dot: u8| {
                            (source as f32 * 0.6 + (dot as f32 * coverage + paper as f32 * (1. - coverage)) * 0.4) as u8
                        };
                        pixels.put_pixel(x + dx, y + dy, image::Rgba([blend(sb, b), blend(sg, g), blend(sr, r), sa]));
                    }
                }
            }
        }
        pixels
    }

    /// Two-pixel dots through a 4×4 Bayer matrix: each dot either its colour at full brightness
    /// or nearly black.
    fn dither(&self) -> image::RgbaImage {
        const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
        let mut pixels = image::RgbaImage::new(self.width, self.height);
        for y in (0..self.height).step_by(2) {
            for x in (0..self.width).step_by(2) {
                let [r, g, b, a] = dither_colour(self.colour(x + 1, y + 1), BAYER[y as usize / 2 % 4][x as usize / 2 % 4]);
                for dy in 0..2.min(self.height - y) {
                    for dx in 0..2.min(self.width - x) {
                        pixels.put_pixel(x + dx, y + dy, image::Rgba([b, g, r, a]));
                    }
                }
            }
        }
        pixels
    }
}

fn dither_colour([r, g, b, a]: [u8; 4], threshold: u8) -> [u8; 4] {
    let peak = r.max(g).max(b) as f32;
    let bright = peak / 255. > (threshold as f32 + 0.5) / 16.;
    let gain = if bright { 255. / peak.max(1.) } else { 0.08 };
    [(r as f32 * gain).round() as u8, (g as f32 * gain).round() as u8, (b as f32 * gain).round() as u8, a]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Source {
        let (width, height) = (24, 16);
        let colours: Vec<[u8; 4]> = (0..width * height).map(|i| [(i * 7 % 256) as u8, 120, 200, 255]).collect();
        Source { width, height, luma: colours.iter().map(|c| ((c[0] as u32 + c[1] as u32 + c[2] as u32) / 3) as u8).collect(), colours }
    }

    #[test]
    fn every_texture_keeps_the_pictures_size_in_gpui_order() {
        let source = source();
        for texture in Texture::ALL {
            for light in [false, true] {
                let image = source.render(texture, light);
                let size = image.size(0);
                assert_eq!((size.width.0, size.height.0), (24, 16), "{texture:?}");
            }
        }
        // The plain picture is the source with red and blue swapped for GPUI.
        let plain = match image::RgbaImage::from_raw(24, 16, source.render(Texture::None, false).as_bytes(0).unwrap().to_vec()) {
            Some(plain) => plain,
            None => panic!("a plain picture of the source's size"),
        };
        let [r, g, b, a] = source.colour(5, 3);
        assert_eq!(plain.get_pixel(5, 3).0, [b, g, r, a]);
    }

    fn picture(width: u32, height: u32, blur: f32) -> Picture {
        Picture {
            path: "a.jpg".into(),
            soft_path: "a-soft.jpg".into(),
            width,
            height,
            focus: (0.5, 0.5),
            zoom: 1.,
            dim: 0.2,
            blur,
            texture: Texture::None,
            shade: [0, 255],
        }
    }

    #[test]
    fn a_baked_copy_is_small_and_as_soft_as_the_blur_chosen() {
        let landscape = picture(4096, 2304, 10.);
        assert_eq!(bake_plan(&picture(4096, 2304, 0.), 1680.), None, "no blur, nothing to bake");
        let (width, sigma) = bake_plan(&landscape, 2016.).unwrap();
        assert!(width < 1024, "blurred, so small: {width}");
        assert!((sigma - BAKE_SIGMA).abs() / BAKE_SIGMA < 0.05, "{sigma}");
        let blank = || Arc::new(RenderImage::new([image::Frame::new(image::RgbaImage::new(1, 1))]));
        let shown = Baked { plain: blank(), vibrant: blank(), width, sigma };
        assert!((shown.blur_at(2016.) - 10.).abs() < 0.01, "as soft as chosen at the window's size");
        // A window a little wider keeps the same copy; one much wider makes another.
        assert_eq!(bake_plan(&landscape, 2040.).unwrap().0, width);
        assert_ne!(bake_plan(&landscape, 2400.).unwrap().0, width);
        assert!((shown.blur_at(2040.) / 10. - 1.).abs() < BAKE_SLACK, "and stands in while it is made");
        // The lightest blur needs a large copy, but never beyond BAKE_EDGE on its long side.
        let (light, light_sigma) = bake_plan(&picture(4096, 2304, 1.), 2016.).unwrap();
        assert_eq!(light, BAKE_EDGE);
        assert!((light_sigma - 2048. / 2016.).abs() < 0.01);
        assert_eq!(bake_plan(&picture(1000, 4000, 1.), 2016.).unwrap().0, 512, "a tall picture's height is its long side");
        assert_eq!(bake_plan(&picture(800, 450, 1.), 2016.).unwrap().0, 800, "never wider than the picture");
    }

    #[test]
    fn baking_softens_both_copies_and_boosts_only_the_vibrant_one() {
        let mut sharp = image::RgbaImage::from_pixel(64, 36, image::Rgba([40, 90, 160, 255]));
        for y in 0..36 {
            for x in 32..64 {
                sharp.put_pixel(x, y, image::Rgba([220, 200, 60, 255]));
            }
        }
        let baked = bake(&image::DynamicImage::ImageRgba8(sharp), 32, 18, 2.);
        let pixels = |image: &RenderImage| image::RgbaImage::from_raw(32, 18, image.as_bytes(0).unwrap().to_vec()).unwrap();
        let (plain, vibrant) = (pixels(&baked.plain), pixels(&baked.vibrant));
        // GPUI's order: blue first. Far from the edge it is the picture's own colour…
        assert_eq!(plain.get_pixel(1, 9).0, [160, 90, 40, 255]);
        // …and across the edge it blends.
        let [b, ..] = plain.get_pixel(15, 9).0;
        assert!(b > 60 && b < 160, "softened across the edge: {b}");
        let [vb, vg, vr, _] = vibrant.get_pixel(1, 9).0;
        assert!(vb > 160 && vr < 40, "the vibrant copy has more colour: {:?}", [vr, vg, vb]);
    }

    #[test]
    fn vibrancy_adds_colour_and_keeps_greys() {
        assert_eq!(boost([128, 128, 128], VIBRANCY), [128, 128, 128], "a grey has no colour to boost");
        let [r, g, b] = boost([180, 120, 200], VIBRANCY);
        assert!(r > 180 && g < 120 && b > 200, "further from its grey: {:?}", [r, g, b]);
        let luma = |[r, g, b]: [u8; 3]| 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
        assert!((luma([r, g, b]) - luma([180, 120, 200])).abs() < 1.5, "as light as before");
    }

    #[test]
    fn dither_is_full_or_nearly_black() {
        assert_eq!(dither_colour([200, 100, 50, 255], 0), [255, 128, 64, 255]);
        assert_eq!(dither_colour([20, 10, 5, 255], 15), [2, 1, 0, 255]);
    }

    #[test]
    fn the_picture_covers_and_keeps_its_focus() {
        let frame = Rect { x: 0., y: 0., width: 1000., height: 500. };
        let tall = cover(frame, (1000, 1000), (0.5, 0.), 1.);
        assert_eq!((tall.width, tall.height, tall.y), (1000., 1000., 0.));
        let bottom = cover(frame, (1000, 1000), (0.5, 1.), 1.);
        assert_eq!(bottom.y, -500.);
        let wide = cover(frame, (4000, 1000), (0., 0.5), 1.);
        assert_eq!((wide.width, wide.height, wide.x), (2000., 500., 0.));
    }

    #[test]
    fn dragging_moves_the_picture_with_the_pointer_and_zoom_keeps_the_middle() {
        let frame = Rect { x: 0., y: 0., width: 400., height: 250. };
        // A square picture in a wide frame overflows only down: across it stays put.
        let moved = pan(frame, (1000, 1000), (0.5, 0.5), 1., (40., -30.));
        assert_eq!(moved.0, 0.5);
        assert!(moved.1 > 0.5, "dragging up shows more of the bottom");
        assert_eq!(pan(frame, (1000, 1000), (0.5, 1.), 1., (0., -500.)).1, 1., "never past the edge");
        // Zoomed in, the point at the middle stays at the middle.
        let at = |focus: (f32, f32), zoom: f32| {
            let drawn = cover(frame, (1000, 1000), focus, zoom);
            ((200. - drawn.x) / drawn.width, (125. - drawn.y) / drawn.height)
        };
        let before = at((0.3, 0.7), 1.5);
        let focus = zoom_focus(frame, (1000, 1000), (0.3, 0.7), 1.5, 3.);
        let after = at(focus, 3.);
        assert!((before.0 - after.0).abs() < 0.001 && (before.1 - after.1).abs() < 0.001, "{before:?} {after:?}");
    }

    #[test]
    fn a_heavy_blur_starts_from_the_softened_copy() {
        assert_eq!(blur_plan(0.), (false, 0.));
        assert_eq!(blur_plan(8.), (false, 8.));
        let (soft, more) = blur_plan(40.);
        assert!(soft && (more - (40f32 * 40. - SOFT_COPY_BLUR * SOFT_COPY_BLUR).sqrt()).abs() < 0.01);
    }
}

