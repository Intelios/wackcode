//! Pictures in the transcript: the user's attachments on their bubble, a tool's under its step,
//! and the ones a reply names (`![what](img:3f2a9c01b4d7)`) under the reply; and the lightbox a
//! click opens.
//!
//! A [`Picture`] is plain data (the base64 the model was sent, which every message keeps), so the
//! transcript's items stay cheap to build. Decoding happens once per picture, off the main
//! thread, into a shared cache ([`Decoded`]): a strip draws a quiet placeholder until its
//! thumbnail is ready, then the window redraws. The cache keeps the most recent pictures only.

use std::{
    collections::{HashMap, VecDeque},
    hash::{Hash as _, Hasher as _},
    sync::Arc,
};

use gpui::{App, Global, ObjectFit, RenderImage, SharedString, div, img, prelude::*, px};
use wack_core::llm::{Block, Image};

use crate::{
    actions::OpenPicture,
    tags::Tag,
    theme::Theme,
    tooltip::tip,
    typography::{Type, Typography},
};

/// A strip's thumbnails: their height, and how many show before "+N".
const STRIP_HEIGHT: f32 = 72.;
const STRIP_MOST: usize = 6;
/// A thumbnail's long edge in pixels (drawn at a third of it, sharp on Retina).
const THUMB_EDGE: u32 = 288;
/// How many decoded pictures the cache keeps, of each size.
const KEPT: usize = 96;

#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    /// A hash of the data: the cache's key, and the element ids'.
    pub key: u64,
    /// The saved original's id, which a reply names it by.
    pub id: Option<String>,
    pub label: Option<SharedString>,
    pub data: Arc<String>,
}

impl Picture {
    pub fn new(data: &str, label: Option<&str>, id: Option<&str>) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        data.hash(&mut hasher);
        Self { key: hasher.finish(), id: id.map(str::to_string), label: label.map(|label| label.to_string().into()), data: Arc::new(data.to_string()) }
    }

    pub fn of_image(image: &Image) -> Self {
        Self::new(&image.data, image.label.as_deref(), image.id.as_deref())
    }

    /// The user's attached pictures in a message.
    pub fn of_message(content: &[Block]) -> Vec<Self> {
        content
            .iter()
            .filter_map(|block| match block {
                Block::Image { data, label, id, .. } => Some(Self::new(data, label.as_deref(), id.as_deref())),
                _ => None,
            })
            .collect()
    }

    /// Whether `reference` (`img:` and the start of an id) names this picture.
    pub fn answers_to(&self, reference: &str) -> bool {
        let prefix = reference.trim_start_matches("img:");
        prefix.len() >= 6 && self.id.as_deref().is_some_and(|id| id.starts_with(prefix))
    }
}

/// Decoded pictures, thumbnails and full size apart. `None` marks one being decoded, or one that
/// could not be.
#[derive(Default)]
struct Decoded {
    thumbs: HashMap<u64, Option<Arc<RenderImage>>>,
    full: HashMap<u64, Option<Arc<RenderImage>>>,
    order: VecDeque<(u64, bool)>,
}

impl Global for Decoded {}

/// The picture's thumbnail, or `None` while it is decoded (the window redraws when it is).
pub fn thumb(picture: &Picture, cx: &mut App) -> Option<Arc<RenderImage>> {
    decoded(picture, false, cx)
}

/// The picture at the size the model was sent it.
pub fn full(picture: &Picture, cx: &mut App) -> Option<Arc<RenderImage>> {
    decoded(picture, true, cx)
}

