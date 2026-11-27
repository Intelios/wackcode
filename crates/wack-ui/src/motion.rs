//! Motion: the few movements that make the app feel alive, in one place.
//!
//! - **Presence** ([`enter`]): something new arrives by rising a few points as it fades in. A
//!   screen, a menu, a settings section.
//! - **From an edge** ([`slide_in`]): a dock's pane arrives from the side it opens on. The
//!   sidebar and the side panel slide in and out from the window's edges on an [`Ease`], the
//!   stage glide beside them (`Solution::sliding`, `Solution::panel_sliding`).
//! - **Between tabs** ([`Bead`]): the open tab's bead stretches towards the next tab, its leading
//!   edge ahead, and gathers up behind.
//! - **Between areas** ([`slide_in`], [`HOP`]): switching between Code and Chat, the sidebar's
//!   page and the stage arrive from the side of the switch's segment while its pill glides.
//! - **Between states** ([`Ease`]): the picture clears on a new chat and frosts over once the
//!   first message is sent.
//! - **At work** ([`paddling`], [`sheen_band`], [`shining_text`], [`breathing`]): the duck
//!   paddles while the agent works, a light crosses its chat's tab and runs along "Working", and
//!   the step under way breathes.
//! - **Landing** ([`land`]): each work-log row drops into place as its step starts.
//! - **Folding** (`transcript::unfolding`, `transcript::chevron`): a group of steps, a row's
//!   diff or output, a commit or a changed file grows open under its line and folds shut again,
//!   fading as it goes, while its chevron turns. The chat notes when each was toggled
//!   (`transcript::Fold`), so it moves only then, never when scrolled back into view.
//! - **Sending** ([`send_moment`]): the composer lifts a point or two and settles as a message
//!   leaves, its rim lit while a glint crosses it.
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
//! The Settings pages' little pictures are their own small kit (`stage.rs`): each plays its story
//! a few times when its page opens, then holds still.
//!
//! Every movement respects the system's Reduce Motion setting: GPUI's `with_animation` draws the
//! end state of an arrival when [`App::reduce_motion`] is set, the shell takes no paddle steps,
//! anything that repeats (a sheen, a breath) or follows the clock (the send moment, the bead)
//! takes its still form, and [`follow_system`] keeps that in step with macOS. The gallery renders
//! with it set, so a scene shows where everything comes to rest.
//!
//! `enter` and `slide_in` offset an element with relative positioning, so they are for elements
//! in the flow: wrap an absolutely placed element in its own positioned box first.

use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use std::f32::consts::PI;

use gpui::{
    AnimationExt, AnyElement, Animation, App, ElementId, HighlightStyle, Hsla, IntoElement, ParentElement, SharedString, Styled, StyledText, div,
    linear_color_stop, linear_gradient, px, relative,
};

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

/// How long each piece of the rebuild takes to land, once its turn comes.
pub const REBUILD: Duration = Duration::from_millis(400);

/// Version 2's rebuild, as Settings opens or closes: each piece of the window flies in from the
/// edge it belongs to, `dx` and `dy` points away, after `delay`, overshooting a touch and
/// settling, so the screen reads as assembling itself. `step` names the rebuild, so each one
/// plays afresh. With Reduce Motion, or before any rebuild, the piece is simply there.
pub fn rebuild<E: IntoElement + Styled + 'static>(step: Option<usize>, name: &'static str, element: E, delay: Duration, dx: f32, dy: f32) -> AnyElement {
    let Some(step) = step else {
        return element.into_any_element();
    };
    let whole = delay + REBUILD;
    let lead = delay.as_secs_f32() / whole.as_secs_f32();
    element
        .with_animation(ElementId::NamedInteger(name.into(), step as u64), Animation::new(whole), move |element, t| {
            let local = ((t - lead) / (1. - lead)).clamp(0., 1.);
            let rest = 1. - pop(local);
            element.opacity(spring(local)).relative().left(px(dx * rest)).top(px(dy * rest))
        })
        .into_any_element()
}

