//! The model catalogue in Settings › Providers (`wack_core::catalog`): the suggestions that hang
//! under a model id field as the user types, the Fetch models list, the catalogue's own card on
//! the overview, and the note under a filled row.
//!
//! - Suggestions are the `/` menu's card (`slash_menu::picker_card`) under the field, and borrow
//!   its keys through `TextField::set_completing`: ↑ ↓ move, ↩ or ⇥ pick, ⎋ closes. A click
//!   anywhere else closes them. Picking fills the row; every field stays editable.
//! - Fetch models asks the connection's own address for its list (`HttpProvider::list_models`),
//!   then shows each id with what the catalogue knows; the ticked ones become folded rows.
//! - Refresh is the only time models.dev is contacted (docs/security.md).
//!
//! The state lives on `AppState::catalog` and `ProviderForm`; this file only draws.

use std::sync::Arc;

use gpui::{AnyElement, ElementId, Entity, SharedString, anchored, deferred, div, prelude::*, px};
use wack_core::catalog::Catalog;

use crate::{
    actions::{
        AddFetchedModels, CloseFetchedModels, CloseModelSuggestions, FetchModels, PickModelSuggestion, RefreshCatalog, TickAllFetched,
        ToggleFetchedModel,
    },
    icon::Icon,
    settings::{Fetched, Filled},
    settings_kit::{card, dispatch, pill},
    slash_menu::picker_card,
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    typography::{Type, Typography},
};

/// The catalogue as the app holds it.
#[derive(Default)]
pub struct CatalogState {
    /// Read the first time Settings › Providers needs it, off the main thread.
    pub current: Option<Arc<Catalog>>,
    pub loading: bool,
    pub refreshing: bool,
    /// The last Refresh: what it found, or why it failed.
    pub outcome: Option<Result<SharedString, SharedString>>,
    /// Tells a Refresh's answer from a later one's.
    pub serial: u64,
}

impl CatalogState {
    /// The overview's line about it.
    pub fn line(&self) -> CatalogLine {
        CatalogLine {
            date: self.current.as_ref().map(|catalog| wack_core::catalog::date(catalog.captured).into()),
            built_in: self.current.as_ref().is_none_or(|catalog| wack_core::catalog::is_built_in(catalog)),
            models: self.current.as_ref().map_or(0, |catalog| catalog.model_count()),
            refreshing: self.refreshing,
            outcome: self.outcome.clone(),
        }
    }
}

/// The catalogue's card on the Providers overview.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogLine {
    /// When its copy was read from models.dev; `None` until it has been loaded.
    pub date: Option<SharedString>,
    pub built_in: bool,
    pub models: usize,
    pub refreshing: bool,
    pub outcome: Option<Result<SharedString, SharedString>>,
}

/// The suggestions under one model row's id field.
#[derive(Clone, Debug, PartialEq)]
pub struct SuggestView {
    /// The row they hang under.
    pub row: usize,
    pub rows: Vec<SuggestRow>,
    pub selected: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SuggestRow {
    pub id: SharedString,
    pub name: SharedString,
    pub facts: SharedString,
    pub source: SharedString,
}

/// Fetch models, as the models card shows it.
#[derive(Clone, Default)]
pub enum FetchView {
    #[default]
    Idle,
    Loading {
        host: SharedString,
    },
    Failed(SharedString),
    Listed {
        host: SharedString,
        filter: Entity<TextField>,
        /// The models the filter leaves, up to [`FETCH_SHOWN`].
        rows: Vec<Fetched>,
        /// How many more the filter leaves than are drawn.
        more: usize,
        /// How many the provider listed.
        total: usize,
        ticked: usize,
        /// Untick models the connection doesn't have yet.
        addable: usize,
    },
}

/// The most fetched models drawn at once; the filter narrows the rest.
pub const FETCH_SHOWN: usize = 200;

/// "1,266".
pub fn thousands(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

const SUGGEST_ROW_TAGS: [&str; 8] = [
    "model-suggest-row-0",
    "model-suggest-row-1",
    "model-suggest-row-2",
    "model-suggest-row-3",
    "model-suggest-row-4",
    "model-suggest-row-5",
    "model-suggest-row-6",
    "model-suggest-row-7",
];
const SUGGEST_ROW_HEIGHT: f32 = 46.;

/// The suggestions card, hung under the id field it belongs to (its holder is `relative`).
pub fn suggest_card(view: &SuggestView, theme: &Theme) -> impl IntoElement {
    let mut list = div().flex().flex_col().px(px(6.)).py(px(6.));
    for (index, row) in view.rows.iter().enumerate() {
        let selected = index == view.selected;
        list = list.child(
            div()
                .id(ElementId::Name(format!("model-suggest-row-{index}").into()))
                .flex_none()
                .h(px(SUGGEST_ROW_HEIGHT))
                .px(px(10.))
                .rounded(px(10.))
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(1.))
                .cursor_pointer()
                .when(selected, |row| row.bg(theme.hover))
                .when(!selected, |row| row.hover(|row| row.bg(theme.hover)))
                .on_click(dispatch(PickModelSuggestion { index }))
                // The id gets the long line, its source the short word beside it; the name and
                // what the catalogue knows share the line below.
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(12.))
                        .child(div().flex_1().min_w_0().truncate().typography(Type::Mono).text_color(theme.text).child(row.id.clone()))
                        .child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(row.source.clone())),
                )
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(12.))
                        .child(div().flex_1().min_w_0().truncate().typography(Type::Small).text_color(theme.text_faint).child(row.name.clone()))
                        .child(div().flex_none().typography(Type::Small).text_color(if selected { theme.accent_text } else { theme.text_dim }).child(row.facts.clone())),
                )
                .tag(SUGGEST_ROW_TAGS.get(index).copied().unwrap_or("model-suggest-row")),
        );
    }
    // Anchored at the field's middle, padded by half the field plus the gap on both sides: it
    // hangs just below the field, or, when the window has no room there, flips to sit just above
    // it (GPUI flips about the same point), never over the field itself.
    let pad = FIELD_HEIGHT / 2. + GAP;
    div().absolute().left_0().top(px(FIELD_HEIGHT / 2.)).child(
        deferred(anchored().child(div().py(px(pad)).child(crate::motion::enter(
            ElementId::NamedInteger("model-suggest-arrive".into(), view.row as u64),
            div().min_w(px(440.)).max_w(px(640.)).child(
                picker_card("model-suggest", list, theme)
                    .w_full()
                    .on_mouse_down_out(|_, window, cx| window.dispatch_action(Box::new(CloseModelSuggestions), cx))
                    .tag("model-suggest"),
            ),
        ))))
        .with_priority(2),
    )
}