fn decoded(picture: &Picture, full: bool, cx: &mut App) -> Option<Arc<RenderImage>> {
    let cache = cx.default_global::<Decoded>();
    let map = if full { &cache.full } else { &cache.thumbs };
    if let Some(found) = map.get(&picture.key) {
        return found.clone();
    }
    let map = if full { &mut cache.full } else { &mut cache.thumbs };
    map.insert(picture.key, None);
    cache.order.push_back((picture.key, full));
    while cache.order.len() > KEPT * 2 {
        if let Some((key, was_full)) = cache.order.pop_front() {
            if was_full { cache.full.remove(&key) } else { cache.thumbs.remove(&key) };
        }
    }
    let (key, data) = (picture.key, picture.data.clone());
    let decoding = cx.background_spawn(async move {
        let bytes = wack_core::images::decode(&data).ok()?;
        let picture = image::load_from_memory(&bytes).ok()?;
        Some(Arc::new(crate::attachments::render_image(&picture, if full { u32::MAX } else { THUMB_EDGE })))
    });
    cx.spawn(async move |cx| {
        let made = decoding.await;
        cx.update(|cx| {
            let cache = cx.default_global::<Decoded>();
            let map = if full { &mut cache.full } else { &mut cache.thumbs };
            if map.contains_key(&key) {
                map.insert(key, made);
            }
            cx.refresh_windows();
        });
    })
    .detach();
    None
}

/// A row of thumbnails, each opening the lightbox. `align_end` lines them up on the right, under
/// the user's bubble.
#[derive(IntoElement)]
pub struct Strip {
    pub pictures: Vec<Picture>,
    pub key: SharedString,
    pub align_end: bool,
}

impl RenderOnce for Strip {
    fn render(self, _: &mut gpui::Window, cx: &mut App) -> impl IntoElement {
        strip(&self.pictures, &self.key, self.align_end, cx)
    }
}

fn strip(pictures: &[Picture], key: &str, align_end: bool, cx: &mut App) -> gpui::AnyElement {
    let theme = Theme::get(cx).clone();
    let shown: Vec<_> = pictures.iter().take(STRIP_MOST).map(|picture| (picture.clone(), thumb(picture, cx))).collect();
    let more = pictures.len().saturating_sub(STRIP_MOST);
    let all: Arc<Vec<Picture>> = Arc::new(pictures.to_vec());
    div()
        .flex()
        .flex_wrap()
        .gap(px(6.))
        .when(align_end, |row| row.justify_end())
        .children(shown.into_iter().enumerate().map(|(index, (picture, decoded))| {
            let label = picture.label.clone().unwrap_or_else(|| "Picture".into());
            let all = all.clone();
            let frame = div()
                .id(SharedString::from(format!("{key}-picture-{index}")))
                .flex_none()
                .h(px(STRIP_HEIGHT))
                .rounded(px(10.))
                .overflow_hidden()
                .border_1()
                .border_color(theme.border)
                .bg(theme.raised)
                .cursor_pointer()
                .tooltip(tip(label))
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(OpenPicture { pictures: all.clone(), index }), cx);
                });
            match decoded {
                Some(decoded) => {
                    let size = decoded.size(0);
                    let width = STRIP_HEIGHT * size.width.0 as f32 / size.height.0.max(1) as f32;
                    frame.w(px(width.clamp(STRIP_HEIGHT * 0.5, STRIP_HEIGHT * 2.2))).child(img(decoded).size_full().rounded(px(10.)).object_fit(ObjectFit::Cover))
                }
                None => frame.w(px(STRIP_HEIGHT * 1.4)),
            }
        }))
        .when(more > 0, |row| {
            let all = all.clone();
            row.child(
                div()
                    .id(SharedString::from(format!("{key}-pictures-more")))
                    .flex_none()
                    .h(px(STRIP_HEIGHT))
                    .px(px(14.))
                    .rounded(px(10.))
                    .flex()
                    .items_center()
                    .border_1()
                    .border_color(theme.border)
                    .cursor_pointer()
                    .hover(|more| more.bg(theme.hover))
                    .typography(Type::Small)
                    .text_color(theme.text_dim)
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(OpenPicture { pictures: all.clone(), index: STRIP_MOST }), cx);
                    })
                    .child(format!("+{more}")),
            )
        })
        .tag("pictures")
        .into_any_element()
}

/// The lightbox: which pictures, which one is showing, and whether at its own size.
#[derive(Clone, Debug, PartialEq)]
pub struct Lightbox {
    pub pictures: Arc<Vec<Picture>>,
    pub index: usize,
    /// At the picture's own size, scrolling, instead of fitted to the window.
    pub zoomed: bool,
}

