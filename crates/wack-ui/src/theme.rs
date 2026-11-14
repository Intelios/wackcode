//! Theme tokens: every colour a view may use.
//!
//! Views never write a raw colour. They read a token from [`Theme`], which is the one place colours
//! are defined, so the whole app follows the system's light or dark appearance (and, later, the
//! user's theme colour and accent). `raw_colours_live_only_here` below guards the rule.

use gpui::{App, Global, Hsla, Window, WindowAppearance, rgb};

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

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub appearance: Appearance,
    /// The window itself.
    pub background: Hsla,
    /// Areas that sit on the window: the sidebar, panes, cards.
    pub surface: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    /// Secondary text: metadata, hints, placeholders.
    pub text_dim: Hsla,
    /// Interaction only (buttons, focus, selection); never meaning.
    pub accent: Hsla,
    /// Text and icons drawn on top of `accent`.
    pub on_accent: Hsla,
}

impl Global for Theme {}

impl Theme {
    pub fn new(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Dark => Self {
                appearance,
                background: rgb(0x111310).into(),
                surface: rgb(0x181b16).into(),
                border: rgb(0x2a2e26).into(),
                text: rgb(0xe9ecdf).into(),
                text_dim: rgb(0x8d9383).into(),
                accent: rgb(0xc2ee4a).into(),
                on_accent: rgb(0x14160f).into(),
            },
            Appearance::Light => Self {
                appearance,
                background: rgb(0xf3f4ee).into(),
                surface: rgb(0xfbfcf8).into(),
                border: rgb(0xdcdfd3).into(),
                text: rgb(0x1a1c17).into(),
                text_dim: rgb(0x6b7062).into(),
                accent: rgb(0xc2ee4a).into(),
                on_accent: rgb(0x14160f).into(),
            },
        }
    }

    pub fn get(cx: &App) -> &Theme {
        cx.global::<Theme>()
    }

    /// Switches every window to `appearance`. A no-op when it is already current.
    pub fn set_appearance(appearance: Appearance, cx: &mut App) {
        if Self::get(cx).appearance != appearance {
            cx.set_global(Theme::new(appearance));
            cx.refresh_windows();
        }
    }
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
                    if ["rgb(", "rgba(", "hsla(", "gpui::black()", "gpui::white()"]
                        .iter()
                        .any(|needle| line.contains(needle))
                    {
                        offenders.push(format!("{}:{}", path.display(), number + 1));
                    }
                }
            }
        }
    }
}
