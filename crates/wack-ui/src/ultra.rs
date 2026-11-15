//! Ultra's look, shared by Ultra Plan and the Ultra reasoning level: a fixed prism (the theme's
//! `ultra*` tokens, which never follow the accent), the pixel wildcard ([`UltraMark`]) and a label
//! that decodes from glyph noise ([`UltraLabel`]). Ported from version 2's `ultra-fx.ts`.
//!
//! The timings and shapes are pure functions ([`mark_frame`], [`scramble`]), tested below; the
//! views only clock and paint them.
//!
//! Motion budget: the mark changes in whole LED steps, so a live mark redraws only when its frame
//! changes (about nine times a second through the orbit, then not at all through the hold), never
//! every display frame. Only the mark's arrival and the label's decode, each under 600 ms, draw
//! every frame. Under Reduce Motion the mark shows its settled wildcard and the label never
//! decodes.

use std::time::{Duration, Instant};

use gpui::{App, ElementId, HighlightStyle, Hsla, Pixels, SharedString, StyledText, Window, div, prelude::*, px};

use crate::{
    theme::Theme,
    typography::{Type, Typography},
};

// ── The mark ──────────────────────────────────────────

/// Cells are numbered row-major on the 3×3 grid. The ring walks the outer eight clockwise from
/// the top-left; the centre (4) is the core that stays lit through the orbit.
pub const RING: [usize; 8] = [0, 1, 2, 5, 8, 7, 6, 3];
const PLUS: [usize; 5] = [1, 3, 4, 5, 7];
const CROSS: [usize; 5] = [0, 2, 4, 6, 8];

/// One LED step, in milliseconds.
pub const STEP_MS: u64 = 110;
const ORBIT_MS: u64 = STEP_MS * RING.len() as u64 * 2;
const PLUS_MS: u64 = 360;
const CROSS_MS: u64 = 360;
const HOLD_MS: u64 = 2400;
/// Two laps of the orbit, a `+`, an `×`, then the settled `✱` held long enough to read.
pub const CYCLE_MS: u64 = ORBIT_MS + PLUS_MS + CROSS_MS + HOLD_MS;
/// How long a mark takes to pop in when it starts animating.
const ARRIVE_MS: u64 = 450;

const DIM: f32 = 0.14;
/// The still wildcard: a bright `+` over dimmer corners. Static marks and Reduce Motion show it.
pub const SETTLED: [f32; 9] = [0.55, 0.95, 0.55, 0.95, 1., 0.95, 0.55, 0.95, 0.55];
/// The orbit's head and the two beads trailing it.
const TRAIL: [f32; 3] = [1., 0.6, 0.3];

/// Cell opacities for a mark `elapsed` milliseconds after it started animating.
pub fn mark_frame(elapsed: u64) -> [f32; 9] {
    let t = elapsed % CYCLE_MS;
    if t < ORBIT_MS {
        let head = (t / STEP_MS) as usize % RING.len();
        return std::array::from_fn(|cell| {
            if cell == 4 {
                return 0.9;
            }
            let place = RING.iter().position(|&ring| ring == cell).unwrap_or(0);
            let behind = (head + RING.len() - place) % RING.len();
            TRAIL.get(behind).copied().unwrap_or(DIM)
        });
    }
    if t < ORBIT_MS + PLUS_MS {
        return std::array::from_fn(|cell| if PLUS.contains(&cell) { 1. } else { DIM });
    }
    if t < ORBIT_MS + PLUS_MS + CROSS_MS {
        return std::array::from_fn(|cell| if CROSS.contains(&cell) { 1. } else { DIM });
    }
    SETTLED
}

/// How long the frame at `elapsed` holds, so a live mark sleeps until its next change.
pub fn frame_holds(elapsed: u64) -> u64 {
    let t = elapsed % CYCLE_MS;
    let end = if t < ORBIT_MS {
        (t / STEP_MS + 1) * STEP_MS
    } else if t < ORBIT_MS + PLUS_MS {
        ORBIT_MS + PLUS_MS
    } else if t < ORBIT_MS + PLUS_MS + CROSS_MS {
        ORBIT_MS + PLUS_MS + CROSS_MS
    } else {
        CYCLE_MS
    };
    (end - t).max(1)
}

