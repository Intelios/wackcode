//! A ring that fills clockwise from the top: how full the model's context is, drawn round the
//! composer's Send button and large in the context popover.
//!
//! GPUI draws no arcs of its own, so the ring is stroked as a path: the track as a whole circle,
//! the share as a polyline fine enough to read as a curve at any size the app uses.

use gpui::{Hsla, IntoElement, PathBuilder, Pixels, Styled, canvas, point, px};

/// Segments in a whole turn; an arc uses its share of them, never fewer than two.
const SEGMENTS: f32 = 96.;

/// A ring `size` points across, `width` thick, `share` (0 to 1) of it in `fill` over `track`.
pub fn ring(share: f32, size: f32, width: f32, track: Hsla, fill: Hsla) -> impl IntoElement {
    let share = share.clamp(0., 1.);
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let centre = bounds.center();
            let radius = (size - width) / 2.;
            if let Some(path) = arc(centre, radius, width, 1.) {
                window.paint_path(path, track);
            }
            if share > 0.
                && let Some(path) = arc(centre, radius, width, share)
            {
                window.paint_path(path, fill);
            }
        },
    )
    .size(px(size))
    .flex_none()
}

/// `share` of a circle round `centre`, from twelve o'clock clockwise, as a stroke.
fn arc(centre: gpui::Point<Pixels>, radius: f32, width: f32, share: f32) -> Option<gpui::Path<Pixels>> {
    let mut path = PathBuilder::stroke(px(width));
    let steps = (SEGMENTS * share).ceil().max(2.) as usize;
    for step in 0..=steps {
        let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * share * step as f32 / steps as f32;
        let at = point(centre.x + px(radius * angle.cos()), centre.y + px(radius * angle.sin()));
        if step == 0 { path.move_to(at) } else { path.line_to(at) }
    }
    path.build().ok()
}
