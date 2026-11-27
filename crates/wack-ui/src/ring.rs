//! A ring that fills clockwise from the top: how full the model's context is, drawn round the
//! composer's Send button and large in the context popover.
//!
//! GPUI draws no arcs, and its paths are dear: every frame with a path in it runs a pass over a
//! window-sized, multisampled texture, which cost an M1 5 to 7 ms of GPU a frame at 1680×1050
//! (2026-11-27), a third of the frame. So the ring is drawn on the CPU into a small picture at the
//! screen's scale (the track as a whole circle, the share over it with square ends), and a frame
//! only paints that picture.
//!
//! Invariants:
//! - A picture is made only when the ring's look changes: its share moves a step (a degree), or
//!   its size, colours or the screen's scale change. The last few are kept.
//! - A picture let go is dropped from GPUI's atlas too, or the atlas would grow as a run fills
//!   the context.
//! - Nothing here paints a path.

use std::{
    f32::consts::{FRAC_PI_2, TAU},
    sync::Arc,
};

use gpui::{App, Corners, Global, Hsla, IntoElement, RenderImage, Rgba, Styled, Window, canvas, px};

/// Steps in a whole turn: the share is drawn to the nearest one.
const STEPS: f32 = 360.;

/// Pictures kept: the composer's ring and the popover's, each in a few recent shares.
const KEPT: usize = 8;

/// A ring `size` points across, `width` thick, `share` (0 to 1) of it in `fill` over `track`.
pub fn ring(share: f32, size: f32, width: f32, track: Hsla, fill: Hsla) -> impl IntoElement {
    let share = share.clamp(0., 1.);
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let scale = window.scale_factor();
            let look = Look { pixels: (size * scale).round() as u32, scale, size, width, step: (share * STEPS).round() as u32, track, fill };
            let image = picture(look, window, cx);
            let _ = window.paint_image(bounds, Corners::default(), image, 0, false);
        },
    )
    .size(px(size))
    .flex_none()
}

/// Everything a ring's picture depends on.
#[derive(Clone, Copy, PartialEq)]
struct Look {
    /// Its side in device pixels.
    pixels: u32,
    scale: f32,
    size: f32,
    width: f32,
    /// The share, in steps of a turn.
    step: u32,
    track: Hsla,
    fill: Hsla,
}

/// Ring pictures, the most recently used last.
#[derive(Default)]
struct Rings(Vec<(Look, Arc<RenderImage>)>);

impl Global for Rings {}

fn picture(look: Look, window: &mut Window, cx: &mut App) -> Arc<RenderImage> {
    let rings = cx.default_global::<Rings>();
    if let Some(index) = rings.0.iter().position(|(kept, _)| *kept == look) {
        let entry = rings.0.remove(index);
        let image = entry.1.clone();
        rings.0.push(entry);
        return image;
    }
    let image = Arc::new(RenderImage::new([image::Frame::new(draw(look))]));
    rings.0.push((look, image.clone()));
    if rings.0.len() > KEPT {
        let (_, old) = rings.0.remove(0);
        let _ = window.drop_image(old);
    }
    image
}

/// The ring as BGRA pixels, as GPUI's pictures are.
fn draw(look: Look) -> image::RgbaImage {
    let side = look.pixels.max(1);
    let centre = side as f32 / 2.;
    let radius = (look.size - look.width) / 2. * look.scale;
    let half = look.width / 2. * look.scale;
    let share = look.step as f32 / STEPS;
    let (track, fill) = (Rgba::from(look.track), Rgba::from(look.fill));
    image::RgbaImage::from_fn(side, side, |x, y| {
        let (dx, dy) = (x as f32 + 0.5 - centre, y as f32 + 0.5 - centre);
        let distance = dx.hypot(dy);
        // How much of the pixel the stroke covers across its width, and along the turn.
        let band = (half - (distance - radius).abs() + 0.5).clamp(0., 1.);
        let along = if share >= 1. { 1. } else if share <= 0. { 0. } else { along(dx, dy, distance, share) };
        let [r, g, b, a] = over(fill, band * along, track, band);
        image::Rgba([b, g, r, a])
    })
}

