//! The working mark: a flock of nine dots in the accent, beside whatever the agent is doing now.
//!
//! It never stops moving and never jumps. The flock holds a shape for a beat while that shape
//! moves (a spark turning and breathing, a ring chasing its tail, a little galaxy, a ripple, a
//! figure eight, three trios spinning on their corners), then swirls into the next: each dot
//! travels from its place in one to its place in the other while the whole flock turns about its
//! middle, most at mid-way, so the change reads as a gust rather than a slide. Every shape moves
//! in whole turns of the cycle, so the last swirl lands exactly where the first shape starts and
//! the loop has no seam.
//!
//! Positions are pure ([`dots`]), so the continuity is tested here; [`mark`] paints them. With
//! Reduce Motion the flock holds still as the spark, the cycle's start.

use std::f32::consts::{PI, TAU};
use std::time::Duration;

use gpui::{Animation, AnimationExt, AnyElement, Bounds, ElementId, Hsla, IntoElement, Styled, point, px, size};

/// How many dots fly.
pub const DOTS: usize = 9;
/// How long the flock holds each shape, and how long it takes to swirl into the next.
const HOLD: f32 = 1.4;
const MORPH: f32 = 0.6;
const SHAPES: usize = 6;
/// The whole loop, in seconds.
pub const CYCLE: f32 = SHAPES as f32 * (HOLD + MORPH);
/// One turn per cycle: every shape moves in whole multiples of it, so the loop closes.
const TURN: f32 = TAU / CYCLE;
/// The mark's square, in points, and its dots' largest radius.
pub const MARK: f32 = 15.;
const DOT: f32 = 1.45;

/// A dot: where it is in the unit disc, how big against the largest, and how lit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub light: f32,
}

/// Dot `i` of shape `shape` at `time` seconds into the cycle.
fn shape(shape: usize, i: usize, time: f32) -> Dot {
    let n = DOTS as f32;
    let k = i as f32;
    let along = k / (n - 1.);
    match shape {
        // The spark: one at the heart, eight round it on four axes, long and short by turns,
        // turning slowly and breathing.
        0 => {
            if i == 0 {
                return Dot { x: 0., y: 0., size: 1., light: 1. };
            }
            let angle = (k - 1.) * TAU / 8. + time * TURN - PI / 2.;
            let long = i % 2 == 1;
            let radius = if long { 0.98 } else { 0.56 } + 0.06 * (time * TURN * 6.).sin();
            Dot { x: radius * angle.cos(), y: radius * angle.sin(), size: if long { 0.8 } else { 0.62 }, light: if long { 1. } else { 0.7 } }
        }
        // The ring: a comet chasing its own tail, brightest at its head.
        1 => {
            let angle = k * TAU / n + time * TURN * 4.;
            Dot { x: 0.86 * angle.cos(), y: 0.86 * angle.sin(), size: 0.5 + 0.5 * along, light: 0.25 + 0.75 * along }
        }
        // The pinwheel: three curved arms of three, turning.
        2 => {
            let (arm, step) = ((i % 3) as f32, (i / 3) as f32);
            let radius = 0.34 + 0.31 * step;
            let angle = arm * TAU / 3. + step * 0.6 + time * TURN * 3.;
            Dot { x: radius * angle.cos(), y: radius * angle.sin(), size: 0.6 + 0.2 * step, light: 0.55 + 0.225 * step }
        }
        // The ripple: a line, a wave running along it, the crests lit.
        3 => {
            let x = -0.94 + 1.88 * along;
            let phase = x * 2.6 - time * TURN * 8.;
            Dot { x, y: 0.42 * phase.sin(), size: 0.7, light: 0.55 + 0.45 * (0.5 + 0.5 * phase.cos()) }
        }
        // The figure eight: a snake chasing round two loops, its head first and brightest.
        4 => {
            let angle = time * TURN * 4. - k * 0.3;
            Dot { x: 0.94 * angle.sin(), y: 0.94 * angle.sin() * angle.cos(), size: 0.85 - 0.04 * k, light: 1. - 0.075 * k }
        }
        // The trefoil: three trios, each spinning on its corner while the three turn round.
        _ => {
            let (corner, within) = ((i % 3) as f32, (i / 3) as f32);
            let round = corner * TAU / 3. + time * TURN - PI / 2.;
            let spin = within * TAU / 3. + time * TURN * 5.;
            Dot {
                x: 0.56 * round.cos() + 0.32 * spin.cos(),
                y: 0.56 * round.sin() + 0.32 * spin.sin(),
                size: 0.7,
                light: 0.6 + 0.2 * within,
            }
        }
    }
}

