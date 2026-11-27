//! The parts every Settings page is built from.
//!
//! A page opens with a [`head`]: the section's name in the serif, a line saying what it is for,
//! and at most one primary action, an accent pill at the right. A section's own page opens with a
//! [`staged_head`] instead, its little animated picture (`stage.rs`) above the action. Below it the page is [`card`]s of
//! [`row`]s: the label and a sentence on the left, the control on the right, a hairline between
//! rows. A control too wide to sit beside its label drops beneath it ([`Row::stacked`]).
//!
//! Controls are round, like the rest of the app: [`pill`] buttons, a [`segmented`] choice in a
//! track, the switch (`tools_page::switch`) and a [`slider`] for a value along a range (its drag
//! is `slide.rs`). A slider
//! applies as it is dragged: each step saves the setting, which is cheap, so the window is the
//! preview.

use std::{cell::Cell, rc::Rc};

use gpui::{App, Bounds, MouseButton, Pixels, SharedString, Window, div, prelude::*, px};

use crate::{
    icon::Icon,
    settings::Section,
    slide,
    stage::Stage,
    tags::Tag,
    theme::Theme,
    typography::{Type, Typography},
};

/// The widest a page's cards grow.
pub const PAGE_WIDTH: f32 = 760.;
const CARD_RADIUS: f32 = 16.;
const ROW_X: f32 = 18.;

pub fn dispatch(action: impl gpui::Action) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static {
    let action = action.boxed_clone();
    move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)
}

/// The page's head: its title, what it is for, and its one primary action.
pub fn head(title: &'static str, about: &'static str, action: Option<gpui::AnyElement>, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .items_end()
        .gap(px(24.))
        .pb(px(22.))
        .child(head_words(title, about, theme))
        .children(action.map(|action| div().flex_none().pb(px(2.)).child(action)))
}

/// A section's own page opens with its [`Stage`] at the right of the head, the page's one action
/// beneath it. Editors and other pages within a section keep the plain [`head`].
pub fn staged_head(section: Section, title: &'static str, about: &'static str, action: Option<gpui::AnyElement>, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .items_end()
        .gap(px(24.))
        .pb(px(22.))
        .child(head_words(title, about, theme))
        .child(div().flex_none().flex().flex_col().items_end().gap(px(12.)).child(Stage::new(section)).children(action.map(|action| div().pb(px(2.)).child(action))))
}

fn head_words(title: &'static str, about: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(div().typography(Type::Title).text_color(theme.text).child(title))
        .child(div().max_w(px(540.)).typography(Type::Body).text_color(theme.text_soft).child(about))
}

