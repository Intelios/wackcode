//! Settings › Appearance: light or dark, the theme, and the backdrop, as two cards of rows
//! (`settings_kit.rs`).
//!
//! Everything applies the moment it is chosen, so the page itself is the preview: the Blur and
//! Dim sliders save as they are dragged. The picture is placed in the Position editor, a
//! miniature of the window: drag to move it (the whole window follows live, and the place is saved
//! when the button comes up), the slider to zoom in, keeping the middle where it is. A theme is chosen from WackCode 2's ten, never made:
//! each tile is a small window drawn in that theme's own tokens, in the
//! appearance showing now, so it shows what choosing it gives. Pictures are a library shown as a
//! strip, each captioned with the theme it remembers: choosing one
//! brings that theme back. With two or more pictures, Shuffle switches to another picture (and its
//! theme) on open or on demand. The page draws from plain data and dispatches actions, like every page.

use std::cell::Cell;

use gpui::{CursorStyle, DispatchPhase, Hsla, MouseButton, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Point, SharedString, canvas, div, img, prelude::*, px};
use wack_core::{Mode, ThemeName};

use crate::{
    actions::{ChooseBackdrop, ChooseBlur, ChooseDim, ChooseMode, ChooseTexture, ChooseTheme, ChooseZoom, CommitCrop, PreviewCrop, RemovePicture, ResetCrop, ShowPicture, ShuffleBackdrop, ToggleBackdropShuffle, ToggleModelLogos},
    backdrop::Picture,
    icon::Icon,
    settings_kit::{card, dispatch, labelled_slider, pill, row, segmented, slider, staged_head},
    tags::Tag,
    theme::{Theme, ThemeInput},
    tools_page::switch,
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
    /// The picture showing, if any.
    pub backdrop: Option<Picture>,
    /// Every picture in the library, in order.
    pub pictures: Vec<Thumbnail>,
    /// Show a different picture each time WackCode opens.
    pub shuffle: bool,
    /// Show each model provider's logo beside models in the composer and model pickers.
    pub model_logos: bool,
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
        .child(row("Light or dark", Some("Matching the system follows macOS as it changes through the day.")).control(modes).view(theme))
        .child(
            row("Model logos", Some("Show each model provider’s logo beside models in the composer and model pickers."))
                .control(
                    div()
                        .id("model-logos")
                        .cursor_pointer()
                        .on_click(dispatch(ToggleModelLogos))
                        .child(switch(data.model_logos, theme))
                        .tag("model-logos"),
                )
                .view(theme),
        );

    div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .child(staged_head(crate::settings::Section::Appearance, "Appearance", "One look, in the theme you choose. Every change shows at once.", None, theme))
        .child(look)
        .child(backdrop(&data, theme))
        .when_some(data.notice.clone(), |page, notice| page.child(div().typography(Type::Ui).text_color(theme.danger).child(notice)))
}

/// One picture in the library.
#[derive(Clone)]
pub struct Thumbnail {
    pub file: SharedString,
    pub picture: Picture,
    /// The small sharp copy to draw the tile from.
    pub thumb: SharedString,
    pub shown: bool,
    /// The theme it remembers; none until it has been shown.
    pub theme: Option<ThemeName>,
}

/// A tile's small window.
const TILE_WIDTH: f32 = 124.;
const TILE_HEIGHT: f32 = 78.;

