//! Motion: the few movements that make the app feel alive, in one place.
//!
//! - **Presence** ([`enter`]): something new arrives by rising a few points as it fades in. A
//!   screen, a menu, a settings section.
//! - **From an edge** ([`slide_in`]): a dock's pane arrives from the side it opens on.
//! - **Between areas** ([`slide_in`], [`HOP`]): switching between Code and Chat, the sidebar's
//!   page and the stage arrive from the side of the switch's segment while its pill glides.
//! - **Between states** ([`Ease`]): the picture clears on a new chat and frosts over once the
//!   first message is sent.
//! - **At work** ([`paddling`]): the duck paddles while the agent works.
//! - **A new name** ([`TitleSwap`]): a title the model wrote swipes in on its tab, the old one
//!   leaving to the left, and an accent glint passes over it; its sidebar row crossfades. Only the
//!   model's title gets this beat: a typed name or the opening-line stand-in just changes.
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

use gpui::{AnimationExt, AnyElement, Animation, App, ElementId, HighlightStyle, Hsla, IntoElement, SharedString, Styled, StyledText, px};

/// How long an arrival takes.
pub const ARRIVE: Duration = Duration::from_millis(260);
/// How far, in points, an area's page travels in when the area switch hops to it.
pub const HOP: f32 = 36.;
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

/// How long the open tab's bead takes to reach a new tab: its leading edge gets there in
/// `BEAD_LEAD`, its tail in `BEAD_TAIL`, so it stretches towards the tab and then gathers up.
pub const BEAD_LEAD: Duration = Duration::from_millis(260);
pub const BEAD_TAIL: Duration = Duration::from_millis(460);

/// The open tab's bead in the tab tray: a span across the tray (its left and right edges) that runs
/// to each newly opened tab like a drop of liquid, leading edge first. The holder calls
/// [`Bead::at`] as it paints and asks for another frame while [`Bead::moving`].
#[derive(Clone, Debug, Default)]
pub struct Bead {
    /// The tab it is heading for.
    key: Option<SharedString>,
    from: (f32, f32),
    to: (f32, f32),
    since: Option<Instant>,
}

impl Bead {
    /// Where the bead is drawn at `now`, on its way to `key`'s tab, which spans `span`. A new key
    /// sets it off from wherever it is; the same key follows its tab at once, so a tab that moves
    /// (another closed, the window resized) keeps its bead. It never moves the first time, or
    /// with Reduce Motion on (`still`).
    pub fn at(&mut self, key: &SharedString, span: (f32, f32), still: bool, now: Instant) -> (f32, f32) {
        if self.key.as_ref() != Some(key) {
            let first = self.key.is_none();
            self.from = if first { span } else { self.value_at(now) };
            self.since = (!first && !still).then_some(now);
            self.key = Some(key.clone());
        }
        self.to = span;
        self.value_at(now)
    }

    fn value_at(&self, now: Instant) -> (f32, f32) {
        let Some(since) = self.since else { return self.to };
        let elapsed = now.saturating_duration_since(since).as_secs_f32();
        // The edge on the side it is heading for leads.
        let rightwards = self.to.0 + self.to.1 > self.from.0 + self.from.1;
        let (left, right) = if rightwards { (BEAD_TAIL, BEAD_LEAD) } else { (BEAD_LEAD, BEAD_TAIL) };
        let along = |time: Duration| spring(elapsed / time.as_secs_f32());
        (self.from.0 + (self.to.0 - self.from.0) * along(left), self.from.1 + (self.to.1 - self.from.1) * along(right))
    }

    pub fn moving(&self, now: Instant) -> bool {
        self.since.is_some_and(|since| now.saturating_duration_since(since) < BEAD_TAIL)
    }
}

/// One breath of a step still under way: down to [`BREATH_LOW`] and back.
pub const BREATH: Duration = Duration::from_millis(1600);
pub const BREATH_LOW: f32 = 0.45;

/// Breathes, for a step still under way: it dims and brightens again, over and over. With Reduce
/// Motion it holds still, fully lit.
pub fn breathing<E: IntoElement + Styled + 'static>(id: impl Into<ElementId>, element: E, still: bool) -> AnyElement {
    if still {
        return element.into_any_element();
    }
    element
        .with_animation(id, Animation::new(BREATH).repeat(), |element, t| element.opacity(breath(t)))
        .into_any_element()
}