/// The model id field's height (`settings::text_input`), and the gap between it and the list.
const FIELD_HEIGHT: f32 = 34.;
const GAP: f32 = 6.;

/// The line under a row the catalogue filled: where its facts came from, and anything to check.
pub fn filled_note(filled: &Filled, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .typography(Type::Small)
                .text_color(theme.text_faint)
                .child(Icon::Check.view(px(12.), theme.text_faint))
                .child(format!("From models.dev · {}. Change anything that's different for this endpoint.", filled.source)),
        )
        .when_some(filled.note, |note, text| note.child(div().pl(px(18.)).typography(Type::Small).text_color(theme.warning).child(text)))
}

const FETCH_ROW_TAGS: [&str; 12] = [
    "fetch-model-0",
    "fetch-model-1",
    "fetch-model-2",
    "fetch-model-3",
    "fetch-model-4",
    "fetch-model-5",
    "fetch-model-6",
    "fetch-model-7",
    "fetch-model-8",
    "fetch-model-9",
    "fetch-model-10",
    "fetch-model-11",
];

/// The Fetch models block in the models card, above its footer. `None` while it isn't open.
pub fn fetch_picker(view: &FetchView, theme: &Theme) -> Option<AnyElement> {
    let block = || div().px(px(18.)).py(px(12.)).border_t_1().border_color(theme.border).flex().flex_col().gap(px(10.));
    Some(match view {
        FetchView::Idle => return None,
        FetchView::Loading { host } => block()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .typography(Type::Ui)
                    .text_color(theme.text_dim)
                    .child(Icon::Download.view(px(14.), theme.text_dim))
                    .child(format!("Asking {host} for its models…")),
            )
            .tag("fetch-models-loading")
            .into_any_element(),
        FetchView::Failed(message) => block()
            .child(div().typography(Type::Ui).text_color(theme.danger).child(message.clone()))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(pill("fetch-again", "Try again", Some(Icon::Reload), false, theme).cursor_pointer().on_click(dispatch(FetchModels)))
                    .child(pill("fetch-models-cancel", "Close", None, false, theme).cursor_pointer().on_click(dispatch(CloseFetchedModels)).tag("fetch-models-cancel")),
            )
            .tag("fetch-models-failed")
            .into_any_element(),
        FetchView::Listed { host, filter, rows, more, total, ticked, addable } => {
            let head = div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().typography(Type::UiStrong).text_color(theme.text).child(format!("{} models at {host}", thousands(*total))))
                        .child(div().typography(Type::Small).text_color(theme.text_faint).child("Tick the ones to add. Each is filled from the catalogue where it knows the model.")),
                )
                .child(div().w(px(220.)).flex_none().child(crate::settings::text_input(filter.clone(), "fetch-models-filter", theme)));
            let mut list = div().id("fetch-list").max_h(px(320.)).overflow_y_scroll().flex().flex_col().gap(px(2.));
            if rows.is_empty() {
                list = list.child(div().py(px(8.)).typography(Type::Ui).text_color(theme.text_dim).child("No model matches the filter."));
            }
            for (index, row) in rows.iter().enumerate() {
                list = list.child(fetched_row(index, row, theme));
            }
            if *more > 0 {
                list = list.child(div().py(px(6.)).typography(Type::Small).text_color(theme.text_faint).child(format!("{} more. Filter to narrow.", thousands(*more))));
            }
            let add_label = match ticked {
                0 => "Add models".to_string(),
                1 => "Add 1 model".to_string(),
                count => format!("Add {count} models"),
            };
            let foot = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .when(*addable > 0, |foot| foot.child(pill("fetch-tick-all", "Tick all new", None, false, theme).on_click(dispatch(TickAllFetched)).tag("fetch-tick-all")))
                .child(div().flex_1())
                .child(pill("fetch-models-cancel", "Cancel", None, false, theme).on_click(dispatch(CloseFetchedModels)).tag("fetch-models-cancel"))
                .child(
                    pill("fetch-models-add", add_label, Some(Icon::Plus), *ticked > 0, theme)
                        .when(*ticked > 0, |pill| pill.on_click(dispatch(AddFetchedModels)))
                        .when(*ticked == 0, |pill| pill.opacity(0.5).cursor_default())
                        .tag("fetch-models-add"),
                );
            block().child(head).child(list).child(foot).tag("fetch-models-list").into_any_element()
        }
    })
}

