//! Point and comment: the Comment tool on the board, its pins and its editor.
//!
//! - With the tool on (C, or its circle), the element under the pointer is outlined from the
//!   artboard's element map (`wack_canvas::render::Element`), drawn by GPUI over the picture: no
//!   page is opened. A click pins a comment to that element; a drag pins it to the smallest
//!   element covering the area.
//! - Only a map of the picture on screen is used (`Shown::map`, same revision), and the comment
//!   keeps that revision, so the next drawing re-anchors it (`comments::follow`).
//! - A comment is a draft until the user sends a message (the composer's chip says so); a draft
//!   can be reworded or deleted, a sent one only deleted. Pins of resolved comments fade out,
//!   still under Reduce Motion.
//! - The editor's field sits inside the pane, whose keys (space, ⇧1, ⇧2) would take its typing:
//!   those actions give the key back while it is open (`typing`).

use std::{rc::Rc, time::Instant};

use gpui::{Context, Entity, Hsla, MouseButton, SharedString, Subscription, Window, div, prelude::*, px};
use wack_canvas::{
    Canvas, Element,
    comments::{self, Anchor, Comment, State},
};

use super::{
    CanvasPane, EDGE,
    board::{Board, Camera, FAR_ZOOM, Rect},
};
use crate::{
    icon::Icon,
    tags::Tag,
    text_field::{TextField, TextFieldEvent},
    theme::Theme,
    tooltip::tip,
    typography::{Type, Typography},
};

const PIN: f32 = 22.;
const EDITOR_WIDTH: f32 = 300.;
/// About how tall the editor draws, to keep it inside the pane.
const EDITOR_HEIGHT: f32 = 190.;
/// How long a resolved pin takes to fade.
pub const FADE: std::time::Duration = std::time::Duration::from_millis(1200);

/// The comment being written or read.
pub struct Editing {
    pub artboard: String,
    pub rev: u64,
    pub anchor: Anchor,
    /// The comment it shows, once pinned; `None` while a new one is written.
    pub id: Option<String>,
    /// Sent comments are read, not reworded.
    pub sent: bool,
    /// The words it was opened with.
    pub body: String,
    pub field: Entity<TextField>,
    pub error: Option<String>,
    _enter: Subscription,
}

/// An area being dragged out with the Comment tool, in board points.
pub struct Region {
    pub artboard: String,
    pub from: (f32, f32),
    pub to: (f32, f32),
}

/// The pin's number: `k3` is 3.
fn number(comment: &Comment) -> String {
    comment.id.trim_start_matches('k').to_string()
}

impl CanvasPane {
    pub(super) fn toggle_commenting(&mut self, cx: &mut Context<Self>) {
        self.commenting = !self.commenting;
        self.hover = None;
        self.region = None;
        if self.commenting {
            self.stop_playing(cx);
            self.export_menu = false;
        }
        cx.notify();
    }

    /// Sets the pane up for a gallery scene of the Comment tool: on, the pointer over an
    /// element, and a new comment being written on another (when `writing`).
    pub fn stage_comments(&mut self, artboard: &str, hover: &str, writing: Option<(&str, &str)>, cx: &mut Context<Self>) {
        self.commenting = true;
        self.selected = Some(artboard.to_string());
        if let Some((rev, map)) = self.map_of(artboard) {
            self.hover = map.iter().find(|e| e.p == hover).cloned().map(|e| (artboard.to_string(), e));
            if let Some((path, words)) = writing
                && let Some(element) = map.iter().find(|e| e.p == path)
            {
                self.open_editor(artboard.to_string(), rev, Anchor::from_element(element), None, words, false, None, cx);
            }
        }
    }

    /// The editor's field has the keys: the pane's own shortcuts give them back.
    pub(super) fn typing(&self) -> bool {
        self.editing.is_some()
    }

