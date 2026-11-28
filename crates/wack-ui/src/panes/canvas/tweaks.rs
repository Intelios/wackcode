//! The Tweaks panel: the levers the agent declared on the selected artboard, moved by the user
//! with no model call (`wack_canvas::Tweak`).
//!
//! - A change is saved (`Canvases::set_tweak`), which bumps the artboard's revision and redraws
//!   its picture; the model hears of it before its next request (`CanvasFeature`).
//! - While the artboard plays, a change reaches its live page at once, and the revision it makes
//!   does not reopen the page (`tweak_rev`).
//! - A number drags along its track: the page follows live while dragging, and the value is
//!   saved once, on release, so one drag is one drawing.
//! - Colours are the design's own, drawn as data through `theme::design_colour`; everything
//!   else is the theme's.

use std::collections::HashMap;

use gpui::{Bounds, Context, Entity, Hsla, MouseButton, Pixels, SharedString, div, prelude::*, px};
use serde_json::Value;
use wack_canvas::{Artboard, Canvas, Tweak, TweakKind};

use super::{CanvasPane, EDGE, PILL_HEIGHT};
use crate::{
    icon::Icon,
    tags::Tag,
    theme::{Theme, design_colour, hue_turns},
    tooltip::tip,
    typography::{Type, Typography},
};

pub const PANEL_WIDTH: f32 = 276.;
/// The least width Play keeps for a page beside the panel.
const MIN_PLAY_ROOM: f32 = 340.;
const SWATCH: f32 = 22.;
const KNOB: f32 = 14.;
/// Swatches a colour with no options of its own offers: the default and hue turns of it. With
/// the value set leading, one row fits the panel.
const TURNS: usize = 6;

/// A number being dragged along its track.
pub struct TweakDrag {
    pub artboard: String,
    pub tweak: String,
}

impl CanvasPane {
    /// Moves a tweak: live on a playing page at once, and saved when `commit` (a click, or the
    /// end of a drag). `None` puts the default back.
    pub(super) fn move_tweak(&mut self, artboard: &str, tweak: &str, value: Option<Value>, commit: bool, cx: &mut Context<Self>) {
        let Some(canvas) = self.current.clone() else { return };
        let live = self.playing.as_ref().filter(|p| p.canvas == canvas && p.artboard == artboard).and_then(|p| p.tab.clone());
        if let Some(tab) = live {
            let (canvases, artboard, tweak, value) = (self.canvases.clone(), artboard.to_string(), tweak.to_string(), value.clone());
            let canvas = canvas.clone();
            gpui_tokio::Tokio::spawn(cx, async move { canvases.apply_live_tweak(&tab, &canvas, &artboard, &tweak, value).await }).detach();
        }
        if !commit {
            self.tweak_preview = Some((artboard.to_string(), tweak.to_string(), value.unwrap_or(Value::Null)));
            cx.notify();
            return;
        }
        self.tweak_preview = None;
        match self.canvases.set_tweak(&canvas, artboard, tweak, value) {
            Ok(rev) => {
                self.tweak_rev = Some((canvas, artboard.to_string(), rev));
                self.tweak_error = None;
            }
            Err(error) => self.tweak_error = Some(error),
        }
        cx.notify();
    }

    /// Puts every tweak of an artboard back to its default.
    pub(super) fn reset_tweaks(&mut self, artboard: &Artboard, cx: &mut Context<Self>) {
        for tweak in artboard.tweaks.iter().filter(|t| artboard.tweak_values.contains_key(&t.id)) {
            self.move_tweak(&artboard.id, &tweak.id, None, true, cx);
        }
    }

    /// The value under the pointer on a number's track, snapped to its step.
    pub(super) fn number_at(&self, tweak: &Tweak, x: Pixels) -> Option<Value> {
        let track = self.tweak_tracks.borrow().get(&tweak.id).copied()?;
        let (min, max) = range(tweak)?;
        let fraction = (f32::from(x - track.origin.x) / f32::from(track.size.width).max(1.)).clamp(0., 1.);
        let raw = min + f64::from(fraction) * (max - min);
        Some(snap(tweak, raw))
    }