/// The arrival pop, `t` (0–1) of the way through: the mark's scale and opacity. It swells a
/// little past full size and settles back, like v2's quarter-turn pop without the turn.
fn arrival(t: f32) -> (f32, f32) {
    let t = t.clamp(0., 1.);
    let ease = |x: f32| x * x * (3. - 2. * x);
    if t < 0.6 {
        let k = ease(t / 0.6);
        (0.45 + (1.12 - 0.45) * k, k)
    } else {
        let k = ease((t - 0.6) / 0.4);
        (1.12 - 0.12 * k, 1.)
    }
}

/// Ring position → spectrum stop, bouncing back round the far side so neighbours always blend.
const STOPS: [usize; 8] = [0, 1, 2, 3, 4, 3, 2, 1];

/// A mark's clock: when it started animating, and whether its next step is already on its way.
struct Clock {
    since: Option<Instant>,
    waiting: bool,
}

/// Ultra's mark: a 3×3 grid of pixels that spells a wildcard. Animated, a lit pixel orbits the
/// ring twice with two beads trailing it, the grid flashes `+`, flips to `×`, then holds the
/// settled `✱`. Each mark starts its cycle when it starts animating, so switching into Ultra
/// boots it from the orbit. Always decorative: the label beside it carries the meaning.
#[derive(IntoElement)]
pub struct UltraMark {
    id: ElementId,
    size: Pixels,
    animated: bool,
}

impl UltraMark {
    /// A still mark. `id` must be unique among the marks on screen.
    pub fn new(id: impl Into<ElementId>, size: Pixels) -> Self {
        Self { id: id.into(), size, animated: false }
    }

    /// Runs the LED cycle: for a mark that says Ultra is on and waiting, not for one that only
    /// labels, and not while a run owns the control.
    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }
}

impl RenderOnce for UltraMark {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let live = self.animated && !cx.reduce_motion();
        let clock = window.use_keyed_state(self.id.clone(), cx, |_, _| Clock { since: None, waiting: false });
        let now = Instant::now();
        let since = clock.update(cx, |clock, _| {
            clock.since = if live { Some(clock.since.unwrap_or(now)) } else { None };
            clock.since
        });

        let (frame, (scale, opacity)) = match since {
            Some(since) => {
                let elapsed = now.duration_since(since).as_millis() as u64;
                let pop = arrival(elapsed as f32 / ARRIVE_MS as f32);
                if elapsed < ARRIVE_MS {
                    // The pop is the one part that moves every frame.
                    window.request_animation_frame();
                } else if !clock.read(cx).waiting {
                    clock.update(cx, |clock, _| clock.waiting = true);
                    let wait = Duration::from_millis(frame_holds(elapsed));
                    let clock = clock.clone();
                    window
                        .spawn(cx, async move |cx| {
                            cx.background_executor().timer(wait).await;
                            clock.update(cx, |clock, cx| {
                                clock.waiting = false;
                                cx.notify();
                            });
                        })
                        .detach();
                }
                (mark_frame(elapsed), pop)
            }
            None => (SETTLED, (1., 1.)),
        };

        // The grid is drawn on an 11-unit square: 3-unit cells on a 4-unit pitch.
        let unit = f32::from(self.size) / 11.;
        let cell = 3. * unit * scale;
        let pitch = 4. * unit * scale;
        let inset = (f32::from(self.size) - (2. * pitch + cell)) / 2.;
        let colour = |index: usize| -> Hsla {
            match RING.iter().position(|&ring| ring == index) {
                Some(place) => theme.ultra_spectrum[STOPS[place]],
                None => theme.ultra_glow,
            }
        };
        div().flex_none().relative().size(self.size).opacity(opacity).children((0..9).map(|index| {
            div()
                .absolute()
                .left(px(inset + (index % 3) as f32 * pitch))
                .top(px(inset + (index / 3) as f32 * pitch))
                .size(px(cell))
                .rounded(px(0.9 * unit * scale))
                .bg(colour(index))
                .opacity(frame[index])
        }))
    }
}

