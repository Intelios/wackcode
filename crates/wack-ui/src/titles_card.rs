//! Settings › Sub-agents › Automatic titles: version 2's auto-title card, beside the helper model
//! (both are small requests a chat makes beside its own).
//!
//! The model is picked from a list that opens inside the card, one row per model of each
//! connection that is on; the switch cannot go on until one is picked. The engine checks the
//! choice again when the switch goes on (`Store::set_auto_title`), and each new chat checks it
//! once more, so a model removed later only costs a note in that chat's transcript.

use gpui::{SharedString, div, prelude::*, px};
use wack_core::{AutoTitleConfig, Connection};

use crate::{
    actions::{ChooseTitleModel, ToggleAutoTitles, ToggleTitleModels},
    brand::{self, Brand},
    icon::Icon,
    settings_kit::{card, dispatch, model_pill, row},
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TitlesData {
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
    pub fn new(config: &AutoTitleConfig, connections: &[Connection], show_logos: bool, open: bool, error: Option<SharedString>) -> Self {
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
        Self { enabled: config.enabled, chosen, chosen_brand, picked, models, open, error }
    }
}

pub fn card_view(data: TitlesData, theme: &Theme) -> impl IntoElement {
    let ready = data.chosen.is_some();
    let toggle = div()
        .id("auto-titles")
        .when(ready || data.enabled, |control| control.cursor_pointer().on_click(dispatch(ToggleAutoTitles)))
        .when(!ready && !data.enabled, |control| control.opacity(0.5).tooltip(tip("Choose a title model first")))
        .child(switch(data.enabled, theme))
        .tag("auto-titles");
    let picker = model_pill(
        "title-model",
        data.chosen.clone().unwrap_or_else(|| "Choose…".into()),
        data.chosen_brand,
        data.open,
        theme,
    )
    .max_w(px(260.))
    .on_click(dispatch(ToggleTitleModels))
    .tag("title-model");

    let list = data.open.then(|| model_list(&data, theme));
    card(Some("Automatic titles"), None, Some("One small request when a chat starts"), theme)
        .child(
            row("Title model", Some("Names each new chat from its first message. A small, fast model is plenty."))
                .first()
                .control(picker)
                .view(theme),
        )
        .children(list)
        .child(row("Automatic titles", None).control(toggle).view(theme))
        .when_some(data.error, |card, error| {
            card.child(div().px(px(18.)).pb(px(14.)).typography(Type::Ui).text_color(theme.danger).child(error))
        })
}

/// Tags for the automation socket, which takes only fixed names.
const ROW_TAGS: [&str; 6] = ["title-model-0", "title-model-1", "title-model-2", "title-model-3", "title-model-4", "title-model-5"];

fn model_list(data: &TitlesData, theme: &Theme) -> impl IntoElement {
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
        let action = ChooseTitleModel { connection: listed.connection.clone(), model: listed.model.to_string() };
        list = list.child(
            div()
                .id(("title-model-row", index))
                .px(px(10.))
                .h(px(30.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .hover(|row| row.bg(theme.hover))
                .on_click(dispatch(action))
                .when_some(listed.brand, |row, brand| row.child(brand.view(px(14.), theme)))
                .child(div().flex_1().min_w_0().truncate().typography(Type::Ui).text_color(theme.text).child(listed.model.clone()))
                .when(ticked, |row| row.child(Icon::Check.view(px(14.), theme.accent_text)))
                .tag(ROW_TAGS.get(index).copied().unwrap_or("title-model-more")),
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
        let data = TitlesData::new(&config, &connections, true, false, None);
        assert_eq!(data.models.len(), 2);
        assert_eq!(data.chosen.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(data.chosen_brand, Some(Brand::Claude));
        assert_eq!(data.models[0].brand, Some(Brand::Claude));
        assert_eq!(data.models[1].brand, None);

        let hidden = TitlesData::new(&config, &connections, false, false, None);
        assert_eq!(hidden.chosen_brand, None);
        assert!(hidden.models.iter().all(|listed| listed.brand.is_none()));
    }

    #[test]
    fn a_pick_that_is_gone_shows_as_unchosen() {
        let connections = [connection("a", true, &["small"])];
        let config = AutoTitleConfig { enabled: true, connection: Some("b".into()), model: Some("other".into()) };
        let data = TitlesData::new(&config, &connections, true, false, None);
        assert_eq!(data.chosen, None);
        assert_eq!(data.chosen_brand, None);
        assert!(data.enabled);
    }
}
