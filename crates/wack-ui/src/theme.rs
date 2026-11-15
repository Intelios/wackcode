//! Theme tokens: every colour a view may use.
//!
//! Views never write a raw colour. They read a token from [`Theme`], which is the one place colours
//! are defined, so the whole app follows the appearance, the theme colour and the accent.
//! `raw_colours_live_only_here` below guards the rule.
//!
//! Every themeable token is derived from three inputs ([`ThemeInput`]):
//!
//! - **Appearance** picks a lightness ramp. Lightness is fixed per token and appearance, so text
//!   contrast never depends on the colours the user picks.
//! - **Theme colour** gives the neutrals their hue. Its chroma sets how strongly they are tinted:
//!   a grey theme colour gives true greys, a vivid one a clearly coloured app.
//! - **Accent** paints interaction only: buttons, focus, selection, links. Text drawn in the accent
//!   is moved in lightness until it reads on the window.
//!
//! Over a picture (the fourth input, [`Ground`]) ink must also read on the window's colour laid
//! over the picture's worst part. The quieter steps of ink move towards the loud end by one shared
//! amount, keeping their order; only when that is not enough is the picture dimmed beyond the
//! user's choice ([`Theme::veil`]). Without a picture, the ramp is used as it stands.
//!
//! Meaning colours (danger, warning, success, diff) are fixed per appearance and never follow the
//! accent. Syntax colours keep their own hues so code stays readable, leaning slightly towards the
//! accent so it looks at home in any theme.

use gpui::{App, Global, Hsla, Rgba, Window, WindowAppearance, rgb};

use crate::colour::{Lch, Srgb, angle_delta, contrast, readable_on};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl From<WindowAppearance> for Appearance {
    fn from(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::Dark,
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::Light,
        }
    }
}

/// The lime of the app icon.
pub const DEFAULT_ACCENT: &str = "#c2ee4a";
/// A muted olive: tints the neutrals towards the icon's green without reading as green.
pub const DEFAULT_THEME_COLOUR: &str = "#6f7a5c";

/// What the user chooses. Everything else is derived.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeInput {
    pub appearance: Appearance,
    /// `#rrggbb`; an unreadable value falls back to the default.
    pub theme_colour: String,
    /// `#rrggbb`; an unreadable value falls back to the default.
    pub accent: String,
    /// The picture behind the window, if there is one.
    pub ground: Option<Ground>,
}

impl ThemeInput {
    pub fn new(appearance: Appearance) -> Self {
        Self {
            appearance,
            theme_colour: DEFAULT_THEME_COLOUR.into(),
            accent: DEFAULT_ACCENT.into(),
            ground: None,
        }
    }
}

/// A picture behind the window, as the theme sees it: text is kept readable over all of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ground {
    /// How much of the window's colour the user lays over the picture, 0 to 1.
    pub dim: f32,
    /// How dark and how light the softened picture gets, 0 to 255
    /// (`wack_core::appearance::shade`).
    pub shade: [u8; 2],
}

