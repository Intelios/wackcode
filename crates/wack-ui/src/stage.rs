//! The little stage at the head of each Settings page, after version 2's: a picture of what the
//! page is for, acted out on a loop. A key slides into its socket and the models light up; ticks
//! land on the tools; a note is written and bookmarked; a cursor crosses a window, presses and
//! types.
//!
//! A scene is drawn from a few plain parts (rounded blocks, dots and the app's own icons) placed
//! on a [`STAGE_WIDTH`] × [`STAGE_HEIGHT`] board, in the theme's ink, lines and accent, so it
//! suits every theme and sits on the glass like the page's other words. Every moving part runs on
//! the one loop ([`LOOP`]), each from its own point in it, so a scene reads as one small story.
//!
//! Invariants:
//! - A scene tells its story [`PASSES`] times when its page opens, then holds at [`REST`]: the
//!   story's middle, everything in place. A page left open draws nothing more (a loop that never
//!   ended would redraw the window for as long as Settings was open).
//! - With Reduce Motion each part is drawn once, at [`REST`].
//! - A part is redrawn from scratch at each moment (`Scene::act`), never restyled: its look is a
//!   plain function of the loop's time, so a scene is the same every pass.

use std::time::Duration;

use gpui::{AnimationExt as _, AnyElement, App, Div, ElementId, Hsla, IntoElement, RenderOnce, Window, div, prelude::*, px};

use crate::{icon::Icon, motion, settings::Section, theme::Theme};

pub const STAGE_WIDTH: f32 = 150.;
pub const STAGE_HEIGHT: f32 = 96.;
/// One pass of every scene.
const LOOP: Duration = Duration::from_millis(6400);
/// How many times a scene plays when its page opens.
const PASSES: f32 = 3.;
/// Where a scene holds when it has played, and with Reduce Motion: past the middle, with
/// everything in place.
const REST: f32 = 0.62;

/// The stage for a Settings section.
#[derive(IntoElement)]
pub struct Stage {
    section: Section,
}

impl Stage {
    pub fn new(section: Section) -> Self {
        Self { section }
    }
}

impl RenderOnce for Stage {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let ink = Ink::of(Theme::get(cx));
        let mut scene = Scene { name: self.section.name(), still: cx.reduce_motion(), parts: Vec::new() };
        match self.section {
            Section::Appearance => appearance(&mut scene, ink),
            Section::Providers => providers(&mut scene, ink),
            Section::Prompts => prompts(&mut scene, ink),
            Section::Tools => tools(&mut scene, ink),
            Section::Memory => memory(&mut scene, ink),
            Section::Skills => skills(&mut scene, ink),
            Section::Commands => commands(&mut scene, ink),
            Section::Subagents => subagents(&mut scene, ink),
            Section::Git => git(&mut scene, ink),
            Section::Integrations => integrations(&mut scene, ink),
            Section::Mcp => mcp(&mut scene, ink),
            Section::Web => web(&mut scene, ink),
            Section::Browser => browser(&mut scene, ink),
            Section::Computer => computer(&mut scene, ink),
        }
        div().relative().flex_none().w(px(STAGE_WIDTH)).h(px(STAGE_HEIGHT)).children(scene.parts)
    }
}

/// The theme's colours a scene draws with.
#[derive(Clone, Copy)]
struct Ink {
    /// A card's ground.
    card: Hsla,
    /// A wash for wells and bars.
    well: Hsla,
    /// Outlines and rules.
    line: Hsla,
    faint: Hsla,
    dim: Hsla,
    text: Hsla,
    accent: Hsla,
    wash: Hsla,
    rim: Hsla,
    success: Hsla,
}

impl Ink {
    fn of(theme: &Theme) -> Self {
        Self {
            card: theme.raised,
            well: theme.hover,
            line: theme.border_strong,
            faint: theme.text_faint,
            dim: theme.text_dim,
            text: theme.text,
            accent: theme.accent_text,
            wash: theme.accent_wash,
            rim: theme.accent_line,
            success: theme.success,
        }
    }
}

struct Scene {
    name: &'static str,
    still: bool,
    parts: Vec<AnyElement>,
}

