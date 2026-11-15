//! Settings › Appearance: light or dark, the theme, and the backdrop, as two cards of rows
//! (`settings_kit.rs`).
//!
//! Everything applies the moment it is chosen, so the page itself is the preview: the Blur and
//! Dim sliders save as they are dragged. A theme is chosen from WackCode 2's ten, never made
//! (`docs/redesign.md`): each tile is a small window drawn in that theme's own tokens, in the
//! appearance showing now, so it shows what choosing it gives. The page draws from plain data and
//! dispatches actions, like every page.

use gpui::{ObjectFit, SharedString, div, img, prelude::*, px};
use wack_core::{Mode, ThemeName};

use crate::{
    actions::{ChooseBackdrop, ChooseBlur, ChooseDim, ChooseFocus, ChooseMode, ChooseTexture, ChooseTheme, RemoveBackdrop},
    backdrop::Picture,
    settings_kit::{card, dispatch, head, pill, row, segmented, slider},
    tags::Tag,
    theme::{Theme, ThemeInput},
    typography::{Type, Typography},
};

/// The most the picture is dimmed behind a conversation, in percent, and blurred, in points.
/// Text sits on glass over the picture, so even none reads.
pub const MOST_DIM: u8 = 80;
pub const MOST_BLUR: u8 = 40;

#[derive(Clone)]
pub struct AppearanceData {
    pub mode: Mode,
    pub theme: ThemeName,
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
        .child(
            row("Theme", Some("Sets the colour of every surface, and the accent on buttons, focus and links. Danger, warnings, success and diffs keep their own colours."))
                .control(themes(data.theme, theme))
                .stacked()
                .first()
                .view(theme),
        )
        .child(row("Light or dark", Some("Matching the system follows macOS as it changes through the day.")).control(modes).view(theme));

    div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .child(head("Appearance", "One look, in the theme you choose. Every change shows at once.", None, theme))
        .child(look)
        .child(backdrop(&data, theme))
        .when_some(data.notice.clone(), |page, notice| page.child(div().typography(Type::Ui).text_color(theme.danger).child(notice)))
}

/// A tile's small window.
const TILE_WIDTH: f32 = 124.;
const TILE_HEIGHT: f32 = 78.;

/// The ten themes as tiles, the chosen one ringed.
fn themes(chosen: ThemeName, theme: &Theme) -> impl IntoElement {
    div().w_full().flex().flex_wrap().gap(px(12.)).children(ThemeName::ALL.into_iter().map(|name| {
        let look = Theme::derive(ThemeInput { appearance: theme.appearance, theme: name, ground: None });
        let picked = name == chosen;
        div()
            .id(("theme", name as usize))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(7.))
            .cursor_pointer()
            .on_click(dispatch(ChooseTheme { theme: name }))
            .child(
                div()
                    .p(px(3.))
                    .rounded(px(15.))
                    .border_2()
                    .border_color(if picked { theme.text } else { gpui::transparent_black() })
                    .when(!picked, |ring| ring.hover(|ring| ring.border_color(theme.border_strong)))
                    .child(window(&look)),
            )
            .child(
                div()
                    .px(px(5.))
                    .typography(if picked { Type::UiStrong } else { Type::Ui })
                    .text_color(if picked { theme.text } else { theme.text_soft })
                    .child(name.name()),
            )
            .tag(tag(name))
    }))
}

/// A tile's tag, for tests: `theme-` and the theme's id.
fn tag(name: ThemeName) -> &'static str {
    match name {
        ThemeName::WackCode => "theme-wackcode",
        ThemeName::Midnight => "theme-midnight",
        ThemeName::Lagoon => "theme-lagoon",
        ThemeName::Verdant => "theme-verdant",
        ThemeName::Grape => "theme-grape",
        ThemeName::Nebula => "theme-nebula",
        ThemeName::Rose => "theme-rose",
        ThemeName::Solar => "theme-solar",
        ThemeName::Ember => "theme-ember",
        ThemeName::Mono => "theme-mono",
    }
}

/// A window in `look`: the sidebar pane, two lines of a conversation, and the composer with its
/// send button in the accent's fill.
fn window(look: &Theme) -> impl IntoElement {
    let line = |width: f32, colour| div().h(px(4.)).w(px(width)).rounded_full().bg(colour);
    div()
        .w(px(TILE_WIDTH))
        .h(px(TILE_HEIGHT))
        .p(px(5.))
        .flex()
        .gap(px(5.))
        .rounded(px(11.))
        .bg(look.window)
        .border_1()
        .border_color(look.border)
        .child(
            div()
                .w(px(30.))
                .h_full()
                .p(px(5.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .rounded(px(7.))
                .bg(look.sidebar)
                .child(div().h(px(6.)).w_full().rounded_full().bg(look.accent_wash).border_1().border_color(look.accent_line))
                .child(line(16., look.text_faint))
                .child(line(12., look.border_strong)),
        )
        .child(
            div()
                .flex_1()
                .h_full()
                .px(px(6.))
                .pt(px(9.))
                .pb(px(4.))
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(div().flex().justify_end().child(line(34., look.text_soft)))
                .child(line(52., look.text_dim))
                .child(line(40., look.border_strong))
                .child(div().flex_1())
                .child(
                    div()
                        .h(px(16.))
                        .pl(px(6.))
                        .pr(px(3.))
                        .flex()
                        .items_center()
                        .justify_end()
                        .rounded_full()
                        .bg(look.surface)
                        .border_1()
                        .border_color(look.border_strong)
                        .child(div().size(px(10.)).rounded_full().bg(look.accent_fill())),
                ),
        )
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
