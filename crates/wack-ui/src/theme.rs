//! Theme tokens: every colour a view may use.
//!
//! Views never write a raw colour. They read a token from [`Theme`], which is the one place colours
//! are defined, so the whole app follows the appearance and the theme.
//! `raw_colours_live_only_here` below guards the rule.
//!
//! Every themeable token is derived from two choices ([`ThemeInput`]):
//!
//! - **Appearance** picks a lightness ramp. Lightness is fixed per token and appearance, so text
//!   contrast never depends on the theme.
//! - **Theme**: one of WackCode 2's ten ([`palette`]), each a background and an accent, four with
//!   a sweep. The user chooses one; nobody picks colours.
//!   - The background gives the neutrals their hue, and its chroma sets how strongly they are
//!     tinted against the ramp: Mono gives true greys, Nebula a clearly violet app. In dark the
//!     window is the background exactly and the other surfaces keep their steps from it, so dark
//!     is WackCode 2's ground; light carries the same hue, as strongly, onto the light ramp.
//!   - The accent paints interaction only: buttons, focus, selection, links. Its fill is the
//!     theme's colour exactly, except that a grey accent mirrors its lightness in light (Mono's
//!     near-white would vanish on paper). Text drawn in the accent is moved in lightness until
//!     it reads on the window.
//!   - A sweep paints the accent's fills ([`Theme::accent_fill`]): the send button, primary
//!     pills, switches that are on, slider fills. Lines, text, the selection and the terminal's
//!     cursor stay the solid accent. Sweeps are three-stop and linear, never Ultra's five-colour
//!     prism. GPUI draws two stops, so for now a sweep runs from its first stop to its last,
//!     blended in OKLab; that misses each middle stop by 0.03 to 0.05 in OKLab, a visible
//!     difference, and painting all three is open with the owner.
//!
//! Over a picture (the fourth input, [`Ground`]) the picture is dimmed exactly as the user chose
//! ([`Theme::veil`]), and text sits on glass panes over it: the window's colour at
//! [`Theme::glass`] over the blurred picture. Ink must read on that glass over the picture's
//! worst part. The glass is never thinner than [`GLASS_MIN`]; the quieter steps of ink move
//! towards the loud end by one shared amount, keeping their order, and only when that is not
//! enough does the glass thicken. Without a picture the panes are solid and the ramp is used as
//! it stands.
//!
//! Meaning colours (danger, warning, success, diff) are fixed per appearance and never follow the
//! accent. Syntax colours keep their own hues so code stays readable, leaning slightly towards the
//! accent so it looks at home in any theme.

use gpui::{App, Background, ColorSpace, Global, Hsla, Rgba, Window, WindowAppearance, linear_color_stop, linear_gradient, rgb};
pub use wack_core::ThemeName;

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

/// A theme's own colours, as WackCode 2 defined them (its `THEME_PRESETS`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// The dark window, exactly; its hue and chroma tint every neutral in both appearances.
    pub background: &'static str,
    pub accent: &'static str,
    /// Three stops the accent's fills sweep through, for the four themes that have one. Every
    /// stop reads under `on_accent`.
    pub sweep: Option<[&'static str; 3]>,
}

/// The colours of `theme`.
pub fn palette(theme: ThemeName) -> Palette {
    let (background, accent, sweep) = match theme {
        // The lime of the app icon on a green-black.
        ThemeName::WackCode => ("#111310", "#c2ee4a", None),
        ThemeName::Midnight => ("#0f1218", "#6cc4ff", None),
        ThemeName::Lagoon => ("#0d1517", "#5fd4c8", Some(["#54e0a8", "#4fd4e0", "#6aa8f2"])),
        ThemeName::Verdant => ("#0a1410", "#6fd99e", Some(["#a8e268", "#4fd9a8", "#3ec9d4"])),
        ThemeName::Grape => ("#14111b", "#b69cff", None),
        ThemeName::Nebula => ("#1a0e20", "#c085f0", Some(["#9f8cf7", "#d47ae8", "#f77fb2"])),
        ThemeName::Rose => ("#181114", "#ff8fb8", None),
        ThemeName::Solar => ("#1c1606", "#ff9f70", Some(["#ffc15e", "#ff8a66", "#f26d9b"])),
        ThemeName::Ember => ("#17120f", "#ff9a5c", None),
        ThemeName::Mono => ("#111111", "#e6e6e6", None),
    };
    Palette { background, accent, sweep }
}