/// The ten themes as tiles, the chosen one ringed.
fn themes(chosen: ThemeName, theme: &Theme) -> impl IntoElement {
    div().w_full().flex().flex_wrap().gap(px(12.)).children(ThemeName::ALL.into_iter().map(|name| {
        let look = Theme::derive(ThemeInput { appearance: theme.appearance, theme: name, ground: None });
        let picked = name == chosen;
        tile(("theme", name as usize), picked, name.name(), window(&look), theme)
            .cursor_pointer().on_click(dispatch(ChooseTheme { theme: name }))
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
    let about = "Behind the window: clear across a new chat, under frosted glass once the chat begins. Each picture brings back the theme you last used with it.";
    let card = card(Some("Backdrop"), None, None, theme)
        .child(row("Pictures", Some(about)).control(strip(data, theme)).stacked().first().view(theme));
    let Some(picture) = data.backdrop.clone() else {
        return card;
    };
    let shuffle = (data.pictures.len() > 1).then(|| {
        let shuffle_now = pill("shuffle-now", "Shuffle now", Some(Icon::Shuffle), false, theme)
            .cursor_pointer().on_click(dispatch(ShuffleBackdrop))
            .tag("shuffle-now");
        let toggle = div()
            .id("backdrop-shuffle")
            .cursor_pointer()
            .on_click(dispatch(ToggleBackdropShuffle))
            .child(switch(data.shuffle, theme))
            .tag("backdrop-shuffle");
        row(
            "Shuffle",
            Some("Show a different picture each time you open WackCode. Shuffle now switches to another one straight away."),
        )
        .control(div().flex().items_center().gap(px(12.)).child(shuffle_now).child(toggle))
        .view(theme)
    });
    let textures: Vec<(wack_core::Texture, &'static str)> = wack_core::Texture::ALL.iter().map(|texture| (*texture, texture.name())).collect();
    let textures = segmented("texture", &textures, picture.texture, |texture| Box::new(ChooseTexture { texture }), theme);
    let blur = slider("blur-slider", picture.blur.round() as u8, MOST_BLUR, 2, " pt", |blur| Box::new(ChooseBlur { blur }), theme);
    let dim = slider("dim-slider", (picture.dim * 100.).round() as u8, MOST_DIM, 5, "%", |dim| Box::new(ChooseDim { dim }), theme);
    let thumb = data.pictures.iter().find(|thumbnail| thumbnail.shown).map(|thumbnail| thumbnail.thumb.clone()).unwrap_or_else(|| picture.path.clone());
    let steps = ((picture.zoom * 100. - 100.) / 10.).round().clamp(0., 30.) as u8;
    let zoom = labelled_slider("zoom-slider", steps, 30, 1, format!("{}%", 100 + steps as u16 * 10).into(), |steps| Box::new(ChooseZoom { steps }), theme);
    let centred = steps == 0 && (picture.focus.0 - 0.5).abs() < 0.001 && (picture.focus.1 - 0.5).abs() < 0.001;
    let reset = div()
        .id("reset-crop")
        .flex_none()
        .h(px(28.))
        .px(px(12.))
        .rounded_full()
        .flex()
        .items_center()
        .border_1()
        .border_color(theme.border)
        .typography(Type::Small)
        .text_color(if centred { theme.text_faint } else { theme.text_soft })
        .when(!centred, |button| button.cursor_pointer().hover(|button| button.bg(theme.hover)).on_click(dispatch(ResetCrop)))
        .child("Reset")
        .tag("reset-crop");
    let position = div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(Position { picture: picture.clone(), thumb })
        .child(div().flex().items_center().gap(px(12.)).child(zoom).child(reset));
    card.children(shuffle)
        .child(row("Texture", Some("How the picture is drawn behind a new chat.")).control(textures).view(theme))
        .child(row("Blur", Some("How soft the picture is behind a chat.")).control(blur).view(theme))
        .child(row("Dim", Some("How far the picture fades toward the window behind a chat.")).control(dim).view(theme))
        .child(
            row("Position", Some("Drag the picture to move it, and zoom to fill the window with the part you want. The outlines are the sidebar and the composer."))
                .control(position)
                .stacked()
                .view(theme),
        )
}

/// The position editor's width.
const EDITOR_WIDTH: f32 = 420.;

/// A drag in the position editor: where the press was, and the focus then.
type CropDrag = (Point<Pixels>, (f32, f32));

thread_local! {
    static CROP_DRAG: Cell<Option<CropDrag>> = const { Cell::new(None) };
}

/// A miniature of the window with the backdrop in it, as the window draws it (`backdrop::cover`
/// over the window's own shape, so what lines up here lines up there). Dragging moves the
/// picture: the whole window follows live (`PreviewCrop`) and the place is saved when the button
/// comes up (`CommitCrop`). Outlines of the sidebar and the composer show what will sit over it.
#[derive(IntoElement)]
struct Position {
    picture: Picture,
    thumb: SharedString,
}

impl RenderOnce for Position {
    fn render(self, window: &mut gpui::Window, cx: &mut gpui::App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let viewport = window.viewport_size();
        let (window_width, window_height) = (f32::from(viewport.width).max(1.), f32::from(viewport.height).max(1.));
        let width = EDITOR_WIDTH;
        let height = (width * window_height / window_width).round();
        let scale = width / window_width;
        let frame = crate::panes::Rect { x: 0., y: 0., width, height };
        let size = (self.picture.width, self.picture.height);
        let (focus, zoom) = (self.picture.focus, self.picture.zoom);
        let drawn = crate::backdrop::cover(frame, size, focus, zoom);
        // The guides, to scale: the sidebar down the left, the composer at the stage's foot.
        let sidebar = (f32::from(crate::sidebar::SIDEBAR_WIDTH) + 2. * crate::panes::solve::GAP) * scale;
        let stage = width - sidebar;
        let composer_width = (760f32.min(stage / scale - 48.)).max(0.) * scale;
        let composer_height = 96. * scale;
        let guide = Hsla { a: 0.85, ..theme.text };
        let shade = Hsla { a: 0.22, ..theme.window };
        let follow = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || event.pressed_button != Some(MouseButton::Left) {
                        return;
                    }
                    let Some((start, from)) = CROP_DRAG.with(Cell::get) else { return };
                    let delta = event.position - start;
                    let (across, down) = crate::backdrop::pan(frame, size, from, zoom, (f32::from(delta.x), f32::from(delta.y)));
                    window.dispatch_action(Box::new(PreviewCrop { across, down }), cx);
                });
                window.on_mouse_event(|_: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble && CROP_DRAG.with(|drag| drag.take()).is_some() {
                        window.dispatch_action(Box::new(CommitCrop), cx);
                    }
                });
            },
        );
        div()
            .id("crop-editor")
            .relative()
            .flex_none()
            .w(px(width))
            .h(px(height))
            .rounded(px(12.))
            .overflow_hidden()
            .bg(theme.window)
            .border_1()
            .border_color(theme.border_strong)
            .cursor(CursorStyle::OpenHand)
            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                cx.stop_propagation();
                CROP_DRAG.with(|drag| drag.set(Some((event.position, focus))));
            })
            .child(
                img(std::path::PathBuf::from(self.thumb.as_ref()))
                    .absolute()
                    .left(px(drawn.x))
                    .top(px(drawn.y))
                    .w(px(drawn.width))
                    .h(px(drawn.height))
                    .object_fit(ObjectFit::Fill),
            )
            .child(div().absolute().left_0().top_0().h_full().w(px(sidebar)).bg(shade).border_r_1().border_color(guide))
            .child(
                div()
                    .absolute()
                    .left(px(sidebar + (stage - composer_width) / 2.))
                    .bottom(px(22. * scale))
                    .w(px(composer_width))
                    .h(px(composer_height))
                    .rounded(px(18. * scale))
                    .bg(shade)
                    .border_1()
                    .border_color(guide),
            )
            .child(follow.absolute().left_0().top_0().size_full())
            .tag("crop-editor")
    }
}

