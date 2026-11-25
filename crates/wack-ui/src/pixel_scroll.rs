//! A scroll container that only ever rests on whole device pixels.
//!
//! GPUI moves a scrolled box by each wheel event's exact delta, so a trackpad leaves it at
//! fractional offsets: text is snapped to the pixel grid while card fills and hairlines are drawn
//! between pixels, and a page of cards and words shimmers against itself as it moves. It feels
//! wrong without being slow (the owner's note on Settings, 2026-11-25). This container keeps the
//! exact offset to itself and shows it rounded to the display's pixels, so everything moves
//! together; slow scrolls still add up, because the exact offset is never thrown away.
//!
//! GPUI applies a wheel event to the box before any listener of ours sees it (bubble listeners
//! run newest first, and the box's own is added after ours), so ours reads where that left the
//! box, works out where it really should be, and puts it on the nearest pixel.

use std::{cell::Cell, rc::Rc};

use gpui::{AnyElement, App, ElementId, IntoElement, RenderOnce, ScrollHandle, ScrollWheelEvent, Window, div, point, prelude::*, px};

#[derive(IntoElement)]
pub struct PixelScroll {
    id: ElementId,
    child: AnyElement,
}

impl PixelScroll {
    /// A box scrolling `child` down; it fills what its parent gives it.
    pub fn new(id: impl Into<ElementId>, child: impl IntoElement) -> Self {
        Self { id: id.into(), child: child.into_any_element() }
    }
}

/// What a container keeps between frames: its handle, and its offset before rounding along
/// with the offset it last showed.
struct Kept {
    handle: ScrollHandle,
    exact: Rc<Cell<f32>>,
    shown: Rc<Cell<f32>>,
}

impl RenderOnce for PixelScroll {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let kept = window.use_keyed_state(self.id.clone(), cx, |_, _| Kept {
            handle: ScrollHandle::new(),
            exact: Rc::new(Cell::new(0.)),
            shown: Rc::new(Cell::new(0.)),
        });
        let (handle, exact, shown) = {
            let kept = kept.read(cx);
            (kept.handle.clone(), kept.exact.clone(), kept.shown.clone())
        };
        let listen = handle.clone();
        div()
            .id(self.id)
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&handle)
            .on_scroll_wheel(move |event: &ScrollWheelEvent, window, _| {
                let delta = f32::from(event.delta.pixel_delta(window.line_height()).y);
                let after = f32::from(listen.offset().y);
                let before = after - delta;
                // Moved some other way since (clamped at an end, or scrolled to a place): start
                // again from where it is.
                if (before - shown.get()).abs() > 0.01 {
                    exact.set(before);
                }
                let wanted = exact.get() + delta;
                exact.set(wanted);
                let rounded = round_to_pixels(wanted, window.scale_factor());
                shown.set(rounded);
                let offset = listen.offset();
                listen.set_offset(point(offset.x, px(rounded)));
            })
            .child(self.child)
    }
}

/// `offset` on the nearest device pixel of a display `scale` pixels to the point.
fn round_to_pixels(offset: f32, scale: f32) -> f32 {
    let scale = scale.max(1.);
    (offset * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_land_on_whole_device_pixels() {
        assert_eq!(round_to_pixels(-10.3, 2.), -10.5);
        assert_eq!(round_to_pixels(-10.2, 2.), -10.);
        assert_eq!(round_to_pixels(-7.4, 1.), -7.);
    }
}