/// Where every dot is at `time` seconds (any time: the cycle repeats).
pub fn dots(time: f32) -> [Dot; DOTS] {
    let time = time.rem_euclid(CYCLE);
    let segment = HOLD + MORPH;
    let at = ((time / segment) as usize).min(SHAPES - 1);
    let into = time - at as f32 * segment;
    std::array::from_fn(|i| {
        let here = shape(at, i, time);
        if into < HOLD {
            return here;
        }
        let t = ((into - HOLD) / MORPH).clamp(0., 1.);
        let eased = t * t * t * (t * (t * 6. - 15.) + 10.);
        let next = shape((at + 1) % SHAPES, i, time);
        let mix = |a: f32, b: f32| a + (b - a) * eased;
        let (x, y) = (mix(here.x, next.x), mix(here.y, next.y));
        // The gust: the flock turns about its middle as it changes, not at all at either end.
        let swirl = (PI * eased).sin() * 0.9;
        let (sin, cos) = swirl.sin_cos();
        Dot { x: x * cos - y * sin, y: x * sin + y * cos, size: mix(here.size, next.size), light: mix(here.light, next.light) }
    })
}

/// The flock at `time`, painted in `colour` in a [`MARK`]-point square.
fn painted(time: f32, colour: Hsla) -> gpui::Canvas<()> {
    gpui::canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let middle = bounds.center();
            let reach = MARK / 2. - DOT;
            for dot in dots(time) {
                let radius = DOT * dot.size;
                let at = point(middle.x + px(dot.x * reach - radius), middle.y + px(dot.y * reach - radius));
                let circle = Bounds::new(at, size(px(radius * 2.), px(radius * 2.)));
                window.paint_quad(gpui::fill(circle, Hsla { a: colour.a * dot.light, ..colour }).corner_radii(px(radius)));
            }
        },
    )
    .flex_none()
    .size(px(MARK))
}

/// The working mark, flocking; `still` (Reduce Motion) holds it as the spark.
pub fn mark(id: impl Into<ElementId>, colour: Hsla, still: bool) -> AnyElement {
    if still {
        return painted(0., colour).into_any_element();
    }
    painted(0., colour)
        .with_animation(id, Animation::new(Duration::from_secs_f32(CYCLE)).repeat(), move |_, t| painted(t * CYCLE, colour))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apart(a: &[Dot; DOTS], b: &[Dot; DOTS]) -> f32 {
        a.iter().zip(b).map(|(a, b)| ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()).fold(0., f32::max)
    }

    #[test]
    fn the_flock_never_jumps_not_even_where_the_loop_closes() {
        let step = 0.002;
        let mut before = dots(0.);
        let mut time = step;
        while time <= CYCLE + step {
            let now = dots(time);
            // The fastest dot crosses a few units a second; a jump would be a good part of one.
            assert!(apart(&before, &now) < 0.03, "a jump at {time}s");
            before = now;
            time += step;
        }
        assert!(apart(&dots(CYCLE - 1e-4), &dots(0.)) < 1e-2, "the last swirl lands on the first shape");
    }

    #[test]
    fn every_dot_stays_inside_the_mark() {
        let mut time = 0.;
        while time < CYCLE {
            for dot in dots(time) {
                assert!((dot.x * dot.x + dot.y * dot.y).sqrt() <= 1.05, "{dot:?} at {time}s");
                assert!(dot.size > 0. && dot.size <= 1. && dot.light > 0. && dot.light <= 1., "{dot:?} at {time}s");
            }
            time += 0.01;
        }
    }

    #[test]
    fn the_still_mark_is_the_spark() {
        let still = dots(0.);
        assert_eq!((still[0].x, still[0].y), (0., 0.), "a dot at the heart");
        assert!(still[1..].iter().all(|dot| (dot.x * dot.x + dot.y * dot.y).sqrt() > 0.5), "eight round it");
    }
}