    /// The element map of an artboard's picture on screen, and its revision.
    fn map_of(&self, artboard: &str) -> Option<(u64, Rc<Vec<Element>>)> {
        let canvas = self.current.clone()?;
        let shown = self.shown.get(&(canvas, artboard.to_string()))?;
        shown.map.clone().map(|map| (shown.rev, map))
    }

    /// The artboard under a pane point and the point in its CSS pixels.
    fn on_artboard(&self, x: f32, y: f32) -> Option<(String, f64, f64)> {
        let (wx, wy) = self.camera().to_world(x, y);
        let frame = self.board().at(wx, wy)?.clone();
        Some((frame.id, f64::from(wx - frame.rect.x), f64::from(wy - frame.rect.y)))
    }

    /// A press with the Comment tool: on an artboard it starts a pin (or an area). False when
    /// the press is the board's (empty space pans as ever).
    pub(super) fn comment_down(&mut self, x: f32, y: f32, cx: &mut Context<Self>) -> bool {
        // A press beside an editor with new words in it keeps them: Esc or Cancel lets go.
        if let Some(editing) = &self.editing {
            let text = editing.field.read(cx).text().trim().to_string();
            if editing.sent || text.is_empty() || text == editing.body.trim() {
                self.editing = None;
                cx.notify();
            } else {
                return true;
            }
        }
        if !self.commenting {
            return false;
        }
        let Some((artboard, _, _)) = self.on_artboard(x, y) else { return false };
        if self.map_of(&artboard).is_none() {
            return false;
        }
        let at = self.camera().to_world(x, y);
        self.region = Some(Region { artboard: artboard.clone(), from: at, to: at });
        self.selected = Some(artboard);
        cx.notify();
        true
    }

    /// The pointer moved with the Comment tool on: the area grows, or the element under it is
    /// outlined.
    pub(super) fn comment_move(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        let at = self.camera().to_world(x, y);
        if let Some(region) = self.region.as_mut() {
            region.to = at;
            cx.notify();
            return;
        }
        if !self.commenting || self.editing.is_some() {
            return;
        }
        let hover = self.on_artboard(x, y).and_then(|(artboard, px_x, px_y)| {
            let (_, map) = self.map_of(&artboard)?;
            comments::element_at(&map, px_x, px_y).cloned().map(|element| (artboard, element))
        });
        if hover.as_ref().map(|(a, e)| (a, &e.p)) != self.hover.as_ref().map(|(a, e)| (a, &e.p)) {
            self.hover = hover;
            cx.notify();
        }
    }