impl Ground {
    pub fn of(backdrop: &wack_core::Backdrop) -> Self {
        // Unmeasured, the picture could be anything from black to white.
        Self { dim: backdrop.dim as f32 / 100., shade: backdrop.shade.unwrap_or([0, 255]) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub input: ThemeInput,
    pub appearance: Appearance,

    // Surfaces, from the deepest to the most raised.
    /// The sidebar: a step away from the window, so it recedes.
    pub sidebar: Hsla,
    /// The window itself, behind the transcript.
    pub window: Hsla,
    /// Cards on the window: the composer, diffs, code blocks.
    pub surface: Hsla,
    /// Menus and popovers above everything else.
    pub raised: Hsla,
    pub hover: Hsla,
    /// Hairlines between areas and around cards.
    pub border: Hsla,
    pub border_strong: Hsla,

    // Ink, from the loudest to the quietest.
    /// Headings and names.
    pub text: Hsla,
    /// Running text: replies, messages.
    pub text_body: Hsla,
    /// Secondary text: tool targets, chat titles in the sidebar.
    pub text_soft: Hsla,
    /// Metadata: times, counts, verbs in the work log.
    pub text_dim: Hsla,
    /// Placeholders, line numbers, disabled controls.
    pub text_faint: Hsla,

    // Interaction.
    /// Fills: the send button, toggles that are on.
    pub accent: Hsla,
    pub accent_hover: Hsla,
    /// Text and icons drawn on `accent`.
    pub on_accent: Hsla,
    /// The accent as text on the window: links, live status.
    pub accent_text: Hsla,
    /// A faint accent ground: the selected row.
    pub accent_wash: Hsla,
    /// Focus rings and the selected row's edge.
    pub accent_line: Hsla,
    /// Selected text, drawn under the glyphs.
    pub selection: Hsla,

    // Meaning: fixed, never follows the accent.
    pub danger: Hsla,
    pub warning: Hsla,
    pub success: Hsla,
    pub diff_added: Hsla,
    pub diff_added_ground: Hsla,
    pub diff_removed: Hsla,
    pub diff_removed_ground: Hsla,

    // Code.
    pub code_ground: Hsla,
    pub code_keyword: Hsla,
    pub code_string: Hsla,
    pub code_number: Hsla,
    pub code_title: Hsla,
    pub code_meta: Hsla,
    pub code_comment: Hsla,

    // Docks and the terminal.
    /// Behind the panes beside and below the transcript: a step back, like the sidebar, so the
    /// transcript is the lit page between them.
    pub dock: Hsla,
    pub terminal_text: Hsla,
    /// The 16 ANSI colours: black, red, green, yellow, blue, magenta, cyan, white, then the
    /// bright eight. Their own hues, leaning slightly towards the accent; never muted, because
    /// red and green in a terminal carry meaning.
    pub ansi: [Hsla; 16],

    /// How much of the window's colour lies over a picture behind the window, 0 to 1: the user's
    /// dim, or more where the picture is too bright or dark for text otherwise. 0 with none.
    pub veil: f32,
}

impl Global for Theme {}

/// Theme-colour chroma at or above this tints the neutrals fully; below it, proportionally less.
const FULL_TINT_CHROMA: f64 = 0.14;
/// How far syntax hues rotate towards the accent (0 none, 1 onto it).
const SYNTAX_HUE_LEAN: f64 = 0.18;

/// One neutral token: its lightness and how much of the theme tint it carries.
struct Step {
    l: f64,
    /// Chroma at full tint.
    c: f64,
}

/// The lightness ramp for one appearance.
struct Ramp {
    sidebar: Step,
    window: Step,
    surface: Step,
    raised: Step,
    hover: Step,
    border: Step,
    border_strong: Step,
    text: Step,
    text_body: Step,
    text_soft: Step,
    text_dim: Step,
    text_faint: Step,
    code_ground: Step,
}

const fn step(l: f64, c: f64) -> Step {
    Step { l, c }
}

const DARK: Ramp = Ramp {
    sidebar: step(0.070, 0.020),
    window: step(0.092, 0.022),
    surface: step(0.128, 0.028),
    raised: step(0.160, 0.032),
    hover: step(0.150, 0.030),
    border: step(0.200, 0.034),
    border_strong: step(0.290, 0.040),
    text: step(0.935, 0.016),
    text_body: step(0.875, 0.020),
    text_soft: step(0.735, 0.030),
    text_dim: step(0.560, 0.034),
    text_faint: step(0.420, 0.030),
    code_ground: step(0.112, 0.026),
};

const LIGHT: Ramp = Ramp {
    sidebar: step(0.925, 0.020),
    window: step(0.968, 0.014),
    surface: step(0.992, 0.007),
    raised: step(1.000, 0.000),
    hover: step(0.900, 0.024),
    border: step(0.870, 0.026),
    border_strong: step(0.760, 0.032),
    text: step(0.150, 0.022),
    text_body: step(0.230, 0.024),
    text_soft: step(0.390, 0.028),
    text_dim: step(0.500, 0.030),
    text_faint: step(0.640, 0.026),
    code_ground: step(0.945, 0.016),
};

impl Theme {
    /// The default theme colour and accent in `appearance`.
    pub fn new(appearance: Appearance) -> Self {
        Self::derive(ThemeInput::new(appearance))
    }

    pub fn derive(input: ThemeInput) -> Self {
        let appearance = input.appearance;
        let dark = appearance == Appearance::Dark;
        let ramp = if dark { &DARK } else { &LIGHT };
        let lch = |hex: &str, fallback: &str| {
            Srgb::from_hex(hex)
                .or_else(|| Srgb::from_hex(fallback))
                .map(Srgb::to_lch)
                .unwrap_or(Lch::new(0.5, 0., 0.))
        };
        let tint_source = lch(&input.theme_colour, DEFAULT_THEME_COLOUR);
        let accent_source = lch(&input.accent, DEFAULT_ACCENT);
        let default_accent = lch(DEFAULT_ACCENT, DEFAULT_ACCENT);

        let tint = (tint_source.c / FULL_TINT_CHROMA).min(1.);
        let neutral = |step: &Step| Lch::new(step.l, step.c * tint, tint_source.h);
        let window = neutral(&ramp.window).to_srgb();

        // Over a picture, ink is drawn on the window's colour laid over it, so it must read over
        // the picture's worst part too (`over_picture`). Without one, the ramp is used as it is.
        let (lift, veil, text_ground) = match input.ground {
            Some(ground) => {
                let (lift, veil) = over_picture(ground, ramp, window, dark, &neutral);
                (lift, veil, worst_ground(ground, window, veil, dark))
            }
            None => (0., 0., window),
        };
        let text_step = |step: &Step| neutral(&lifted(step, lift, dark));

        // The accent fill is the user's colour exactly; only text drawn in it is adjusted.
        let accent = accent_source;
        let accent_hover = accent.with_l(accent.l + if dark { 0.05 } else { -0.05 });
        let accent_text = readable_on(
            if dark { accent } else { accent.with_c(accent.c.max(0.12)) },
            text_ground,
            4.5,
        );
        let ink = Lch::new(0.16, (accent.c * 0.3).min(0.06), accent.h).to_srgb();
        let paper = Lch::new(0.985, 0.008, accent.h).to_srgb();
        let accent_fill = accent.to_srgb();
        let on_accent = if contrast(ink, accent_fill) >= contrast(paper, accent_fill) {
            ink
        } else {
            paper
        };
        let wash_strength = (accent.c / 0.2).min(1.);
        let accent_wash = if dark {
            Lch::new(0.135, 0.035 * wash_strength, accent.h)
        } else {
            Lch::new(0.935, 0.05 * wash_strength, accent.h)
        };
        let accent_line = if dark {
            Lch::new(0.50, accent.c * 0.6, accent.h)
        } else {
            Lch::new(0.58, accent.c * 0.7, accent.h)
        };

        // Syntax: fixed hue slots that lean a little towards the accent; a grey accent mutes them.
        let lean = SYNTAX_HUE_LEAN * angle_delta(default_accent.h, accent.h);
        let syntax_chroma = 0.4 + 0.6 * (accent.c / default_accent.c).min(1.);
        let syntax = |hue: f64, chroma: f64| {
            let (l, c) = if dark { (0.76, chroma) } else { (0.44, chroma * 1.5) };
            Lch::new(l, c * syntax_chroma, hue + lean)
        };
        let code_keyword = if dark {
            Lch::new(0.86, accent.c * 0.6, accent.h)
        } else {
            readable_on(Lch::new(0.42, accent.c.max(0.12), accent.h), window, 4.5)
        };

        let hsla = |colour: Lch| to_hsla(colour.to_srgb());
        let fixed = meaning(dark);

        // ANSI: six hues at a fixed lightness, and four tinted greys for black and white.
        let ansi_hue = |hue: f64, bright: bool| {
            let (l, c) = match (dark, bright) {
                (true, false) => (0.72, 0.13),
                (true, true) => (0.82, 0.12),
                (false, false) => (0.50, 0.15),
                // In light, "bright" is deeper rather than lighter, or it would not read on the ground.
                (false, true) => (0.45, 0.17),
            };
            hsla(Lch::new(l, c, hue + lean))
        };
        let ansi_grey = |l: f64| hsla(Lch::new(l, 0.02 * tint, tint_source.h));
        let greys = if dark { [0.30, 0.82, 0.52, 0.97] } else { [0.22, 0.86, 0.48, 0.96] };
        let hues = [25., 145., 95., 255., 330., 200.];
        let mut ansi = [Hsla::default(); 16];
        for (bright, offset) in [(false, 0), (true, 8)] {
            ansi[offset] = ansi_grey(greys[if bright { 2 } else { 0 }]);
            ansi[offset + 7] = ansi_grey(greys[if bright { 3 } else { 1 }]);
            for (index, hue) in hues.iter().enumerate() {
                ansi[offset + 1 + index] = ansi_hue(*hue, bright);
            }
        }
        Theme {
            appearance,
            sidebar: hsla(neutral(&ramp.sidebar)),
            window: to_hsla(window),
            surface: hsla(neutral(&ramp.surface)),
            raised: hsla(neutral(&ramp.raised)),
            hover: hsla(neutral(&ramp.hover)),
            border: hsla(neutral(&ramp.border)),
            border_strong: hsla(neutral(&ramp.border_strong)),
            text: hsla(text_step(&ramp.text)),
            text_body: hsla(text_step(&ramp.text_body)),
            text_soft: hsla(text_step(&ramp.text_soft)),
            text_dim: hsla(text_step(&ramp.text_dim)),
            text_faint: hsla(text_step(&ramp.text_faint)),
            accent: to_hsla(accent_fill),
            accent_hover: hsla(accent_hover),
            on_accent: to_hsla(on_accent),
            accent_text: hsla(accent_text),
            accent_wash: hsla(accent_wash),
            accent_line: hsla(accent_line),
            selection: Hsla { a: if dark { 0.32 } else { 0.42 }, ..to_hsla(accent_fill) },
            danger: fixed.danger.into(),
            warning: fixed.warning.into(),
            success: fixed.success.into(),
            diff_added: fixed.diff_added.into(),
            diff_added_ground: fixed.diff_added_ground.into(),
            diff_removed: fixed.diff_removed.into(),
            diff_removed_ground: fixed.diff_removed_ground.into(),
            code_ground: hsla(neutral(&ramp.code_ground)),
            code_keyword: hsla(code_keyword),
            code_string: hsla(syntax(64., 0.07)),
            code_number: hsla(syntax(320., 0.08)),
            code_title: hsla(syntax(226., 0.06)),
            code_meta: hsla(syntax(180., 0.05)),
            code_comment: hsla(neutral(&ramp.text_dim)),
            dock: hsla(neutral(&ramp.sidebar)),
            terminal_text: hsla(neutral(&ramp.text_body)),
            ansi,
            veil: veil as f32,
            input,
        }
    }

    /// One of a terminal's 256 indexed colours: the ANSI sixteen, then the standard 6×6×6 cube
    /// and the 24-step grey ramp.
    pub fn indexed(&self, index: u8) -> Hsla {
        match index {
            0..16 => self.ansi[index as usize],
            16..232 => {
                let cube = index - 16;
                let level = |step: u8| if step == 0 { 0. } else { (55. + 40. * step as f64) / 255. };
                to_hsla(Srgb { r: level(cube / 36), g: level(cube / 6 % 6), b: level(cube % 6) })
            }
            _ => {
                let grey = (8. + 10. * (index - 232) as f64) / 255.;
                to_hsla(Srgb { r: grey, g: grey, b: grey })
            }
        }
    }

    /// A colour a terminal program gave exactly. Shown as asked: it is the program's choice.
    pub fn exact(r: u8, g: u8, b: u8) -> Hsla {
        to_hsla(Srgb { r: r as f64 / 255., g: g as f64 / 255., b: b as f64 / 255. })
    }

    pub fn get(cx: &App) -> &Theme {
        cx.global::<Theme>()
    }

    /// Switches every window to `appearance`, keeping the user's colours. A no-op when it is
    /// already current.
    pub fn set_appearance(appearance: Appearance, cx: &mut App) {
        let current = Self::get(cx);
        if current.appearance != appearance {
            let input = ThemeInput { appearance, ..current.input.clone() };
            cx.set_global(Theme::derive(input));
            cx.refresh_windows();
        }
    }
}

struct Meaning {
    danger: Rgba,
    warning: Rgba,
    success: Rgba,
    diff_added: Rgba,
    diff_added_ground: Rgba,
    diff_removed: Rgba,
    diff_removed_ground: Rgba,
}

fn meaning(dark: bool) -> Meaning {
    if dark {
        Meaning {
            danger: rgb(0xf2867a),
            warning: rgb(0xe9c46a),
            success: rgb(0x8fd49a),
            diff_added: rgb(0x93d6a0),
            diff_added_ground: rgb(0x152219),
            diff_removed: rgb(0xf0958b),
            diff_removed_ground: rgb(0x2b1918),
        }
    } else {
        Meaning {
            danger: rgb(0xc4372c),
            warning: rgb(0x9a6400),
            success: rgb(0x23803a),
            diff_added: rgb(0x1d7434),
            diff_added_ground: rgb(0xdff2e2),
            diff_removed: rgb(0xb3281f),
            diff_removed_ground: rgb(0xfae3e0),
        }
    }
}

/// `#rrggbb` for a token, for the specimen sheet and debugging.
pub fn hex(colour: Hsla) -> String {
    let rgba = colour.to_rgb();
    Srgb { r: rgba.r as f64, g: rgba.g as f64, b: rgba.b as f64 }.to_hex()
}

fn to_hsla(colour: Srgb) -> Hsla {
    Rgba { r: colour.r as f32, g: colour.g as f32, b: colour.b as f32, a: 1. }.into()
}

/// The least contrast each step of ink keeps over the worst part of a picture: text, body, soft,
/// dim and faint. Lower than on the window, since the worst part is seldom right under the text;
/// and never more than the step has on the plain window, so a quiet step stays quiet.
const PICTURE_FLOORS: [f64; 5] = [6., 5., 4.5, 3.5, 3.];
/// How far ink may move towards its loud end before the picture is dimmed further instead: past
/// this the steps would be too close to tell apart.
const MAX_LIFT: f64 = 0.6;

/// `step` moved `lift` of the way towards the loud end: white in dark, black in light. One lift
/// moves every step, so the quieter ones move most and the order is kept.
fn lifted(step: &Step, lift: f64, dark: bool) -> Step {
    let end = if dark { 1. } else { 0. };
    Step { l: step.l + (end - step.l) * lift, c: step.c }
}

/// [`PICTURE_FLOORS`] for the steps of ink as drawn on the plain `window`.
fn picture_floors(inks: [Srgb; 5], window: Srgb) -> [f64; 5] {
    let mut floors = PICTURE_FLOORS;
    for (floor, ink) in floors.iter_mut().zip(inks) {
        *floor = floor.min(contrast(ink, window));
    }
    floors
}

/// What ink sits on over the worst part of a picture: the window's colour at `veil` over the
/// picture's lightest shade (light ink) or its darkest (dark ink), blended as the window draws.
fn worst_ground(ground: Ground, window: Srgb, veil: f64, dark: bool) -> Srgb {
    let shade = ground.shade[if dark { 1 } else { 0 }] as f64 / 255.;
    let mix = |channel: f64| channel * veil + shade * (1. - veil);
    Srgb { r: mix(window.r), g: mix(window.g), b: mix(window.b) }
}

/// After WackCode 2's picture readability: the least lift, and the least veil from the user's
/// dim upwards, at which every step of ink clears its floor over the picture's worst part. The
/// picture keeps the user's dim unless even the most lift is not enough.
fn over_picture(ground: Ground, ramp: &Ramp, window: Srgb, dark: bool, neutral: &dyn Fn(&Step) -> Lch) -> (f64, f64) {
    let steps = [&ramp.text, &ramp.text_body, &ramp.text_soft, &ramp.text_dim, &ramp.text_faint];
    let floors = picture_floors(steps.map(|step| neutral(step).to_srgb()), window);
    let reads = |lift: f64, veil: f64| {
        let behind = worst_ground(ground, window, veil, dark);
        steps
            .iter()
            .zip(floors)
            .all(|(step, floor)| contrast(neutral(&lifted(step, lift, dark)).to_srgb(), behind) >= floor)
    };
    // The least passing value in `low..=high`, which passes at `high`.
    let least = |mut low: f64, mut high: f64, passes: &dyn Fn(f64) -> bool| {
        if passes(low) {
            return low;
        }
        for _ in 0..24 {
            let middle = (low + high) / 2.;
            if passes(middle) { high = middle } else { low = middle }
        }
        high
    };
    // With the veil whole the ground is the window itself, where every step reads.
    let veil = least((ground.dim as f64).clamp(0., 1.), 1., &|veil| reads(MAX_LIFT, veil));
    (least(0., MAX_LIFT, &|lift| reads(lift, veil)), veil)
}

/// The user's choices the theme is derived from (Settings › Appearance), kept so a change of the
/// system's appearance can be followed without asking the store.
#[derive(Clone, Debug, Default, PartialEq)]
struct Preference(wack_core::Appearance);

impl Global for Preference {}

/// The theme for the user's choices, given the system's current appearance.
pub fn input_for(choice: &wack_core::Appearance, system: Appearance) -> ThemeInput {
    ThemeInput {
        appearance: match choice.mode {
            wack_core::Mode::System => system,
            wack_core::Mode::Light => Appearance::Light,
            wack_core::Mode::Dark => Appearance::Dark,
        },
        theme_colour: choice.theme_colour.clone().unwrap_or_else(|| DEFAULT_THEME_COLOUR.into()),
        accent: choice.accent.clone().unwrap_or_else(|| DEFAULT_ACCENT.into()),
        ground: choice.backdrop.as_ref().map(Ground::of),
    }
}

/// Derives the theme from the user's choices and redraws every window with it.
pub fn apply(choice: &wack_core::Appearance, cx: &mut App) {
    cx.set_global(Preference(choice.clone()));
    let theme = Theme::derive(input_for(choice, cx.window_appearance().into()));
    if *Theme::get(cx) != theme {
        cx.set_global(theme);
        cx.refresh_windows();
    }
}

/// Keeps the theme in step with the system's light or dark appearance for as long as `window`
/// lives, unless the user chose one. The app calls this for its window; the gallery does not, so
/// a scene keeps the theme it was asked to render in.
pub fn follow_system_appearance(window: &mut Window) {
    window
        .observe_window_appearance(|window, cx| {
            let choice = cx.try_global::<Preference>().map(|preference| preference.0.clone()).unwrap_or_default();
            let theme = Theme::derive(input_for(&choice, window.appearance().into()));
            if *Theme::get(cx) != theme {
                cx.set_global(theme);
                cx.refresh_windows();
            }
        })
        .detach();
}

/// Colours offered as theme colours: tints for the neutrals, from none to strong.
pub const THEME_SWATCHES: &[&str] = &[
    DEFAULT_THEME_COLOUR,
    "#808080",
    "#5d6b7c",
    "#3e5a8c",
    "#4b7470",
    "#5b7a4c",
    "#8a7a58",
    "#8c5a44",
    "#7d5574",
    "#5a4c8a",
];

/// Colours offered as accents.
pub const ACCENT_SWATCHES: &[&str] = &[
    DEFAULT_ACCENT,
    "#6ee7b7",
    "#6cc4ff",
    "#5b7cff",
    "#a78bfa",
    "#ff8fb8",
    "#ff7a59",
    "#ffc94a",
    "#e8e8e8",
    "#ff4d5e",
];

/// A swatch's colour, to draw it.
pub fn swatch(hex: &str) -> Hsla {
    Srgb::from_hex(hex).map(to_hsla).unwrap_or_default()
}

pub(crate) fn init(cx: &mut App) {
    cx.set_global(Theme::new(cx.window_appearance().into()));
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;

    fn srgb(colour: Hsla) -> Srgb {
        let rgba = colour.to_rgb();
        Srgb { r: rgba.r as f64, g: rgba.g as f64, b: rgba.b as f64 }
    }

    fn themes() -> Vec<Theme> {
        let mut themes = Vec::new();
        for appearance in [Appearance::Light, Appearance::Dark] {
            for (theme_colour, accent) in [
                (DEFAULT_THEME_COLOUR, DEFAULT_ACCENT),
                ("#808080", "#e6e6e6"),
                ("#2f5bd6", "#ff8fb8"),
                ("#d64f2f", "#6cc4ff"),
                ("#7a3fd0", "#ffd400"),
                ("#000000", "#1a1a1a"),
                ("#ffffff", "#ffffff"),
            ] {
                themes.push(Theme::derive(ThemeInput {
                    appearance,
                    theme_colour: theme_colour.into(),
                    accent: accent.into(),
                    ground: None,
                }));
            }
        }
        themes
    }

    /// Whatever colours the user picks, text reads on the window and on cards.
    #[test]
    fn text_stays_readable_in_every_theme() {
        for theme in themes() {
            let label = format!("{:?}", theme.input);
            for (name, ink, minimum) in [
                ("text", theme.text, 12.),
                ("text_body", theme.text_body, 9.),
                ("text_soft", theme.text_soft, 6.),
                ("text_dim", theme.text_dim, 3.5),
                ("accent_text", theme.accent_text, 4.5),
            ] {
                for (ground_name, ground) in
                    [("window", theme.window), ("surface", theme.surface)]
                {
                    let ratio = contrast(srgb(ink), srgb(ground));
                    assert!(
                        ratio >= minimum - 0.3,
                        "{name} on {ground_name} is {ratio:.2} in {label}"
                    );
                }
            }
            let on_accent = contrast(srgb(theme.on_accent), srgb(theme.accent));
            assert!(on_accent >= 4.5, "on_accent is {on_accent:.2} in {label}");
        }
    }

    /// The six ANSI hues read on the terminal's ground in every theme.
    #[test]
    fn terminal_colours_read_on_the_dock() {
        for theme in themes() {
            for index in [1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14] {
                let ratio = contrast(srgb(theme.ansi[index]), srgb(theme.dock));
                assert!(ratio >= 3., "ANSI {index} is {ratio:.2} in {:?}", theme.input);
            }
            assert_eq!(theme.indexed(3), theme.ansi[3]);
            assert_eq!(hex(theme.indexed(16)), "#000000");
            assert_eq!(hex(theme.indexed(231)), "#ffffff");
        }
    }

    /// Over any picture, at any dim, every step of ink clears its floor over the picture's worst
    /// part, keeps its order, and the picture is dimmed further only when ink alone cannot do it.
    #[test]
    fn text_reads_over_any_picture() {
        for base in themes() {
            for shade in [[0, 255], [40, 215], [10, 60], [190, 250], [119, 119]] {
                for dim in [0., 0.45, 0.65, 0.9] {
                    let ground = Ground { dim, shade };
                    let theme = Theme::derive(ThemeInput { ground: Some(ground), ..base.input.clone() });
                    let label = format!("{:?} over {shade:?} at {dim}", base.input.appearance);
                    assert!(theme.veil >= dim, "{label}: the veil is never thinner than chosen");
                    let dark = theme.appearance == Appearance::Dark;
                    let behind = worst_ground(ground, srgb(theme.window), theme.veil as f64, dark);
                    let inks = [theme.text, theme.text_body, theme.text_soft, theme.text_dim, theme.text_faint];
                    let plain = [base.text, base.text_body, base.text_soft, base.text_dim, base.text_faint];
                    let floors = picture_floors(plain.map(srgb), srgb(base.window));
                    for (index, (ink, floor)) in inks.iter().zip(floors).enumerate() {
                        let ratio = contrast(srgb(*ink), behind);
                        assert!(ratio >= floor - 0.05, "ink {index} is {ratio:.2} in {label}");
                    }
                    let lightness: Vec<f64> = inks.iter().map(|ink| srgb(*ink).to_lch().l).collect();
                    assert!(
                        lightness.windows(2).all(|pair| if dark { pair[0] >= pair[1] } else { pair[0] <= pair[1] }),
                        "the steps keep their order in {label}: {lightness:?}"
                    );
                }
            }
        }
        // A picture darker than the window, under light text, needs nothing changed.
        let calm = Theme::derive(ThemeInput {
            ground: Some(Ground { dim: 0.65, shade: [0, 5] }),
            ..ThemeInput::new(Appearance::Dark)
        });
        let plain = Theme::new(Appearance::Dark);
        assert_eq!((calm.text_dim, calm.text_faint, calm.veil), (plain.text_dim, plain.text_faint, 0.65));
        assert_eq!(plain.veil, 0., "no picture, no veil");
    }

    #[test]
    fn a_grey_theme_colour_gives_true_greys() {
        let theme = Theme::derive(ThemeInput {
            appearance: Appearance::Dark,
            theme_colour: "#808080".into(),
            accent: DEFAULT_ACCENT.into(),
            ground: None,
        });
        for token in [theme.window, theme.surface, theme.text, theme.text_dim] {
            assert!(srgb(token).to_lch().c < 0.002);
        }
    }

    #[test]
    fn meaning_colours_ignore_the_accent() {
        let base = Theme::new(Appearance::Dark);
        let other = Theme::derive(ThemeInput {
            appearance: Appearance::Dark,
            theme_colour: "#2f5bd6".into(),
            accent: "#ff3b30".into(),
            ground: None,
        });
        assert_eq!(base.danger, other.danger);
        assert_eq!(base.success, other.success);
        assert_eq!(base.diff_added, other.diff_added);
        assert_eq!(base.diff_removed_ground, other.diff_removed_ground);
    }

    #[test]
    fn an_unreadable_colour_falls_back_to_the_default() {
        let broken = Theme::derive(ThemeInput {
            appearance: Appearance::Light,
            theme_colour: "olive".into(),
            accent: "".into(),
            ground: None,
        });
        let default = Theme::new(Appearance::Light);
        assert_eq!(broken.window, default.window);
        assert_eq!(broken.accent, default.accent);
    }

    /// A raw colour anywhere else would not follow the theme.
    #[test]
    fn raw_colours_live_only_here() {
        let mut offenders = Vec::new();
        scan(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src").as_path(),
            &mut offenders,
        );
        assert!(
            offenders.is_empty(),
            "raw colours outside theme.rs: {offenders:?}"
        );
    }

    fn scan(dir: &Path, offenders: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                scan(&path, offenders);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && path.file_name().unwrap() != "theme.rs"
            {
                let source = fs::read_to_string(&path).unwrap();
                for (number, line) in source.lines().enumerate() {
                    if ["rgb(", "rgba(", "hsla(", "gpui::black()", "gpui::white()", "Srgb {"]
                        .iter()
                        .any(|needle| line.contains(needle))
                        && !path.ends_with("colour.rs")
                    {
                        offenders.push(format!("{}:{}", path.display(), number + 1));
                    }
                }
            }
        }
    }
}