/// A card: an optional title line (with a badge and a note at its right), then its rows.
pub fn card(title: Option<&'static str>, badge: Option<SharedString>, note: Option<&'static str>, theme: &Theme) -> gpui::Div {
    let title_line = title.map(|title| {
        div()
            .px(px(ROW_X))
            .pt(px(14.))
            .pb(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().typography(Type::Label).text_color(theme.text_dim).child(title))
            .when_some(badge, |line, badge| {
                line.child(
                    div()
                        .h(px(18.))
                        .px(px(7.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .bg(theme.hover)
                        .typography(Type::Small)
                        .text_color(theme.text_soft)
                        .child(badge),
                )
            })
            .child(div().flex_1())
            .when_some(note, |line, note| line.child(div().typography(Type::Small).text_color(theme.text_faint).child(note)))
    });
    div()
        .w_full()
        .flex()
        .flex_col()
        .rounded(px(CARD_RADIUS))
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .overflow_hidden()
        .children(title_line)
}

/// One row of a card. The first row of a card with no title draws no hairline above it.
pub struct Row {
    icon: Option<Icon>,
    label: SharedString,
    about: Option<SharedString>,
    control: Option<gpui::AnyElement>,
    stacked: bool,
    first: bool,
}

pub fn row(label: impl Into<SharedString>, about: Option<&'static str>) -> Row {
    let label = label.into();
    Row { icon: mark(&label), label, about: about.map(Into::into), control: None, stacked: false, first: false }
}

/// The icon a setting carries in its row, as version 2's settings did: each page's own
/// settings get one, by their label, so a page reads at a glance. Rows that list things (an
/// app, a skill's fields) have none.
fn mark(label: &str) -> Option<Icon> {
    Some(match label {
        "Theme" => Icon::Palette,
        "Light or dark" => Icon::Eye,
        "Model logos" => Icon::Spark,
        "Pictures" => Icon::Board,
        "Shuffle" => Icon::Shuffle,
        "Texture" => Icon::Sliders,
        "Blur" | "Dim" => Icon::Sliders,
        "Keep in view" => Icon::Pin,
        "Use memory" | "Memory" => Icon::Brain,
        "Delegation" | "In a Team" | "Default crew" => Icon::Duck,
        "Helper model" | "Title model" | "Model" => Icon::Spark,
        "Automatic titles" => Icon::Pencil,
        "Web Fetch" => Icon::Link,
        "Web Search" => Icon::Search,
        "Remove Plan / Ultra Plan read-only restrictions" => Icon::Lock,
        "WackCode as co-author" => Icon::Commit,
        "Record TokenTrail usage" => Icon::Plug,
        "Let the agent use your Mac’s apps" => Icon::Cursor,
        "Accessibility" => Icon::Hand,
        "Screen Recording" => Icon::Eye,
        "Show the Browser pane when the agent opens a tab" => Icon::Window,
        "Open at" => Icon::Phone,
        "Clear browsing data" => Icon::Trash,
        _ => return None,
    })
}

impl Row {
    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.control = Some(control.into_any_element());
        self
    }

    /// Words under the label that aren't fixed text (a path, a name).
    pub fn about(mut self, about: impl Into<SharedString>) -> Self {
        self.about = Some(about.into());
        self
    }

    /// The control beneath the words, for one too wide to sit beside them.
    pub fn stacked(mut self) -> Self {
        self.stacked = true;
        self
    }

    pub fn first(mut self) -> Self {
        self.first = true;
        self
    }

    /// The row's mark: its icon on a round wash of the accent.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn view(self, theme: &Theme) -> gpui::Div {
        let mark = self.icon.map(|icon| {
            div()
                .flex_none()
                .size(px(30.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.accent_wash)
                .border_1()
                .border_color(theme.accent_line)
                .child(icon.view(px(14.), theme.accent_text))
        });
        let words = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(div().typography(Type::UiStrong).text_color(theme.text).child(self.label))
            .when_some(self.about, |words, about| words.child(div().typography(Type::Small).text_color(theme.text_dim).child(about)));
        div()
            .px(px(ROW_X))
            .py(px(13.))
            .flex()
            .gap(px(18.))
            .when(!self.first, |row| row.border_t_1().border_color(theme.border))
            .when(self.stacked, |row| row.flex_col().gap(px(12.)))
            .when(!self.stacked, |row| row.items_center())
            .child(div().flex_1().min_w_0().flex().items_center().gap(px(12.)).children(mark).child(words))
            .children(self.control.map(|control| div().flex_none().flex().when(self.stacked, |slot| slot.w_full()).child(control)))
    }
}

/// A round button. `strong` is the page's one primary: filled with the accent.
pub fn pill(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, icon: Option<Icon>, strong: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let ink = if strong { theme.on_accent } else { theme.text_soft };
    div()
        .id(id)
        .flex_none()
        .h(px(32.))
        .pl(px(if icon.is_some() { 11. } else { 14. }))
        .pr(px(14.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(7.))
        .cursor_pointer()
        .when(strong, |pill| pill.bg(theme.accent_fill()).hover(|pill| pill.bg(theme.accent_fill_hover())))
        .when(!strong, |pill| pill.border_1().border_color(theme.border_strong).hover(|pill| pill.bg(theme.hover)))
        .children(icon.map(|icon| icon.view(px(14.), ink)))
        .child(div().typography(Type::UiStrong).text_color(ink).child(label.into()))
}

/// A round picker pill for choosing a model: disclosure chevron, optional provider logo, and a
/// truncated model label.
pub fn model_pill(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    brand: Option<crate::brand::Brand>,
    open: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
    div()
        .id(id)
        .flex_none()
        .h(px(32.))
        .pl(px(11.))
        .pr(px(14.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(7.))
        .cursor_pointer()
        .border_1()
        .border_color(theme.border_strong)
        .hover(|pill| pill.bg(theme.hover))
        .child(chevron.view(px(14.), theme.text_soft))
        .children(brand.map(|brand| brand.view(px(14.), theme)))
        .child(div().min_w_0().truncate().typography(Type::UiStrong).text_color(theme.text_soft).child(label.into()))
}

/// A choice of a few words in a round track; the chosen one is a raised pill.
pub fn segmented<T: Copy + PartialEq + 'static>(
    id: &'static str,
    choices: &[(T, &'static str)],
    chosen: T,
    action: impl Fn(T) -> Box<dyn gpui::Action>,
    theme: &Theme,
) -> impl IntoElement {
    div()
        .flex_none()
        .p(px(3.))
        .rounded_full()
        .bg(theme.hover)
        .flex()
        .gap(px(2.))
        .children(choices.iter().enumerate().map(|(index, (value, label))| {
            let on = *value == chosen;
            let choose = action(*value);
            div()
                .id((id, index))
                .h(px(26.))
                .px(px(12.))
                .rounded_full()
                .flex()
                .items_center()
                .cursor_pointer()
                .typography(if on { Type::UiStrong } else { Type::Ui })
                .text_color(if on { theme.text } else { theme.text_dim })
                .when(on, |choice| choice.bg(theme.raised).border_1().border_color(theme.border).shadow_sm())
                .when(!on, |choice| choice.hover(|choice| choice.text_color(theme.text)))
                .on_click(move |_, window, cx| window.dispatch_action(choose.boxed_clone(), cx))
                .child(*label)
        }))
}

/// A value along `0..=max` in steps of `step`, shown with `unit` beside the track. Pressing or
/// dragging along the track moves the knob; each new step dispatches `action`.
pub fn slider(
    id: &'static str,
    value: u8,
    max: u8,
    step: u8,
    unit: &'static str,
    action: fn(u8) -> Box<dyn gpui::Action>,
    theme: &Theme,
) -> impl IntoElement {
    let shown: SharedString = if value == 0 { "None".into() } else { format!("{}{unit}", value.min(max)).into() };
    labelled_slider(id, value, max, step, shown, action, theme)
}

/// A slider whose value reads as `shown` beside it ("250%").
pub fn labelled_slider(
    id: &'static str,
    value: u8,
    max: u8,
    step: u8,
    shown: SharedString,
    action: fn(u8) -> Box<dyn gpui::Action>,
    theme: &Theme,
) -> impl IntoElement {
    const WIDTH: f32 = 220.;
    const HEIGHT: f32 = 22.;
    const KNOB: f32 = 18.;
    let value = value.min(max);
    let knob_x = slider_x(value, max, WIDTH, HEIGHT);

    // The track remembers where it was drawn, so a press or a drag finds the nearest step.
    let drawn: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::default();
    let pick: slide::Pick = Rc::new({
        let drawn = drawn.clone();
        move |x: Pixels, window: &mut Window, cx: &mut App| {
            let Some(bounds) = drawn.get() else {
                return;
            };
            let picked = slider_value(f32::from(x - bounds.left()), max, step, f32::from(bounds.size.width), HEIGHT);
            if picked != value {
                window.dispatch_action(action(picked), cx);
            }
        }
    });
    let track = div()
        .id(id)
        .flex_none()
        .relative()
        .w(px(WIDTH))
        .h(px(HEIGHT))
        .rounded_full()
        .bg(theme.hover)
        .cursor_pointer()
        .child(slide::follow(id, drawn, pick.clone()).absolute().left_0().top_0().size_full())
        .child(div().absolute().left_0().top_0().bottom_0().w(px(knob_x + HEIGHT / 2.)).rounded_full().bg(theme.accent_fill()))
        .child(
            div()
                .absolute()
                .top(px((HEIGHT - KNOB) / 2.))
                .left(px(knob_x - KNOB / 2.))
                .size(px(KNOB))
                .rounded_full()
                .bg(theme.raised)
                .border_1()
                .border_color(theme.border_strong)
                .shadow_sm(),
        )
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            cx.stop_propagation();
            slide::begin(id);
            pick(event.position.x, window, cx)
        });
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(track.tag(id))
        .child(div().w(px(42.)).flex_none().typography(Type::Small).text_color(theme.text_soft).child(shown))
}

/// Where the knob's middle sits for `value` along a track `width` wide and `height` tall.
fn slider_x(value: u8, max: u8, width: f32, height: f32) -> f32 {
    let inset = height / 2.;
    inset + (width - 2. * inset) * value as f32 / max.max(1) as f32
}

/// The step nearest `x` along the track.
fn slider_value(x: f32, max: u8, step: u8, width: f32, height: f32) -> u8 {
    let inset = height / 2.;
    let along = ((x - inset) / (width - 2. * inset).max(1.)).clamp(0., 1.);
    let step = step.max(1) as f32;
    ((along * max as f32 / step).round() * step).min(max as f32) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slider_snaps_to_steps_and_stays_in_range() {
        let (width, height) = (220., 22.);
        assert_eq!(slider_value(0., 40, 2, width, height), 0);
        assert_eq!(slider_value(width, 40, 2, width, height), 40);
        assert_eq!(slider_value(width * 2., 80, 5, width, height), 80, "past the end is the end");
        let middle = slider_x(20, 40, width, height);
        assert_eq!(slider_value(middle, 40, 2, width, height), 20);
        assert_eq!(slider_value(slider_x(42, 80, width, height), 80, 5, width, height), 40, "the nearest step");
    }
}