/// How lit a breathing step is `t` (0–1) of the way through a breath: full at both ends.
fn breath(t: f32) -> f32 {
    let wave = 0.5 + 0.5 * (t * std::f32::consts::TAU).cos();
    BREATH_LOW + (1. - BREATH_LOW) * wave
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
    fn a_breath_starts_and_ends_fully_lit() {
        assert_eq!(breath(0.), 1.);
        assert!((breath(1.) - 1.).abs() < 1e-5);
        assert!((breath(0.5) - BREATH_LOW).abs() < 1e-5);
    }

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
    fn the_bead_stretches_towards_a_new_tab_and_gathers_up() {
        let (first, second): (SharedString, SharedString) = ("a".into(), "b".into());
        let start = Instant::now();
        let mut bead = Bead::default();
        assert_eq!(bead.at(&first, (10., 110.), false, start), (10., 110.), "the first tab: no motion");
        assert!(!bead.moving(start));
        // Off to the right: the right edge leads, so the bead is wider on the way.
        let (left, right) = bead.at(&second, (200., 300.), false, start);
        assert_eq!((left, right), (10., 110.));
        let (left, right) = bead.at(&second, (200., 300.), false, start + BEAD_LEAD / 2);
        assert!(right - left > 100., "stretched: {left}..{right}");
        assert!(bead.moving(start + BEAD_LEAD));
        assert_eq!(bead.at(&second, (200., 300.), false, start + BEAD_TAIL), (200., 300.), "gathered at the tab");
        assert!(!bead.moving(start + BEAD_TAIL));
        // The same tab moving (another closed) is followed at once.
        assert_eq!(bead.at(&second, (150., 250.), false, start + BEAD_TAIL * 2), (150., 250.));
        // Reduce Motion lands at once.
        assert_eq!(bead.at(&first, (10., 110.), true, start + BEAD_TAIL * 3), (10., 110.));
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

/// How long the old title takes to leave its tab.
const TITLE_OUT: Duration = Duration::from_millis(180);
/// How long the new title takes to settle on its tab.
const TITLE_IN: Duration = Duration::from_millis(360);
/// How far the titles travel on a tab, in points.
const TITLE_SHIFT: f32 = 16.;
/// The glint waits for the new title to arrive, then sweeps across it.
const GLINT_DELAY: Duration = Duration::from_millis(250);
const GLINT: Duration = Duration::from_millis(800);
/// A sidebar row's crossfade.
const ROW_FADE: Duration = Duration::from_millis(280);
/// How wide the glint's bright band is, as a share of the title.
const GLINT_BAND: f32 = 0.14;

/// A title the model just wrote, from the moment it landed: what the chat was called before,
/// for the old title to leave with. The app keeps one per chat until another replaces it.
#[derive(Clone, Debug)]
pub struct TitleSwap {
    pub from: SharedString,
    at: Instant,
}

/// One frame of a tab's title swap. Offsets are in points; a title at rest has 0 and 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabFrame {
    /// The old title's offset and opacity while it is still leaving.
    pub old: Option<(f32, f32)>,
    pub new: (f32, f32),
    /// Where the glint's band is across the new title, 0 at its start and 1 at its end (it runs
    /// a little past both), while it is passing.
    pub glint: Option<f32>,
}

impl TitleSwap {
    pub fn new(from: SharedString) -> Self {
        Self { from, at: Instant::now() }
    }

    /// Still moving: the views ask for another frame while this is true.
    pub fn live(&self) -> bool {
        self.at.elapsed() < GLINT_DELAY + GLINT
    }

    /// This frame of the swap, for the views; `None` once it is over.
    pub fn look(&self, still: bool) -> Option<SwapLook> {
        self.live().then(|| SwapLook::at(self.from.clone(), self.at.elapsed(), still))
    }
}

/// What a title looks like partway through its swap: on its tab, and on its sidebar row, where
/// `row` is how far the crossfade has come (0 to 1).
#[derive(Clone, Debug, PartialEq)]
pub struct SwapLook {
    pub from: SharedString,
    pub tab: TabFrame,
    pub row: f32,
}

impl SwapLook {
    /// The swap `elapsed` after the title landed: the gallery draws chosen moments with this.
    pub fn at(from: SharedString, elapsed: Duration, still: bool) -> Self {
        Self { from, tab: tab_frame(elapsed, still), row: (elapsed.as_secs_f32() / ROW_FADE.as_secs_f32()).min(1.) }
    }
}

/// A tab's swap `elapsed` after the title landed. With Reduce Motion the titles only crossfade,
/// in place, and nothing glints.
fn tab_frame(elapsed: Duration, still: bool) -> TabFrame {
    let t = elapsed.as_secs_f32();
    let out = (t / TITLE_OUT.as_secs_f32()).min(1.);
    let old = (out < 1.).then(|| {
        let eased = out * (2. - out);
        (if still { 0. } else { -TITLE_SHIFT * eased }, 1. - out)
    });
    let new = if still {
        (0., (t / ARRIVE.as_secs_f32()).min(1.))
    } else {
        // It shows only as fast as the old title goes, so the two never read on top of each other.
        let arrive = spring((t / TITLE_IN.as_secs_f32()).min(1.));
        (TITLE_SHIFT * (1. - arrive), arrive.min(1.) * out)
    };
    let since_glint = t - GLINT_DELAY.as_secs_f32();
    let glint = (!still && since_glint >= 0. && since_glint < GLINT.as_secs_f32()).then(|| {
        let g = since_glint / GLINT.as_secs_f32();
        let eased = if g < 0.5 { 2. * g * g } else { 1. - (-2. * g + 2.).powi(2) / 2. };
        -GLINT_BAND + (1. + 2. * GLINT_BAND) * eased
    });
    TabFrame { old, new: (new.0, new.1.clamp(0., 1.)), glint }
}

/// `text` with the glint's band at `head`: each letter takes on the accent the nearer it is to
/// the band's middle.
pub fn glint_text(text: SharedString, head: f32, colour: Hsla, accent: Hsla) -> StyledText {
    let count = text.chars().count().max(1) as f32;
    let highlights: Vec<_> = text
        .char_indices()
        .enumerate()
        .filter_map(|(index, (at, letter))| {
            let place = (index as f32 + 0.5) / count;
            let weight = 1. - ((place - head).abs() / GLINT_BAND);
            (weight > 0.).then(|| {
                let lit = colour.blend(Hsla { a: accent.a * weight, ..accent });
                (at..at + letter.len_utf8(), HighlightStyle { color: Some(lit), ..Default::default() })
            })
        })
        .collect();
    StyledText::new(text).with_highlights(highlights)
}

#[cfg(test)]
mod title_swap_tests {
    use super::*;

    #[test]
    fn the_old_title_leaves_left_and_the_new_one_settles() {
        let start = tab_frame(Duration::ZERO, false);
        assert_eq!(start.old, Some((0., 1.)));
        assert!(start.new.0 > 15. && start.new.1 < 0.1, "the new title starts to the right, unseen");
        let middle = tab_frame(Duration::from_millis(90), false);
        let (offset, opacity) = middle.old.unwrap();
        assert!(offset < -8. && opacity < 1.);
        let end = tab_frame(GLINT_DELAY + GLINT + Duration::from_millis(1), false);
        assert_eq!(end.old, None);
        assert!(end.new.0.abs() < 0.5 && end.new.1 > 0.99);
        assert_eq!(end.glint, None);
    }

    #[test]
    fn the_glint_sweeps_left_to_right_after_the_title_arrives() {
        assert_eq!(tab_frame(Duration::from_millis(100), false).glint, None);
        let early = tab_frame(GLINT_DELAY + Duration::from_millis(100), false).glint.unwrap();
        let late = tab_frame(GLINT_DELAY + Duration::from_millis(700), false).glint.unwrap();
        assert!(early < late);
    }

    #[test]
    fn reduce_motion_only_crossfades() {
        for ms in [0, 90, 200, 500, 900] {
            let frame = tab_frame(Duration::from_millis(ms), true);
            assert_eq!(frame.new.0, 0.);
            assert_eq!(frame.glint, None);
            if let Some((offset, _)) = frame.old {
                assert_eq!(offset, 0.);
            }
        }
        let end = tab_frame(ARRIVE, true);
        assert_eq!(end.new, (0., 1.));
    }
}