/// What the user chooses. Everything else is derived.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeInput {
    pub appearance: Appearance,
    pub theme: ThemeName,
    /// The picture behind the window, if there is one.
    pub ground: Option<Ground>,
}

impl ThemeInput {
    pub fn new(appearance: Appearance) -> Self {
        Self { appearance, theme: ThemeName::default(), ground: None }
    }
}

/// A picture behind the window, as the theme sees it: text is kept readable over all of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ground {
    /// How much of the window's colour the user lays over the picture, 0 to 1. Drawn exactly.
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
    /// The accent as one colour: lines, the terminal's cursor, and fills too small for a sweep.
    /// A fill a person presses is painted with [`Theme::accent_fill`] instead.
    pub accent: Hsla,
    pub accent_hover: Hsla,
    /// The stops a sweeping theme's fills run through, and the same lifted for hover.
    pub accent_sweep: Option<[Hsla; 3]>,
    pub accent_sweep_hover: Option<[Hsla; 3]>,
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

    // Ultra (Ultra Plan and the Ultra reasoning level): a fixed prism, cyan through peach, so no
    // accent can ever match it. Like the meaning colours, it never follows the accent.
    /// The lavender base: Ultra's text, tints and lines.
    pub ultra: Hsla,
    /// The bright core: the mark's centre pixel, and Ultra's primary button when hovered.
    pub ultra_glow: Hsla,
    /// Text and icons drawn on `ultra`.
    pub on_ultra: Hsla,
    /// The prism's stops, in order: cyan, violet, orchid, pink, peach.
    pub ultra_spectrum: [Hsla; 5],

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
    /// dim, exactly. 0 with none.
    pub veil: f32,
    /// How much of the window's colour the glass panes (the sidebar and the stage) lay over the
    /// dimmed picture, 0 to 1: at least [`GLASS_MIN`], more where the picture is too bright or
    /// dark for text otherwise. 1 with no picture, where the panes are solid.
    pub glass: f32,
}

impl Global for Theme {}

/// The thinnest the glass panes get over a picture: enough of the picture shows to feel it, and
/// the blur under the glass does the rest.
pub const GLASS_MIN: f32 = 0.72;