/// How much of a pixel lies within the arc's turn, from twelve o'clock clockwise to `share`: one
/// inside, nothing beyond, and a pixel's worth of softening across each square end.
fn along(dx: f32, dy: f32, distance: f32, share: f32) -> f32 {
    let angle = dx.atan2(-dy).rem_euclid(TAU);
    let end = share * TAU;
    // Turned from the nearer end: positive inside the arc, negative outside it.
    let turned = if angle <= end { angle.min(end - angle) } else { -(angle - end).min(TAU - angle) };
    (0.5 + distance * turned.clamp(-FRAC_PI_2, FRAC_PI_2).sin()).clamp(0., 1.)
}

/// `top` covering `top_share` of a pixel, laid over `under` covering `under_share`, as straight
/// RGBA bytes: what painting the two strokes one over the other gives.
fn over(top: Rgba, top_share: f32, under: Rgba, under_share: f32) -> [u8; 4] {
    let top_alpha = top.a * top_share;
    let under_alpha = under.a * under_share * (1. - top_alpha);
    let alpha = top_alpha + under_alpha;
    if alpha <= 0. {
        return [0; 4];
    }
    let mix = |top: f32, under: f32| ((top * top_alpha + under * under_alpha) / alpha * 255.).round().clamp(0., 255.) as u8;
    [mix(top.r, under.r), mix(top.g, under.g), mix(top.b, under.b), (alpha * 255.).round().clamp(0., 255.) as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(step: u32) -> Look {
        let track = Hsla { h: 0., s: 0., l: 0.5, a: 1. };
        let fill = Hsla { h: 0.6, s: 0.8, l: 0.5, a: 1. };
        Look { pixels: 52, scale: 2., size: 26., width: 3., step, track, fill }
    }

    /// The pixel at `(x, y)` points from the ring's centre, in device pixels, as RGBA.
    fn at(picture: &image::RgbaImage, x: f32, y: f32) -> [u8; 4] {
        let centre = picture.width() as f32 / 2.;
        let [b, g, r, a] = picture.get_pixel((centre + x) as u32, (centre + y) as u32).0;
        [r, g, b, a]
    }

    #[test]
    fn the_share_is_drawn_clockwise_from_twelve_over_the_track() {
        let quarter = draw(look(90));
        let (track, fill) = (Rgba::from(look(0).track), Rgba::from(look(0).fill));
        let on_ring = (26. - 3.) / 2. * 2.;
        let byte = |value: f32| (value * 255.).round() as u8;
        // Just after twelve and just before three: the fill.
        let early = at(&quarter, 4., -on_ring);
        assert_eq!(early, [byte(fill.r), byte(fill.g), byte(fill.b), 255]);
        let late = at(&quarter, on_ring - 0.5, -4.);
        assert_eq!(late, early);
        // Past three, and at nine: only the track.
        assert_eq!(at(&quarter, on_ring - 0.5, 4.), [byte(track.r), byte(track.g), byte(track.b), 255]);
        assert_eq!(at(&quarter, -on_ring, 0.), [byte(track.r), byte(track.g), byte(track.b), 255]);
    }

    #[test]
    fn inside_and_outside_the_stroke_are_clear() {
        let full = draw(look(360));
        assert_eq!(at(&full, 0., 0.), [0; 4]);
        assert_eq!(full.get_pixel(0, 0).0, [0; 4]);
        assert_eq!(at(&full, 0., -23.)[3], 255);
    }

    #[test]
    fn a_translucent_track_lets_the_ground_through() {
        let mut faint = look(0);
        faint.track.a = 0.25;
        assert_eq!(at(&draw(faint), 0., 23.)[3], 64);
    }
}
