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
}

impl ThemeInput {
    pub fn new(appearance: Appearance) -> Self {
        Self {
            appearance,
            theme_colour: DEFAULT_THEME_COLOUR.into(),
            accent: DEFAULT_ACCENT.into(),
        }
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

        // The accent fill is the user's colour exactly; only text drawn in it is adjusted.
        let accent = accent_source;
        let accent_hover = accent.with_l(accent.l + if dark { 0.05 } else { -0.05 });
        let accent_text = readable_on(
            if dark { accent } else { accent.with_c(accent.c.max(0.12)) },
            window,
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
        Theme {
            appearance,
            sidebar: hsla(neutral(&ramp.sidebar)),
            window: to_hsla(window),
            surface: hsla(neutral(&ramp.surface)),
            raised: hsla(neutral(&ramp.raised)),
            hover: hsla(neutral(&ramp.hover)),
            border: hsla(neutral(&ramp.border)),
            border_strong: hsla(neutral(&ramp.border_strong)),
            text: hsla(neutral(&ramp.text)),
            text_body: hsla(neutral(&ramp.text_body)),
            text_soft: hsla(neutral(&ramp.text_soft)),
            text_dim: hsla(neutral(&ramp.text_dim)),
            text_faint: hsla(neutral(&ramp.text_faint)),
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
            input,
        }
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

/// Keeps the theme in step with the system's light or dark appearance for as long as `window`
/// lives. The app calls this for its window; the gallery does not, so a scene keeps the theme it
/// was asked to render in.
pub fn follow_system_appearance(window: &mut Window) {
    window
        .observe_window_appearance(|window, cx| {
            Theme::set_appearance(window.appearance().into(), cx)
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

    #[test]
    fn a_grey_theme_colour_gives_true_greys() {
        let theme = Theme::derive(ThemeInput {
            appearance: Appearance::Dark,
            theme_colour: "#808080".into(),
            accent: DEFAULT_ACCENT.into(),
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