    /// Follows a number's drag to `x`; `done` ends it and saves where it stopped. False when
    /// no tweak is being dragged, so the board takes the pointer.
    pub(super) fn drag_tweak(&mut self, x: Pixels, done: bool, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.tweak_drag.as_ref() else { return false };
        let (artboard, id) = (drag.artboard.clone(), drag.tweak.clone());
        let tweak = self.canvas().and_then(|c| c.artboards.iter().find(|a| a.id == artboard)).and_then(|a| a.tweaks.iter().find(|t| t.id == id)).cloned();
        let Some(tweak) = tweak else {
            self.tweak_drag = None;
            self.tweak_preview = None;
            return true;
        };
        let value = self.number_at(&tweak, x).or_else(|| self.tweak_preview.as_ref().map(|(_, _, value)| value.clone()));
        if done {
            self.tweak_drag = None;
        }
        match value {
            Some(value) => self.move_tweak(&artboard, &id, Some(value), done, cx),
            None => {
                self.tweak_preview = None;
                cx.notify();
            }
        }
        true
    }

    /// The artboard whose tweaks the panel shows: the one playing, else the one selected, when
    /// the panel is open and that artboard has tweaks.
    pub(super) fn tweak_target(&self) -> Option<&Artboard> {
        if !self.tweaks_open || self.presenting.is_some() {
            return None;
        }
        let id = self.playing.as_ref().map(|p| &p.artboard).or(self.selected.as_ref())?;
        self.canvas()?.artboards.iter().find(|a| &a.id == id).filter(|a| !a.tweaks.is_empty())
    }

    /// Room Play keeps clear on the right for the panel, so the live page (a native view, drawn
    /// over GPUI) never covers it. A pane too narrow to spare it keeps the whole width, and a
    /// page the panel would cover waits behind its picture (`panel_left`).
    pub(super) fn panel_room(&self) -> f32 {
        let room = PANEL_WIDTH + 2. * EDGE;
        if self.tweak_target().is_some() && self.size().0 - room >= MIN_PLAY_ROOM { room } else { 0. }
    }

    /// Where the panel's left edge is, in pane points, while it shows.
    pub(super) fn panel_left(&self) -> Option<f32> {
        self.tweak_target().map(|_| self.size().0 - EDGE - PANEL_WIDTH)
    }

    pub(super) fn toggle_tweaks(&mut self, cx: &mut Context<Self>) {
        self.tweaks_open = !self.tweaks_open;
        self.tweak_error = None;
        self.export_menu = false;
        self.refit_playing(cx);
        cx.notify();
    }

    /// The value a tweak shows: a drag's preview, else the user's setting, else the default.
    fn shown_value(&self, artboard: &Artboard, tweak: &Tweak) -> Value {
        match &self.tweak_preview {
            Some((board, id, value)) if board == &artboard.id && id == &tweak.id && !value.is_null() => value.clone(),
            _ => tweak.value(&artboard.tweak_values).clone(),
        }
    }