/// The most a theme's background may tint the neutrals, as a multiple of the ramp's own chroma.
const MAX_TINT: f64 = 2.;
/// Below this chroma an accent is grey: in light its lightness is mirrored so its fill shows.
const GREY_ACCENT: f64 = 0.02;
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
    /// How much of the window's colour lies over the picture where text sits on the panes: the
    /// dim and the glass together. Something laid straight on the clear picture takes this much
    /// window, frosted, for the ink floors to hold there too.
    pub fn cover(&self) -> f32 {
        cover_of(self.veil as f64, self.glass as f64) as f32
    }

    /// The default theme in `appearance`.
    pub fn new(appearance: Appearance) -> Self {
        Self::derive(ThemeInput::new(appearance))
    }

    pub fn derive(input: ThemeInput) -> Self {
        let appearance = input.appearance;
        let dark = appearance == Appearance::Dark;
        let ramp = if dark { &DARK } else { &LIGHT };
        let colours = palette(input.theme);
        let lch = |hex: &str| Srgb::from_hex(hex).map(Srgb::to_lch).unwrap_or(Lch::new(0.5, 0., 0.));
        let background = lch(colours.background);
        let default_accent = lch(palette(ThemeName::default()).accent);

        // The background's chroma against the window step's: Mono none, Nebula nearly twice.
        let tint = (background.c / DARK.window.c).min(MAX_TINT);
        let neutral = |step: &Step| Lch::new(step.l, step.c * tint, background.h);
        // In dark the surfaces keep their steps from the theme's own window; ink keeps its
        // lightness, so contrast never depends on the theme.
        let shift = if dark { background.l - DARK.window.l } else { 0. };
        let surface = |step: &Step| neutral(&Step { l: step.l + shift, c: step.c });
        let window = if dark { Srgb::from_hex(colours.background).unwrap_or(surface(&ramp.window).to_srgb()) } else { surface(&ramp.window).to_srgb() };

        // Over a picture, ink is drawn on glass over the dimmed picture, so it must read over the
        // picture's worst part too (`over_picture`). Without one, the ramp is used as it is.
        let (lift, veil, glass, text_ground) = match input.ground {
            Some(ground) => {
                let (lift, cover) = over_picture(ground, ramp, window, dark, &neutral);
                let dim = (ground.dim as f64).clamp(0., MAX_DIM);
                (lift, dim, glass_over(dim, cover), worst_ground(ground, window, cover, dark))
            }
            None => (0., 0., 1., window),
        };
        let text_step = |step: &Step| neutral(&lifted(step, lift, dark));

        // The accent fill is the theme's colour exactly; only text drawn in it is adjusted. A grey
        // accent is the exception: in light it mirrors, so Mono's near-white becomes near-black.
        let accent_source = lch(colours.accent);
        let grey_accent = accent_source.c < GREY_ACCENT;
        let accent = if !dark && grey_accent { accent_source.with_l(1. - accent_source.l) } else { accent_source };
        // Hover lightens a fill in dark and darkens it in light, except a fill already near black,
        // which can only show a change by lightening.
        let accent_hover = accent.with_l(accent.l + if dark || accent.l < 0.3 { 0.05 } else { -0.05 });
        // Text in the accent is given colour in light so it reads as the accent, not as ink; a
        // grey accent stays grey rather than taking on whatever hue it happens to have.
        let text_chroma = if grey_accent { 0. } else { accent.c.max(0.12) };
        let accent_text = readable_on(
            if dark { accent } else { accent.with_c(text_chroma) },
            text_ground,
            4.5,
        );
        let ink = Lch::new(0.16, (accent.c * 0.3).min(0.06), accent.h).to_srgb();
        let paper = Lch::new(0.985, 0.008, accent.h).to_srgb();
        let accent_fill = accent.to_srgb();
        // A sweep's hover lifts every stop a little, as the solid fill's does.
        let sweep = colours.sweep.map(|stops| stops.map(lch));
        let sweep_hover = sweep.map(|stops| stops.map(|stop| stop.with_l(stop.l + 0.04)));
        // Ink on the accent must read on every colour a fill can show, its sweep's stops included.
        let grounds: Vec<Srgb> = std::iter::once(accent_fill)
            .chain(sweep.into_iter().chain(sweep_hover).flatten().map(Lch::to_srgb))
            .collect();
        let least = |on: Srgb| grounds.iter().map(|ground| contrast(on, *ground)).fold(f64::INFINITY, f64::min);
        let on_accent = if least(ink) >= least(paper) { ink } else { paper };
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
            readable_on(Lch::new(0.42, text_chroma, accent.h), window, 4.5)
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
        let ansi_grey = |l: f64| hsla(Lch::new(l, 0.02 * tint, background.h));
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
            sidebar: hsla(surface(&ramp.sidebar)),
            window: to_hsla(window),
            surface: hsla(surface(&ramp.surface)),
            raised: hsla(surface(&ramp.raised)),
            hover: hsla(surface(&ramp.hover)),
            border: hsla(surface(&ramp.border)),
            border_strong: hsla(surface(&ramp.border_strong)),
            text: hsla(text_step(&ramp.text)),
            text_body: hsla(text_step(&ramp.text_body)),
            text_soft: hsla(text_step(&ramp.text_soft)),
            text_dim: hsla(text_step(&ramp.text_dim)),
            text_faint: hsla(text_step(&ramp.text_faint)),
            accent: to_hsla(accent_fill),
            accent_hover: hsla(accent_hover),
            accent_sweep: sweep.map(|stops| stops.map(hsla)),
            accent_sweep_hover: sweep_hover.map(|stops| stops.map(hsla)),
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
            ultra: fixed.ultra.into(),
            ultra_glow: fixed.ultra_glow.into(),
            on_ultra: fixed.on_ultra.into(),
            ultra_spectrum: fixed.ultra_spectrum.map(Into::into),
            code_ground: hsla(surface(&ramp.code_ground)),
            code_keyword: hsla(code_keyword),
            code_string: hsla(syntax(64., 0.07)),
            code_number: hsla(syntax(320., 0.08)),
            code_title: hsla(syntax(226., 0.06)),
            code_meta: hsla(syntax(180., 0.05)),
            code_comment: hsla(neutral(&ramp.text_dim)),
            dock: hsla(surface(&ramp.sidebar)),
            terminal_text: hsla(neutral(&ramp.text_body)),
            ansi,
            veil: veil as f32,
            glass: glass as f32,
            input,
        }
    }

    /// What a fill in the accent is painted with: the theme's sweep, or the solid accent.
    pub fn accent_fill(&self) -> Background {
        sweep_or(self.accent_sweep, self.accent)
    }

    /// [`Theme::accent_fill`] under the pointer.
    pub fn accent_fill_hover(&self) -> Background {
        sweep_or(self.accent_sweep_hover, self.accent_hover)
    }

    /// The prism `along` (0–1) of the way from cyan to peach, blending between its stops.
    pub fn ultra_at(&self, along: f32) -> Hsla {
        let stops = &self.ultra_spectrum;
        let along = along.clamp(0., 1.) * (stops.len() - 1) as f32;
        let low = (along.floor() as usize).min(stops.len() - 2);
        let mix = along - low as f32;
        let (a, b) = (stops[low].to_rgb(), stops[low + 1].to_rgb());
        Rgba { r: a.r + (b.r - a.r) * mix, g: a.g + (b.g - a.g) * mix, b: a.b + (b.b - a.b) * mix, a: 1. }.into()
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

    /// Switches every window to `appearance`, keeping the user's theme. A no-op when it is
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
    ultra: Rgba,
    ultra_glow: Rgba,
    on_ultra: Rgba,
    ultra_spectrum: [Rgba; 5],
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
            // Version 2's prism, which was drawn on dark grounds.
            ultra: rgb(0xcfc6ff),
            ultra_glow: rgb(0xf6f2ff),
            on_ultra: rgb(0x17142a),
            ultra_spectrum: [rgb(0x7fd8ff), rgb(0xa9a6ff), rgb(0xe59cff), rgb(0xff9fc6), rgb(0xffc48a)],
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
            // The same prism, deepened until it reads on a light ground.
            ultra: rgb(0x5546c4),
            ultra_glow: rgb(0x3a2d9a),
            on_ultra: rgb(0xffffff),
            ultra_spectrum: [rgb(0x0a84bd), rgb(0x5a50e0), rgb(0xa23ccb), rgb(0xcf3576), rgb(0xbf6410)],
        }
    }
}

