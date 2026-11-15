//! Icons and the asset source that serves them.
//!
//! Every icon is a hand-drawn SVG in `assets/icons/`, compiled into the binary. GPUI draws an SVG
//! as a mask, so an icon takes its colour from the element's text colour: set it from a theme
//! token like any text. Icons are drawn on a 16-point grid with a 1.5-point stroke; keep new ones
//! the same so they sit together. Prefer a word to an icon: the app is typographic first.

use std::borrow::Cow;

use gpui::{AssetSource, Hsla, Pixels, SharedString, Styled, Svg, svg};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    ArrowUp,
    Branch,
    Check,
    ChevronDown,
    ChevronRight,
    Close,
    /// The rubber duck: WackCode's mark, and the agent at work.
    Duck,
    File,
    Folder,
    Pencil,
    Pin,
    Plus,
    Search,
    Settings,
    Sidebar,
    Stop,
    Terminal,
}

/// Every icon file, by asset path. One list, so a missing file fails the test below.
const FILES: &[(&str, &[u8])] = &[
    ("icons/arrow-up.svg", include_bytes!("../assets/icons/arrow-up.svg")),
    ("icons/branch.svg", include_bytes!("../assets/icons/branch.svg")),
    ("icons/check.svg", include_bytes!("../assets/icons/check.svg")),
    ("icons/chevron-down.svg", include_bytes!("../assets/icons/chevron-down.svg")),
    ("icons/chevron-right.svg", include_bytes!("../assets/icons/chevron-right.svg")),
    ("icons/close.svg", include_bytes!("../assets/icons/close.svg")),
    ("icons/duck.svg", include_bytes!("../assets/icons/duck.svg")),
    ("icons/file.svg", include_bytes!("../assets/icons/file.svg")),
    ("icons/folder.svg", include_bytes!("../assets/icons/folder.svg")),
    ("icons/pencil.svg", include_bytes!("../assets/icons/pencil.svg")),
    ("icons/pin.svg", include_bytes!("../assets/icons/pin.svg")),
    ("icons/plus.svg", include_bytes!("../assets/icons/plus.svg")),
    ("icons/search.svg", include_bytes!("../assets/icons/search.svg")),
    ("icons/settings.svg", include_bytes!("../assets/icons/settings.svg")),
    ("icons/sidebar.svg", include_bytes!("../assets/icons/sidebar.svg")),
    ("icons/stop.svg", include_bytes!("../assets/icons/stop.svg")),
    ("icons/terminal.svg", include_bytes!("../assets/icons/terminal.svg")),
];

impl Icon {
    pub fn path(self) -> &'static str {
        match self {
            Icon::ArrowUp => "icons/arrow-up.svg",
            Icon::Branch => "icons/branch.svg",
            Icon::Check => "icons/check.svg",
            Icon::ChevronDown => "icons/chevron-down.svg",
            Icon::ChevronRight => "icons/chevron-right.svg",
            Icon::Close => "icons/close.svg",
            Icon::Duck => "icons/duck.svg",
            Icon::File => "icons/file.svg",
            Icon::Folder => "icons/folder.svg",
            Icon::Pencil => "icons/pencil.svg",
            Icon::Pin => "icons/pin.svg",
            Icon::Plus => "icons/plus.svg",
            Icon::Search => "icons/search.svg",
            Icon::Settings => "icons/settings.svg",
            Icon::Sidebar => "icons/sidebar.svg",
            Icon::Stop => "icons/stop.svg",
            Icon::Terminal => "icons/terminal.svg",
        }
    }

    /// The icon as an element, `size` points square, in `colour`.
    pub fn view(self, size: Pixels, colour: Hsla) -> Svg {
        svg()
            .path(self.path())
            .flex_none()
            .size(size)
            .text_color(colour)
    }
}

/// Serves the compiled-in assets. The app and the gallery both install it.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(FILES
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::new_static(name))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_has_a_file() {
        for icon in [
            Icon::ArrowUp,
            Icon::Branch,
            Icon::Check,
            Icon::ChevronDown,
            Icon::ChevronRight,
            Icon::Close,
            Icon::Duck,
            Icon::File,
            Icon::Folder,
            Icon::Pencil,
            Icon::Pin,
            Icon::Plus,
            Icon::Search,
            Icon::Settings,
            Icon::Sidebar,
            Icon::Stop,
            Icon::Terminal,
        ] {
            let bytes = Assets.load(icon.path()).unwrap();
            assert!(bytes.is_some_and(|bytes| bytes.starts_with(b"<svg")), "{icon:?}");
        }
    }
}