/// Fades in, without moving: for something laid over the window, such as the backdrop's hero.
pub fn fade_in<E: IntoElement + Styled + 'static>(id: impl Into<ElementId>, element: E) -> impl IntoElement {
    element.with_animation(id, Animation::new(FADE).with_easing(spring), |element, t| element.opacity(t))
}

/// A value that eases to each new target over [`FADE`] (or [`Ease::lasting`] its own time), for
/// a change that outlives any one element, such as the picture clearing on a new chat and
/// frosting over when it is sent. The holder calls [`Ease::toward`] as it renders and asks for
/// another frame while [`Ease::moving`]. Nothing moves the first time, or with Reduce Motion on.
///
/// Its clock starts when the target changes, not when the next frame is drawn, so a move made
/// in answer to a click is already under way in the first frame that shows it.
#[derive(Clone, Copy, Debug)]
pub struct Ease {
    from: f32,
    to: f32,
    since: Option<Instant>,
    length: Duration,
}

impl Ease {
    pub fn at(value: f32) -> Self {
        Self { from: value, to: value, since: None, length: FADE }
    }

    /// The same, taking `length` for each move.
    pub fn lasting(self, length: Duration) -> Self {
        Self { length, ..self }
    }

    /// Where the value is now.
    pub fn value(&self) -> f32 {
        self.value_at(Instant::now())
    }

    fn value_at(&self, now: Instant) -> f32 {
        match self.since {
            Some(since) => {
                let t = now.saturating_duration_since(since).as_secs_f32() / self.length.as_secs_f32();
                self.from + (self.to - self.from) * spring(t)
            }
            None => self.to,
        }
    }

    pub fn moving(&self) -> bool {
        self.since.is_some_and(|since| since.elapsed() < self.length)
    }

