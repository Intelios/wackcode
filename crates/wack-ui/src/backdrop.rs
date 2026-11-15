//! The image backdrop, after WackCode 2 and Zeron.
//!
//! - **Behind everything** ([`behind`]): on every screen the picture fills the window, blurred
//!   and dimmed towards the window's colour exactly as the user chose. The sidebar and the stage
//!   are glass panes over it (`glass.rs`), thick enough that text on them reads; only the window's
//!   frame around them shows the picture as chosen.
//! - **The hero** ([`hero`]): on the new-chat and welcome screens the picture also shows clear
//!   across the top of the stage, as it is or through a texture. It fades out down its whole
//!   height and is feathered away around the centred block, so it dissolves into the glass
//!   without an edge.
//!
//! Invariants:
//! - Both layers are fitted to the whole window with the same focus ([`cover`]), so the clear
//!   picture lies exactly over the soft one, and neither moves when the sidebar comes and goes.
//! - The hero's mask is drawn by the GPU (`Window::paint_image_fitted_masked`): resizing the
//!   window or moving the block rasterizes nothing.
//! - A texture is worked out once per picture, texture, and light or dark, off the UI thread,
//!   and kept ([`artwork`]). Until it is ready the hero is simply not drawn.
//! - A heavy blur starts from the softened copy made on import, so the GPU only blurs the rest
//!   of the way ([`SOFT_COPY_BLUR`]).
//!
//! The textures and the hero's mask are adapted from Zeron (MIT, github.com/zeronsh/zeron,
//! `crates/ui/src/new_thread_background_effects.rs` and `new_thread_background_mask.rs`).

use std::{cell::Cell, path::PathBuf, rc::Rc, sync::Arc};

use gpui::{
    App, Bounds, Corners, Global, Hsla, ImageAlphaMask, ObjectFit, Pixels, RenderImage, SharedString, Window, canvas,
    div, img, point, prelude::*, px, size,
};
use wack_core::Texture;

use crate::panes::Rect;

/// The hero's height as a share of the space below the title strip, and its most.
const HERO_SHARE: f32 = 0.72;
const HERO_MAX: f32 = 760.;
/// The long side of the copy textures are drawn from.
const ARTWORK_EDGE: u32 = 2048;
/// How many textures and decoded pictures are kept.
const KEPT: usize = 4;

/// Where the centred block is, measured while the frame is laid out and read when the hero is
/// painted, so the mask follows the block in the same frame.
pub type Cutout = Rc<Cell<Option<Bounds<Pixels>>>>;

/// What the shell needs to draw a backdrop.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub path: SharedString,
    /// The softened copy, for behind a conversation.
    pub soft_path: SharedString,
    pub width: u32,
    pub height: u32,
    pub focus: (f32, f32),
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

/// How tall the hero is below a title strip, given the height under it.
pub fn hero_height(content_height: f32) -> f32 {
    (content_height * HERO_SHARE).clamp(0., HERO_MAX)
}

/// About how soft the copy made on import already is, in points across a laptop's window.
pub const SOFT_COPY_BLUR: f32 = 14.;

/// Which copy of the picture to draw for a blur of `blur` points, and how much more the GPU must
/// blur it: blurs add in squares, so the softened copy covers its share of a heavy one.
pub fn blur_plan(blur: f32) -> (bool, f32) {
    if blur >= SOFT_COPY_BLUR {
        (true, (blur * blur - SOFT_COPY_BLUR * SOFT_COPY_BLUR).sqrt())
    } else {
        (false, blur.max(0.))
    }
}

/// The picture across the whole `window`, blurred and dimmed as the user chose: `dim` of
/// `ground` (the window's colour) over it.
pub fn behind(picture: &Picture, window: Rect, ground: Hsla, dim: f32) -> impl IntoElement + use<> {
    let drawn = cover(window, (picture.width, picture.height), picture.focus);
    let (soft, blur) = blur_plan(picture.blur);
    let path = if soft { &picture.soft_path } else { &picture.path };
    div()
        .absolute()
        .left_0()
        .top_0()
        .w(px(window.width))
        .h(px(window.height))
        .overflow_hidden()
        .child(
            img(PathBuf::from(path.as_ref()))
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

/// The clear picture filling its parent, fitted to the whole `window` like [`behind`], fading out
/// downwards and feathered away around `cutout`; its top corners rounded by `radius`, to sit in
/// the stage.
pub fn hero(artwork: Arc<RenderImage>, picture: &Picture, window: Rect, height: f32, radius: f32, cutout: Cutout) -> impl IntoElement + use<> {
    let fitted = cover(window, (picture.width, picture.height), picture.focus);
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window: &mut Window, _| {
            let fitted = Bounds::new(point(px(fitted.x), px(fitted.y)), size(px(fitted.width), px(fitted.height)));
            let height = f32::from(bounds.size.height).max(1.);
            let fade = Some((bounds.bottom(), px(height)));
            let mask = match cutout.get() {
                // The cleared area runs on to the hero's foot, so the picture never comes back
                // in beneath the block.
                Some(block) => ImageAlphaMask {
                    bounds: Bounds::new(block.origin, size(block.size.width, block.bottom().max(bounds.bottom()) - block.top())),
                    radius: px(18.),
                    feather: px((height * 0.52).clamp(120., 280.)),
                    clearance: px(8.),
                    bottom_fade: fade,
                },
                // Nothing to clear: an exclusion below the hero leaves only the fade.
                None => ImageAlphaMask {
                    bounds: Bounds::new(point(bounds.left(), bounds.bottom() + px(1.)), bounds.size),
                    radius: px(0.),
                    feather: px(1.),
                    clearance: px(0.),
                    bottom_fade: fade,
                },
            };
            let corners = Corners { top_left: px(radius), top_right: px(radius), ..Default::default() };
            let _ = window.paint_image_fitted_masked(bounds, fitted, corners, artwork, 0, false, Some(mask));
        },
    )
    .absolute()
    .left_0()
    .top_0()
    .w_full()
    .h(px(height))
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
                let source = match source {
                    Some(source) => source,
                    None => Arc::new(Source::open(path.as_ref())?),
                };
                let image = Arc::new(source.render(texture, light));
                Some((source, image))
            })
            .await;
        cx.update(|cx| {
            let artworks = cx.default_global::<Artworks>();
            let Some((source, image)) = made else {
                // A picture that cannot be read leaves no hero, and is not retried until it changes.
                return;
            };
            if !artworks.sources.iter().any(|(path, _)| *path == key.0) {
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

    #[test]
    fn dither_is_full_or_nearly_black() {
        assert_eq!(dither_colour([200, 100, 50, 255], 0), [255, 128, 64, 255]);
        assert_eq!(dither_colour([20, 10, 5, 255], 15), [2, 1, 0, 255]);
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

    #[test]
    fn the_hero_takes_most_of_the_height_up_to_a_limit() {
        assert_eq!(hero_height(500.), 360.);
        assert_eq!(blur_plan(0.), (false, 0.));
        assert_eq!(blur_plan(8.), (false, 8.));
        let (soft, more) = blur_plan(40.);
        assert!(soft && (more - (40f32 * 40. - SOFT_COPY_BLUR * SOFT_COPY_BLUR).sqrt()).abs() < 0.01);
        assert_eq!(hero_height(2000.), HERO_MAX);
    }
}
