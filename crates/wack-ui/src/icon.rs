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
    /// A flagged problem, on a suggestion card.
    Alert,
    ArrowUp,
    /// Back, in the browser.
    ArrowLeft,
    /// Forward, in the browser.
    ArrowRight,
    /// Reload a page.
    Reload,
    /// A web page.
    Globe,
    /// The agent's pointer, and taking control of a page from it.
    Cursor,
    /// A page's viewport size.
    Phone,
    /// A skill.
    Book,
    /// Memory: a note kept for later chats.
    Bookmark,
    /// Plan mode.
    Brain,
    Branch,
    /// A suggested task, and the tray's mark.
    Bulb,
    Check,
    ChevronDown,
    ChevronRight,
    Close,
    /// A Git commit node.
    Commit,
    Copy,
    Download,
    /// The rubber duck: WackCode's mark, and the agent at work.
    Duck,
    Erase,
    /// Shows a secret, as at a key field's eye.
    Eye,
    /// Hides a secret again.
    EyeOff,
    File,
    Folder,
    /// Build mode.
    Hammer,
    Key,
    Link,
    Lock,
    Pencil,
    Pin,
    Play,
    Plus,
    Question,
    Search,
    Settings,
    Sidebar,
    /// One of WackCode's own slash commands.
    Slash,
    /// A touch of polish, on a suggestion card.
    Spark,
    Stop,
    Terminal,
    Trash,
}

/// Every icon file, by asset path. One list, so a missing file fails the test below.
const FILES: &[(&str, &[u8])] = &[
    ("icons/alert.svg", include_bytes!("../assets/icons/alert.svg")),
    ("icons/arrow-up.svg", include_bytes!("../assets/icons/arrow-up.svg")),
    ("icons/arrow-left.svg", include_bytes!("../assets/icons/arrow-left.svg")),
    ("icons/arrow-right.svg", include_bytes!("../assets/icons/arrow-right.svg")),
    ("icons/reload.svg", include_bytes!("../assets/icons/reload.svg")),
    ("icons/globe.svg", include_bytes!("../assets/icons/globe.svg")),
    ("icons/cursor.svg", include_bytes!("../assets/icons/cursor.svg")),
    ("icons/phone.svg", include_bytes!("../assets/icons/phone.svg")),
    ("icons/book.svg", include_bytes!("../assets/icons/book.svg")),
    ("icons/bookmark.svg", include_bytes!("../assets/icons/bookmark.svg")),
    ("icons/brain.svg", include_bytes!("../assets/icons/brain.svg")),
    ("icons/branch.svg", include_bytes!("../assets/icons/branch.svg")),
    ("icons/bulb.svg", include_bytes!("../assets/icons/bulb.svg")),
    ("icons/check.svg", include_bytes!("../assets/icons/check.svg")),
    ("icons/chevron-down.svg", include_bytes!("../assets/icons/chevron-down.svg")),
    ("icons/chevron-right.svg", include_bytes!("../assets/icons/chevron-right.svg")),
    ("icons/close.svg", include_bytes!("../assets/icons/close.svg")),
    ("icons/commit.svg", include_bytes!("../assets/icons/commit.svg")),
    ("icons/copy.svg", include_bytes!("../assets/icons/copy.svg")),
    ("icons/download.svg", include_bytes!("../assets/icons/download.svg")),
    ("icons/duck.svg", include_bytes!("../assets/icons/duck.svg")),
    ("icons/erase.svg", include_bytes!("../assets/icons/erase.svg")),
    ("icons/eye.svg", include_bytes!("../assets/icons/eye.svg")),
    ("icons/eye-off.svg", include_bytes!("../assets/icons/eye-off.svg")),
    ("icons/file.svg", include_bytes!("../assets/icons/file.svg")),
    ("icons/folder.svg", include_bytes!("../assets/icons/folder.svg")),
    ("icons/hammer.svg", include_bytes!("../assets/icons/hammer.svg")),
    ("icons/key.svg", include_bytes!("../assets/icons/key.svg")),
    ("icons/link.svg", include_bytes!("../assets/icons/link.svg")),
    ("icons/lock.svg", include_bytes!("../assets/icons/lock.svg")),
    ("icons/pencil.svg", include_bytes!("../assets/icons/pencil.svg")),
    ("icons/pin.svg", include_bytes!("../assets/icons/pin.svg")),
    ("icons/play.svg", include_bytes!("../assets/icons/play.svg")),
    ("icons/plus.svg", include_bytes!("../assets/icons/plus.svg")),
    ("icons/question.svg", include_bytes!("../assets/icons/question.svg")),
    ("icons/search.svg", include_bytes!("../assets/icons/search.svg")),
    ("icons/settings.svg", include_bytes!("../assets/icons/settings.svg")),
    ("icons/sidebar.svg", include_bytes!("../assets/icons/sidebar.svg")),
    ("icons/slash.svg", include_bytes!("../assets/icons/slash.svg")),
    ("icons/spark.svg", include_bytes!("../assets/icons/spark.svg")),
    ("icons/stop.svg", include_bytes!("../assets/icons/stop.svg")),
    ("icons/terminal.svg", include_bytes!("../assets/icons/terminal.svg")),
    ("icons/trash.svg", include_bytes!("../assets/icons/trash.svg")),
];