    /// Heads for `to` from wherever the value is now; `still` jumps straight there.
    pub fn toward(&mut self, to: f32, still: bool) {
        if to == self.to {
            return;
        }
        let now = Instant::now();
        *self = Self { from: self.value_at(now), to, since: (!still).then_some(now), length: self.length };
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

/// How far a work-log mark drops as it lands.
const LAND_DROP: f32 = 7.;

/// Drops into place from a few points above, fading in: a work-log mark as its step starts. With
/// Reduce Motion it is simply there.
pub fn land<E: IntoElement + Styled + 'static>(id: impl Into<ElementId>, element: E, still: bool) -> AnyElement {
    if still {
        return element.into_any_element();
    }
    element
        .with_animation(id, Animation::new(ARRIVE).with_easing(spring), |element, t| {
            element.opacity(t).relative().top(px(-LAND_DROP * (1. - t)))
        })
        .into_any_element()
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

/// One pass of a sheen over something at work (a working tab, the Working chip): it crosses in
/// the first [`SHEEN_CROSS`] of the time, then rests before the next pass.
pub const SHEEN: Duration = Duration::from_millis(2600);
const SHEEN_CROSS: f32 = 0.55;
/// A sheen band's share of what it crosses.
const SHEEN_BAND: f32 = 0.42;

/// How far across a sheen is `t` (0–1) into its pass, eased in and out; `None` while it rests.
pub fn sheen_at(t: f32) -> Option<f32> {
    let cross = t / SHEEN_CROSS;
    (cross < 1.).then(|| smooth(cross.max(0.)))
}

/// A small pop: past its size by a few percent, then settling, for a tick landing. It is the one
/// curve that overshoots, as version 2's check did; everything that arrives uses [`spring`].
pub fn pop(t: f32) -> f32 {
    const BACK: f32 = 1.4;
    let t = t.clamp(0., 1.) - 1.;
    1. + (BACK + 1.) * t * t * t + BACK * t * t
}

fn smooth(x: f32) -> f32 {
    x * x * (3. - 2. * x)
}

/// `text` with a light passing along it again and again, for words about something at work.
/// With Reduce Motion it is just the words.
pub fn shining_text(id: impl Into<ElementId>, text: SharedString, colour: Hsla, light: Hsla, still: bool) -> AnyElement {
    if still {
        return text.into_any_element();
    }
    StyledText::new(text.clone())
        .with_animation(id, Animation::new(SHEEN).repeat(), move |_, t| match sheen_at(t) {
            Some(across) => glint_text(text.clone(), -GLINT_BAND + (1. + 2. * GLINT_BAND) * across, colour, light),
            None => StyledText::new(text.clone()),
        })
        .into_any_element()
}

/// A soft band of `light` crossing its parent again and again, under what the parent holds: a
/// sheen on something at work. The parent must be relative, and `radius` its corners, so the
/// band's ends round as its parent's do. With Reduce Motion there is none.
pub fn sheen_band(id: impl Into<ElementId>, light: Hsla, radius: f32, still: bool) -> Option<AnyElement> {
    if still {
        return None;
    }
    let clear = Hsla { a: 0., ..light };
    let half = |from: Hsla, to: Hsla| div().flex_1().h_full().bg(linear_gradient(90., linear_color_stop(from, 0.), linear_color_stop(to, 1.)));
    let band = div()
        .absolute()
        .top_0()
        .h_full()
        .w(relative(SHEEN_BAND))
        .flex()
        .child(half(clear, light).rounded_l(px(radius)))
        .child(half(light, clear).rounded_r(px(radius)))
        .with_animation(id, Animation::new(SHEEN).repeat(), |band, t| match sheen_at(t) {
            // It fades in from one end and out at the other, so it never shows a hard edge.
            Some(across) => band.left(relative(across * (1. - SHEEN_BAND))).opacity((across * PI).sin()),
            None => band.opacity(0.),
        });
    Some(band.into_any_element())
}

/// How long the composer's send moment lasts: it lifts and settles while a glint crosses it.
pub const SEND: Duration = Duration::from_millis(720);
/// How far the composer lifts as a message leaves, in points.
const SEND_LIFT: f32 = 2.;

/// The composer at one point of its send moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SendMoment {
    /// Points above its place.
    pub lift: f32,
    /// Where the glint is, 0 to 1 across.
    pub across: f32,
    /// How bright the glint and the rim are, 0 to 1.
    pub light: f32,
}

/// The send moment `elapsed` after a send; `None` once it is over, and always with Reduce Motion.
pub fn send_moment(elapsed: Duration, still: bool) -> Option<SendMoment> {
    let t = elapsed.as_secs_f32() / SEND.as_secs_f32();
    if still || t >= 1. {
        return None;
    }
    let swell = (t * PI).sin();
    Some(SendMoment { lift: SEND_LIFT * swell, across: smooth(t), light: swell })
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
    fn a_pop_overshoots_a_little_and_lands() {
        assert!(pop(0.).abs() < 1e-5);
        assert!((pop(1.) - 1.).abs() < 1e-5);
        let peak = (1..100).map(|step| pop(step as f32 / 100.)).fold(0., f32::max);
        assert!(peak > 1.02 && peak < 1.12, "a few percent past, never a bounce: {peak}");
    }

    #[test]
    fn a_sheen_crosses_then_rests() {
        assert_eq!(sheen_at(0.), Some(0.));
        let middle = sheen_at(SHEEN_CROSS / 2.).unwrap();
        assert!((middle - 0.5).abs() < 1e-5);
        assert_eq!(sheen_at(SHEEN_CROSS + 0.01), None, "it rests before the next pass");
    }

    #[test]
    fn the_send_moment_lifts_and_settles() {
        let start = send_moment(Duration::ZERO, false).unwrap();
        assert_eq!((start.lift, start.across), (0., 0.));
        let peak = send_moment(SEND / 2, false).unwrap();
        assert!((peak.lift - SEND_LIFT).abs() < 1e-4 && (peak.light - 1.).abs() < 1e-4);
        assert_eq!(send_moment(SEND, false), None, "over, it is back in its place");
        assert_eq!(send_moment(SEND / 2, true), None, "Reduce Motion: no moment at all");
    }

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

    #[test]
    fn an_ease_can_take_its_own_time() {
        let length = Duration::from_millis(200);
        let mut ease = Ease::at(0.).lasting(length);
        ease.toward(1., false);
        let start = ease.since.unwrap();
        assert_eq!(ease.value_at(start + length), 1.);
        assert!(ease.value_at(start + length / 2) < 1.);
        ease.toward(0., false);
        assert_eq!(ease.length, length, "each later move keeps its time");
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
