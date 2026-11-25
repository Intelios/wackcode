//! Copy buttons: a pill that puts words on the clipboard and, for a beat after, turns into a check
//! that says so, as version 2's did.
//!
//! Which button was pressed last, and when, is a global ([`Copied`]): the views are drawn from
//! plain data on every frame and keep nothing themselves, and only one thing can have just been
//! copied. A timer redraws the windows once the beat is over, so the check goes back to Copy
//! without anything else asking for a frame.

use std::time::{Duration, Instant};

use gpui::{App, ClipboardItem, ElementId, Global, IntoElement, RenderOnce, SharedString, Window, div, prelude::*, px};

use crate::{
    icon::Icon,
    tags::Tag,
    theme::Theme,
    typography::{Type, Typography},
};

/// How long a button says Copied.
const COPIED_FOR: Duration = Duration::from_millis(1400);

/// The button pressed last, when, and how many presses there have been (so each press arrives
/// afresh).
#[derive(Default)]
struct Copied {
    key: Option<SharedString>,
    at: Option<Instant>,
    presses: u64,
}

impl Global for Copied {}

/// A pill that copies `text`. `key` names it among the window's copy buttons.
#[derive(IntoElement)]
pub struct CopyButton {
    key: SharedString,
    text: SharedString,
    label: SharedString,
    tag: &'static str,
}

impl CopyButton {
    pub fn new(key: impl Into<SharedString>, text: impl Into<SharedString>, label: impl Into<SharedString>, tag: &'static str) -> Self {
        Self { key: key.into(), text: text.into(), label: label.into(), tag }
    }
}

impl RenderOnce for CopyButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let still = cx.reduce_motion();
        let just = cx
            .try_global::<Copied>()
            .filter(|copied| copied.key.as_ref() == Some(&self.key) && copied.at.is_some_and(|at| at.elapsed() < COPIED_FOR))
            .map(|copied| copied.presses);
        let (key, text) = (self.key.clone(), self.text.clone());
        let button = div()
            .id(ElementId::Name(format!("{}-copy", self.key).into()))
            .flex_none()
            .h(px(22.))
            .px(px(8.))
            .rounded_full()
            .flex()
            .items_center()
            .typography(Type::Small)
            .cursor_pointer()
            .on_click(move |_, window, cx| copy(key.clone(), text.clone(), window, cx));
        // One hover style each way: GPUI takes only one per element.
        match just {
            // The check arrives with a small rise, in success's colour.
            Some(presses) => {
                let words = div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_color(theme.success)
                    .child(Icon::Check.view(px(12.), theme.success))
                    .child("Copied");
                let words = if still {
                    words.into_any_element()
                } else {
                    crate::motion::enter(ElementId::NamedInteger(format!("{}-copied", self.key).into(), presses), words).into_any_element()
                };
                button.hover(|button| button.bg(theme.hover)).child(words).tag(self.tag).into_any_element()
            }
            None => button
                .gap(px(5.))
                .text_color(theme.text_dim)
                .hover(|button| button.bg(theme.hover).text_color(theme.text))
                .child(Icon::Copy.view(px(12.), theme.text_dim))
                .child(self.label)
                .tag(self.tag)
                .into_any_element(),
        }
    }
}

fn copy(key: SharedString, text: SharedString, window: &mut Window, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
    let presses = cx.try_global::<Copied>().map_or(0, |copied| copied.presses) + 1;
    cx.set_global(Copied { key: Some(key), at: Some(Instant::now()), presses });
    cx.refresh_windows();
    window
        .spawn(cx, async move |cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            cx.update(|_, cx| cx.refresh_windows()).ok();
        })
        .detach();
}
