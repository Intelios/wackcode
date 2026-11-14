//! The type scale: every text style a view may use.
//!
//! Three system faces, no bundled fonts:
//!
//! - **New York** (the system serif) for display: screen titles and the user's own words in the
//!   transcript. It is what makes a page read as WackCode at a glance.
//! - **SF Pro** (the system UI font) for everything you operate: rows, controls, running text.
//! - **SF Mono** for paths, commands and code.
//!
//! The serif and mono faces are hidden system families: CoreText resolves the dotted names below,
//! and the plain names ("New York", "SF Mono") are not installed and silently fall back.
//!
//! Views pick a [`Type`] rather than setting sizes, so the hierarchy stays consistent.

use gpui::{FontWeight, Styled, px};

pub const UI_FONT: &str = ".SystemUIFont";
pub const DISPLAY_FONT: &str = ".AppleSystemUIFontSerif";
pub const MONO_FONT: &str = ".AppleSystemUIFontMonospaced";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    /// The one big line on a screen: the project on the new-chat screen.
    Display,
    /// A screen or section title.
    Title,
    /// The user's message in the transcript.
    Prompt,
    /// Section names in lists: "Pinned", "Projects".
    Label,
    /// Headings inside a reply.
    Heading,
    /// Running text in the transcript.
    Body,
    /// Rows, buttons, fields.
    Ui,
    /// A row's name, a button that matters.
    UiStrong,
    /// Metadata: times, counts, hints.
    Small,
    /// Paths and commands in rows.
    Mono,
    /// Code blocks and diffs.
    Code,
}

pub struct TypeStyle {
    pub family: &'static str,
    pub size: f32,
    pub line_height: f32,
    pub weight: FontWeight,
    pub italic: bool,
}

impl Type {
    pub fn style(self) -> TypeStyle {
        let style = |family, size, line_height, weight| TypeStyle {
            family,
            size,
            line_height,
            weight,
            italic: false,
        };
        match self {
            Type::Display => style(DISPLAY_FONT, 44., 50., FontWeight::MEDIUM),
            Type::Title => style(DISPLAY_FONT, 24., 30., FontWeight::MEDIUM),
            Type::Prompt => style(DISPLAY_FONT, 18., 27., FontWeight::NORMAL),
            Type::Label => TypeStyle {
                italic: true,
                ..style(DISPLAY_FONT, 13.5, 18., FontWeight::NORMAL)
            },
            Type::Heading => style(UI_FONT, 15., 22., FontWeight::SEMIBOLD),
            Type::Body => style(UI_FONT, 14., 22., FontWeight::NORMAL),
            Type::Ui => style(UI_FONT, 13., 18., FontWeight::NORMAL),
            Type::UiStrong => style(UI_FONT, 13., 18., FontWeight::MEDIUM),
            Type::Small => style(UI_FONT, 11.5, 15., FontWeight::NORMAL),
            Type::Mono => style(MONO_FONT, 12., 18., FontWeight::NORMAL),
            Type::Code => style(MONO_FONT, 12., 19., FontWeight::NORMAL),
        }
    }
}

/// `.typography(Type::Body)` on any styled element.
pub trait Typography: Styled + Sized {
    fn typography(self, kind: Type) -> Self {
        let style = kind.style();
        let element = self
            .font_family(style.family)
            .text_size(px(style.size))
            .line_height(px(style.line_height))
            .font_weight(style.weight);
        if style.italic { element.italic() } else { element }
    }
}

impl<T: Styled> Typography for T {}
