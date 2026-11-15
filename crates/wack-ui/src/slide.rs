//! Dragging a slider's knob: shared by the composer's effort slider and Settings' sliders.
//!
//! A press on a track starts a slide ([`begin`]); from then until the button is released the
//! pointer is followed anywhere in the window, not only over the track, so running past either
//! end reaches the end ([`follow`]). Views are rebuilt every frame, so which track is sliding is
//! remembered here rather than in an element.

use std::{cell::Cell, rc::Rc};

use gpui::{App, Bounds, Canvas, DispatchPhase, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Window, canvas};

/// What a slider does with the pointer's x, in window coordinates.
pub type Pick = Rc<dyn Fn(Pixels, &mut Window, &mut App)>;

thread_local! {
    /// The track a press began on, while the button stays down.
    static SLIDING: Cell<Option<&'static str>> = const { Cell::new(None) };
}

/// A press on track `id`: the slide starts.
pub fn begin(id: &'static str) {
    SLIDING.with(|sliding| sliding.set(Some(id)));
}

/// Put inside track `id`, filling it: records where the track was drawn into `drawn`, and hands
/// every pointer move of a slide that began on this track to `pick` until the button is released.
pub fn follow(id: &'static str, drawn: Rc<Cell<Option<Bounds<Pixels>>>>, pick: Pick) -> Canvas<()> {
    canvas(
        move |bounds, _, _| drawn.set(Some(bounds)),
        move |_, _, window, _| {
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble
                    && event.pressed_button == Some(MouseButton::Left)
                    && SLIDING.with(|sliding| sliding.get() == Some(id))
                {
                    pick(event.position.x, window, cx)
                }
            });
            window.on_mouse_event(|_: &MouseUpEvent, phase, _, _| {
                if phase == DispatchPhase::Bubble {
                    SLIDING.with(|sliding| sliding.set(None));
                }
            });
        },
    )
}
