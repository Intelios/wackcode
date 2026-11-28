//! Settings › Sub-agents › Automatic titles and Step summaries: version 2's auto-title card, and
//! its twin for the labels of settled steps, beside the helper model (all small requests a chat
//! makes beside its own). One card, worded for its [`Helper`].
//!
//! The model is picked from a list that opens inside the card, one row per model of each
//! connection that is on; the switch cannot go on until one is picked. The engine checks the
//! choice again when the switch goes on (`Store::set_auto_title`, `Store::set_step_summary`), and
//! each request checks it once more, so a model removed later only costs a note in the chat's
//! transcript.

use gpui::{SharedString, div, prelude::*, px};
use wack_core::{AutoTitleConfig, Connection};

use crate::{
    actions::{ChooseSummaryModel, ChooseTitleModel, ToggleAutoTitles, ToggleStepSummaries, ToggleSummaryModels, ToggleTitleModels},
    brand::{self, Brand},
    icon::Icon,
    settings_kit::{card, model_pill, row},
    tags::Tag,
    theme::Theme,
    tools_page::switch,
    tooltip::tip,
    typography::{Type, Typography},
};

/// A model the list offers.
#[derive(Clone, Debug, PartialEq)]
pub struct TitleModel {
    pub connection: String,
    pub connection_name: SharedString,
    pub model: SharedString,
    pub brand: Option<Brand>,
}

/// Which small request the card sets up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Helper {
    /// Automatic titles: names a new chat.
    #[default]
    Titles,
    /// Step summaries: names each settled group of steps.
    Summaries,
}

/// A card's words and the automation socket's names for its parts.
struct Words {
    card: &'static str,
    note: &'static str,
    model: &'static str,
    about: &'static str,
    switch: &'static str,
    first: &'static str,
    switch_tag: &'static str,
    model_tag: &'static str,
    rows: [&'static str; 6],
    more: &'static str,
}

impl Helper {
    fn words(self) -> Words {
        match self {
            Helper::Titles => Words {
                card: "Automatic titles",
                note: "One small request when a chat starts",
                model: "Title model",
                about: "Names each new chat from its first message. A small, fast model is plenty.",
                switch: "Automatic titles",
                first: "Choose a title model first",
                switch_tag: "auto-titles",
                model_tag: "title-model",
                rows: ["title-model-0", "title-model-1", "title-model-2", "title-model-3", "title-model-4", "title-model-5"],
                more: "title-model-more",
            },
            Helper::Summaries => Words {
                card: "Step summaries",
                note: "One small request as each group of steps settles",
                model: "Summary model",
                about: "Sums up the agent’s finished steps in one line, so the chat’s own model never spends on it. A small, fast model is plenty.",
                switch: "Step summaries",
                first: "Choose a summary model first",
                switch_tag: "step-summaries",
                model_tag: "summary-model",
                rows: ["summary-model-0", "summary-model-1", "summary-model-2", "summary-model-3", "summary-model-4", "summary-model-5"],
                more: "summary-model-more",
            },
        }
    }

    fn toggle(self) -> Box<dyn gpui::Action> {
        match self {
            Helper::Titles => Box::new(ToggleAutoTitles),
            Helper::Summaries => Box::new(ToggleStepSummaries),
        }
    }

    fn toggle_models(self) -> Box<dyn gpui::Action> {
        match self {
            Helper::Titles => Box::new(ToggleTitleModels),
            Helper::Summaries => Box::new(ToggleSummaryModels),
        }
    }

