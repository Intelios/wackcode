//! Motion: the few movements that make the app feel alive, in one place.
//!
//! - **Presence** ([`enter`]): something new arrives by rising a few points as it fades in. A
//!   screen, a menu, a settings section.
//! - **From an edge** ([`slide_in`]): a dock's pane arrives from the side it opens on.
//! - **At work** ([`paddling`]): the duck paddles while the agent works.
//!
//! Arrivals move on a [`spring`]: critically damped, so they start quickly and settle without
//! overshooting. Nothing winds up or bounces, so nothing gets in the way of reading.
//!
//! The duck paddles in whole steps, a few a second, like a figure in a print rather than a
//! cartoon: it suits the halftone backdrop, and an agent that works for minutes costs a few
//! redraws a second instead of one every frame. The shell takes the steps ([`paddle_on`]) and
//! puts the duck back at rest ([`paddle_rest`]) when nothing is working.
//!
//! Every movement respects the system's Reduce Motion setting: GPUI's `with_animation` draws the
//! end state of an arrival when [`App::reduce_motion`] is set, the shell takes no paddle steps,
//! and [`follow_system`] keeps that in step with macOS.
//!
//! `enter` and `slide_in` offset an element with relative positioning, so they are for elements
//! in the flow: wrap an absolutely placed element in its own positioned box first.

use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use gpui::{Animation, AnimationExt, App, ElementId, IntoElement, Styled, px};

/// How long an arrival takes.
pub const ARRIVE: Duration = Duration::from_millis(260);
/// How far an arriving thing rises, in points.
const RISE: f32 = 8.;
/// How long the working duck holds each step of its paddle.
pub const PADDLE_STEP: Duration = Duration::from_millis(320);
/// The paddle's steps: how far above its rest the duck sits, in points.
const PADDLE: [f32; 4] = [0., -1., -1.5, -1.];
/// The paddle's current step; 0 is at rest.
static PADDLE_AT: AtomicUsize = AtomicUsize::new(0);
/// The spring's stiffness over one arrival: high enough that it has settled by the end.
const STIFFNESS: f32 = 9.;

/// Reads macOS's Reduce Motion setting into GPUI. The shell calls this at launch and whenever
/// the window comes forward: the setting can only change while the user is in System Settings.
pub fn follow_system(cx: &mut App) {
    cx.set_reduce_motion(system_reduces_motion());
}

fn system_reduces_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// A critically damped spring from 0 to 1 over the animation's time, scaled so it ends exactly
/// at rest.
pub fn spring(t: f32) -> f32 {
    let at = |t: f32| 1. - (1. + STIFFNESS * t) * (-STIFFNESS * t).exp();
    (at(t.clamp(0., 1.)) / at(1.)).min(1.)
}

/// Fades in while rising into place.
pub fn enter<E: IntoElement + Styled + 'static>(id: impl Into<ElementId>, element: E) -> impl IntoElement {
    element.with_animation(id, Animation::new(ARRIVE).with_easing(spring), |element, t| {
        element.opacity(t).relative().top(px((1. - t) * RISE))
    })
}

/// Fades in while sliding in from one side: `from_right` and `from_below` are the distances in
/// points it starts away from its place.
pub fn slide_in<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    element: E,
    from_right: f32,
    from_below: f32,
) -> impl IntoElement {
    element.with_animation(id, Animation::new(ARRIVE).with_easing(spring), move |element, t| {
        let rest = 1. - t;
        element.opacity(t).relative().left(px(from_right * rest)).top(px(from_below * rest))
    })
}

/// Raises a working duck to the paddle's current step.
pub fn paddling<E: Styled>(element: E) -> E {
    element.relative().top(px(PADDLE[PADDLE_AT.load(Ordering::Relaxed) % PADDLE.len()]))
}

/// Takes the paddle's next step.
pub fn paddle_on() {
    PADDLE_AT.fetch_add(1, Ordering::Relaxed);
}

/// Puts the duck back at rest.
pub fn paddle_rest() {
    PADDLE_AT.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spring_settles_without_overshooting() {
        assert_eq!(spring(0.), 0.);
        assert_eq!(spring(1.), 1.);
        let mut last = 0.;
        for step in 1..=100 {
            let value = spring(step as f32 / 100.);
            assert!(value >= last && value <= 1., "step {step}: {value}");
            last = value;
        }
        assert!(spring(0.3) > 0.7, "most of the way there early: {}", spring(0.3));
    }
}
