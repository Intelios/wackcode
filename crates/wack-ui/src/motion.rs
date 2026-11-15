//! Motion: the few movements that make the app feel alive, in one place.
//!
//! - **Presence** ([`enter`]): something new arrives by rising a few points as it fades in. A
//!   screen, a menu, a settings section.
//! - **From an edge** ([`slide_in`]): a dock's pane arrives from the side it opens on.
//! - **Between states** ([`Ease`]): the picture clears on a new chat and frosts over once the
//!   first message is sent.
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
    time::{Duration, Instant},
};

use gpui::{Animation, AnimationExt, App, ElementId, IntoElement, Styled, px};

/// How long an arrival takes.
pub const ARRIVE: Duration = Duration::from_millis(260);
/// How long something laid over the window takes to fade in.
const FADE: Duration = Duration::from_millis(450);
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

/// Fades in, without moving: for something laid over the window, such as the backdrop's hero.
pub fn fade_in<E: IntoElement + Styled + 'static>(id: impl Into<ElementId>, element: E) -> impl IntoElement {
    element.with_animation(id, Animation::new(FADE).with_easing(spring), |element, t| element.opacity(t))
}

/// A value that eases to each new target over [`FADE`], for a change that outlives any one
/// element, such as the picture clearing on a new chat and frosting over when it is sent. The
/// holder calls [`Ease::toward`] as it renders and asks for another frame while
/// [`Ease::moving`]. Nothing moves the first time, or with Reduce Motion on.
#[derive(Clone, Copy, Debug)]
pub struct Ease {
    from: f32,
    to: f32,
    since: Option<Instant>,
}

impl Ease {
    pub fn at(value: f32) -> Self {
        Self { from: value, to: value, since: None }
    }

    /// Where the value is now.
    pub fn value(&self) -> f32 {
        self.value_at(Instant::now())
    }

    fn value_at(&self, now: Instant) -> f32 {
        match self.since {
            Some(since) => {
                let t = now.saturating_duration_since(since).as_secs_f32() / FADE.as_secs_f32();
                self.from + (self.to - self.from) * spring(t)
            }
            None => self.to,
        }
    }

    pub fn moving(&self) -> bool {
        self.since.is_some_and(|since| since.elapsed() < FADE)
    }

    /// Heads for `to` from wherever the value is now; `still` jumps straight there.
    pub fn toward(&mut self, to: f32, still: bool) {
        if to == self.to {
            return;
        }
        let now = Instant::now();
        *self = Self { from: self.value_at(now), to, since: (!still).then_some(now) };
    }
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

    #[test]
    fn an_ease_starts_still_and_heads_for_each_new_target_from_where_it_is() {
        let mut ease = Ease::at(1.);
        assert_eq!(ease.value(), 1.);
        assert!(!ease.moving());
        ease.toward(0., false);
        assert!(ease.moving());
        let start = ease.since.unwrap();
        assert_eq!(ease.value_at(start), 1.);
        assert_eq!(ease.value_at(start + FADE), 0.);
        let halfway = ease.value_at(start + FADE / 4);
        assert!(halfway > 0. && halfway < 1.);
        ease.toward(0., false);
        assert_eq!(ease.since, Some(start), "the same target does not restart it");
        ease.toward(1., true);
        assert_eq!(ease.value(), 1.);
        assert!(!ease.moving(), "Reduce Motion lands at once");
    }
}