    fn choose(self, connection: String, model: String) -> Box<dyn gpui::Action> {
        match self {
            Helper::Titles => Box::new(ChooseTitleModel { connection, model }),
            Helper::Summaries => Box::new(ChooseSummaryModel { connection, model }),
        }
    }
}

/// Sends `action` when clicked.
fn send(action: Box<dyn gpui::Action>) -> impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static {
    move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TitlesData {
    pub helper: Helper,
    pub enabled: bool,
    /// The picked model's id, when it is still listed. The list shows its connection.
    pub chosen: Option<SharedString>,
    /// The picked model's provider logo, when logos are on and recognised.
    pub chosen_brand: Option<Brand>,
    /// The picked one, to tick in the list.
    pub picked: Option<(String, String)>,
    pub models: Vec<TitleModel>,
    pub open: bool,
    pub error: Option<SharedString>,
}

impl TitlesData {
    pub fn new(helper: Helper, config: &AutoTitleConfig, connections: &[Connection], show_logos: bool, open: bool, error: Option<SharedString>) -> Self {
        let models: Vec<TitleModel> = connections
            .iter()
            .filter(|connection| connection.enabled)
            .flat_map(|connection| {
                connection.models.iter().map(move |model| TitleModel {
                    connection: connection.id.clone(),
                    connection_name: connection.name.clone().into(),
                    model: model.id.clone().into(),
                    brand: show_logos.then(|| brand::resolve_for(model, connection)).flatten(),
                })
            })
            .collect();
        let picked = config.connection.clone().zip(config.model.clone());
        let matched = picked.as_ref().and_then(|(connection, model)| {
            models.iter().find(|listed| &listed.connection == connection && listed.model.as_ref() == model)
        });
        let chosen = matched.map(|listed| listed.model.clone());
        let chosen_brand = matched.and_then(|listed| listed.brand);
        Self { helper, enabled: config.enabled, chosen, chosen_brand, picked, models, open, error }
    }
}

pub fn card_view(data: TitlesData, theme: &Theme) -> impl IntoElement {
    let ready = data.chosen.is_some();
    let words = data.helper.words();
    let toggle = div()
        .id(words.switch_tag)
        .when(ready || data.enabled, |control| control.cursor_pointer().on_click(send(data.helper.toggle())))
        .when(!ready && !data.enabled, |control| control.opacity(0.5).tooltip(tip(words.first)))
        .child(switch(data.enabled, theme))
        .tag(words.switch_tag);
    let picker = model_pill(
        words.model_tag,
        data.chosen.clone().unwrap_or_else(|| "Choose…".into()),
        data.chosen_brand,
        data.open,
        theme,
    )
    .max_w(px(260.))
    .on_click(send(data.helper.toggle_models()))
    .tag(words.model_tag);

    let list = data.open.then(|| model_list(&data, &words, theme));
    card(Some(words.card), None, Some(words.note), theme)
        .child(row(words.model, Some(words.about)).first().control(picker).view(theme))
        .children(list)
        .child(row(words.switch, None).control(toggle).view(theme))
        .when_some(data.error, |card, error| {
            card.child(div().px(px(18.)).pb(px(14.)).typography(Type::Ui).text_color(theme.danger).child(error))
        })
}

fn model_list(data: &TitlesData, words: &Words, theme: &Theme) -> impl IntoElement {
    let mut list = div().mx(px(18.)).mb(px(14.)).p(px(6.)).rounded(px(12.)).bg(theme.window).border_1().border_color(theme.border).flex().flex_col();
    if data.models.is_empty() {
        return list.child(
            div().px(px(10.)).py(px(8.)).typography(Type::Ui).text_color(theme.text_dim).child("Switch on a connection with a model to choose one."),
        );
    }
    let mut last: Option<&str> = None;
    for (index, listed) in data.models.iter().enumerate() {
        if last != Some(listed.connection.as_str()) {
            last = Some(listed.connection.as_str());
            list = list.child(
                div().px(px(10.)).pt(px(if index == 0 { 4. } else { 10. })).pb(px(4.)).typography(Type::Label).text_color(theme.text_faint).child(listed.connection_name.clone()),
            );
        }
        let ticked = data.picked.as_ref().is_some_and(|(connection, model)| connection == &listed.connection && model == listed.model.as_ref());
        let action = data.helper.choose(listed.connection.clone(), listed.model.to_string());
        list = list.child(
            div()
                .id((words.model_tag, index))
                .px(px(10.))
                .h(px(30.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .hover(|row| row.bg(theme.hover))
                .on_click(send(action))
                .when_some(listed.brand, |row, brand| row.child(brand.view(px(14.), theme)))
                .child(div().flex_1().min_w_0().truncate().typography(Type::Ui).text_color(theme.text).child(listed.model.clone()))
                .when(ticked, |row| row.child(Icon::Check.view(px(14.), theme.accent_text)))
                // The automation socket takes only fixed names.
                .tag(words.rows.get(index).copied().unwrap_or(words.more)),
        );
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use wack_core::{Model, WireFormat};

    fn connection(id: &str, enabled: bool, models: &[&str]) -> Connection {
        Connection {
            id: id.into(),
            name: id.to_uppercase(),
            format: WireFormat::OpenaiChat,
            base_url: "https://llm.example/v1".into(),
            models: models.iter().map(|id| Model::new(*id, None)).collect(),
            enabled,
        }
    }

    #[test]
    fn lists_only_models_of_connections_that_are_on() {
        let connections = [connection("a", true, &["claude-haiku-5-5", "big"]), connection("b", false, &["other"])];
        let config = AutoTitleConfig { enabled: false, connection: Some("a".into()), model: Some("claude-haiku-5-5".into()) };
        let data = TitlesData::new(Helper::Titles, &config, &connections, true, false, None);
        assert_eq!(data.models.len(), 2);
        assert_eq!(data.chosen.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(data.chosen_brand, Some(Brand::Claude));
        assert_eq!(data.models[0].brand, Some(Brand::Claude));
        assert_eq!(data.models[1].brand, None);

        let hidden = TitlesData::new(Helper::Summaries, &config, &connections, false, false, None);
        assert_eq!(hidden.helper, Helper::Summaries);
        assert_eq!(hidden.chosen_brand, None);
        assert!(hidden.models.iter().all(|listed| listed.brand.is_none()));
    }

    #[test]
    fn a_pick_that_is_gone_shows_as_unchosen() {
        let connections = [connection("a", true, &["small"])];
        let config = AutoTitleConfig { enabled: true, connection: Some("b".into()), model: Some("other".into()) };
        let data = TitlesData::new(Helper::Titles, &config, &connections, true, false, None);
        assert_eq!(data.chosen, None);
        assert_eq!(data.chosen_brand, None);
        assert!(data.enabled);
    }
}