/// A sweep from its first stop to its last, towards the lower right as WackCode 2 drew it.
fn sweep_or(sweep: Option<[Hsla; 3]>, solid: Hsla) -> Background {
    match sweep {
        Some([first, _, last]) => {
            linear_gradient(135., linear_color_stop(first, 0.), linear_color_stop(last, 1.)).color_space(ColorSpace::Oklab)
        }
        None => solid.into(),
    }
}

/// `#rrggbb` for a token, for the specimen sheet and debugging.
pub fn hex(colour: Hsla) -> String {
    let rgba = colour.to_rgb();
    Srgb { r: rgba.r as f64, g: rgba.g as f64, b: rgba.b as f64 }.to_hex()
}

/// A design's own colour, shown as data (a canvas tweak's swatch), never as the app's: `#rgb` or
/// `#rrggbb`. The one way a view may draw a colour that is not the theme's.
pub fn design_colour(hex: &str) -> Option<Hsla> {
    Srgb::from_hex(hex).map(to_hsla)
}

/// Hue turns of a design colour, for a colour tweak with no options of its own: the colour
/// itself first, then `count - 1` turns at its lightness and chroma, as `#rrggbb`.
pub fn hue_turns(hex: &str, count: usize) -> Vec<String> {
    let Some(base) = Srgb::from_hex(hex) else { return Vec::new() };
    let lch = base.to_lch();
    let mut turns = vec![base.to_hex()];
    for step in 1..count {
        let h = (lch.h + 360. * step as f64 / count as f64) % 360.;
        turns.push(Lch::new(lch.l, lch.c.max(0.06), h).to_srgb().to_hex());
    }
    turns
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

/// The most a picture is dimmed (`wack_core::appearance::MAX_DIM`): below whole, so the glass
/// over it always has a share to take.
const MAX_DIM: f64 = 0.9;

/// How much of the window's colour lies over the picture altogether, where glass at `glass` sits
/// on a picture dimmed by `dim`.
fn cover_of(dim: f64, glass: f64) -> f64 {
    1. - (1. - dim) * (1. - glass)
}

/// The glass that, over a picture dimmed by `dim`, makes `cover` altogether.
fn glass_over(dim: f64, cover: f64) -> f64 {
    (1. - (1. - cover) / (1. - dim)).clamp(GLASS_MIN as f64, 1.)
}

/// What ink sits on over the worst part of a picture: the window's colour at `veil` over the
/// picture's lightest shade (light ink) or its darkest (dark ink), blended as the window draws.
fn worst_ground(ground: Ground, window: Srgb, veil: f64, dark: bool) -> Srgb {
    let shade = ground.shade[if dark { 1 } else { 0 }] as f64 / 255.;
    let mix = |channel: f64| channel * veil + shade * (1. - veil);
    Srgb { r: mix(window.r), g: mix(window.g), b: mix(window.b) }
}

/// After WackCode 2's picture readability: the least lift, and the least cover (the dim and the
/// glass together) from the thinnest glass upwards, at which every step of ink clears its floor
/// over the picture's worst part. The glass stays at its thinnest unless even the most lift is
/// not enough.
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
    // With the cover whole the ground is the window itself, where every step reads.
    let thinnest = cover_of((ground.dim as f64).clamp(0., MAX_DIM), GLASS_MIN as f64);
    let cover = least(thinnest, 1., &|cover| reads(MAX_LIFT, cover));
    (least(0., MAX_LIFT, &|lift| reads(lift, cover)), cover)
}