impl Lightbox {
    pub fn new(pictures: Arc<Vec<Picture>>, index: usize) -> Self {
        let index = index.min(pictures.len().saturating_sub(1));
        Self { pictures, index, zoomed: false }
    }

    /// Moves `by` pictures, stopping at either end.
    pub fn step(&mut self, by: i32) {
        let last = self.pictures.len().saturating_sub(1) as i64;
        self.index = (self.index as i64 + i64::from(by)).clamp(0, last) as usize;
        self.zoomed = false;
    }
}

/// The lightbox over the whole window: the picture fitted inside a margin (or at its own size,
/// scrolling), its name and place above it, round buttons to step and close. A click outside
/// the picture closes it; a click on it zooms. It draws in GPUI's overlay plane, above browser
/// pages, and arrives with the app's enter motion (still under reduced motion).
#[derive(IntoElement)]
pub struct LightboxView {
    pub lightbox: Lightbox,
}

/// The margin a fitted picture keeps from the window's edges.
const LIGHTBOX_MARGIN: f32 = 72.;

impl RenderOnce for LightboxView {
    fn render(self, window: &mut gpui::Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let Lightbox { pictures, index, zoomed } = self.lightbox;
        let Some(picture) = pictures.get(index).cloned() else {
            return div().into_any_element();
        };
        let viewport = window.viewport_size();
        let shown = full(&picture, cx).or_else(|| thumb(&picture, cx));
        let round = |id: &'static str, icon: crate::icon::Icon, theme: &Theme| {
            div()
                .id(id)
                .flex_none()
                .size(px(34.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.raised)
                .border_1()
                .border_color(theme.border_strong)
                .cursor_pointer()
                .hover(|button| button.bg(theme.hover))
                .child(icon.view(px(14.), theme.text))
        };
        let picture_box = match shown {
            Some(decoded) => {
                let size = decoded.size(0);
                let (width, height) = (size.width.0.max(1) as f32, size.height.0.max(1) as f32);
                // Pixels at the scale the window draws them: a Retina window shows a picture at
                // half its pixel size, as Preview does.
                let scale = window.scale_factor().max(1.);
                let (own_width, own_height) = (width / scale, height / scale);
                let fit = ((f32::from(viewport.width) - LIGHTBOX_MARGIN * 2.) / own_width).min((f32::from(viewport.height) - LIGHTBOX_MARGIN * 2.5) / own_height).min(1.);
                let (draw_width, draw_height) = if zoomed { (own_width, own_height) } else { (own_width * fit, own_height * fit) };
                let image = div()
                    .id("lightbox-picture")
                    .flex_none()
                    .w(px(draw_width))
                    .h(px(draw_height))
                    .rounded(px(if zoomed { 0. } else { 12. }))
                    .overflow_hidden()
                    .cursor(if zoomed || fit < 1. { gpui::CursorStyle::PointingHand } else { gpui::CursorStyle::Arrow })
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::actions::ZoomPicture), cx))
                    .child(img(decoded).size_full().rounded(px(if zoomed { 0. } else { 12. })).object_fit(ObjectFit::Contain));
                if zoomed {
                    div()
                        .id("lightbox-scroll")
                        .max_w(px(f32::from(viewport.width) - LIGHTBOX_MARGIN))
                        .max_h(px(f32::from(viewport.height) - LIGHTBOX_MARGIN * 1.5))
                        .overflow_scroll()
                        .child(image)
                        .into_any_element()
                } else {
                    image.into_any_element()
                }
            }
            None => div().size(px(240.)).rounded(px(12.)).bg(theme.raised).into_any_element(),
        };
        let label = picture.label.clone().unwrap_or_else(|| "Picture".into());
        let place = (pictures.len() > 1).then(|| format!("{} of {}", index + 1, pictures.len()));
        let several = pictures.len() > 1;
        let scrim = gpui::Hsla { a: 0.94, ..theme.window };
        gpui::deferred(
            div()
                .id("lightbox")
                .absolute()
                .left_0()
                .top_0()
                .size_full()
                .bg(scrim)
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(14.))
                .on_mouse_down(gpui::MouseButton::Left, |_, window, cx| window.dispatch_action(Box::new(crate::actions::ClosePicture), cx))
                .child(crate::motion::enter(gpui::ElementId::Name(format!("lightbox-{}", picture.key).into()), div().child(picture_box)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .when(several, |bar| {
                            bar.child(
                                round("lightbox-previous", crate::icon::Icon::ArrowLeft, &theme)
                                    .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::actions::StepPicture { by: -1 }), cx)),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .max_w(px(480.))
                                .child(div().max_w(px(480.)).truncate().typography(Type::UiStrong).text_color(theme.text).child(label))
                                .child(div().typography(Type::Small).text_color(theme.text_dim).child(match place {
                                    Some(place) => format!("{place} · click to {} · esc to close", if zoomed { "fit" } else { "zoom" }),
                                    None => format!("click to {} · esc to close", if zoomed { "fit" } else { "zoom" }),
                                })),
                        )
                        .when(several, |bar| {
                            bar.child(
                                round("lightbox-next", crate::icon::Icon::ArrowRight, &theme)
                                    .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::actions::StepPicture { by: 1 }), cx)),
                            )
                        })
                        .child(
                            round("lightbox-close", crate::icon::Icon::Close, &theme)
                                .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::actions::ClosePicture), cx)),
                        ),
                )
                .tag("lightbox"),
        )
        .with_priority(20)
        .into_any_element()
    }
}