impl Icon {
    pub fn path(self) -> &'static str {
        match self {
            Icon::Alert => "icons/alert.svg",
            Icon::ArrowUp => "icons/arrow-up.svg",
            Icon::ArrowLeft => "icons/arrow-left.svg",
            Icon::ArrowRight => "icons/arrow-right.svg",
            Icon::Reload => "icons/reload.svg",
            Icon::Globe => "icons/globe.svg",
            Icon::Cursor => "icons/cursor.svg",
            Icon::Phone => "icons/phone.svg",
            Icon::Book => "icons/book.svg",
            Icon::Brain => "icons/brain.svg",
            Icon::Branch => "icons/branch.svg",
            Icon::Bulb => "icons/bulb.svg",
            Icon::Check => "icons/check.svg",
            Icon::ChevronDown => "icons/chevron-down.svg",
            Icon::ChevronRight => "icons/chevron-right.svg",
            Icon::Close => "icons/close.svg",
            Icon::Commit => "icons/commit.svg",
            Icon::Copy => "icons/copy.svg",
            Icon::Download => "icons/download.svg",
            Icon::Bookmark => "icons/bookmark.svg",
            Icon::Duck => "icons/duck.svg",
            Icon::Erase => "icons/erase.svg",
            Icon::Eye => "icons/eye.svg",
            Icon::EyeOff => "icons/eye-off.svg",
            Icon::File => "icons/file.svg",
            Icon::Folder => "icons/folder.svg",
            Icon::Hammer => "icons/hammer.svg",
            Icon::Key => "icons/key.svg",
            Icon::Link => "icons/link.svg",
            Icon::Lock => "icons/lock.svg",
            Icon::Pencil => "icons/pencil.svg",
            Icon::Pin => "icons/pin.svg",
            Icon::Play => "icons/play.svg",
            Icon::Plus => "icons/plus.svg",
            Icon::Question => "icons/question.svg",
            Icon::Search => "icons/search.svg",
            Icon::Settings => "icons/settings.svg",
            Icon::Sidebar => "icons/sidebar.svg",
            Icon::Slash => "icons/slash.svg",
            Icon::Spark => "icons/spark.svg",
            Icon::Stop => "icons/stop.svg",
            Icon::Terminal => "icons/terminal.svg",
            Icon::Trash => "icons/trash.svg",
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
            Icon::ArrowLeft,
            Icon::ArrowRight,
            Icon::Reload,
            Icon::Globe,
            Icon::Cursor,
            Icon::Phone,
            Icon::Bookmark,
            Icon::Brain,
            Icon::Branch,
            Icon::Check,
            Icon::ChevronDown,
            Icon::ChevronRight,
            Icon::Close,
            Icon::Commit,
            Icon::Copy,
            Icon::Download,
            Icon::Duck,
            Icon::Erase,
            Icon::Eye,
            Icon::EyeOff,
            Icon::File,
            Icon::Folder,
            Icon::Hammer,
            Icon::Key,
            Icon::Link,
            Icon::Lock,
            Icon::Pencil,
            Icon::Pin,
            Icon::Play,
            Icon::Plus,
            Icon::Question,
            Icon::Search,
            Icon::Settings,
            Icon::Sidebar,
            Icon::Spark,
            Icon::Stop,
            Icon::Terminal,
            Icon::Trash,
        ] {
            let bytes = Assets.load(icon.path()).unwrap();
            assert!(bytes.is_some_and(|bytes| bytes.starts_with(b"<svg")), "{icon:?}");
        }
    }
}