/// The user's choices the theme is derived from (Settings › Appearance), kept so a change of the
/// system's appearance can be followed without asking the store.
#[derive(Clone, Debug, Default, PartialEq)]
struct Preference(wack_core::Appearance);

impl Global for Preference {}

impl Theme {
    /// The colours for the agent's cursor, drawn natively over other apps' windows: the accent
    /// arrow edged in the window's colour, its label on a card's ground. Alpha is dropped, so
    /// the label is solid over any app.
    pub fn cursor_appearance(&self) -> wack_computer::cursor::Appearance {
        let rgb = |colour: Hsla| {
            let rgba = colour.to_rgb();
            [rgba.r as f64, rgba.g as f64, rgba.b as f64]
        };
        wack_computer::cursor::Appearance { accent: rgb(self.accent), outline: rgb(self.window), surface: rgb(self.surface), text: rgb(self.text), muted: rgb(self.text_soft) }
    }
}

/// The theme for the user's choices, given the system's current appearance.
pub fn input_for(choice: &wack_core::Appearance, system: Appearance) -> ThemeInput {
    ThemeInput {
        appearance: match choice.mode {
            wack_core::Mode::System => system,
            wack_core::Mode::Light => Appearance::Light,
            wack_core::Mode::Dark => Appearance::Dark,
        },
        theme: choice.theme,
        ground: choice.backdrop().map(Ground::of),
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

    /// Every theme in both appearances.
    fn themes() -> Vec<Theme> {
        let mut themes = Vec::new();
        for appearance in [Appearance::Light, Appearance::Dark] {
            for theme in ThemeName::ALL {
                themes.push(Theme::derive(ThemeInput { appearance, theme, ground: None }));
            }
        }
        themes
    }

    /// In every theme, text reads on the window and on cards.
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

    /// Over any picture, at any dim, every step of ink clears its floor on the glass over the
    /// picture's worst part and keeps its order; the dim is drawn as chosen, and the glass
    /// thickens past its thinnest only when ink alone cannot do it.
    #[test]
    fn text_reads_over_any_picture() {
        for base in themes() {
            for shade in [[0, 255], [40, 215], [10, 60], [190, 250], [119, 119]] {
                for dim in [0., 0.45, 0.65, 0.9] {
                    let ground = Ground { dim, shade };
                    let theme = Theme::derive(ThemeInput { ground: Some(ground), ..base.input.clone() });
                    let label = format!("{:?} over {shade:?} at {dim}", base.input.appearance);
                    assert_eq!(theme.veil, dim, "{label}: the dim is drawn as chosen");
                    assert!(theme.glass >= GLASS_MIN, "{label}: the glass is never thinner than its thinnest");
                    let dark = theme.appearance == Appearance::Dark;
                    let cover = cover_of(dim as f64, theme.glass as f64);
                    let behind = worst_ground(ground, srgb(theme.window), cover, dark);
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
        assert_eq!((calm.text_dim, calm.text_faint, calm.veil, calm.glass), (plain.text_dim, plain.text_faint, 0.65, GLASS_MIN));
        assert_eq!((plain.veil, plain.glass), (0., 1.), "no picture: no veil, solid panes");
    }

    /// Dark is WackCode 2's: the window is the theme's background and the accent its accent.
    #[test]
    fn dark_is_version_two_exactly() {
        for theme in ThemeName::ALL {
            let derived = Theme::derive(ThemeInput { appearance: Appearance::Dark, theme, ground: None });
            let colours = palette(theme);
            assert_eq!(hex(derived.window), colours.background, "{theme:?}");
            assert_eq!(hex(derived.accent), colours.accent, "{theme:?}");
        }
    }

    /// Ink on the accent reads on every colour a fill shows: a sweep's stops, hovered or not.
    #[test]
    fn ink_reads_on_every_stop_of_a_sweep() {
        for theme in themes() {
            let stops = theme.accent_sweep.into_iter().chain(theme.accent_sweep_hover).flatten();
            for stop in stops.chain([theme.accent]) {
                let ratio = contrast(srgb(theme.on_accent), srgb(stop));
                assert!(ratio >= 4.5, "on_accent on {} is {ratio:.2} in {:?}", hex(stop), theme.input);
            }
        }
        let sweeping = ThemeName::ALL.into_iter().filter(|theme| palette(*theme).sweep.is_some()).count();
        assert_eq!(sweeping, 4, "Lagoon, Verdant, Nebula and Solar sweep");
    }

    /// A fill in the accent stands off the window in every theme (Mono's mirrors in light), and
    /// hovering it visibly changes it.
    #[test]
    fn the_accent_fill_shows_on_the_window() {
        for theme in themes() {
            let ratio = contrast(srgb(theme.accent), srgb(theme.window));
            assert!(ratio >= 1.2, "the accent is {ratio:.2} on the window in {:?}", theme.input);
            let hover = contrast(srgb(theme.accent), srgb(theme.accent_hover));
            assert!(hover >= 1.08, "hover changes the accent by only {hover:.2} in {:?}", theme.input);
        }
    }

    #[test]
    fn mono_is_true_grey_and_nebula_is_violet() {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let mono = Theme::derive(ThemeInput { appearance, theme: ThemeName::Mono, ground: None });
            for token in [mono.window, mono.surface, mono.text, mono.text_dim, mono.accent, mono.accent_text, mono.code_keyword] {
                assert!(srgb(token).to_lch().c < 0.002, "{appearance:?}");
            }
            let nebula = Theme::derive(ThemeInput { appearance, theme: ThemeName::Nebula, ground: None });
            let window = srgb(nebula.window).to_lch();
            assert!(window.c > 0.012 && (window.h - 314.).abs() < 10., "{appearance:?}: {window:?}");
        }
    }

    #[test]
    fn meaning_colours_ignore_the_theme() {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let base = Theme::new(appearance);
            let other = Theme::derive(ThemeInput { appearance, theme: ThemeName::Rose, ground: None });
            assert_eq!(base.danger, other.danger);
            assert_eq!(base.success, other.success);
            assert_eq!(base.diff_added, other.diff_added);
            assert_eq!(base.diff_removed_ground, other.diff_removed_ground);
            assert_eq!(base.ultra, other.ultra);
            assert_eq!(base.ultra_spectrum, other.ultra_spectrum);
        }
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