// ── The label ─────────────────────────────────────────

/// What a scrambled letter shows.
pub const NOISE: &str = "*+x#%@&?<>/=";
/// How long a decode takes, in milliseconds.
pub const DECODE_MS: u64 = 560;

/// The label `k` (0–1) of the way through decoding: each letter resolves in turn from the left,
/// the rest show noise. Spaces never scramble, so the words keep their shape.
pub fn scramble(text: &str, k: f32, mut random: impl FnMut() -> f32) -> String {
    let noise: Vec<char> = NOISE.chars().collect();
    let count = text.chars().count().max(1) as f32;
    text.chars()
        .enumerate()
        .map(|(index, letter)| {
            if letter == ' ' || k > (index as f32 / count) * 0.7 + 0.22 {
                letter
            } else {
                noise[((random() * noise.len() as f32) as usize).min(noise.len() - 1)]
            }
        })
        .collect()
}

/// The text with each glyph in the prism.
fn prism_text(text: SharedString, theme: &Theme, alpha: f32) -> StyledText {
    let count = text.chars().count();
    let highlights: Vec<_> = text
        .char_indices()
        .enumerate()
        .map(|(index, (at, letter))| {
            let colour = theme.ultra_at(if count <= 1 { 0. } else { index as f32 / (count - 1) as f32 });
            (at..at + letter.len_utf8(), HighlightStyle { color: Some(Hsla { a: alpha, ..colour }), ..Default::default() })
        })
        .collect();
    StyledText::new(text).with_highlights(highlights)
}

struct Decode {
    started: Option<Instant>,
    seed: u32,
}

/// An Ultra label ("Ultra", "Ultra Plan") in the prism, decoding from scrambled glyphs as it
/// arrives and again when hovered; at rest it stays still, since it sits beside where you type.
/// The real text keeps its place under the noise (turned clear, never removed), so the label's
/// width never jumps.
#[derive(IntoElement)]
pub struct UltraLabel {
    id: ElementId,
    text: SharedString,
    kind: Type,
    decode_on_arrival: bool,
    enabled: bool,
}

impl UltraLabel {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>, kind: Type) -> Self {
        Self { id: id.into(), text: text.into(), kind, decode_on_arrival: true, enabled: true }
    }

    /// Off for a label that is only listed (a menu), not switched into.
    pub fn decode_on_arrival(mut self, decode: bool) -> Self {
        self.decode_on_arrival = decode;
        self
    }

    /// A disabled control's label never decodes.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

