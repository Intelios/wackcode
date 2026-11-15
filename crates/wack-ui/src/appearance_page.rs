//! Settings › Appearance: light or dark, the theme colour, the accent, and the backdrop.
//!
//! Everything applies the moment it is chosen, so the page itself is the preview. Colours are
//! offered as a short row of paint chips with a hex field beside them for anything else; the
//! chips live in `theme.rs` with every other colour. The page draws from plain data and
//! dispatches actions, like every page.

use gpui::{Entity, Hsla, ObjectFit, SharedString, div, img, prelude::*, px};
use wack_core::Mode;

use crate::{
    actions::{ChooseAccent, ChooseBackdrop, ChooseBlur, ChooseDim, ChooseFocus, ChooseMode, ChooseTexture, ChooseThemeColour, RemoveBackdrop},
    backdrop::Picture,
    tags::Tag,
    text_field::TextField,
    theme::{self, ACCENT_SWATCHES, DEFAULT_ACCENT, DEFAULT_THEME_COLOUR, THEME_SWATCHES, Theme},
    typography::{Type, Typography},
};

/// How much the picture is dimmed behind a conversation, in percent, and the names offered.
/// The picture's dims and blurs on offer. Text sits on glass over the picture, so even none reads.
pub const DIMS: [(u8, &str); 5] = [(0, "None"), (20, "Light"), (40, "Medium"), (60, "Strong"), (80, "Most")];
pub const BLURS: [(u8, &str); 5] = [(0, "None"), (8, "Light"), (16, "Soft"), (28, "Heavy"), (40, "Most")];

#[derive(Clone)]
pub struct AppearanceData {
    pub mode: Mode,
    /// The colours in use, as `#rrggbb`.
    pub theme_colour: String,
    pub accent: String,
    pub theme_field: Entity<TextField>,
    pub accent_field: Entity<TextField>,
    pub backdrop: Option<Picture>,
    /// A picture is being read.
    pub importing: bool,
    pub notice: Option<SharedString>,
}

fn dispatch(action: impl gpui::Action) -> impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static {
    let action = action.boxed_clone();
    move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)
}

/// A choice drawn as a word: the one chosen in strong type over a short accent rule.
pub fn word(id: impl Into<SharedString>, label: &'static str, chosen: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(gpui::ElementId::Name(id.into()))
        .relative()
        .pb(px(5.))
        .typography(if chosen { Type::UiStrong } else { Type::Ui })
        .text_color(if chosen { theme.text } else { theme.text_dim })
        .hover(|word| word.text_color(theme.text))
        .child(label)
        .when(chosen, |word| word.child(div().absolute().bottom_0().left_0().right_0().h(px(2.)).rounded(px(1.)).bg(theme.accent_line)))
}