    pub(super) fn tweaks_panel(&self, canvas: &Canvas, artboard: &Artboard, height: f32, theme: &Theme, pane: Entity<CanvasPane>) -> impl IntoElement {
        let changed = !artboard.tweak_values.is_empty();
        let head = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(14.))
            .pt(px(12.))
            .pb(px(6.))
            .child(Icon::Sliders.view(px(13.), theme.accent_text))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().typography(Type::UiStrong).text_color(theme.text).child("Tweaks"))
                    .child(div().truncate().typography(Type::Small).text_color(theme.text_dim).child(format!("{} · {}", artboard.name, canvas.title))),
            )
            .when(changed, |head| {
                let (pane, board) = (pane.clone(), artboard.clone());
                head.child(
                    div()
                        .id("canvas-tweaks-reset")
                        .flex_none()
                        .h(px(24.))
                        .px(px(10.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .bg(theme.hover)
                        .hover(|reset| reset.bg(theme.accent_wash))
                        .typography(Type::Small)
                        .text_color(theme.text_soft)
                        .tooltip(tip("Put every tweak back to the design's own"))
                        .on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.reset_tweaks(&board, cx)))
                        .child("Reset")
                        .tag("canvas-tweaks-reset"),
                )
            })
            .child({
                let pane = pane.clone();
                super::circle("canvas-tweaks-close", Icon::Close, "Close  esc", theme).on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.toggle_tweaks(cx)))
            });
        let mut rows = div().flex().flex_col().gap(px(14.)).px(px(14.)).pt(px(6.)).pb(px(14.));
        for tweak in &artboard.tweaks {
            rows = rows.child(self.tweak_row(artboard, tweak, theme, pane.clone()));
        }
        let error = self.tweak_error.clone().map(|error| div().px(px(14.)).pb(px(12.)).typography(Type::Small).text_color(theme.danger).child(error));
        // The panel keeps its presses and scrolls from the board; the rows scroll inside it.
        div()
            .absolute()
            .top(px(EDGE + PILL_HEIGHT + 8.))
            .right(px(EDGE))
            .w(px(PANEL_WIDTH))
            .max_h(px((height - 2. * EDGE - PILL_HEIGHT - 8.).max(120.)))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(16.))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(head)
            .child(div().flex_none().h(px(1.)).mx(px(14.)).mb(px(8.)).bg(theme.border))
            .child(div().id("canvas-tweaks-rows").min_h_0().flex_shrink(1.).overflow_y_scroll().child(rows).children(error))
            .tag("canvas-tweaks")
    }

    fn tweak_row(&self, artboard: &Artboard, tweak: &Tweak, theme: &Theme, pane: Entity<CanvasPane>) -> impl IntoElement {
        let value = self.shown_value(artboard, tweak);
        let set = artboard.tweak_values.contains_key(&tweak.id);
        let readout = match (tweak.kind, &value) {
            (TweakKind::Toggle, Value::Bool(on)) => if *on { "On" } else { "Off" }.to_string(),
            _ => tweak.css_value(&value),
        };
        let label = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(div().flex_1().min_w_0().truncate().typography(Type::Ui).text_color(theme.text).child(tweak.label.clone()))
            .when(set, |label| label.child(div().size(px(6.)).rounded_full().bg(theme.accent_line)))
            .child(div().flex_none().typography(Type::Small).text_color(if set { theme.text_soft } else { theme.text_faint }).child(readout));
        let control = match tweak.kind {
            TweakKind::Choice => self.choices(artboard, tweak, &value, tweak.options.clone(), theme, pane).into_any_element(),
            TweakKind::Toggle => {
                let options = vec!["Off".to_string(), "On".to_string()];
                let shown = Value::String(if value == Value::Bool(true) { "On" } else { "Off" }.into());
                self.choices(artboard, tweak, &shown, options, theme, pane).into_any_element()
            }
            TweakKind::Color => self.swatches(artboard, tweak, &value, theme, pane).into_any_element(),
            TweakKind::Number => self.slider(artboard, tweak, &value, theme, pane).into_any_element(),
        };
        div().flex().flex_col().gap(px(7.)).child(label).child(control)
    }

    /// Segmented pills: a choice's options, or a toggle's Off and On.
    fn choices(&self, artboard: &Artboard, tweak: &Tweak, value: &Value, options: Vec<String>, theme: &Theme, pane: Entity<CanvasPane>) -> impl IntoElement {
        let toggle = tweak.kind == TweakKind::Toggle;
        let mut row = div().flex().flex_wrap().gap(px(4.)).p(px(3.)).rounded(px(12.)).bg(theme.hover);
        for option in options {
            let picked = value.as_str() == Some(option.as_str());
            let (pane, board, id) = (pane.clone(), artboard.id.clone(), tweak.id.clone());
            let chosen = if toggle { Value::Bool(option == "On") } else { Value::String(option.clone()) };
            row = row.child(
                div()
                    .id(SharedString::from(format!("canvas-tweak-{}-{option}", tweak.id)))
                    .flex_grow(1.)
                    .h(px(26.))
                    .px(px(10.))
                    .rounded(px(9.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .typography(Type::Small)
                    .when(picked, |pill| pill.bg(theme.raised).shadow_sm().text_color(theme.text))
                    .when(!picked, |pill| pill.text_color(theme.text_dim).hover(|pill| pill.text_color(theme.text)))
                    .on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.move_tweak(&board, &id, Some(chosen.clone()), true, cx)))
                    .child(div().whitespace_nowrap().child(option)),
            );
        }
        row
    }

    /// A colour's swatches: its options, else its default turned round the hue circle. The
    /// value set, when it is none of them, leads.
    fn swatches(&self, artboard: &Artboard, tweak: &Tweak, value: &Value, theme: &Theme, pane: Entity<CanvasPane>) -> impl IntoElement {
        let default = tweak.default.as_str().unwrap_or("#000000");
        let mut colours: Vec<String> = if tweak.options.is_empty() { hue_turns(default, TURNS) } else { tweak.options.clone() };
        let current = value.as_str().unwrap_or(default).to_ascii_lowercase();
        if !colours.iter().any(|c| c.eq_ignore_ascii_case(&current)) {
            colours.insert(0, current.clone());
            colours.truncate(TURNS + 1);
        }
        let mut row = div().flex().flex_wrap().gap(px(8.));
        for colour in colours {
            let Some(fill) = design_colour(&colour) else { continue };
            let picked = colour.eq_ignore_ascii_case(&current);
            let (pane, board, id) = (pane.clone(), artboard.id.clone(), tweak.id.clone());
            let chosen = Value::String(colour.clone());
            row = row.child(
                div()
                    .id(SharedString::from(format!("canvas-tweak-{}-{colour}", tweak.id)))
                    .flex_none()
                    .size(px(SWATCH + 6.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_2()
                    .border_color(if picked { theme.accent_line } else { Hsla::transparent_black() })
                    .cursor_pointer()
                    .tooltip(tip(colour.clone()))
                    .on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.move_tweak(&board, &id, Some(chosen.clone()), true, cx)))
                    .child(div().size(px(SWATCH)).rounded_full().bg(fill).border_1().border_color(theme.border)),
            );
        }
        row
    }

    /// A number: − and + step it; its track takes a click or a drag.
    fn slider(&self, artboard: &Artboard, tweak: &Tweak, value: &Value, theme: &Theme, pane: Entity<CanvasPane>) -> impl IntoElement {
        let Some((min, max)) = range(tweak) else {
            return div().typography(Type::Small).text_color(theme.text_faint).child("No range to move in").into_any_element();
        };
        let now = value.as_f64().unwrap_or(min);
        let fraction = (((now - min) / (max - min).max(f64::EPSILON)) as f32).clamp(0., 1.);
        let step_by = |by: f64| {
            let (pane, board, id, tweak) = (pane.clone(), artboard.id.clone(), tweak.id.clone(), tweak.clone());
            move |_: &gpui::ClickEvent, _: &mut gpui::Window, cx: &mut gpui::App| {
                let next = snap(&tweak, (now + by * step(&tweak)).clamp(min, max));
                pane.update(cx, |pane, cx| pane.move_tweak(&board, &id, Some(next.clone()), true, cx));
            }
        };
        let tracks = self.tweak_tracks.clone();
        let track_id = tweak.id.clone();
        let track = div()
            .id(SharedString::from(format!("canvas-tweak-{}-track", tweak.id)))
            .relative()
            .flex_1()
            .h(px(KNOB + 8.))
            .mx(px(KNOB / 2.))
            .cursor_pointer()
            .child(
                gpui::canvas(
                    move |bounds: Bounds<Pixels>, _, _| {
                        tracks.borrow_mut().insert(track_id.clone(), bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(div().absolute().left_0().right_0().top(px((KNOB + 8.) / 2. - 2.)).h(px(4.)).rounded_full().bg(theme.hover))
            .child(div().absolute().left_0().w(gpui::relative(fraction)).top(px((KNOB + 8.) / 2. - 2.)).h(px(4.)).rounded_full().bg(theme.accent_line))
            .child(
                div()
                    .absolute()
                    .left(gpui::relative(fraction))
                    .ml(px(-KNOB / 2.))
                    .top(px(4.))
                    .size(px(KNOB))
                    .rounded_full()
                    .bg(theme.raised)
                    .border_2()
                    .border_color(theme.accent)
                    .shadow_sm(),
            )
            .on_mouse_down(MouseButton::Left, {
                let (pane, board, tweak) = (pane.clone(), artboard.id.clone(), tweak.clone());
                move |event, _, cx| {
                    cx.stop_propagation();
                    pane.update(cx, |pane, cx| {
                        pane.tweak_drag = Some(TweakDrag { artboard: board.clone(), tweak: tweak.id.clone() });
                        if let Some(value) = pane.number_at(&tweak, event.position.x) {
                            pane.move_tweak(&board, &tweak.id, Some(value), false, cx);
                        }
                    })
                }
            });
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(super::circle(format!("canvas-tweak-{}-less", tweak.id), Icon::Minus, "Less", theme).cursor_pointer().on_click(step_by(-1.)))
            .child(track)
            .child(super::circle(format!("canvas-tweak-{}-more", tweak.id), Icon::Plus, "More", theme).cursor_pointer().on_click(step_by(1.)))
            .into_any_element()
    }
}

/// A number's range: its own, or one around its default when it gave none.
fn range(tweak: &Tweak) -> Option<(f64, f64)> {
    let default = tweak.default.as_f64().unwrap_or(0.);
    let min = tweak.min.unwrap_or(default.min(0.));
    let max = tweak.max.unwrap_or(if default > 0. { default * 2. } else { 100. });
    (max > min).then_some((min, max))
}

fn step(tweak: &Tweak) -> f64 {
    tweak.step.filter(|s| *s > 0.).unwrap_or_else(|| range(tweak).map_or(1., |(min, max)| ((max - min) / 20.).max(f64::EPSILON)))
}

/// A number on its step from the range's start, kept in range, as JSON.
fn snap(tweak: &Tweak, raw: f64) -> Value {
    let (min, max) = range(tweak).unwrap_or((raw, raw));
    let step = step(tweak);
    let snapped = (min + ((raw - min) / step).round() * step).clamp(min, max);
    // Clear the float noise a step like 0.1 leaves.
    let tidy = (snapped * 1e6).round() / 1e6;
    serde_json::Number::from_f64(tidy).map_or(Value::Null, Value::Number)
}

/// Tracks of the numbers on screen, by tweak id, from the last paint.
pub type Tracks = std::rc::Rc<std::cell::RefCell<HashMap<String, Bounds<Pixels>>>>;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn number(min: Option<f64>, max: Option<f64>, step: Option<f64>, default: f64) -> Tweak {
        serde_json::from_value(json!({ "id": "r", "label": "Radius", "kind": "number", "default": default, "min": min, "max": max, "step": step, "var": "--r" })).unwrap()
    }

    #[test]
    fn numbers_snap_to_their_step_and_stay_in_range() {
        let radius = number(Some(0.), Some(24.), Some(2.), 10.);
        assert_eq!(snap(&radius, 7.1), json!(8.0));
        assert_eq!(snap(&radius, 99.), json!(24.0));
        let fine = number(Some(0.), Some(1.), Some(0.1), 0.5);
        assert_eq!(snap(&fine, 0.34), json!(0.3));
        let open = number(None, None, None, 12.);
        assert_eq!(range(&open), Some((0., 24.)));
        assert_eq!(step(&open), 1.2);
    }
}
