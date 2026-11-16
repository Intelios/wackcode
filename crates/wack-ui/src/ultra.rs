//! Ultra's look, shared by Ultra Plan and the Ultra reasoning level: a fixed prism (the theme's
//! `ultra*` tokens, which never follow the accent), the pixel wildcard ([`UltraMark`]), a label
//! that decodes from glyph noise ([`UltraLabel`]) and the reasoning slider's glyph field
//! ([`UltraField`]). Ported from version 2's `ultra-fx.ts` and `UltraGlyphField`.
//!
//! The timings and shapes are pure functions ([`mark_frame`], [`scramble`], [`field_frame`]),
//! tested below; the views only clock and paint them.
//!
//! Motion budget: the mark changes in whole LED steps, so a live mark redraws only when its frame
//! changes (about nine times a second through the orbit, then not at all through the hold), never
//! every display frame; the field steps about twenty-five times a second, not sixty. Only the
//! mark's arrival and the label's decode, each under 600 ms, draw every frame. Under Reduce Motion
//! the mark shows its settled wildcard, the label never decodes, and the field paints one still
//! frame.

use std::time::{Duration, Instant};

use gpui::{
    App, BoxShadow, Bounds, ContentMask, ElementId, HighlightStyle, Hsla, Pixels, SharedString, StyledText, TextAlign,
    TextRun, Window, canvas, div, fill, font, linear_color_stop, linear_gradient, point, prelude::*, px, size,
};

use crate::{
    theme::Theme,
    typography::{MONO_FONT, Type, Typography},
};

// ── The edge ──────────────────────────────────────────

/// How thick the prism's edge is, in points.
const EDGE: f32 = 1.5;

/// `child` (a card rounded by `radius`) inside a hairline of the prism, cyan to peach from left to
/// right, glowing faintly: Ultra Plan's composer. GPUI's gradients have two stops, so the edge is
/// painted in slices, one between each pair of the prism's stops, each clipped to its share of
/// the width. It never moves, so Reduce Motion has nothing to stop.
pub fn prism_edge(radius: f32, child: impl IntoElement, theme: &Theme) -> gpui::Div {
    let stops = theme.ultra_spectrum;
    let outer = radius + EDGE;
    div()
        .relative()
        .p(px(EDGE))
        .rounded(px(outer))
        .shadow(vec![BoxShadow {
            color: Hsla { a: 0.22, ..theme.ultra },
            offset: point(px(0.), px(0.)),
            blur_radius: px(28.),
            spread_radius: px(0.),
            inset: false,
        }])
        .child(
            canvas(
                |_, _, _| {},
                move |bounds: Bounds<Pixels>, _, window: &mut Window, _| {
                    let spans = stops.len() - 1;
                    let width = bounds.size.width / spans as f32;
                    for span in 0..spans {
                        let slice = Bounds::new(point(bounds.origin.x + width * span as f32, bounds.origin.y), size(width, bounds.size.height));
                        let (from, to) = (span as f32 / spans as f32, (span + 1) as f32 / spans as f32);
                        let gradient = linear_gradient(90., linear_color_stop(stops[span], from), linear_color_stop(stops[span + 1], to));
                        window.with_content_mask(Some(ContentMask { bounds: slice }), |window| {
                            window.paint_quad(fill(bounds, gradient).corner_radii(px(outer)));
                        });
                    }
                },
            )
            .absolute()
            .left_0()
            .top_0()
            .size_full(),
        )
        .child(child)
}

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
    ink: Option<Hsla>,
}

impl UltraMark {
    /// A still mark. `id` must be unique among the marks on screen.
    pub fn new(id: impl Into<ElementId>, size: Pixels) -> Self {
        Self { id: id.into(), size, animated: false, ink: None }
    }