pub fn page(data: AppearanceData, theme: &Theme) -> impl IntoElement {
    let section = |title: &'static str, note: &'static str, control: gpui::AnyElement| {
        div()
            .pt(px(30.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().typography(Type::Heading).text_color(theme.text).child(title))
            .child(div().max_w(px(520.)).typography(Type::Ui).text_color(theme.text_dim).child(note))
            .child(div().pt(px(12.)).child(control))
    };

    let modes = div()
        .flex()
        .gap(px(22.))
        .children([(Mode::System, "Match the system"), (Mode::Light, "Light"), (Mode::Dark, "Dark")].map(|(mode, label)| {
            word(format!("mode-{label}"), label, data.mode == mode, theme).on_click(dispatch(ChooseMode { mode }))
        }))
        .into_any_element();

    div()
        .flex()
        .flex_col()
        .child(div().typography(Type::Title).text_color(theme.text).child("Appearance"))
        .child(div().pt(px(8.)).max_w(px(560.)).typography(Type::Body).text_color(theme.text_soft).child(
            "One look, in your colours. Every change shows at once.",
        ))
        .child(section("Light or dark", "Matching the system follows macOS as it changes through the day.", modes))
        .child(section(
            "Theme colour",
            "Tints every surface and line. A grey gives true greys; a strong colour, a clearly coloured app.",
            colours("theme", THEME_SWATCHES, &data.theme_colour, DEFAULT_THEME_COLOUR, data.theme_field.clone(), true, theme),
        ))
        .child(section(
            "Accent",
            "Buttons, focus, selection and links. Danger, warnings, success and diffs keep their own colours.",
            colours("accent", ACCENT_SWATCHES, &data.accent, DEFAULT_ACCENT, data.accent_field.clone(), false, theme),
        ))
        .child(section(
            "Backdrop",
            "A picture behind the window: clear across the top of a new chat, and under frosted glass everywhere else. The glass keeps text readable on its own; blur and dim set how the picture looks around it.",
            backdrop(&data, theme),
        ))
        .when_some(data.notice.clone(), |page, notice| {
            page.child(div().pt(px(18.)).typography(Type::Ui).text_color(theme.danger).child(notice))
        })
}

/// A row of paint chips, then the hex field and a way back to the default.
fn colours(
    kind: &'static str,
    swatches: &'static [&'static str],
    current: &str,
    default: &'static str,
    field: Entity<TextField>,
    theme_colour: bool,
    theme: &Theme,
) -> gpui::AnyElement {
    let action = move |hex: Option<String>| -> Box<dyn gpui::Action> {
        if theme_colour { Box::new(ChooseThemeColour { hex }) } else { Box::new(ChooseAccent { hex }) }
    };
    let chips = div().flex().gap(px(6.)).children(swatches.iter().enumerate().map(|(index, hex)| {
        let chosen = hex.eq_ignore_ascii_case(current);
        let choose = action(Some(hex.to_string()));
        div()
            .id((if theme_colour { "theme-chip" } else { "accent-chip" }, index))
            .size(px(26.))
            .p(px(3.))
            .rounded(px(8.))
            .border_2()
            .border_color(if chosen { theme.text } else { Hsla::transparent_black() })
            .hover(|chip| chip.border_color(if chosen { theme.text } else { theme.border_strong }))
            .on_click(move |_, window, cx| window.dispatch_action(choose.boxed_clone(), cx))
            // A hairline keeps a pale chip visible on a light ground.
            .child(div().size_full().rounded(px(4.)).bg(theme::swatch(hex)).border_1().border_color(theme.border))
    }));
    let reset = action(None);
    let is_default = current.eq_ignore_ascii_case(default);
    div()
        .flex()
        .items_center()
        .gap(px(18.))
        .child(chips)
        .child(
            div()
                .flex_none()
                .w(px(108.))
                .px(px(8.))
                .py(px(4.))
                .flex()
                .items_center()
                .gap(px(7.))
                .rounded(px(7.))
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(div().flex_none().size(px(12.)).rounded(px(3.)).bg(theme::swatch(current)))
                .child(div().flex_1().min_w_0().child(field)),
        )
        .when(!is_default, |row| {
            row.child(
                div()
                    .id(SharedString::from(format!("{kind}-default")))
                    .typography(Type::Small)
                    .text_color(theme.text_dim)
                    .hover(|word| word.text_color(theme.text))
                    .on_click(move |_, window, cx| window.dispatch_action(reset.boxed_clone(), cx))
                    .child("Default"),
            )
        })
        .into_any_element()
}

