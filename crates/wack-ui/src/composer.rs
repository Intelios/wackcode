//! The composer: where the user writes to the agent.
//!
//! One card, no toolbar of pills: the model and effort are plain words you click, the context
//! meter is a number, and the one filled control is the send button (the stop button while the
//! agent works). Messages waiting for the run sit just above the card, so the user sees what goes
//! next.
//!
//! Renders from plain data ([`ComposerData`]) and reports clicks as actions (`crate::actions`),
//! which the shell handles; in a gallery scene nothing handles them.

use gpui::{
    AnchoredPositionMode, App, Entity, Focusable, Hsla, MouseButton, SharedString, Window, anchored, deferred, div,
    point, prelude::*, px,
};
use wack_core::Effort;

use crate::{
    actions::{ChooseEffort, ChooseModel, CloseMenu, Compact, OpenEffortMenu, OpenModelMenu, Send, Stop, Unqueue},
    app::Menu,
    icon::Icon,
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    typography::{Type, Typography},
};

#[derive(Clone)]
pub enum Input {
    /// A live field.
    Field(Entity<TextField>),
    /// Drawn text, for scenes: the placeholder shows when `text` is `None`.
    Drawn { text: Option<SharedString>, placeholder: SharedString },
}

#[derive(Clone)]
pub struct ComposerData {
    pub input: Input,
    pub model: SharedString,
    pub effort: Option<SharedString>,
    /// "14% context", or a token count when the model's window is unknown.
    pub meter: Option<Meter>,
    pub working: bool,
    /// Messages waiting for the run, oldest first.
    pub waiting: Vec<Waiting>,
    /// A taller card for the new-chat screen.
    pub roomy: bool,
    /// The model or effort menu, when open.
    pub menu: Option<MenuData>,
}

#[derive(Clone, Debug)]
pub struct MenuData {
    pub kind: Menu,
    pub groups: Vec<MenuGroup>,
}

#[derive(Clone, Debug)]
pub struct MenuGroup {
    pub title: Option<SharedString>,
    pub items: Vec<MenuItem>,
}

#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub selected: bool,
    pub choice: Choice,
}

#[derive(Clone, Debug)]
pub enum Choice {
    Model { connection: String, model: String },
    Effort(Option<Effort>),
}

#[derive(Clone, Debug)]
pub struct Meter {
    pub label: SharedString,
    /// Getting full: drawn in the warning colour.
    pub high: bool,
    /// Offers Compact beside it.
    pub can_compact: bool,
}

#[derive(Clone, Debug)]
pub struct Waiting {
    pub text: SharedString,
    /// Sent to the agent mid-run, rather than queued for after it.
    pub steering: bool,
}

#[derive(IntoElement)]
pub struct Composer {
    data: ComposerData,
}

impl Composer {
    pub fn new(data: ComposerData) -> Self {
        Self { data }
    }
}

fn dispatch(action: impl gpui::Action) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static {
    let action = action.boxed_clone();
    move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)
}

impl RenderOnce for Composer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let data = self.data;
        let viewport = window.viewport_size();
        let menu_for = |kind: Menu| data.menu.clone().filter(|menu| menu.kind == kind);
        let has_text = match &data.input {
            Input::Field(field) => !field.read(cx).text().trim().is_empty(),
            Input::Drawn { text, .. } => text.is_some(),
        };

        let send = if data.working && !has_text {
            div()
                .id("stop")
                .flex_none()
                .size(px(30.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.raised)
                .border_1()
                .border_color(theme.border_strong)
                .hover(|button| button.bg(theme.hover))
                .on_click(dispatch(Stop))
                .child(Icon::Stop.view(px(14.), theme.text))
                .tag("stop")
                .into_any_element()
        } else {
            div()
                .id("send")
                .flex_none()
                .size(px(30.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(if has_text { theme.accent } else { theme.hover })
                .when(has_text, |button| button.hover(|button| button.bg(theme.accent_hover)))
                .on_click(dispatch(Send))
                .child(Icon::ArrowUp.view(px(15.), if has_text { theme.on_accent } else { theme.text_faint }))
                .tag("send")
                .into_any_element()
        };

        let field = match data.input.clone() {
            Input::Field(field) => div().child(field),
            Input::Drawn { text, placeholder } => div().typography(Type::Body).map(|text_box| match text {
                Some(typed) => text_box.text_color(theme.text).child(typed),
                None => text_box.text_color(theme.text_faint).child(placeholder),
            }),
        };
        let focus_target = match &data.input {
            Input::Field(field) => Some(field.clone()),
            Input::Drawn { .. } => None,
        };

        let mut controls = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(Icon::Plus.view(px(16.), theme.text_dim))
            // In a narrow composer the model's name gives way first; the send button never does.
            .child(picker("model", data.model.clone(), theme.text_soft, OpenModelMenu, menu_for(Menu::Model), viewport, &theme));
        if let Some(effort) = data.effort.clone() {
            controls =
                controls.child(picker("effort", effort, theme.text_dim, OpenEffortMenu, menu_for(Menu::Effort), viewport, &theme));
        }
        controls = controls.child(div().flex_1());
        if let Some(meter) = data.meter {
            controls = controls.child(meter_view(meter, data.working, &theme));
        }
        let controls = controls.child(send);

        let card = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .px(px(16.))
            .pt(px(14.))
            .pb(px(10.))
            .rounded(px(14.))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .child(
                field
                    .min_h(px(if data.roomy { 64. } else { 22. }))
                    .when_some(focus_target, |field, target| {
                        // The whole text area takes a click, not only the line of text.
                        field.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            let handle = target.read(cx).focus_handle(cx);
                            window.focus(&handle, cx);
                        })
                    }),
            )
            .child(controls)
            .tag("composer");

        let mut column = div().w_full().flex().flex_col().gap(px(6.));
        for (index, waiting) in data.waiting.into_iter().enumerate() {
            column = column.child(waiting_row(index, waiting, &theme));
        }
        column.child(card)
    }
}