/// A quiet line for a reply's picture reference nothing in the conversation answers to.
pub fn missing(reference: &str, theme: &Theme) -> impl IntoElement {
    div().typography(Type::Small).text_color(theme.text_faint).child(format!("Picture {reference} isn't in this chat."))
}

/// The picture references (`img:…`) a reply's Markdown shows as pictures, in order, each once.
pub fn referenced(blocks: &[crate::markdown::Block]) -> Vec<String> {
    fn walk(blocks: &[crate::markdown::Block], out: &mut Vec<String>) {
        use crate::markdown::Block;
        for block in blocks {
            let spans: Vec<&crate::markdown::Span> = match block {
                Block::Paragraph(spans) | Block::Heading(_, spans) => spans.iter().collect(),
                Block::Table { header, rows } => header.iter().flatten().chain(rows.iter().flatten().flatten()).collect(),
                Block::List { items, .. } => {
                    for item in items {
                        walk(&item.blocks, out);
                    }
                    Vec::new()
                }
                Block::Quote(inner) => {
                    walk(inner, out);
                    Vec::new()
                }
                _ => Vec::new(),
            };
            for span in spans {
                if let Some(reference) = &span.picture
                    && !out.contains(reference)
                {
                    out.push(reference.clone());
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(blocks, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_answers_to_its_reference_only() {
        let picture = Picture::new("AAAA", Some("shot"), Some("3f2a9c01b4d7e5f6"));
        assert!(picture.answers_to("img:3f2a9c01b4d7"));
        assert!(!picture.answers_to("img:3f2"));
        assert!(!picture.answers_to("img:000000000000"));
        assert_eq!(picture.key, Picture::new("AAAA", None, None).key, "the key follows the data");
    }

    #[test]
    fn the_lightbox_steps_within_its_pictures() {
        let pictures = Arc::new(vec![Picture::new("A", None, None), Picture::new("B", None, None)]);
        let mut lightbox = Lightbox::new(pictures, 9);
        assert_eq!(lightbox.index, 1);
        lightbox.zoomed = true;
        lightbox.step(1);
        assert_eq!((lightbox.index, lightbox.zoomed), (1, false));
        lightbox.step(-5);
        assert_eq!(lightbox.index, 0);
    }

    #[test]
    fn a_reply_names_its_pictures_once_in_order() {
        let blocks = crate::markdown::parse("Before ![the page](img:aaaaaaaaaaaa) and\n\n- ![x](img:bbbbbbbbbbbb)\n\n![again](img:aaaaaaaaaaaa) ![web](https://example.com/a.png)");
        assert_eq!(referenced(&blocks), ["img:aaaaaaaaaaaa", "img:bbbbbbbbbbbb"]);
    }
}
