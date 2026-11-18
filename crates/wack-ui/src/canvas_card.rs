//! The canvas card: what a turn drew on a canvas, under its log. One card per canvas the turn
//! changed, with the title, what changed ("Added 2 · Revised 1") and a strip of small pictures of
//! the artboards it touched. A click opens the Canvas pane on them (`ShowCanvas`).
//!
//! - Built from the `canvas` calls' results (`wack_canvas::tools::outcome`), the only record the
//!   transcript keeps, so an old turn's card still says what that turn did.
//! - Its pictures and names are filled in by the chat from its canvases (`ChatSession`), never
//!   read from disk while drawing; a picture not drawn yet shows its name on a quiet tile.
//! - An artboard whose report named a problem carries the danger dot, which stays fixed in every
//!   theme (the accent never carries meaning).

use std::path::PathBuf;

use gpui::{ClickEvent, ObjectFit, SharedString, div, img, prelude::*, px};

use crate::{
    actions::ShowCanvas,
    icon::Icon,
    motion,
    tags::Tag,
    theme::Theme,
    typography::{Type, Typography},
};

/// The most pictures a card shows; the rest are counted.
pub const SHOWN: usize = 6;
/// A tile's usual size; its width follows the artboard's shape.
const TILE: (f32, f32) = (112., 84.);