impl Scene {
    /// A part that holds still.
    fn put(&mut self, part: Div) {
        self.parts.push(part.into_any_element());
    }

    /// A part acted out over the loop: `draw` makes it as it is `t` (0–1) of the way through.
    fn act(&mut self, draw: impl Fn(f32) -> Div + 'static) {
        let index = self.parts.len() as u64;
        let part = if self.still {
            draw(REST).into_any_element()
        } else {
            draw(0.)
                .with_animation(
                    ElementId::NamedInteger(format!("stage-{}", self.name).into(), index),
                    gpui::Animation::new(LOOP.mul_f32(played())),
                    move |_, t| draw(moment(t)),
                )
                .into_any_element()
        };
        self.parts.push(part);
    }
}

/// The whole showing, in passes: the last one stops at [`REST`].
fn played() -> f32 {
    PASSES - 1. + REST
}

/// Where in its pass a scene is, `t` (0–1) of the way through the whole showing.
fn moment(t: f32) -> f32 {
    if t >= 1. { REST } else { (t * played()).fract() }
}

/// A rounded block, placed on the board.
fn block(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Div {
    div().absolute().left(px(x)).top(px(y)).w(px(width.max(0.))).h(px(height.max(0.))).rounded(px(radius))
}

fn dot(cx: f32, cy: f32, r: f32) -> Div {
    block(cx - r, cy - r, 2. * r, 2. * r, r)
}

/// An icon centred on (`cx`, `cy`).
fn glyph(icon: Icon, cx: f32, cy: f32, size: f32, ink: Hsla) -> Div {
    block(cx - size / 2., cy - size / 2., size, size, 0.).flex().items_center().justify_center().child(icon.view(px(size), ink))
}

/// 0 before `from`, 1 after `to`, eased between.
fn ramp(t: f32, from: f32, to: f32) -> f32 {
    let x = ((t - from) / (to - from)).clamp(0., 1.);
    x * x * (3. - 2. * x)
}

/// Up over `from`..`full`, held, down over `fade`..`gone`.
fn span(t: f32, from: f32, full: f32, fade: f32, gone: f32) -> f32 {
    ramp(t, from, full) * (1. - ramp(t, fade, gone))
}

fn lerp(a: f32, b: f32, s: f32) -> f32 {
    a + (b - a) * s
}

fn alpha(colour: Hsla, a: f32) -> Hsla {
    Hsla { a: colour.a * a.clamp(0., 1.), ..colour }
}

/// Every scene empties out at the end of its pass, so the next begins from nothing.
fn shown(t: f32, from: f32, full: f32) -> f32 {
    span(t, from, full, 0.9, 0.97)
}

/// Three theme tiles dealt from a stack; the chosen one lifts, ringed in the accent, its swatch lit.
fn appearance(scene: &mut Scene, ink: Ink) {
    for (k, x) in [18., 56., 94.].into_iter().enumerate() {
        let start = 0.04 + 0.06 * k as f32;
        let chosen = k == 1;
        scene.act(move |t| {
            let dealt = ramp(t, start, start + 0.14);
            let lit = if chosen { span(t, 0.34, 0.4, 0.82, 0.88) } else { 0. };
            let left = lerp(18., x, dealt);
            block(left, 22. - 6. * lit, 38., 52., 9.)
                .bg(ink.card)
                .border_1()
                .border_color(if lit > 0.5 { ink.rim } else { ink.line })
                .opacity(shown(t, start - 0.02, start + 0.06))
                .child(block(7., 9., 22., 4., 2.).bg(ink.line))
                .child(block(7., 17., 15., 4., 2.).bg(ink.well))
                .child(dot(28., 41., 4.).bg(if lit > 0.5 { ink.accent } else { ink.faint }))
        });
    }
}

/// A key slides into its socket and turns the light on; the models it opens light up beside it.
fn providers(scene: &mut Scene, ink: Ink) {
    scene.act(move |t| {
        let on = span(t, 0.38, 0.42, 0.84, 0.9);
        block(14., 16., 40., 64., 10.).bg(ink.card).border_1().border_color(if on > 0.5 { ink.rim } else { ink.line })
    });
    scene.act(move |t| dot(28., 38., 5.).bg(ink.well).border_1().border_color(if span(t, 0.38, 0.42, 0.84, 0.9) > 0.5 { ink.rim } else { ink.line }));
    scene.act(move |t| dot(34., 66., 3.).bg(if span(t, 0.38, 0.42, 0.84, 0.9) > 0.5 { ink.accent } else { ink.faint }));
    // The key comes in shaft first and is pushed home.
    scene.act(move |t| {
        let x = lerp(128., 40., ramp(t, 0.1, 0.3)) - 3. * ramp(t, 0.32, 0.36);
        block(x - 11., 27., 22., 22., 0.)
            .flex()
            .items_center()
            .justify_center()
            .opacity(shown(t, 0.04, 0.12))
            .child(Icon::Key.view(px(22.), ink.accent).with_transformation(gpui::Transformation::rotate(gpui::radians(-0.75 * std::f32::consts::PI))))
    });
    for (k, width) in [52., 40., 46.].into_iter().enumerate() {
        let start = 0.44 + 0.06 * k as f32;
        scene.act(move |t| block(70., 56. + 10. * k as f32, width * shown(t, start, start + 0.1), 4., 2.).bg(alpha(ink.accent, 0.75)));
    }
}

/// A page of instructions writes itself line by line, an accent caret leading, then a spark lands.
fn prompts(scene: &mut Scene, ink: Ink) {
    const WIDTHS: [f32; 4] = [44., 36., 46., 30.];
    scene.act(move |t| {
        let lit = span(t, 0.56, 0.62, 0.86, 0.92);
        block(42., 8., 66., 80., 9.).bg(ink.card).border_1().border_color(if lit > 0.5 { ink.rim } else { ink.line })
    });
    scene.put(block(52., 20., 24., 5., 2.5).bg(ink.accent));
    for (k, width) in WIDTHS.into_iter().enumerate() {
        let start = 0.1 + 0.1 * k as f32;
        scene.act(move |t| block(52., 34. + 11. * k as f32, width * ramp(t, start, start + 0.1), 4., 2.).bg(ink.line).opacity(shown(t, 0., 0.01)));
    }
    scene.act(move |t| {
        let along = ((t - 0.1) / 0.1).clamp(0., 3.999);
        let (line, part) = (along.floor() as usize, along.fract());
        let x = 52. + WIDTHS[line] * ramp(part, 0., 1.);
        block(x + 2., 32. + 11. * line as f32, 2., 8., 1.).bg(ink.accent).opacity(span(t, 0.08, 0.1, 0.52, 0.56))
    });
    scene.act(move |t| glyph(Icon::Spark, 98., 22., 12. * motion::pop(ramp(t, 0.52, 0.6)).max(0.01), ink.accent).opacity(shown(t, 0.52, 0.56)));
}

/// Rows of tools; a tick lands on each in turn.
fn tools(scene: &mut Scene, ink: Ink) {
    for (k, width) in [70., 54., 62., 46.].into_iter().enumerate() {
        let y = 14. + 19. * k as f32;
        let at = 0.12 + 0.14 * k as f32;
        scene.act(move |t| {
            let done = shown(t, at, at + 0.02) > 0.5;
            dot(26., y + 6., 7.).bg(if done { alpha(ink.success, 0.14) } else { ink.well }).border_1().border_color(if done { alpha(ink.success, 0.6) } else { ink.line })
        });
        scene.put(block(42., y + 4., width, 4., 2.).bg(ink.line));
        scene.act(move |t| {
            let size = 10. * motion::pop(ramp(t, at, at + 0.08)).max(0.01);
            glyph(Icon::Check, 26., y + 6., size, ink.success).opacity(shown(t, at, at + 0.03))
        });
    }
}

/// A note is written line by line, a caret at its end, and bookmarked.
fn memory(scene: &mut Scene, ink: Ink) {
    const WIDTHS: [f32; 5] = [42., 34., 44., 28., 38.];
    scene.put(block(44., 6., 62., 84., 8.).bg(ink.card).border_1().border_color(ink.line));
    for (k, width) in WIDTHS.into_iter().enumerate() {
        let start = 0.08 + 0.09 * k as f32;
        scene.act(move |t| block(54., 20. + 11. * k as f32, width * ramp(t, start, start + 0.09), 4., 2.).bg(ink.line).opacity(shown(t, 0., 0.01)));
    }
    scene.act(move |t| {
        let along = ((t - 0.08) / 0.09).clamp(0., 4.999);
        let (line, part) = (along.floor() as usize, along.fract());
        let x = 54. + WIDTHS[line] * ramp(part, 0., 1.);
        block(x + 2., 18. + 11. * line as f32, 2., 8., 1.).bg(ink.accent).opacity(span(t, 0.06, 0.08, 0.52, 0.56))
    });
    scene.act(move |t| glyph(Icon::Bookmark, 96., lerp(-2., 10., ramp(t, 0.5, 0.58)), 14., ink.accent).opacity(shown(t, 0.5, 0.54)));
}

/// Books on a shelf; one lifts, lit, and sparks.
fn skills(scene: &mut Scene, ink: Ink) {
    scene.put(block(30., 82., 90., 2., 1.).bg(ink.line));
    for k in 0..3 {
        let x = 47. + 20. * k as f32;
        if k == 1 {
            scene.act(move |t| {
                let lift = span(t, 0.2, 0.32, 0.78, 0.9);
                block(x, 38. - 14. * lift, 14., 44., 3.).bg(if lift > 0.5 { ink.wash } else { ink.card }).border_1().border_color(if lift > 0.5 { ink.rim } else { ink.line })
            });
        } else {
            scene.put(block(x, 38., 14., 44., 3.).bg(ink.card).border_1().border_color(ink.line));
        }
    }
    for (k, (x, y)) in [(58., 22.), (90., 18.), (76., 10.)].into_iter().enumerate() {
        let start = 0.34 + 0.05 * k as f32;
        scene.act(move |t| glyph(Icon::Spark, x, y - 4. * ramp(t, start, 0.78), 11., ink.accent).opacity(span(t, start, start + 0.05, 0.7, 0.78)));
    }
}

/// A slash opens the command menu; the choice moves down it and is picked into the composer.
fn commands(scene: &mut Scene, ink: Ink) {
    scene.put(block(18., 66., 114., 22., 11.).bg(ink.card).border_1().border_color(ink.line));
    scene.act(move |t| glyph(Icon::Slash, 32., 77., 12., ink.accent).opacity(shown(t, 0.06, 0.1)));
    let menu = move |t: f32| span(t, 0.14, 0.22, 0.62, 0.7);
    scene.act(move |t| block(18., 10. + 6. * (1. - ramp(t, 0.14, 0.22)), 114., 48., 10.).bg(ink.card).border_1().border_color(ink.line).opacity(menu(t)));
    scene.act(move |t| {
        let row = ramp(t, 0.3, 0.38) + ramp(t, 0.42, 0.5);
        block(24., 15. + 12. * row, 102., 12., 6.).bg(ink.wash).opacity(menu(t))
    });
    for (k, width) in [60., 48., 54.].into_iter().enumerate() {
        scene.act(move |t| block(30., 19. + 12. * k as f32, width, 4., 2.).bg(if k == 2 { ink.accent } else { ink.line }).opacity(menu(t)));
    }
    scene.act(move |t| block(44., 75., 50. * shown(t, 0.66, 0.74), 4., 2.).bg(ink.accent));
}

/// The agent sends three helpers out; their work fills in at their own pace and finishes.
fn subagents(scene: &mut Scene, ink: Ink) {
    scene.put(glyph(Icon::Duck, 20., 48., 22., ink.accent));
    for (k, end) in [0.5, 0.72, 0.62].into_iter().enumerate() {
        let y = 22. + 26. * k as f32;
        scene.put(glyph(Icon::Duck, 52., y, 12., ink.dim));
        scene.put(block(64., y - 2., 66., 4., 2.).bg(ink.well));
        scene.act(move |t| {
            let progress = ramp(t, 0.08 + 0.04 * k as f32, end);
            block(64., y - 2., 66. * progress, 4., 2.).bg(if progress >= 1. { ink.success } else { ink.accent }).opacity(shown(t, 0., 0.01))
        });
    }
}

/// A commit card is stamped with the duck and lands on the branch, and the head moves onto it.
fn git(scene: &mut Scene, ink: Ink) {
    scene.put(block(10., 70., 130., 2., 1.).bg(ink.line));
    for x in [24., 52., 80.] {
        scene.put(dot(x, 71., 5.).bg(ink.card).border_1().border_color(ink.line));
    }
    scene.act(move |t| {
        let landed = shown(t, 0.48, 0.52);
        dot(108., 71., 5. * motion::pop(ramp(t, 0.48, 0.56)).max(0.01)).bg(ink.accent).opacity(landed)
    });
    scene.act(move |t| {
        let x = lerp(80., 108., ramp(t, 0.52, 0.6));
        dot(x, 71., 9.).border_1().border_color(ink.rim).opacity(shown(t, 0., 0.01))
    });
    scene.act(move |t| {
        let arrive = ramp(t, 0.12, 0.26);
        let drop = ramp(t, 0.38, 0.48);
        block(76., lerp(-6., 10., arrive) + 40. * drop, 64., 34., 8.)
            .bg(ink.card)
            .border_1()
            .border_color(ink.line)
            .opacity(arrive * (1. - drop))
            .child(block(9., 9., 32., 4., 2.).bg(ink.line))
            .child(block(9., 18., 22., 4., 2.).bg(ink.well))
            .child(glyph(Icon::Duck, 51., 17., 12., ink.accent).opacity(ramp(t, 0.3, 0.34)))
    });
}

/// Two services joined, with messages flowing between them.
fn integrations(scene: &mut Scene, ink: Ink) {
    scene.put(block(16., 28., 38., 38., 11.).bg(ink.card).border_1().border_color(ink.line).child(glyph(Icon::Code, 19., 19., 16., ink.dim)));
    scene.put(block(96., 28., 38., 38., 11.).bg(ink.card).border_1().border_color(ink.rim).child(glyph(Icon::Duck, 19., 19., 16., ink.accent)));
    scene.put(block(54., 46., 42., 2., 1.).bg(ink.line));
    for trip in 0..2 {
        scene.act(move |t| {
            let along = ((t * 2.) - trip as f32).clamp(0., 1.);
            dot(lerp(56., 94., along), 47., 3.).bg(ink.accent).opacity((along * std::f32::consts::PI).sin())
        });
    }
}

/// A server's lights blink as calls flow out to the app and answers flow back.
fn mcp(scene: &mut Scene, ink: Ink) {
    scene.put(block(12., 18., 36., 60., 8.).bg(ink.card).border_1().border_color(ink.line));
    for k in 0..3 {
        let phase = 0.15 + 0.22 * k as f32;
        scene.act(move |t| dot(22., 32. + 14. * k as f32, 2.5).bg(if ((t + phase) * 4.).fract() < 0.35 { ink.accent } else { ink.faint }));
        scene.put(block(30., 30. + 14. * k as f32, 12., 4., 2.).bg(ink.well));
    }
    scene.put(block(102., 30., 36., 36., 10.).bg(ink.card).border_1().border_color(ink.rim).child(glyph(Icon::Duck, 18., 18., 16., ink.accent)));
    scene.put(block(48., 40., 54., 2., 1.).bg(ink.line));
    scene.put(block(48., 56., 54., 2., 1.).bg(ink.line));
    for (y, outward, phase) in [(41., true, 0.), (57., false, 0.5)] {
        scene.act(move |t| {
            let along = ((t * 2.) + phase).fract();
            let x = if outward { lerp(50., 100., along) } else { lerp(100., 50., along) };
            dot(x, y, 2.5).bg(if outward { ink.accent } else { ink.success }).opacity((along * std::f32::consts::PI).sin())
        });
    }
}

/// A search runs along the bar and its results land, the globe lighting as they do.
fn web(scene: &mut Scene, ink: Ink) {
    scene.act(move |t| glyph(Icon::Globe, 28., 48., 32., if shown(t, 0.34, 0.38) > 0.5 { ink.accent } else { ink.faint }));
    scene.put(block(58., 14., 80., 16., 8.).bg(ink.card).border_1().border_color(ink.line).child(glyph(Icon::Search, 10., 8., 9., ink.faint)));
    scene.act(move |t| block(lerp(60., 118., ramp(t, 0.06, 0.3)), 15., 18., 14., 7.).bg(ink.wash).opacity(span(t, 0.06, 0.1, 0.26, 0.32)));
    for (k, width) in [80., 66., 72.].into_iter().enumerate() {
        let start = 0.32 + 0.08 * k as f32;
        scene.act(move |t| {
            let land = ramp(t, start, start + 0.1);
            block(58., 40. + 16. * k as f32 - 8. * (1. - land), width, 10., 5.).bg(ink.well).border_1().border_color(ink.line).opacity(shown(t, start, start + 0.1))
        });
    }
}

/// A page loads in the browser: a light runs along the address, then the page fills in.
fn browser(scene: &mut Scene, ink: Ink) {
    scene.put(block(14., 8., 122., 80., 10.).bg(ink.card).border_1().border_color(ink.line));
    for x in [24., 32., 40.] {
        scene.put(dot(x, 18., 2.5).bg(ink.line));
    }
    scene.put(block(50., 13., 78., 10., 5.).bg(ink.well));
    scene.act(move |t| block(lerp(50., 112., ramp(t, 0.04, 0.24)), 13., 16., 10., 5.).bg(ink.wash).opacity(span(t, 0.04, 0.08, 0.2, 0.26)));
    scene.act(move |t| block(24., 32., 102., 24., 6.).bg(ink.wash).opacity(shown(t, 0.26, 0.36)));
    for (k, width) in [80., 64.].into_iter().enumerate() {
        let start = 0.34 + 0.06 * k as f32;
        scene.act(move |t| block(24., 62. + 9. * k as f32, width, 4., 2.).bg(ink.line).opacity(shown(t, start, start + 0.08)));
    }
}

/// The agent's cursor crosses a window, presses its button and types into its field.
fn computer(scene: &mut Scene, ink: Ink) {
    scene.put(block(10., 14., 100., 66., 10.).bg(ink.card).border_1().border_color(ink.line));
    scene.put(block(22., 28., 76., 12., 6.).bg(ink.well).border_1().border_color(ink.line));
    scene.act(move |t| block(27., 32., 56. * shown(t, 0.52, 0.78), 4., 2.).bg(ink.text));
    scene.act(move |t| {
        let press = span(t, 0.36, 0.38, 0.44, 0.5);
        block(64., 52., 34., 14., 7.).bg(if press > 0.5 { ink.accent } else { ink.wash }).border_1().border_color(ink.rim)
    });
    scene.act(move |t| {
        let to_button = ramp(t, 0.1, 0.34);
        let to_field = ramp(t, 0.46, 0.52);
        let away = ramp(t, 0.86, 0.96);
        let x = lerp(lerp(lerp(138., 84., to_button), 56., to_field), 138., away);
        let y = lerp(lerp(lerp(92., 62., to_button), 38., to_field), 92., away);
        glyph(Icon::Cursor, x, y, 16., ink.text).opacity(span(t, 0.04, 0.1, 0.9, 0.97))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_ease_between_their_ends_and_spans_come_back_down() {
        assert_eq!(ramp(0.1, 0.2, 0.4), 0.);
        assert_eq!(ramp(0.5, 0.2, 0.4), 1.);
        assert!((ramp(0.3, 0.2, 0.4) - 0.5).abs() < 1e-5);
        assert_eq!(span(0.5, 0.1, 0.2, 0.8, 0.9), 1.);
        assert_eq!(span(0.95, 0.1, 0.2, 0.8, 0.9), 0.);
    }

    #[test]
    fn a_showing_plays_its_passes_and_ends_at_rest() {
        assert_eq!(moment(0.), 0.);
        assert!((moment(0.5 / played()) - 0.5).abs() < 1e-5, "the first pass runs through");
        assert!((moment(1.5 / played()) - 0.5).abs() < 1e-5, "and the next");
        assert_eq!(moment(1.), REST, "then it holds");
    }

    #[test]
    fn every_scene_is_empty_at_the_end_of_its_pass_and_full_at_rest() {
        assert_eq!(shown(0.99, 0.1, 0.2), 0., "the next pass starts from nothing");
        assert_eq!(shown(REST, 0.1, 0.2), 1., "Reduce Motion shows the scene complete");
    }
}