impl RenderOnce for UltraLabel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let still = cx.reduce_motion() || !self.enabled;
        let arrive = self.decode_on_arrival && !still;
        let decode = window.use_keyed_state(self.id.clone(), cx, move |_, _| Decode {
            started: arrive.then(Instant::now),
            seed: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |since| since.subsec_nanos()) | 1,
        });
        let now = Instant::now();
        let progress = decode.read(cx).started.map(|started| now.duration_since(started).as_millis() as f32 / DECODE_MS as f32);
        let noise = match progress {
            Some(k) if k < 1. && !still => {
                window.request_animation_frame();
                // A small xorshift: the noise only has to look random.
                let seed = decode.update(cx, |decode, _| {
                    decode.seed ^= decode.seed << 13;
                    decode.seed ^= decode.seed >> 17;
                    decode.seed ^= decode.seed << 5;
                    decode.seed
                });
                let mut state = seed;
                Some(scramble(&self.text, k, move || {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    (state % 1000) as f32 / 1000.
                }))
            }
            Some(_) => {
                decode.update(cx, |decode, _| decode.started = None);
                None
            }
            None => None,
        };

        let replay = decode.clone();
        div()
            .id(self.id)
            .flex_none()
            .relative()
            .whitespace_nowrap()
            .typography(self.kind)
            .when(!still, |label| {
                label.on_hover(move |hovered, _, cx| {
                    if *hovered {
                        replay.update(cx, |decode, cx| {
                            if decode.started.is_none() {
                                decode.started = Some(Instant::now());
                                cx.notify();
                            }
                        });
                    }
                })
            })
            .child(prism_text(self.text.clone(), &theme, if noise.is_some() { 0. } else { 1. }))
            .when_some(noise, |label, noise| {
                label.child(div().absolute().inset_0().overflow_hidden().child(prism_text(noise.into(), &theme, 1.)))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brightest(frame: &[f32; 9]) -> usize {
        (0..9).fold(0, |best, cell| if frame[cell] > frame[best] { cell } else { best })
    }

    fn lit(frame: &[f32; 9]) -> Vec<usize> {
        (0..9).filter(|&cell| frame[cell] == 1.).collect()
    }

    #[test]
    fn the_orbit_walks_round_the_ring_in_order_with_the_core_lit() {
        for step in 0..RING.len() * 2 {
            let frame = mark_frame(step as u64 * STEP_MS + 1);
            assert_eq!(brightest(&frame), RING[step % RING.len()]);
            assert!(frame[4] > 0.8);
        }
    }

    #[test]
    fn two_fading_beads_trail_the_head() {
        let frame = mark_frame(3 * STEP_MS + 1);
        assert_eq!(frame[RING[3]], 1.);
        assert!(frame[RING[2]] < 1.);
        assert!(frame[RING[1]] < frame[RING[2]]);
        assert!(frame[RING[0]] < frame[RING[1]]);
    }

    #[test]
    fn it_flashes_a_plus_flips_to_a_cross_then_settles() {
        let orbit = RING.len() as u64 * 2 * STEP_MS;
        assert_eq!(lit(&mark_frame(orbit + 10)), vec![1, 3, 4, 5, 7]);
        assert_eq!(lit(&mark_frame(orbit + 370)), vec![0, 2, 4, 6, 8]);
        assert_eq!(mark_frame(orbit + 800), SETTLED);
    }

    #[test]
    fn the_cycle_repeats() {
        assert_eq!(mark_frame(CYCLE_MS + 1), mark_frame(1));
    }

    #[test]
    fn it_settles_on_a_bright_plus_over_dimmer_corners() {
        assert_eq!(SETTLED[4], 1.);
        for edge in [1, 3, 5, 7] {
            assert!(SETTLED[edge] > SETTLED[0]);
        }
    }

    #[test]
    fn a_live_mark_sleeps_until_its_frame_changes() {
        for elapsed in (0..CYCLE_MS * 2).step_by(7) {
            let holds = frame_holds(elapsed);
            assert!(holds >= 1);
            assert_eq!(mark_frame(elapsed), mark_frame(elapsed + holds - 1), "at {elapsed}");
        }
        // Through the hold it sleeps for the whole of it.
        let hold_start = ORBIT_MS + PLUS_MS + CROSS_MS;
        assert_eq!(frame_holds(hold_start), HOLD_MS);
    }

    #[test]
    fn the_pop_ends_at_rest() {
        assert_eq!(arrival(1.), (1., 1.));
        assert!(arrival(0.6).0 > 1.);
        assert_eq!(arrival(0.).1, 0.);
    }

    #[test]
    fn scramble_shows_noise_first_and_the_label_once_done_keeping_spaces() {
        let start = scramble("Ultra Plan", 0., || 0.);
        assert_eq!(start.chars().count(), "Ultra Plan".chars().count());
        assert_eq!(start.chars().nth(5), Some(' '));
        assert!(start.chars().filter(|c| *c != ' ').all(|c| NOISE.contains(c)));
        assert_eq!(scramble("Ultra Plan", 1., || 0.5), "Ultra Plan");
    }

    #[test]
    fn scramble_resolves_from_the_left() {
        let halfway = scramble("Ultra", 0.5, || 0.);
        assert!(halfway.starts_with("Ul"));
        assert!(halfway.ends_with(NOISE.chars().next().unwrap()));
    }
}