fn fetched_row(index: usize, row: &Fetched, theme: &Theme) -> impl IntoElement {
    let tick = div()
        .flex_none()
        .size(px(18.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .when(row.ticked, |tick| tick.bg(theme.accent_fill()).child(Icon::Check.view(px(11.), theme.on_accent)))
        .when(!row.ticked, |tick| tick.border_1().border_color(theme.border_strong));
    let id = row.id.clone();
    div()
        .id(ElementId::Name(format!("fetched-{index}").into()))
        .flex_none()
        .h(px(34.))
        .px(px(8.))
        .rounded(px(9.))
        .flex()
        .items_center()
        .gap(px(10.))
        .when(!row.present, |line| line.cursor_pointer().hover(|line| line.bg(theme.hover)).on_click(dispatch(ToggleFetchedModel { id })))
        .child(if row.present { Icon::Check.view(px(14.), theme.text_faint).into_any_element() } else { tick.into_any_element() })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .typography(Type::Mono)
                .text_color(if row.present { theme.text_faint } else { theme.text })
                .child(row.id.clone()),
        )
        .child(
            div()
                .flex_none()
                .typography(Type::Small)
                .text_color(theme.text_faint)
                .child(if row.present { SharedString::from("Added") } else { row.facts.clone().unwrap_or_else(|| "Not in the catalogue".into()) }),
        )
        .tag(FETCH_ROW_TAGS.get(index).copied().unwrap_or("fetch-model"))
}

/// The catalogue's card on the Providers overview: which copy is in use, and Refresh.
pub fn catalog_card(line: &CatalogLine, theme: &Theme) -> impl IntoElement {
    let badge: Option<SharedString> = (line.models > 0).then(|| format!("{} models", thousands(line.models)).into());
    let copy: SharedString = match (&line.date, line.built_in) {
        (None, _) => "Reading the catalogue…".into(),
        (Some(date), true) => format!("Built in · from models.dev on {date}").into(),
        (Some(date), false) => format!("Refreshed · from models.dev on {date}").into(),
    };
    let refresh = if line.refreshing {
        pill("catalog-refresh", "Refreshing…", Some(Icon::Reload), false, theme).opacity(0.6).cursor_default()
    } else {
        pill("catalog-refresh", "Refresh", Some(Icon::Reload), false, theme).on_click(dispatch(RefreshCatalog))
    };
    let outcome = line.outcome.clone().map(|outcome| {
        let (text, ink) = match outcome {
            Ok(text) => (text, theme.success),
            Err(text) => (text, theme.danger),
        };
        div().px(px(18.)).pb(px(14.)).mt(px(-4.)).typography(Type::Small).text_color(ink).child(text).tag("catalog-outcome")
    });
    card(Some("Model catalogue"), badge, None, theme)
        .child(
            div()
                .px(px(18.))
                .pt(px(2.))
                .pb(px(14.))
                .flex()
                .items_center()
                .gap(px(14.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(div().typography(Type::Ui).text_color(theme.text_soft).child(copy))
                        .child(div().typography(Type::Small).text_color(theme.text_faint).child(
                            "Type a model's id and pick it to fill its context, max output, reasoning and vision. Refresh asks models.dev for newer models; nothing else is sent.",
                        )),
                )
                .child(refresh.tag("catalog-refresh")),
        )
        .children(outcome)
        .tag("catalog-card")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_with_thousands_marked() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_266), "1,266");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
