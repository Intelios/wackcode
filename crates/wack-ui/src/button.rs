//! Buttons for cards that ask the user to decide: the question card, the plan card, a
//! confirmation. A word on a rounded ground; one primary per row.

use gpui::{App, Background, ClickEvent, Hsla, SharedString, Window, div, prelude::*, px};

use crate::{
    theme::Theme,
    tooltip::tip,
    typography::{Type, Typography},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The one thing the card is for: filled with the accent.
    Primary,
    /// Everything else: a quiet outline.
    Secondary,
    /// Something that throws work away: danger-coloured text.
    Danger,
    /// Ultra's primary: filled with the prism's base.
    Ultra,
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct Button {
    id: SharedString,
    label: SharedString,
    kind: Kind,
    enabled: bool,
    tooltip: Option<SharedString>,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>, kind: Kind) -> Self {
        Self { id: id.into(), label: label.into(), kind, enabled: true, tooltip: None, on_click: None }
    }

    /// Dispatches `action` when clicked.
    pub fn action(mut self, action: impl gpui::Action) -> Self {
        let action = action.boxed_clone();
        self.on_click = Some(Box::new(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)));
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx);
        let (ground, hover, ink, line): (Option<Background>, Background, Hsla, Option<Hsla>) = match self.kind {
            Kind::Primary => (Some(theme.accent_fill()), theme.accent_fill_hover(), theme.on_accent, None),
            Kind::Ultra => (Some(theme.ultra.into()), theme.ultra_glow.into(), theme.on_ultra, None),
            Kind::Secondary => (None, theme.hover.into(), theme.text_soft, Some(theme.border_strong)),
            Kind::Danger => (None, theme.hover.into(), theme.danger, Some(theme.border_strong)),
        };
        let enabled = self.enabled;
        div()
            .id(self.id.clone())
            .flex_none()
            .h(px(30.))
            .px(px(13.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .when_some(ground, |button, ground| button.bg(ground))
            .when_some(line, |button, line| button.border_1().border_color(line))
            .when(enabled, |button| button.hover(move |button| button.bg(hover)))
            .when(!enabled, |button| button.opacity(0.45))
            .when_some(self.tooltip, |button, text| button.tooltip(tip(text)))
            .when_some(self.on_click.filter(|_| enabled), |button, on_click| button.on_click(on_click))
            .child(
                div()
                    .typography(if matches!(self.kind, Kind::Primary | Kind::Ultra) { Type::UiStrong } else { Type::Ui })
                    .text_color(ink)
                    .child(self.label),
            )
    }
}