/// A message waiting above the card. A queued one can be taken back to edit.
fn waiting_row(index: usize, waiting: Waiting, theme: &Theme) -> impl IntoElement {
    div()
        .id(("waiting", index))
        .group("waiting")
        .mx(px(14.))
        .px(px(12.))
        .py(px(6.))
        .flex()
        .items_center()
        .gap(px(10.))
        .rounded(px(9.))
        .bg(theme.sidebar)
        .border_1()
        .border_color(theme.border)
        .child(
            div()
                .flex_none()
                .typography(Type::Small)
                .text_color(if waiting.steering { theme.accent_text } else { theme.text_dim })
                .child(if waiting.steering { "Steering" } else { "Next" }),
        )
        .child(div().flex_1().min_w_0().truncate().typography(Type::Ui).text_color(theme.text_soft).child(waiting.text))
        .when(!waiting.steering, |row| {
            row.child(
                div()
                    .id(("unqueue", index))
                    .flex_none()
                    .invisible()
                    .group_hover("waiting", |button| button.visible())
                    .typography(Type::Small)
                    .text_color(theme.text_dim)
                    .hover(|button| button.text_color(theme.text))
                    .on_click(dispatch(Unqueue { index }))
                    .child("Edit"),
            )
        })
}

/// "14% context", and Compact beside it when there is something to compact.
fn meter_view(meter: Meter, working: bool, theme: &Theme) -> impl IntoElement {
    let colour: Hsla = if meter.high { theme.warning } else { theme.text_faint };
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .typography(Type::Small)
        .child(div().text_color(colour).child(meter.label))
        .when(meter.can_compact && !working, |row| {
            row.child(div().text_color(theme.text_faint).child("·")).child(
                div()
                    .id("compact")
                    .text_color(if meter.high { theme.text_soft } else { theme.text_dim })
                    .hover(|word| word.text_color(theme.accent_text))
                    .on_click(dispatch(Compact))
                    .child("Compact")
                    .tag("compact"),
            )
        })
}

/// A word that opens a menu: the model, the effort. The menu opens upwards from the word, over
/// everything, and a click anywhere else closes it.
fn picker(
    tag: &'static str,
    name: SharedString,
    colour: Hsla,
    open: impl gpui::Action,
    menu: Option<MenuData>,
    viewport: gpui::Size<gpui::Pixels>,
    theme: &Theme,
) -> impl IntoElement {
    let word = div()
        .id(tag)
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(3.))
        .text_color(colour)
        .hover(|word| word.text_color(theme.text))
        .on_click(dispatch(open))
        .child(div().min_w_0().truncate().typography(Type::Ui).child(name))
        .child(Icon::ChevronDown.view(px(12.), theme.text_faint))
        .tag(tag);
    // Only the model's name gives way in a narrow composer; the effort is a short word.
    let shrinks = tag == "model";
    div().relative().when(shrinks, |picker| picker.min_w_0()).when(!shrinks, |picker| picker.flex_none()).child(word).when_some(menu, |picker, menu| {
        let backdrop = anchored().position_mode(AnchoredPositionMode::Window).position(point(px(0.), px(0.))).child(
            div().id("menu-backdrop").w(viewport.width).h(viewport.height).on_mouse_down(MouseButton::Left, |_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(Box::new(CloseMenu), cx);
            }),
        );
        picker.child(deferred(backdrop).with_priority(1)).child(deferred(menu_view(menu, theme)).with_priority(2))
    })
}

fn menu_view(menu: MenuData, theme: &Theme) -> impl IntoElement {
    let mut card = div()
        .id("menu")
        .absolute()
        .bottom(px(28.))
        .left(px(-12.))
        .min_w(px(260.))
        .max_w(px(420.))
        .py(px(6.))
        .rounded(px(12.))
        .bg(theme.raised)
        .border_1()
        .border_color(theme.border_strong)
        .shadow_lg()
        .flex()
        .flex_col();
    let mut index: usize = 0;
    for (number, group) in menu.groups.into_iter().enumerate() {
        if number > 0 {
            card = card.child(div().my(px(6.)).h(px(1.)).bg(theme.border));
        }
        if let Some(title) = group.title {
            card = card.child(div().px(px(14.)).pt(px(4.)).pb(px(2.)).typography(Type::Label).text_color(theme.text_dim).child(title));
        }
        for item in group.items {
            let action: Box<dyn gpui::Action> = match item.choice {
                Choice::Model { connection, model } => Box::new(ChooseModel { connection, model }),
                Choice::Effort(effort) => Box::new(ChooseEffort { effort }),
            };
            card = card.child(
                div()
                    .id(("menu-item", index))
                    .mx(px(6.))
                    .px(px(8.))
                    .h(px(30.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .hover(|row| row.bg(theme.hover))
                    .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
                    .child(div().w(px(14.)).flex_none().when(item.selected, |mark| mark.child(Icon::Check.view(px(13.), theme.accent_text))))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .typography(if item.selected { Type::UiStrong } else { Type::Ui })
                            .text_color(theme.text)
                            .child(item.label),
                    )
                    .when_some(item.detail, |row, detail| {
                        row.child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(detail))
                    }),
            );
            index += 1;
        }
    }
    card
}