fn backdrop(data: &AppearanceData, theme: &Theme) -> gpui::AnyElement {
    let button = |id: &'static str, label: &'static str, strong: bool| {
        div()
            .id(id)
            .h(px(30.))
            .px(px(12.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .when(strong, |button| button.bg(theme.accent).hover(|button| button.bg(theme.accent_hover)))
            .when(!strong, |button| button.border_1().border_color(theme.border_strong).hover(|button| button.bg(theme.hover)))
            .child(div().typography(Type::UiStrong).text_color(if strong { theme.on_accent } else { theme.text }).child(label))
    };
    let Some(picture) = data.backdrop.clone() else {
        return div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(
                button("choose-backdrop", if data.importing { "Reading the picture…" } else { "Choose a picture…" }, true)
                    .on_click(dispatch(ChooseBackdrop))
                    .tag("choose-backdrop"),
            )
            .child(div().typography(Type::Small).text_color(theme.text_faint).child("PNG, JPEG, WebP or GIF"))
            .into_any_element();
    };

    // The picture small, with its focus marked where the window will keep it in view.
    let (width, height) = (220., 138.);
    let frame = crate::panes::Rect { x: 0., y: 0., width, height };
    let drawn = crate::backdrop::cover(frame, (picture.width, picture.height), picture.focus);
    let thumbnail = div()
        .flex_none()
        .relative()
        .w(px(width))
        .h(px(height))
        .rounded(px(10.))
        .overflow_hidden()
        .border_1()
        .border_color(theme.border_strong)
        .child(
            img(std::path::PathBuf::from(picture.path.as_ref()))
                .absolute()
                .left(px(drawn.x))
                .top(px(drawn.y))
                .w(px(drawn.width))
                .h(px(drawn.height))
                .object_fit(ObjectFit::Fill),
        );

    let dims = div().flex().gap(px(16.)).children(DIMS.map(|(dim, label)| {
        let chosen = (picture.dim * 100.).round() as u8 == dim;
        word(format!("dim-{label}"), label, chosen, theme).on_click(dispatch(ChooseDim { dim }))
    }));
    let blurs = div().flex().gap(px(16.)).children(BLURS.map(|(blur, label)| {
        let chosen = picture.blur.round() as u8 == blur;
        word(format!("blur-{label}"), label, chosen, theme).on_click(dispatch(ChooseBlur { blur }))
    }));
    let textures = div().flex().flex_wrap().gap(px(16.)).children(wack_core::Texture::ALL.map(|texture| {
        word(format!("texture-{}", texture.name()), texture.name(), picture.texture == texture, theme)
            .on_click(dispatch(ChooseTexture { texture }))
    }));
    // A 3×3 grid: which part of the picture stays in view when the window crops it.
    let focus = div().flex().flex_col().gap(px(3.)).children((0..3u8).map(|row| {
        div().flex().gap(px(3.)).children((0..3u8).map(move |column| {
            let chosen = (picture.focus.0 * 2.).round() as u8 == column && (picture.focus.1 * 2.).round() as u8 == row;
            div()
                .id(("focus", (row * 3 + column) as usize))
                .size(px(14.))
                .rounded(px(3.))
                .when(chosen, |cell| cell.bg(theme.accent))
                .when(!chosen, |cell| cell.border_1().border_color(theme.border_strong).hover(|cell| cell.bg(theme.hover)))
                .on_click(dispatch(ChooseFocus { across: column, down: row }))
        }))
    }));
    let label = |text: &'static str| div().w(px(64.)).flex_none().typography(Type::Ui).text_color(theme.text_soft).child(text);
    div()
        .flex()
        .gap(px(24.))
        .child(thumbnail)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .child(div().flex().items_center().child(label("Texture")).child(textures))
                .child(div().flex().items_center().child(label("Blur")).child(blurs))
                .child(div().flex().items_center().child(label("Dim")).child(dims))
                .child(div().flex().items_start().child(label("Keep")).child(focus))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            button("replace-backdrop", if data.importing { "Reading…" } else { "Choose another…" }, false)
                                .on_click(dispatch(ChooseBackdrop)),
                        )
                        .child(
                            div()
                                .id("remove-backdrop")
                                .px(px(6.))
                                .typography(Type::Ui)
                                .text_color(theme.text_dim)
                                .hover(|word| word.text_color(theme.danger))
                                .on_click(dispatch(RemoveBackdrop))
                                .child("Remove")
                                .tag("remove-backdrop"),
                        ),
                ),
        )
        .into_any_element()
}