#[derive(Clone, Debug, PartialEq)]
pub struct CanvasCard {
    pub key: SharedString,
    pub canvas: String,
    pub title: SharedString,
    /// "Started · 3 artboards", "Added 2 · Revised 1".
    pub summary: SharedString,
    pub boards: Vec<Thumb>,
    /// The turn is still running: the canvas may change again.
    pub live: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Thumb {
    pub id: String,
    pub name: SharedString,
    pub picture: Option<PathBuf>,
    /// Width over height, so a phone artboard's tile is tall and a desktop one's wide.
    pub aspect: f32,
    pub problem: bool,
}

/// One turn's canvas calls, folded into a card per canvas, in the order they were first drawn on.
pub fn cards(turn_key: &str, outcomes: &[wack_canvas::tools::Outcome], live: bool) -> Vec<CanvasCard> {
    let mut order: Vec<&str> = Vec::new();
    for outcome in outcomes {
        if !order.contains(&outcome.canvas.as_str()) {
            order.push(&outcome.canvas);
        }
    }
    order
        .into_iter()
        .map(|canvas| {
            let calls: Vec<&wack_canvas::tools::Outcome> = outcomes.iter().filter(|o| o.canvas == canvas).collect();
            let created = calls.iter().any(|o| o.created);
            let mut added: Vec<String> = Vec::new();
            let mut revised: Vec<String> = Vec::new();
            let mut removed: Vec<String> = Vec::new();
            let mut problems: Vec<String> = Vec::new();
            for call in &calls {
                for id in &call.removed {
                    added.retain(|a| a != id);
                    revised.retain(|r| r != id);
                    if !removed.contains(id) {
                        removed.push(id.clone());
                    }
                }
                for id in &call.added {
                    removed.retain(|r| r != id);
                    if !added.contains(id) {
                        added.push(id.clone());
                    }
                }
                for id in &call.revised {
                    if !added.contains(id) && !revised.contains(id) {
                        revised.push(id.clone());
                    }
                }
                // The last report on an artboard is the one that stands.
                for id in call.added.iter().chain(&call.revised) {
                    problems.retain(|p| p != id);
                }
                problems.extend(call.problems.iter().cloned());
            }
            let summary = if created {
                let count = added.len();
                format!("Started · {count} artboard{}", if count == 1 { "" } else { "s" })
            } else {
                [("Added", added.len()), ("Revised", revised.len()), ("Removed", removed.len())]
                    .into_iter()
                    .filter(|(_, n)| *n > 0)
                    .map(|(verb, n)| format!("{verb} {n}"))
                    .collect::<Vec<_>>()
                    .join(" · ")
            };
            let boards = added
                .iter()
                .chain(&revised)
                .map(|id| Thumb { id: id.clone(), name: id.clone().into(), picture: None, aspect: TILE.0 / TILE.1, problem: problems.contains(id) })
                .collect();
            let title = calls.last().map(|o| o.title.clone()).unwrap_or_default();
            CanvasCard { key: format!("{turn_key}-canvas-{canvas}").into(), canvas: canvas.to_string(), title: title.into(), summary: summary.into(), boards, live }
        })
        .collect()
}

/// What a card shows of one artboard: its name and its newest small picture. Reads the disk, so
/// the chat keeps what it finds until the canvases next signal.
pub fn face(canvases: &wack_canvas::Canvases, canvas: &str, id: &str) -> Face {
    let found = canvases.get(canvas).and_then(|found| found.artboards.into_iter().find(|a| a.id == id));
    let drawn = canvases.report(canvas, id).map(|report| report.height as f32);
    let aspect = found
        .as_ref()
        .map(|a| a.width as f32 / a.height.map(|h| h as f32).or(drawn).unwrap_or(a.width as f32 * 0.75).max(1.))
        .unwrap_or(TILE.0 / TILE.1);
    Face { name: found.map_or_else(|| id.to_string(), |a| a.name).into(), picture: canvases.thumbnail(canvas, id), aspect }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub name: SharedString,
    pub picture: Option<PathBuf>,
    pub aspect: f32,
}

impl Thumb {
    pub fn wear(&mut self, face: &Face) {
        self.name = face.name.clone();
        self.picture = face.picture.clone();
        self.aspect = face.aspect;
    }
}

pub fn view(card: &CanvasCard, theme: &Theme) -> impl IntoElement {
    let show = |canvas: String, artboards: Vec<String>| {
        move |_: &ClickEvent, window: &mut gpui::Window, cx: &mut gpui::App| {
            window.dispatch_action(Box::new(ShowCanvas { canvas: canvas.clone(), artboards: artboards.clone() }), cx)
        }
    };
    let all: Vec<String> = card.boards.iter().map(|b| b.id.clone()).collect();
    let problems = card.boards.iter().filter(|b| b.problem).count();
    let mut strip = div().flex().flex_wrap().gap(px(8.)).px(px(14.)).pb(px(12.));
    for board in card.boards.iter().take(SHOWN) {
        let tile = div()
            .id(SharedString::from(format!("{}-{}", card.key, board.id)))
            .relative()
            .flex_none()
            .w(px((TILE.1 * board.aspect).clamp(TILE.1 * 0.45, TILE.0 * 1.6)))
            .h(px(TILE.1))
            .rounded(px(8.))
            .overflow_hidden()
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border)
            .cursor_pointer()
            .hover(|tile| tile.border_color(theme.accent_line))
            .tooltip(crate::tooltip::tip(format!("Show {} on the canvas", board.name)))
            .on_click(show(card.canvas.clone(), vec![board.id.clone()]))
            .child(match &board.picture {
                Some(path) => img(path.clone()).size_full().object_fit(ObjectFit::Fill).into_any_element(),
                None => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(8.))
                    .typography(Type::Small)
                    .text_color(theme.text_faint)
                    .child(div().truncate().child(board.name.clone()))
                    .into_any_element(),
            })
            .when(board.problem, |tile| {
                tile.child(div().absolute().top(px(6.)).right(px(6.)).size(px(8.)).rounded_full().bg(theme.danger).border_1().border_color(theme.surface))
            });
        strip = strip.child(tile);
    }
    let more = card.boards.len().saturating_sub(SHOWN);
    if more > 0 {
        strip = strip.child(
            div()
                .flex_none()
                .h(px(TILE.1))
                .px(px(12.))
                .flex()
                .items_center()
                .typography(Type::Small)
                .text_color(theme.text_dim)
                .child(format!("+{more} more")),
        );
    }
    let mut facts = vec![card.summary.to_string()];
    if problems > 0 {
        facts.push(format!("{problems} to fix"));
    }
    if card.live {
        facts.push("drawing".into());
    }
    let head = div()
        .id(SharedString::from(format!("{}-head", card.key)))
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(14.))
        .py(px(11.))
        .cursor_pointer()
        .on_click(show(card.canvas.clone(), all))
        .child(
            div()
                .flex_none()
                .size(px(28.))
                .rounded_full()
                .bg(theme.accent_wash)
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::Board.view(px(14.), theme.accent_text)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().truncate().typography(Type::UiStrong).text_color(theme.text).child(card.title.clone()))
                .child(div().truncate().typography(Type::Small).text_color(theme.text_dim).child(facts.join(" · "))),
        )
        .child(
            div()
                .flex_none()
                .typography(Type::Small)
                .text_color(theme.accent_text)
                .child("Open canvas"),
        )
        .tag("canvas-card");
    motion::enter(
        card.key.clone(),
        div()
            .rounded(px(14.))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .child(head)
            .when(!card.boards.is_empty(), |card| card.child(strip)),
    )
}

#[cfg(test)]
mod tests {
    use wack_canvas::tools::Outcome;

    use super::*;

    fn call(canvas: &str, created: bool, added: &[&str], revised: &[&str], problems: &[&str]) -> Outcome {
        let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect();
        Outcome { created, canvas: canvas.into(), title: canvas.to_uppercase(), added: ids(added), revised: ids(revised), removed: vec![], problems: ids(problems) }
    }

    #[test]
    fn a_turn_folds_into_one_card_per_canvas() {
        let outcomes = [
            call("settings", true, &["calm", "dense"], &[], &["dense"]),
            call("onboarding", false, &[], &["intro"], &[]),
            call("settings", false, &["bold"], &["dense", "calm"], &[]),
        ];
        let cards = cards("turn-3", &outcomes, false);
        assert_eq!(cards.len(), 2);
        let settings = &cards[0];
        assert_eq!(settings.summary.as_ref(), "Started · 3 artboards");
        let ids: Vec<&str> = settings.boards.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["calm", "dense", "bold"]);
        // The later report on "dense" was clean.
        assert!(settings.boards.iter().all(|b| !b.problem));
        assert_eq!(cards[1].summary.as_ref(), "Revised 1");
    }
}