    /// Runs the LED cycle: for a mark that says Ultra is on and waiting, not for one that only
    /// labels, and not while a run owns the control.
    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    /// Draws the whole mark in one colour, for a ground the prism cannot read on: the reasoning
    /// slider's glow-white thumb.
    pub fn ink(mut self, colour: Hsla) -> Self {
        self.ink = Some(colour);
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
        let ink = self.ink;
        let colour = |index: usize| -> Hsla {
            if let Some(ink) = ink {
                return ink;
            }
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

// ── The glyph field ───────────────────────────────────

/// Sparse to dense. The space lets the field thin out at the low end.
const GLYPHS: [char; 10] = [' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];
/// What floods the fill while it boots, before each column resolves.
const FLOOD: [char; 4] = ['@', '#', '%', '&'];
/// The field's left margin, and the room kept clear for the thumb at the track's end.
const FIELD_LEFT: f32 = 8.;
const THUMB_ROOM: f32 = 30.;
/// The glyphs' size: SF Mono's advance at 11 points is 6.6, version 2's pitch.
const FIELD_TEXT: f32 = 11.;
/// How often a share of the cells re-rolls, and how many of them do.
const REROLL_MS: u64 = 80;
const REROLL_SHARE: f32 = 0.35;
/// How long the fill takes to resolve from left to right when it appears.
const ENTRANCE_MS: u64 = 750;
/// How fast the spectrum slides along the field: one stop-span per 20 seconds.
const DRIFT_PER_MS: f32 = 1. / 20_000.;
/// How long the scan head takes to cross the track, and how far past its end it rests.
const SWEEP_MS: u64 = 1700;
/// How often the field steps. It is decorative, and an open slider should not cost a redraw for
/// every display frame.
const FIELD_STEP_MS: u64 = 40;

/// How dense a cell is: sparse at the track's start and packed toward the thumb (`x`, 0–1),
/// shaken by the cell's own `noise` (0–1) and lifted by the scan head passing over it (`scan`,
/// 0–1).
pub fn field_density(x: f32, noise: f32, scan: f32) -> f32 {
    (0.1 + 0.62 * x + (noise - 0.5) * 0.35 + scan * 0.6).clamp(0., 1.)
}

/// The glyph for a density, from the ramp.
pub fn glyph_for(density: f32) -> char {
    GLYPHS[(density.clamp(0., 1.) * (GLYPHS.len() - 1) as f32).round() as usize]
}

/// Where the scan head is, in columns: it sweeps past the last one, then rests off the track.
pub fn scan_head(elapsed: u64, columns: usize) -> f32 {
    ((elapsed as f32 / SWEEP_MS as f32) % 1.25) * columns as f32
}

/// How close a column is to the head (0–1): a soft bump a few columns wide.
pub fn scan_closeness(column: f32, head: f32) -> f32 {
    (-((column - head).powi(2)) / 5.).exp()
}

/// A colour's place along the prism, `t` running 0→1 across the stops and bouncing back at each
/// end, so a drifting offset never jumps from the last stop to the first.
pub fn bounced(t: f32) -> f32 {
    let wrapped = t.rem_euclid(2.);
    if wrapped <= 1. { wrapped } else { 2. - wrapped }
}

/// One column of the field: what it shows, where its colour sits along the prism, how close the
/// scan head is, and how opaque it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldCell {
    pub glyph: char,
    pub along: f32,
    pub near: f32,
    pub alpha: f32,
    /// A column still waiting for the entrance to reach it: drawn in the glow, not the prism.
    pub flood: bool,
}

/// A flood glyph for a column, changing with the field's step seed.
fn flood_glyph(column: usize, seed: u32) -> char {
    let mut value = seed ^ (column as u32).wrapping_mul(0x9e37_79b9);
    value ^= value >> 15;
    value = value.wrapping_mul(0x85eb_ca6b);
    value ^= value >> 13;
    FLOOD[(value % FLOOD.len() as u32) as usize]
}

/// The field `elapsed` milliseconds after it appeared. Under Reduce Motion it is one still frame:
/// every column resolved, the head parked at seven tenths.
pub fn field_frame(elapsed: u64, columns: usize, noise: &[f32], seed: u32, still: bool) -> Vec<FieldCell> {
    let head = if still { columns as f32 * 0.7 } else { scan_head(elapsed, columns) };
    let reveal = if still { 1. } else { (elapsed as f32 / ENTRANCE_MS as f32).min(1.) };
    (0..columns)
        .map(|column| {
            let x = column as f32 / columns.max(1) as f32;
            if x > reveal {
                return FieldCell { glyph: flood_glyph(column, seed), along: 0., near: 1., alpha: 0.95, flood: true };
            }
            let near = scan_closeness(column as f32, head);
            let density = field_density(x, noise.get(column).copied().unwrap_or(0.5), near);
            FieldCell {
                glyph: glyph_for(density),
                along: bounced(x + elapsed as f32 * DRIFT_PER_MS),
                near,
                alpha: 0.5 + 0.5 * (near + x * 0.4).min(1.),
                flood: false,
            }
        })
        .collect()
}

/// `amount` of `to` over `from`.
fn mix(from: Hsla, to: Hsla, amount: f32) -> Hsla {
    from.blend(to.opacity(amount.clamp(0., 1.)))
}

/// Paints the field's cells across the track, one monospace glyph per column at the font's own
/// advance, so the grid stays even.
fn paint_field(cells: &[FieldCell], bounds: Bounds<Pixels>, theme: &Theme, window: &mut Window, cx: &mut App) {
    let font = font(MONO_FONT);
    let font_size = px(FIELD_TEXT);
    let text: String = cells.iter().map(|cell| cell.glyph).collect();
    let runs: Vec<TextRun> = cells
        .iter()
        .map(|cell| TextRun {
            len: cell.glyph.len_utf8(),
            font: font.clone(),
            color: if cell.flood {
                theme.ultra_glow
            } else {
                mix(theme.ultra_at(cell.along), theme.ultra_glow, cell.near * 0.8)
            }
            .opacity(cell.alpha),
            ..TextRun::default()
        })
        .collect();
    let shaped = window.text_system().shape_line(text.into(), font_size, &runs, None);
    let origin = bounds.origin + point(px(FIELD_LEFT), px(0.));
    let _ = shaped.paint(origin, bounds.size.height, TextAlign::Left, None, window, cx);
}

/// The field's clock and its columns' noise, kept between frames while the slider is open at
/// Ultra.
struct Field {
    since: Instant,
    noise: Vec<f32>,
    seed: u32,
    rolled_at: u64,
    waiting: bool,
}

impl Field {
    fn new(columns: usize) -> Self {
        let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |since| since.subsec_nanos()) | 1;
        let noise = (0..columns).map(|_| random(&mut seed)).collect();
        Self { since: Instant::now(), noise, seed, rolled_at: 0, waiting: false }
    }

    /// Keeps the noise in step with a track that has been resized.
    fn fit(&mut self, columns: usize) {
        if self.noise.len() != columns {
            self.noise = (0..columns).map(|_| random(&mut self.seed)).collect();
        }
    }

    /// Re-rolls a share of the columns, as the field does every [`REROLL_MS`].
    fn roll(&mut self, elapsed: u64) {
        self.rolled_at = elapsed;
        for value in &mut self.noise {
            if random(&mut self.seed) < REROLL_SHARE {
                *value = random(&mut self.seed);
            }
        }
    }
}

/// A small xorshift: the field only has to look random.
fn random(seed: &mut u32) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    (*seed % 1000) as f32 / 1000.
}

/// The reasoning slider's fill at Ultra: a strip of monospace glyphs, sparse at the track's start
/// and packed towards the thumb, its cells re-rolling, a bright scan head sweeping through, in
/// the prism running cyan to peach. Version 2's `UltraGlyphField`, ported.
///
/// It steps rather than drawing every display frame, so an open slider costs twenty-five redraws
/// a second instead of sixty. Under Reduce Motion it paints one still frame, the head parked.
#[derive(IntoElement)]
pub struct UltraField {
    id: ElementId,
    /// The whole track's width: the field is drawn across it and the fill's edge reveals it.
    width: Pixels,
    height: Pixels,
}

impl UltraField {
    pub fn new(id: impl Into<ElementId>, width: Pixels, height: Pixels) -> Self {
        Self { id: id.into(), width, height }
    }
}

impl RenderOnce for UltraField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let still = cx.reduce_motion();
        let font = font(MONO_FONT);
        let font_size = px(FIELD_TEXT);
        let font_id = window.text_system().resolve_font(&font);
        let advance = window.text_system().advance(font_id, font_size, 'm').map_or(6.6, |size| f32::from(size.width)).max(1.);
        let columns = (((f32::from(self.width) - FIELD_LEFT - THUMB_ROOM) / advance).floor().max(0.) as usize) + 1;
        let field = window.use_keyed_state(self.id.clone(), cx, |_, _| Field::new(columns));
        let now = Instant::now();
        let (cells, elapsed) = field.update(cx, |field, _| {
            field.fit(columns);
            let elapsed = now.saturating_duration_since(field.since).as_millis() as u64;
            if !still && elapsed.saturating_sub(field.rolled_at) >= REROLL_MS {
                field.roll(elapsed);
            }
            (field_frame(elapsed, columns, &field.noise, field.seed, still), elapsed)
        });
        if !still && !field.read(cx).waiting {
            field.update(cx, |field, _| field.waiting = true);
            let wait = Duration::from_millis(FIELD_STEP_MS - elapsed % FIELD_STEP_MS);
            let field = field.clone();
            window
                .spawn(cx, async move |cx| {
                    cx.background_executor().timer(wait).await;
                    field.update(cx, |field, cx| {
                        field.waiting = false;
                        cx.notify();
                    });
                })
                .detach();
        }
        canvas(|_, _, _| {}, move |bounds, _, window, cx| paint_field(&cells, bounds, &theme, window, cx))
            .absolute()
            .left_0()
            .top_0()
            .w(self.width)
            .h(self.height)
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

