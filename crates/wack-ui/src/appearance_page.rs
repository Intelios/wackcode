//! Settings › Appearance: light or dark, the theme colour, the accent, and the backdrop, as two
//! cards of rows (`settings_kit.rs`).
//!
//! Everything applies the moment it is chosen, so the page itself is the preview: the Blur and
//! Dim sliders save as they are dragged. Colours are still offered as paint chips with a hex field
//! beside them; WackCode 2's fixed themes replace both (`docs/redesign.md`, phase 6). The page
//! draws from plain data and dispatches actions, like every page.

use gpui::{Entity, Hsla, ObjectFit, SharedString, div, img, prelude::*, px};
use wack_core::Mode;

use crate::{
    actions::{ChooseAccent, ChooseBackdrop, ChooseBlur, ChooseDim, ChooseFocus, ChooseMode, ChooseTexture, ChooseThemeColour, RemoveBackdrop},
    backdrop::Picture,
    settings_kit::{card, dispatch, head, pill, row, segmented, slider},
    tags::Tag,
    text_field::TextField,
    theme::{self, ACCENT_SWATCHES, DEFAULT_ACCENT, DEFAULT_THEME_COLOUR, THEME_SWATCHES, Theme},
    typography::{Type, Typography},
};

/// The most the picture is dimmed behind a conversation, in percent, and blurred, in points.
/// Text sits on glass over the picture, so even none reads.
pub const MOST_DIM: u8 = 80;
pub const MOST_BLUR: u8 = 40;

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

pub fn page(data: AppearanceData, theme: &Theme) -> impl IntoElement {
    let modes = segmented(
        "mode",
        &[(Mode::System, "Match the system"), (Mode::Light, "Light"), (Mode::Dark, "Dark")],
        data.mode,
        |mode| Box::new(ChooseMode { mode }),
        theme,
    );
    let look = card(Some("Look"), None, None, theme)
        .child(row("Light or dark", Some("Matching the system follows macOS as it changes through the day.")).control(modes).view(theme))
        .child(
            row("Theme colour", Some("Tints every surface and line. A grey gives true greys; a strong colour, a clearly coloured app."))
                .control(colours("theme", THEME_SWATCHES, &data.theme_colour, DEFAULT_THEME_COLOUR, data.theme_field.clone(), true, theme))
                .stacked()
                .view(theme),
        )
        .child(
            row("Accent", Some("Buttons, focus, selection and links. Danger, warnings, success and diffs keep their own colours."))
                .control(colours("accent", ACCENT_SWATCHES, &data.accent, DEFAULT_ACCENT, data.accent_field.clone(), false, theme))
                .stacked()
                .view(theme),
        );

    div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .child(head("Appearance", "One look, in your colours. Every change shows at once.", None, theme))
        .child(look)
        .child(backdrop(&data, theme))
        .when_some(data.notice.clone(), |page, notice| page.child(div().typography(Type::Ui).text_color(theme.danger).child(notice)))
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

fn backdrop(data: &AppearanceData, theme: &Theme) -> gpui::Div {
    let about = "A picture behind the window: shown clear across a new chat, and under frosted glass once the chat begins. The glass keeps text readable on its own; blur and dim set how the picture looks around it.";
    let card = card(Some("Backdrop"), None, None, theme);
    let Some(picture) = data.backdrop.clone() else {
        let choose = pill("choose-backdrop", if data.importing { "Reading the picture…" } else { "Choose a picture…" }, None, false, theme)
            .on_click(dispatch(ChooseBackdrop))
            .tag("choose-backdrop");
        return card.child(row("Picture", Some(about)).control(choose).view(theme));
    };

    // The picture small, with its focus marked where the window will keep it in view.
    let (width, height) = (176., 110.);
    let frame = crate::panes::Rect { x: 0., y: 0., width, height };
    let drawn = crate::backdrop::cover(frame, (picture.width, picture.height), picture.focus);
    let thumbnail = div()
        .flex_none()
        .relative()
        .w(px(width))
        .h(px(height))
        .rounded(px(12.))
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
    let buttons = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(pill("replace-backdrop", if data.importing { "Reading…" } else { "Choose another…" }, None, false, theme).on_click(dispatch(ChooseBackdrop)))
        .child(
            div()
                .id("remove-backdrop")
                .h(px(32.))
                .px(px(12.))
                .rounded_full()
                .flex()
                .items_center()
                .cursor_pointer()
                .typography(Type::UiStrong)
                .text_color(theme.text_dim)
                .hover(|word| word.bg(theme.hover).text_color(theme.danger))
                .on_click(dispatch(RemoveBackdrop))
                .child("Remove")
                .tag("remove-backdrop"),
        );
    let top = div()
        .px(px(18.))
        .pb(px(16.))
        .flex()
        .gap(px(18.))
        .child(thumbnail)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .justify_between()
                .gap(px(12.))
                .child(div().typography(Type::Small).text_color(theme.text_dim).child(about))
                .child(buttons),
        );

    let textures: Vec<(wack_core::Texture, &'static str)> = wack_core::Texture::ALL.iter().map(|texture| (*texture, texture.name())).collect();
    let textures = segmented("texture", &textures, picture.texture, |texture| Box::new(ChooseTexture { texture }), theme);
    let blur = slider("blur-slider", picture.blur.round() as u8, MOST_BLUR, 2, " pt", |blur| Box::new(ChooseBlur { blur }), theme);
    let dim = slider("dim-slider", (picture.dim * 100.).round() as u8, MOST_DIM, 5, "%", |dim| Box::new(ChooseDim { dim }), theme);
    // A 3×3 grid: which part of the picture stays in view when the window crops it.
    let focus = div().flex().flex_col().gap(px(3.)).children((0..3u8).map(|row| {
        div().flex().gap(px(3.)).children((0..3u8).map(move |column| {
            let chosen = (picture.focus.0 * 2.).round() as u8 == column && (picture.focus.1 * 2.).round() as u8 == row;
            div()
                .id(("focus", (row * 3 + column) as usize))
                .size(px(14.))
                .rounded_full()
                .cursor_pointer()
                .when(chosen, |cell| cell.bg(theme.accent))
                .when(!chosen, |cell| cell.border_1().border_color(theme.border_strong).hover(|cell| cell.bg(theme.hover)))
                .on_click(dispatch(ChooseFocus { across: column, down: row }))
        }))
    }));
    card.child(top)
        .child(row("Texture", Some("How the picture is drawn behind a new chat.")).control(textures).view(theme))
        .child(row("Blur", Some("How soft the picture is behind a chat.")).control(blur).view(theme))
        .child(row("Dim", Some("How far the picture fades toward the window behind a chat.")).control(dim).view(theme))
        .child(row("Keep in view", Some("The part of the picture kept when the window crops it.")).control(focus).view(theme))
}