/// A picture tile's size: the shape of a laptop's window.
const PICTURE_WIDTH: f32 = 128.;
const PICTURE_HEIGHT: f32 = 82.;

/// No picture, each picture in the library captioned with its theme, then a way to add one.
fn strip(data: &AppearanceData, theme: &Theme) -> impl IntoElement {
    let none_shown = data.backdrop.is_none();
    let none = frame(none_shown, theme).bg(theme.window).flex().items_center().justify_center().child(Icon::Close.view(px(16.), theme.text_faint));
    let none = tile("no-picture", none_shown, "No picture", none, theme).cursor_pointer().on_click(dispatch(ShowPicture { file: None })).tag("no-picture");
    let pictures = data.pictures.iter().enumerate().map(|(index, thumbnail)| {
        let picture = &thumbnail.picture;
        let area = crate::panes::Rect { x: 0., y: 0., width: PICTURE_WIDTH, height: PICTURE_HEIGHT };
        let drawn = crate::backdrop::cover(area, (picture.width, picture.height), picture.focus, picture.zoom);
        let group = SharedString::from(format!("picture-{index}"));
        let file = thumbnail.file.to_string();
        // Shown on hover; a press here removes the picture without showing it first.
        let remove = div()
            .id(("remove-picture", index))
            .absolute()
            .top(px(5.))
            .right(px(5.))
            .size(px(22.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border_strong)
            .invisible()
            .group_hover(group.clone(), |button| button.visible())
            .hover(|button| button.bg(theme.hover))
            .cursor_pointer().on_click({
                let file = file.clone();
                move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(Box::new(RemovePicture { file: file.clone() }), cx);
                }
            })
            .child(Icon::Trash.view(px(12.), theme.danger));
        let image = frame(thumbnail.shown, theme)
            .relative()
            .child(
                img(std::path::PathBuf::from(thumbnail.thumb.as_ref()))
                    .absolute()
                    .left(px(drawn.x))
                    .top(px(drawn.y))
                    .w(px(drawn.width))
                    .h(px(drawn.height))
                    .object_fit(ObjectFit::Fill),
            )
            .child(remove);
        let caption = thumbnail.theme.map_or("No theme yet", ThemeName::name);
        tile(("picture", index), thumbnail.shown, caption, image, theme)
            .group(group)
            .cursor_pointer().on_click(dispatch(ShowPicture { file: Some(file) }))
    });
    let add = frame(false, theme)
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(theme.border_strong)
        .child(Icon::Plus.view(px(18.), theme.text_soft));
    let add = tile("choose-backdrop", false, if data.importing { "Reading…" } else { "Add a picture…" }, add, theme)
        .cursor_pointer().on_click(dispatch(ChooseBackdrop))
        .tag("choose-backdrop");
    div().w_full().flex().flex_wrap().gap(px(12.)).child(none).children(pictures).child(add)
}

/// A tile in the strip: `content` ringed when chosen, its caption under it.
fn tile(id: impl Into<gpui::ElementId>, chosen: bool, caption: &'static str, content: impl IntoElement, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(7.))
        .cursor_pointer()
        .child(
            div()
                .p(px(3.))
                .rounded(px(15.))
                .border_2()
                .border_color(if chosen { theme.text } else { gpui::transparent_black() })
                .when(!chosen, |ring| ring.hover(|ring| ring.border_color(theme.border_strong)))
                .child(content),
        )
        .child(
            div()
                .px(px(5.))
                .typography(if chosen { Type::UiStrong } else { Type::Ui })
                .text_color(if chosen { theme.text } else { theme.text_soft })
                .child(caption),
        )
}

/// The rounded area a tile shows its picture in.
fn frame(chosen: bool, theme: &Theme) -> gpui::Div {
    div()
        .w(px(PICTURE_WIDTH))
        .h(px(PICTURE_HEIGHT))
        .rounded(px(11.))
        .overflow_hidden()
        .when(!chosen, |frame| frame.border_1().border_color(theme.border))
}