    #[test]
    fn the_field_maps_density_onto_the_ramp_and_clamps_it() {
        assert_eq!(glyph_for(-1.), GLYPHS[0]);
        assert_eq!(glyph_for(0.), ' ');
        assert_eq!(glyph_for(1.), '@');
        assert_eq!(glyph_for(4.), '@');
    }

    #[test]
    fn the_field_thickens_toward_the_thumb_and_under_the_scan_head() {
        assert!(field_density(0.9, 0.5, 0.) > field_density(0.1, 0.5, 0.));
        assert!(field_density(0.4, 0.5, 1.) > field_density(0.4, 0.5, 0.));
    }

    #[test]
    fn the_scan_head_sweeps_across_the_track_and_rests_past_its_end() {
        assert_eq!(scan_head(0, 40), 0.);
        assert!((scan_head(SWEEP_MS / 2, 40) - 20.).abs() < 0.001);
        assert!(scan_head(SWEEP_MS * 11 / 10, 40) > 40.);
        assert_eq!(scan_closeness(10., 10.), 1.);
        assert!(scan_closeness(20., 10.) < 0.01);
    }

    #[test]
    fn the_spectrum_bounces_back_instead_of_jumping() {
        assert_eq!(bounced(0.), 0.);
        assert_eq!(bounced(0.5), 0.5);
        assert_eq!(bounced(1.), 1.);
        assert_eq!(bounced(1.5), 0.5);
        assert_eq!(bounced(2.), 0.);
        assert_eq!(bounced(-0.5), 0.5);
    }