    /// The press ended: a click pins to the element under it, a drag to the smallest element
    /// covering the area. True when the Comment tool had it.
    pub(super) fn comment_up(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(region) = self.region.take() else { return false };
        let Some(frame) = self.board().frame(&region.artboard).cloned() else { return true };
        let Some((rev, map)) = self.map_of(&region.artboard) else { return true };
        let zoom = self.camera().zoom.max(0.01);
        let (ax, ay) = (f64::from(region.from.0 - frame.rect.x), f64::from(region.from.1 - frame.rect.y));
        let (bx, by) = (f64::from(region.to.0 - frame.rect.x), f64::from(region.to.1 - frame.rect.y));
        let dragged = ((bx - ax).hypot(by - ay) as f32) * zoom >= super::CLICK_SLOP;
        let found = if dragged {
            comments::element_covering(&map, [ax.min(bx), ay.min(by), (bx - ax).abs(), (by - ay).abs()])
        } else {
            comments::element_at(&map, ax, ay)
        };
        if let Some(element) = found.cloned() {
            self.open_editor(region.artboard, rev, Anchor::from_element(&element), None, "", false, Some(window), cx);
        }
        cx.notify();
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn open_editor(&mut self, artboard: String, rev: u64, anchor: Anchor, id: Option<String>, body: &str, sent: bool, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let field = cx.new(|cx| {
            let mut field = TextField::new(Type::Ui, if sent { "" } else { "What should change here?" }, cx).multiline(6);
            field.set_text(body, cx);
            field
        });
        let enter = cx.subscribe(&field, |pane, _, event: &TextFieldEvent, cx| {
            if matches!(event, TextFieldEvent::Enter | TextFieldEvent::SendNow) {
                pane.save_comment(cx);
            }
        });
        if let Some(window) = window {
            field.update(cx, |field, cx| field.focus(window, cx));
        }
        self.hover = None;
        self.editing = Some(Editing { artboard, rev, anchor, id, sent, body: body.to_string(), field, error: None, _enter: enter });
        cx.notify();
    }

    /// Opens a pinned comment: to reword a draft, or to read or delete a sent one.
    pub(super) fn open_comment(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(comment) = self.current.as_ref().and_then(|canvas| self.comment_list.get(canvas)).and_then(|list| list.iter().find(|c| c.id == id)).cloned() else {
            return;
        };
        self.selected = Some(comment.artboard.clone());
        let sent = comment.state != State::Draft;
        self.open_editor(comment.artboard, comment.rev, comment.anchor, Some(comment.id), &comment.body, sent, Some(window), cx);
    }

    fn save_comment(&mut self, cx: &mut Context<Self>) {
        let (Some(canvas), Some(editing)) = (self.current.clone(), self.editing.as_mut()) else { return };
        if editing.sent {
            self.editing = None;
            cx.notify();
            return;
        }
        let body = editing.field.read(cx).text().to_string();
        let saved = match &editing.id {
            Some(id) => self.canvases.edit_comment(&canvas, id, Some(&body)),
            None => self.canvases.add_comment(&canvas, &editing.artboard, editing.rev, editing.anchor.clone(), &body).map(|comment| {
                // Its element's markup goes with it to the model, read from a page of the
                // artboard in the background.
                let canvases = self.canvases.clone();
                let canvas = canvas.clone();
                gpui_tokio::Tokio::spawn(cx, async move { canvases.capture_markup(&canvas, &comment.id).await }).detach();
            }),
        };
        match saved {
            Ok(()) => {
                self.editing = None;
                self.refresh_comments();
            }
            Err(error) => editing.error = Some(error),
        }
        cx.notify();
    }

    fn delete_comment(&mut self, cx: &mut Context<Self>) {
        let (Some(canvas), Some(editing)) = (self.current.clone(), self.editing.as_mut()) else { return };
        if let Some(id) = editing.id.clone()
            && let Err(error) = self.canvases.edit_comment(&canvas, &id, None)
        {
            editing.error = Some(error);
            cx.notify();
            return;
        }
        self.editing = None;
        self.refresh_comments();
        cx.notify();
    }

    pub(super) fn close_editor(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        cx.notify();
    }

    /// Reads the comments of every canvas again; a pin newly resolved starts to fade.
    pub(super) fn refresh_comments(&mut self) {
        let mut next = std::collections::HashMap::new();
        for canvas in &self.list {
            let list = self.canvases.comments(&canvas.id);
            if let Some(before) = self.comment_list.get(&canvas.id) {
                for comment in list.iter().filter(|c| c.state == State::Resolved) {
                    if before.iter().any(|b| b.id == comment.id && b.state != State::Resolved) {
                        self.fading.insert((canvas.id.clone(), comment.id.clone()), Instant::now());
                    }
                }
            }
            next.insert(canvas.id.clone(), list);
        }
        self.comment_list = next;
    }

    /// The canvas's open comments, as the switcher lists them: each opens on its artboard.
    pub(super) fn comment_rows(&self, canvas: &Canvas, theme: &Theme, pane: &Entity<CanvasPane>) -> Vec<gpui::AnyElement> {
        let list = self.comment_list.get(&canvas.id).map(Vec::as_slice).unwrap_or_default();
        list.iter()
            .filter(|c| c.state != State::Resolved)
            .map(|comment| {
                let artboard = canvas.artboards.iter().find(|a| a.id == comment.artboard).map_or(comment.artboard.clone(), |a| a.name.clone());
                let state = match (comment.state, comment.detached) {
                    (_, true) => "its element changed",
                    (State::Draft, _) => "draft",
                    _ => "sent",
                };
                let (pane, id, board) = (pane.clone(), comment.id.clone(), comment.artboard.clone());
                let draft = comment.state == State::Draft;
                div()
                    .id(SharedString::from(format!("canvas-comment-row-{}", comment.id)))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .hover(|row| row.bg(theme.hover))
                    .on_click(move |_, window, cx| {
                        pane.update(cx, |pane, cx| {
                            pane.switcher = false;
                            pane.go_to(std::slice::from_ref(&board), cx);
                            pane.open_comment(&id, window, cx);
                        })
                    })
                    .child(
                        div()
                            .flex_none()
                            .size(px(20.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(draft, |pin| pin.bg(theme.accent).text_color(theme.on_accent))
                            .when(!draft, |pin| pin.border_2().border_color(theme.accent_line).text_color(theme.accent_text))
                            .typography(Type::Small)
                            .child(number(comment)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(div().truncate().typography(Type::Ui).text_color(theme.text).child(comment.body.clone()))
                            .child(div().truncate().typography(Type::Small).text_color(theme.text_dim).child(format!("{artboard} · {state}"))),
                    )
                    .tag("canvas-comment-row")
                    .into_any_element()
            })
            .collect()
    }

    /// Open comments of the canvas on screen.
    pub(super) fn open_comments(&self, canvas: &str) -> usize {
        self.comment_list.get(canvas).map_or(0, |list| list.iter().filter(|c| c.state != State::Resolved).count())
    }

    /// The pins, the outline under the pointer and the area being dragged, over the pictures.
    /// True while a pin is fading.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn comment_marks(&self, canvas: &Canvas, board: &Board, camera: Camera, theme: &Theme, still: bool, pane: &Entity<CanvasPane>, layer: &mut Vec<gpui::AnyElement>) -> bool {
        let far = camera.zoom < FAR_ZOOM;
        let on_screen = |artboard: &str, rect: [f64; 4]| {
            let frame = board.frame(artboard)?;
            let world = Rect { x: frame.rect.x + rect[0] as f32, y: frame.rect.y + rect[1] as f32, width: rect[2] as f32, height: rect[3] as f32 };
            Some((camera.to_screen(world), camera.to_screen(frame.rect)))
        };
        // The element under the pointer, and the area being dragged.
        if let Some((artboard, element)) = &self.hover
            && self.editing.is_none()
            && let Some((screen, _)) = on_screen(artboard, [element.x, element.y, element.w, element.h])
        {
            layer.push(outline(screen, theme.accent_line).into_any_element());
            let name = if element.i.is_empty() { element.t.clone() } else { format!("{}#{}", element.t, element.i) };
            layer.push(
                div()
                    .absolute()
                    .left(px(screen.x))
                    .top(px((screen.y - 20.).max(0.)))
                    .px(px(6.))
                    .rounded(px(6.))
                    .bg(theme.accent)
                    .typography(Type::Small)
                    .text_color(theme.on_accent)
                    .whitespace_nowrap()
                    .child(name)
                    .into_any_element(),
            );
        }
        if let Some(region) = &self.region {
            let (x, y) = (region.from.0.min(region.to.0), region.from.1.min(region.to.1));
            let rect = camera.to_screen(Rect { x, y, width: (region.from.0 - region.to.0).abs(), height: (region.from.1 - region.to.1).abs() });
            layer.push(outline(rect, theme.accent).bg(Hsla { a: 0.08, ..theme.accent }).into_any_element());
        }
        if let Some(editing) = &self.editing
            && let Some((screen, _)) = on_screen(&editing.artboard, editing.anchor.rect)
        {
            layer.push(outline(screen, theme.accent).into_any_element());
        }
        if far {
            return false;
        }
        let mut fading = false;
        let list = self.comment_list.get(&canvas.id).map(Vec::as_slice).unwrap_or_default();
        for comment in list {
            let alpha = match comment.state {
                State::Resolved => match self.fading.get(&(canvas.id.clone(), comment.id.clone())) {
                    Some(since) if !still && since.elapsed() < FADE => {
                        fading = true;
                        1. - since.elapsed().as_secs_f32() / FADE.as_secs_f32()
                    }
                    _ => continue,
                },
                _ => 1.,
            };
            let Some((screen, artboard)) = on_screen(&comment.artboard, comment.anchor.rect) else { continue };
            // At the element's top right, kept on the artboard.
            let x = (screen.x + screen.width - PIN / 2.).clamp(artboard.x - PIN / 2., artboard.x + artboard.width - PIN / 2.);
            let y = (screen.y - PIN / 2.).clamp(artboard.y - PIN / 2., artboard.y + artboard.height - PIN / 2.);
            let draft = comment.state == State::Draft;
            let (fill, ink, ring) = match (draft, comment.detached) {
                (true, _) => (theme.accent, theme.on_accent, theme.raised),
                (false, false) => (theme.raised, theme.accent_text, theme.accent_line),
                (false, true) => (theme.raised, theme.text_faint, theme.border_strong),
            };
            let fade = |colour: Hsla| Hsla { a: colour.a * alpha, ..colour };
            let state = match (comment.state, comment.detached) {
                (State::Resolved, _) => "Resolved",
                (_, true) => "Its element has changed",
                (State::Draft, _) => "Draft · goes with your next message",
                (State::Sent, _) => "Sent · waiting for the agent",
            };
            let id = comment.id.clone();
            let pane = pane.clone();
            layer.push(
                div()
                    .id(SharedString::from(format!("canvas-pin-{}", comment.id)))
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .size(px(PIN))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(fade(fill))
                    .border_2()
                    .border_color(fade(ring))
                    .shadow_sm()
                    .cursor_pointer()
                    .typography(Type::Small)
                    .text_color(fade(ink))
                    .tooltip(tip(format!("{} — {state}", comment.body)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| pane.update(cx, |pane, cx| pane.open_comment(&id, window, cx)))
                    .child(number(comment))
                    .tag("canvas-pin")
                    .into_any_element(),
            );
        }
        fading
    }

    /// The editor, beside its element and inside the pane.
    pub(super) fn comment_editor(&self, canvas: &Canvas, board: &Board, camera: Camera, size: (f32, f32), theme: &Theme, pane: &Entity<CanvasPane>) -> Option<impl IntoElement> {
        let editing = self.editing.as_ref()?;
        let frame = board.frame(&editing.artboard)?;
        let [x, y, w, h] = editing.anchor.rect;
        let screen = camera.to_screen(Rect { x: frame.rect.x + x as f32, y: frame.rect.y + y as f32, width: w as f32, height: h as f32 });
        let (width, height) = size;
        // Beside the element where there is room, else under or over it: never on it.
        let (gap, top_room) = (12., EDGE + 44.);
        let (left, top) = if screen.x + screen.width + gap + EDITOR_WIDTH <= width - EDGE {
            (screen.x + screen.width + gap, screen.y)
        } else if screen.x - gap - EDITOR_WIDTH >= EDGE {
            (screen.x - gap - EDITOR_WIDTH, screen.y)
        } else if screen.y + screen.height + gap + EDITOR_HEIGHT <= height - EDGE {
            (screen.x, screen.y + screen.height + gap)
        } else {
            (screen.x, screen.y - gap - EDITOR_HEIGHT)
        };
        let left = left.min(width - EDITOR_WIDTH - EDGE).max(EDGE);
        let top = top.min(height - EDITOR_HEIGHT - EDGE).max(top_room);
        let artboard = canvas.artboards.iter().find(|a| a.id == editing.artboard).map(|a| a.name.clone()).unwrap_or_default();
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .h(px(26.))
                .px(px(12.))
                .rounded_full()
                .flex()
                .items_center()
                .cursor_pointer()
                .typography(Type::Small)
                .when(primary, |button| button.bg(theme.accent).text_color(theme.on_accent).hover(|button| button.bg(theme.accent_hover)))
                .when(!primary, |button| button.text_color(theme.text_soft).hover(|button| button.bg(theme.hover)))
                .child(label)
        };
        let save = {
            let pane = pane.clone();
            move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut gpui::App| pane.update(cx, |pane, cx| pane.save_comment(cx))
        };
        let close = {
            let pane = pane.clone();
            move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut gpui::App| pane.update(cx, |pane, cx| pane.close_editor(cx))
        };
        let delete = {
            let pane = pane.clone();
            move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut gpui::App| pane.update(cx, |pane, cx| pane.delete_comment(cx))
        };
        let title = match (&editing.id, editing.sent) {
            (None, _) => "New comment".to_string(),
            (Some(id), true) => format!("Comment {} · sent", id.trim_start_matches('k')),
            (Some(id), false) => format!("Comment {} · draft", id.trim_start_matches('k')),
        };
        let body: gpui::AnyElement = if editing.sent {
            div().typography(Type::Ui).text_color(theme.text).child(editing.body.clone()).into_any_element()
        } else {
            div().px(px(10.)).py(px(8.)).rounded(px(10.)).bg(theme.hover).child(editing.field.clone()).into_any_element()
        };
        Some(
            div()
                .id("canvas-comment-editor")
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(EDITOR_WIDTH))
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .rounded(px(14.))
                .bg(theme.raised)
                .border_1()
                .border_color(theme.border)
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().typography(Type::UiStrong).text_color(theme.text).child(title))
                        .child(div().truncate().typography(Type::Small).text_color(theme.text_dim).child(format!("{artboard} · {}", editing.anchor.describe()))),
                )
                .child(body)
                .when_some(editing.error.clone(), |editor, error| editor.child(div().typography(Type::Small).text_color(theme.danger).child(error)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .when(editing.id.is_some(), |row| row.child(button("canvas-comment-delete", "Delete", false).on_click(delete).tag("canvas-comment-delete")))
                        .child(div().flex_1())
                        .when(!editing.sent, |row| {
                            row.child(div().typography(Type::Small).text_color(theme.text_faint).child("↩ to save"))
                                .child(button("canvas-comment-cancel", "Cancel", false).on_click(close.clone()).tag("canvas-comment-cancel"))
                                .child(button("canvas-comment-save", if editing.id.is_some() { "Save" } else { "Comment" }, true).on_click(save).tag("canvas-comment-save"))
                        })
                        .when(editing.sent, |row| row.child(button("canvas-comment-close", "Close", true).on_click(close).tag("canvas-comment-close"))),
                )
                .tag("canvas-comment-editor"),
        )
    }

    /// The tool's hint while it is on and nothing is being written.
    pub(super) fn comment_hint(&self, theme: &Theme, canvas: &str) -> Option<impl IntoElement> {
        (self.commenting && self.editing.is_none()).then(|| {
            let open = self.open_comments(canvas);
            let count = match open {
                0 => String::new(),
                1 => " · 1 open".to_string(),
                n => format!(" · {n} open"),
            };
            div()
                .absolute()
                .bottom(px(EDGE))
                .left(px(EDGE))
                .h(px(32.))
                .px(px(14.))
                .rounded_full()
                .flex()
                .items_center()
                .gap(px(8.))
                .bg(theme.raised)
                .border_1()
                .border_color(theme.border)
                .shadow_sm()
                .child(Icon::Comment.view(px(12.), theme.accent_text))
                .child(div().typography(Type::Small).text_color(theme.text_soft).whitespace_nowrap().child(format!("Click an element to comment, or drag over an area · esc to stop{count}")))
                .tag("canvas-comment-hint")
        })
    }
}

fn outline(rect: Rect, colour: Hsla) -> gpui::Div {
    div().absolute().left(px(rect.x - 1.)).top(px(rect.y - 1.)).w(px(rect.width + 2.)).h(px(rect.height + 2.)).rounded(px(3.)).border_2().border_color(colour)
}