    #[test]
    fn the_entrance_floods_from_the_left_then_resolves() {
        let noise = [0.5; 20];
        let start = field_frame(0, 20, &noise, 1, false);
        assert!(!start[0].flood, "the first column resolves at once");
        assert!(start.iter().skip(1).all(|cell| cell.flood));
        let halfway = field_frame(ENTRANCE_MS / 2, 20, &noise, 1, false);
        assert!(!halfway[9].flood);
        assert!(halfway[11].flood);
        let done = field_frame(ENTRANCE_MS, 20, &noise, 1, false);
        assert!(done.iter().all(|cell| !cell.flood));
    }

    #[test]
    fn the_still_field_is_resolved_with_the_head_parked() {
        let noise = [0.5; 20];
        let still = field_frame(0, 20, &noise, 1, true);
        assert!(still.iter().all(|cell| !cell.flood));
        assert!(still[14].near > 0.9, "the head rests at seven tenths");
        assert!(still[0].near < 0.01);
    }

    #[test]
    fn the_field_is_sparse_at_the_start_and_packed_at_the_end() {
        let noise = [0.5; 20];
        let cells = field_frame(ENTRANCE_MS, 20, &noise, 1, false);
        let rank = |glyph: char| GLYPHS.iter().position(|candidate| *candidate == glyph).unwrap_or(0);
        assert!(rank(cells[19].glyph) > rank(cells[0].glyph));
    }
}
