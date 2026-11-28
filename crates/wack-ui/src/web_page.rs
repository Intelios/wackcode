//! Settings › Web, after version 2's page: reading pages, searching, and the search providers.
//!
//! - **Reading pages** has the Web Fetch switch and "Read pages through <provider>", which only
//!   DeepSeek and Tavily can do and which needs search on.
//! - **Web Search** has the master switch, the providers (one active, chosen by its radio), what
//!   each answered this month from the local meter, and a tile per preset to add one.
//! - A provider opens in an editor in place of the overview: its name, its address (SearXNG and
//!   Custom), where its key comes from (its own, or for DeepSeek a connection's on the same
//!   origin), Custom's request and answer shapes, its prices for the meter's estimate, and Test
//!   search, which runs the draft as it stands.
//!
//! A key is typed into a masked field, handed to the store once and cleared, as in Settings ›
//! Providers: blank keeps the saved key and, unlike a connection's, nothing ever reads a search
//! key back. The editor's fields live in [`WebState`], owned by the app, so leaving the page keeps
//! what was typed.

use gpui::{AnyElement, App, AppContext, Entity, SharedString, div, prelude::*, px};
use wack_core::web::{
    ProviderSummary, SearchDraft,
    search::{CustomSpec, HttpMethod, KeySource, SearchKind, needs_key, preset, supports_fetch},
};

use crate::{
    actions::{
        AddSearchProvider, AskDeleteSearchProvider, CancelDeleteSearchProvider, ChooseSearchKey, ChooseSearchMethod, ChooseSearchProvider,
        CloseSearchEditor, ConfirmDeleteSearchProvider, EditSearchProvider, ResetWebUsage, SaveSearchProvider, TestSearchProvider, ToggleRouteFetch,
        ToggleWebFetch, ToggleWebSearch,
    },
    button::{Button, Kind as ButtonKind},
    icon::Icon,
    settings::{field, text_area, text_input},
    settings_kit::{card, dispatch, head, pill, row, segmented, staged_head},
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    tools_page::switch,
    typography::{Type, Typography},
};

/// What Test search found, or why it failed.
#[derive(Clone, Debug, PartialEq)]
pub enum TestOutcome {
    Found { elapsed_ms: u64, hits: Vec<(SharedString, SharedString)> },
    Failed(SharedString),
}

/// The provider being added or edited, kept while the user looks elsewhere.
pub struct WebEditor {
    /// The saved provider being edited; `None` while adding one.
    pub editing: Option<String>,
    pub kind: SearchKind,
    pub key_source: KeySource,
    pub method: HttpMethod,
    pub name: Entity<TextField>,
    pub url: Entity<TextField>,
    pub key: Entity<TextField>,
    pub auth_header: Entity<TextField>,
    pub auth_prefix: Entity<TextField>,
    pub query_param: Entity<TextField>,
    pub count_param: Entity<TextField>,
    pub body_template: Entity<TextField>,
    pub results_path: Entity<TextField>,
    pub title_path: Entity<TextField>,
    pub url_path: Entity<TextField>,
    pub snippet_path: Entity<TextField>,
    pub date_path: Entity<TextField>,
    pub price_search: Entity<TextField>,
    pub price_fetch: Entity<TextField>,
    pub error: Option<SharedString>,
    pub testing: bool,
    pub test: Option<TestOutcome>,
    /// Tells one Test search's answer from a later one's.
    pub test_serial: u64,
    pub confirming_delete: bool,
}

impl WebEditor {
    pub fn new(draft: &SearchDraft, cx: &mut App) -> Self {
        let text = |placeholder: &'static str, value: &str, cx: &mut App| {
            let field = cx.new(|cx| TextField::new(Type::Ui, placeholder, cx));
            field.update(cx, |field, cx| field.set_text(value, cx));
            field
        };
        let mono = |placeholder: &'static str, value: &str, cx: &mut App| {
            let field = cx.new(|cx| TextField::new(Type::Mono, placeholder, cx));
            field.update(cx, |field, cx| field.set_text(value, cx));
            field
        };
        let custom = draft.custom.clone().unwrap_or_default();
        let price = |value: Option<f64>| value.map(|price| format!("{price}")).unwrap_or_default();
        let url_placeholder = if draft.kind == SearchKind::Searxng { "https://searx.example.org" } else { "https://api.example.com/search" };
        let template = cx.new(|cx| TextField::new(Type::Mono, r#"{"q": "{{query}}", "size": {{count}}}"#, cx).multiline(4));
        template.update(cx, |field, cx| field.set_text(custom.body_template.as_deref().unwrap_or(""), cx));
        Self {
            editing: draft.id.clone(),
            kind: draft.kind,
            key_source: draft.key_source.clone(),
            method: custom.method,
            name: text("My search", &draft.name, cx),
            url: text(url_placeholder, draft.base_url.as_deref().unwrap_or(""), cx),
            key: cx.new(|cx| TextField::new(Type::Ui, "Paste the key", cx).masked()),
            auth_header: mono("Authorization", custom.auth_header.as_deref().unwrap_or(""), cx),
            auth_prefix: mono("Bearer ", custom.auth_prefix.as_deref().unwrap_or(""), cx),
            query_param: mono("q", custom.query_param.as_deref().unwrap_or(""), cx),
            count_param: mono("count", custom.count_param.as_deref().unwrap_or(""), cx),
            body_template: template,
            results_path: mono("data.results", &custom.results_path, cx),
            title_path: mono("title", &custom.title_path, cx),
            url_path: mono("url", &custom.url_path, cx),
            snippet_path: mono("snippet", custom.snippet_path.as_deref().unwrap_or(""), cx),
            date_path: mono("date", custom.date_path.as_deref().unwrap_or(""), cx),
            price_search: text("0.75", &price(draft.price_search_per_1k), cx),
            price_fetch: text("0.75", &price(draft.price_fetch_per_1k), cx),
            error: None,
            testing: false,
            test: None,
            test_serial: 0,
            confirming_delete: false,
        }
    }

    /// Every field, for the app to watch as they change.
    pub fn fields(&self) -> Vec<Entity<TextField>> {
        vec![
            self.name.clone(),
            self.url.clone(),
            self.key.clone(),
            self.auth_header.clone(),
            self.auth_prefix.clone(),
            self.query_param.clone(),
            self.count_param.clone(),
            self.body_template.clone(),
            self.results_path.clone(),
            self.title_path.clone(),
            self.url_path.clone(),
            self.snippet_path.clone(),
            self.date_path.clone(),
            self.price_search.clone(),
            self.price_fetch.clone(),
        ]
    }

    /// The typed key, if any. Blank keeps the saved one.
    pub fn key(&self, cx: &App) -> Option<String> {
        Some(self.key.read(cx).text().trim().to_string()).filter(|key| !key.is_empty())
    }

    /// What the editor describes, or the first thing to fix. The store checks again on save.
    pub fn draft(&self, cx: &App) -> Result<SearchDraft, String> {
        let text = |field: &Entity<TextField>| field.read(cx).text().trim().to_string();
        let optional = |field: &Entity<TextField>| Some(text(field)).filter(|value| !value.is_empty());
        let price = |field: &Entity<TextField>| -> Result<Option<f64>, String> {
            let value = text(field).trim_start_matches('$').to_string();
            if value.is_empty() {
                return Ok(None);
            }
            value.parse::<f64>().map(Some).map_err(|_| "Enter prices as a number, such as 0.75.".to_string())
        };
        let custom = (self.kind == SearchKind::Custom).then(|| CustomSpec {
            method: self.method,
            // The prefix keeps its spaces: "Bearer " ends in one.
            auth_header: optional(&self.auth_header),
            auth_prefix: Some(self.auth_prefix.read(cx).text().to_string()).filter(|prefix| !prefix.is_empty()),
            query_param: optional(&self.query_param),
            count_param: optional(&self.count_param),
            body_template: optional(&self.body_template),
            results_path: text(&self.results_path),
            title_path: text(&self.title_path),
            url_path: text(&self.url_path),
            snippet_path: optional(&self.snippet_path),
            date_path: optional(&self.date_path),
        });
        Ok(SearchDraft {
            id: self.editing.clone(),
            name: text(&self.name),
            kind: self.kind,
            key_source: self.key_source.clone(),
            base_url: matches!(self.kind, SearchKind::Searxng | SearchKind::Custom).then(|| text(&self.url)),
            custom,
            price_search_per_1k: price(&self.price_search)?,
            price_fetch_per_1k: if supports_fetch(self.kind) { price(&self.price_fetch)? } else { None },
        })
    }

    /// Whether this provider sends a key, as the editor stands.
    fn keyed(&self, cx: &App) -> bool {
        match self.kind {
            SearchKind::Custom => !self.auth_header.read(cx).text().trim().is_empty(),
            kind => needs_key(&wack_core::web::search::SearchProvider {
                id: String::new(),
                name: String::new(),
                kind,
                key_source: KeySource::None,
                base_url: None,
                custom: None,
                price_search_per_1k: None,
                price_fetch_per_1k: None,
            }),
        }
    }
}

/// Settings › Web's state, owned by the app so leaving the page keeps it.
#[derive(Default)]
pub struct WebState {
    pub editor: Option<WebEditor>,
    /// A failure from the overview: a switch or a reset.
    pub error: Option<SharedString>,
}

/// The editor as the page draws it.
#[derive(Clone)]
pub struct EditorData {
    pub new: bool,
    pub kind: SearchKind,
    pub key_source: KeySource,
    pub method: HttpMethod,
    pub keyed: bool,
    pub has_saved_key: bool,
    /// Connections DeepSeek may borrow a key from: id and name.
    pub lenders: Vec<(String, String)>,
    pub fields: EditorFields,
    pub error: Option<SharedString>,
    pub testing: bool,
    pub test: Option<TestOutcome>,
    pub confirming_delete: bool,
}

#[derive(Clone)]
pub struct EditorFields {
    pub name: Entity<TextField>,
    pub url: Entity<TextField>,
    pub key: Entity<TextField>,
    pub auth_header: Entity<TextField>,
    pub auth_prefix: Entity<TextField>,
    pub query_param: Entity<TextField>,
    pub count_param: Entity<TextField>,
    pub body_template: Entity<TextField>,
    pub results_path: Entity<TextField>,
    pub title_path: Entity<TextField>,
    pub url_path: Entity<TextField>,
    pub snippet_path: Entity<TextField>,
    pub date_path: Entity<TextField>,
    pub price_search: Entity<TextField>,
    pub price_fetch: Entity<TextField>,
}

impl EditorData {
    pub fn of(editor: &WebEditor, has_saved_key: bool, lenders: Vec<(String, String)>, cx: &App) -> Self {
        Self {
            new: editor.editing.is_none(),
            kind: editor.kind,
            key_source: editor.key_source.clone(),
            method: editor.method,
            keyed: editor.keyed(cx),
            has_saved_key,
            lenders,
            fields: EditorFields {
                name: editor.name.clone(),
                url: editor.url.clone(),
                key: editor.key.clone(),
                auth_header: editor.auth_header.clone(),
                auth_prefix: editor.auth_prefix.clone(),
                query_param: editor.query_param.clone(),
                count_param: editor.count_param.clone(),
                body_template: editor.body_template.clone(),
                results_path: editor.results_path.clone(),
                title_path: editor.title_path.clone(),
                url_path: editor.url_path.clone(),
                snippet_path: editor.snippet_path.clone(),
                date_path: editor.date_path.clone(),
                price_search: editor.price_search.clone(),
                price_fetch: editor.price_fetch.clone(),
            },
            error: editor.error.clone(),
            testing: editor.testing,
            test: editor.test.clone(),
            confirming_delete: editor.confirming_delete,
        }
    }
}

/// Everything the page shows.
#[derive(Clone, Default)]
pub struct WebData {
    pub fetch: bool,
    pub search: bool,
    pub route_fetch: bool,
    pub providers: Vec<ProviderSummary>,
    /// The names of connections, for "Uses ‘…’ key".
    pub connection_names: Vec<(String, String)>,
    pub editor: Option<EditorData>,
    pub error: Option<SharedString>,
}

impl WebData {
    fn active(&self) -> Option<&ProviderSummary> {
        self.providers.iter().find(|summary| summary.active)
    }
}

pub fn page(data: WebData, theme: &Theme) -> AnyElement {
    if let Some(editor) = &data.editor {
        return editor_page(editor, theme).into_any_element();
    }
    div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .child(staged_head(crate::settings::Section::Web, 
            "Web",
            "Reading public pages and searching the web. Pages are read from this Mac; searches go to the provider you choose. Changes apply from a chat’s next message.",
            None,
            theme,
        ))
        .child(reading(&data, theme))
        .child(searching(&data, theme))
        .children(data.error.clone().map(|error| div().typography(Type::Small).text_color(theme.danger).child(error)))
        .into_any_element()
}

fn toggle(id: &'static str, on: bool, enabled: bool, action: impl gpui::Action, theme: &Theme) -> impl IntoElement {
    div()
        .id(id)
        .when(enabled, |control| control.cursor_pointer().on_click(dispatch(action)))
        .when(!enabled, |control| control.opacity(0.4))
        .child(switch(on, theme))
        .tag(id)
}

fn reading(data: &WebData, theme: &Theme) -> impl IntoElement {
    let active = data.active();
    let can_route = active.is_some_and(|summary| supports_fetch(summary.provider.kind));
    let routing_about: SharedString = match active {
        Some(summary) if can_route && data.search => {
            format!("Every page goes to {}’s own reader instead, which may get past pages that block plain requests. It counts towards its usage.", summary.provider.name).into()
        }
        Some(summary) if can_route => format!("Switch Web Search on to read pages through {}.", summary.provider.name).into(),
        Some(summary) => format!("{} can’t read pages. Only DeepSeek and Tavily can.", summary.provider.name).into(),
        None => "Add DeepSeek or Tavily below to read pages through it.".into(),
    };
    let route_label: SharedString = match active {
        Some(summary) if can_route => format!("Read pages through {}", summary.provider.name).into(),
        _ => "Read pages through a search provider".into(),
    };
    let routable = can_route && data.search && data.fetch;
    card(Some("Reading pages"), None, None, theme)
        .child(
            row("Web Fetch", Some("Reads public web pages the agent has a link to. It never reaches this Mac or your local network, and sends no cookies or sign-ins."))
                .first()
                .control(toggle("web-fetch", data.fetch, true, ToggleWebFetch, theme))
                .view(theme),
        )
        .child(
            div()
                .px(px(18.))
                .py(px(13.))
                .border_t_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .gap(px(18.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().typography(Type::UiStrong).text_color(if routable { theme.text } else { theme.text_dim }).child(route_label))
                        .child(div().typography(Type::Small).text_color(theme.text_dim).child(routing_about)),
                )
                .child(toggle("route-fetch", data.route_fetch && routable, routable, ToggleRouteFetch, theme)),
        )
}

fn searching(data: &WebData, theme: &Theme) -> impl IntoElement {
    let status: SharedString = match data.active() {
        _ if data.providers.is_empty() => "No providers".into(),
        _ if !data.search => "Off".into(),
        None => "Pick a provider".into(),
        Some(summary) if summary.problem.is_some() => format!("{} · needs attention", summary.provider.name).into(),
        Some(summary) => format!("{} · {}", summary.provider.name, plural(summary.usage.searches, "search", "searches")).into(),
    };
    let counted = data.providers.iter().any(|summary| summary.usage.searches + summary.usage.fetches > 0);
    let mut search_card = card(Some("Web Search"), Some(status), None, theme).child(
        row(
            "Web Search",
            Some("Lets the agent search the web through the provider marked below. Queries leave this Mac for that provider, so the agent is told never to put anything private in one."),
        )
        .first()
        .control(toggle("web-search", data.search, !data.providers.is_empty(), ToggleWebSearch, theme))
        .view(theme),
    );
    for summary in &data.providers {
        search_card = search_card.child(provider_row(summary, &data.connection_names, theme));
    }
    search_card
        .child(
            div()
                .px(px(18.))
                .py(px(13.))
                .border_t_1()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(div().typography(Type::Small).text_color(theme.text_dim).child(if data.providers.is_empty() {
                    "Add a provider to search with. DeepSeek, Brave and Tavily need an API key; SearXNG is a search engine you host."
                } else {
                    "Add another provider."
                }))
                .child(
                    div().flex().flex_wrap().gap(px(8.)).children(SearchKind::ALL.iter().map(|kind| {
                        let label = preset(*kind).label;
                        pill(("add-search", *kind as usize), label, Some(Icon::Plus), false, theme).cursor_pointer().on_click(dispatch(AddSearchProvider { kind: *kind }))
                    })),
                ),
        )
        .when(counted, |search_card| {
            search_card.child(
                div()
                    .px(px(18.))
                    .py(px(11.))
                    .border_t_1()
                    .border_color(theme.border)
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(div().flex_1().typography(Type::Small).text_color(theme.text_faint).child("Counts are kept on this Mac, per calendar month. Estimates use the prices you enter for each provider."))
                    .child(pill("reset-web-usage", "Reset counts", None, false, theme).cursor_pointer().on_click(dispatch(ResetWebUsage))),
            )
        })
}

fn plural(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// What a provider answered this month, and what that cost by the user's prices.
pub fn usage_line(summary: &ProviderSummary) -> Option<String> {
    let usage = summary.usage;
    if usage.searches + usage.fetches == 0 {
        return None;
    }
    let mut parts = vec![plural(usage.searches, "search", "searches")];
    if usage.fetches > 0 {
        parts.push(plural(usage.fetches, "page read", "page reads"));
    }
    if usage.failed > 0 {
        parts.push(format!("{} failed", usage.failed));
    }
    let provider = &summary.provider;
    let cost = provider.price_search_per_1k.map(|price| price * usage.billed_searches as f64 / 1000.)
        .into_iter()
        .chain(provider.price_fetch_per_1k.map(|price| price * usage.billed_fetches as f64 / 1000.))
        .reduce(|a, b| a + b);
    if let Some(cost) = cost {
        parts.push(format!("≈ ${cost:.2}"));
    }
    Some(format!("{} this month", parts.join(" · ")))
}

fn key_note(summary: &ProviderSummary, connections: &[(String, String)]) -> String {
    if let Some(problem) = &summary.problem {
        return problem.clone();
    }
    match &summary.provider.key_source {
        KeySource::None => "No key needed".into(),
        KeySource::Own => "Key saved".into(),
        KeySource::Connection { connection } => {
            let name = connections.iter().find(|(id, _)| id == connection).map(|(_, name)| name.as_str()).unwrap_or("a connection");
            format!("Uses ‘{name}’ key")
        }
    }
}

fn provider_row(summary: &ProviderSummary, connections: &[(String, String)], theme: &Theme) -> impl IntoElement {
    let provider = &summary.provider;
    let id = provider.id.clone();
    let radio = div()
        .flex_none()
        .size(px(16.))
        .rounded_full()
        .border_1()
        .border_color(if summary.active { theme.accent } else { theme.border_strong })
        .flex()
        .items_center()
        .justify_center()
        .when(summary.active, |radio| radio.child(div().size(px(8.)).rounded_full().bg(theme.accent)));
    // The kind only when the name doesn't already say it.
    let label = preset(provider.kind).label;
    let mut facts: Vec<String> = (provider.name != label).then(|| label.to_string()).into_iter().collect();
    facts.push(key_note(summary, connections));
    if let Some(usage) = usage_line(summary) {
        facts.push(usage);
    }
    div()
        .id(SharedString::from(format!("search-provider-{}", provider.id)))
        .px(px(18.))
        .py(px(12.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(14.))
        .cursor_pointer()
        .hover(|row| row.bg(theme.hover))
        .on_click(dispatch(ChooseSearchProvider { id: id.clone() }))
        .child(radio)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().typography(Type::UiStrong).text_color(theme.text).child(provider.name.clone()))
                .child(
                    div()
                        .typography(Type::Small)
                        .text_color(if summary.problem.is_some() { theme.warning } else { theme.text_dim })
                        .child(facts.join(" · ")),
                ),
        )
        .child(pill(SharedString::from(format!("edit-search-{}", provider.id)), "Edit", Some(Icon::Pencil), false, theme).cursor_pointer().on_click(dispatch(EditSearchProvider { id })))
        .tag("search-provider")
}

fn editor_page(editor: &EditorData, theme: &Theme) -> impl IntoElement {
    let fields = &editor.fields;
    let back = div()
        .id("back-web")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .pb(px(18.))
        .cursor_pointer()
        .typography(Type::Ui)
        .text_color(theme.text_dim)
        .hover(|link| link.text_color(theme.text))
        .on_click(dispatch(CloseSearchEditor))
        .child(Icon::ChevronRight.view(px(13.), theme.text_faint).with_transformation(gpui::Transformation::rotate(gpui::radians(std::f32::consts::PI))))
        .child("Web")
        .tag("back-web");
    let section = |content: AnyElement| div().px(px(18.)).py(px(13.)).border_t_1().border_color(theme.border).child(content);
    let pair = |left: AnyElement, right: AnyElement| div().flex().gap(px(14.)).child(left).child(right).into_any_element();
    let kind = editor.kind;
    let preset = preset(kind);

    let mut connection_card = card(Some("Connection"), Some(preset.label.into()), None, theme)
        .child(section(field("Name", Some("How Settings and the transcript name it.".into()), text_input(fields.name.clone(), "search-name", theme), theme).into_any_element()));
    if matches!(kind, SearchKind::Searxng | SearchKind::Custom) {
        let (label, hint) = if kind == SearchKind::Searxng {
            ("Instance URL", "Your SearXNG instance, with JSON output switched on in its settings.yml. /search is added if missing.")
        } else {
            ("Search URL", "The endpoint every search is sent to. It must use https:// when it sends a key, unless it is on this Mac.")
        };
        connection_card = connection_card.child(section(field(label, Some(hint.into()), text_input(fields.url.clone(), "search-url", theme), theme).into_any_element()));
    }
    if kind == SearchKind::Custom {
        connection_card = connection_card.child(section(custom_request(editor, theme)));
    }
    if editor.keyed {
        connection_card = connection_card.child(section(key_section(editor, theme)));
    }

    let mut answers = None;
    if kind == SearchKind::Custom {
        answers = Some(
            card(Some("Results"), None, Some("Dotted paths into the answer, with numbers for list items: data.results, items.0.link."), theme)
                .child(section(pair(
                    field("Results list", Some("Blank when the answer is the list.".into()), text_input(fields.results_path.clone(), "search-results-path", theme), theme).into_any_element(),
                    field("Title", None, text_input(fields.title_path.clone(), "search-title-path", theme), theme).into_any_element(),
                )))
                .child(section(pair(
                    field("URL", None, text_input(fields.url_path.clone(), "search-url-path", theme), theme).into_any_element(),
                    field("Snippet", Some("Optional.".into()), text_input(fields.snippet_path.clone(), "search-snippet-path", theme), theme).into_any_element(),
                )))
                .child(section(field("Date", Some("Optional.".into()), text_input(fields.date_path.clone(), "search-date-path", theme), theme).into_any_element())),
        );
    }

    let fetch_price = supports_fetch(kind).then(|| {
        field("Per 1,000 page reads", None, text_input(fields.price_fetch.clone(), "search-price-fetch", theme), theme).into_any_element()
    });
    let search_price = field("Per 1,000 searches", None, text_input(fields.price_search.clone(), "search-price-search", theme), theme).into_any_element();
    let prices = card(Some("Usage estimate"), None, Some("Prices in dollars, for the monthly estimate only. Leave blank to count without an estimate."), theme).child(section(match fetch_price {
        Some(fetch_price) => pair(search_price, fetch_price),
        None => search_price,
    }));

    div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .child(div().flex().flex_col().child(back).child(head(
            if editor.new { "New search provider" } else { "Edit search provider" },
            "Where web_search sends its queries. Only this provider's own endpoint is contacted, and a request it redirects is refused.",
            None,
            theme,
        )))
        .child(connection_card)
        .children(answers)
        .child(prices)
        .children(editor.test.clone().map(|test| test_result(test, theme)))
        .child(footer(editor, theme))
}

fn key_section(editor: &EditorData, theme: &Theme) -> AnyElement {
    let fields = &editor.fields;
    let borrowing = match &editor.key_source {
        KeySource::Connection { connection } => Some(connection.clone()),
        _ => None,
    };
    let mut block = div().flex().flex_col().gap(px(12.));
    if !editor.lenders.is_empty() {
        let mut choices: Vec<(Option<usize>, &'static str)> = vec![(None, "Its own key")];
        // A segmented choice needs static words: the lenders are listed by name beside it.
        choices.push((Some(0), "A connection’s key"));
        let chosen = borrowing.as_ref().map(|_| 0);
        let first = editor.lenders[0].0.clone();
        block = block.child(field(
            "Key",
            Some("A DeepSeek connection in Settings › Providers can lend its key; it is only ever sent to api.deepseek.com.".into()),
            segmented("search-key-source", &choices, chosen, move |choice| Box::new(ChooseSearchKey { connection: choice.map(|_| first.clone()) }), theme),
            theme,
        ));
        if let Some(borrowed) = &borrowing {
            block = block.child(div().flex().flex_wrap().gap(px(8.)).children(editor.lenders.iter().enumerate().map(|(index, (id, name))| {
                pill(("search-lender", index), name.clone(), (id == borrowed).then_some(Icon::Check), id == borrowed, theme)
                    .cursor_pointer().on_click(dispatch(ChooseSearchKey { connection: Some(id.clone()) }))
            })));
        }
    }
    if borrowing.is_none() {
        let hint: SharedString = if editor.has_saved_key { "A key is saved. Leave this blank to keep it.".into() } else { "Kept in WackCode’s secret store on this Mac, and sent only to this provider.".into() };
        block = block.child(field("API key", Some(hint), text_input(fields.key.clone(), "search-key", theme), theme));
    }
    block.into_any_element()
}

fn custom_request(editor: &EditorData, theme: &Theme) -> AnyElement {
    let fields = &editor.fields;
    let pair = |left: AnyElement, right: AnyElement| div().flex().gap(px(14.)).child(left).child(right).into_any_element();
    let mut block = div().flex().flex_col().gap(px(12.)).child(field(
        "Request",
        Some("GET puts the query in the URL; POST sends a JSON body.".into()),
        segmented("search-method", &[(HttpMethod::Get, "GET"), (HttpMethod::Post, "POST")], editor.method, |method| Box::new(ChooseSearchMethod { method }), theme),
        theme,
    ));
    block = match editor.method {
        HttpMethod::Get => block.child(pair(
            field("Query parameter", None, text_input(fields.query_param.clone(), "search-query-param", theme), theme).into_any_element(),
            field("Count parameter", Some("Optional.".into()), text_input(fields.count_param.clone(), "search-count-param", theme), theme).into_any_element(),
        )),
        HttpMethod::Post => block.child(field(
            "Body",
            Some("JSON with {{query}} where the query goes and, optionally, {{count}}.".into()),
            text_area(fields.body_template.clone(), 84., theme).tag("search-body-template"),
            theme,
        )),
    };
    block
        .child(pair(
            field("Key header", Some("Blank for an API without a key.".into()), text_input(fields.auth_header.clone(), "search-auth-header", theme), theme).into_any_element(),
            field("Before the key", Some("Such as “Bearer ”.".into()), text_input(fields.auth_prefix.clone(), "search-auth-prefix", theme), theme).into_any_element(),
        ))
        .into_any_element()
}

fn test_result(test: TestOutcome, theme: &Theme) -> impl IntoElement {
    let body: AnyElement = match test {
        TestOutcome::Failed(message) => div().typography(Type::Ui).text_color(theme.danger).child(message).into_any_element(),
        TestOutcome::Found { elapsed_ms, hits } if hits.is_empty() => div()
            .typography(Type::Ui)
            .text_color(theme.text_dim)
            .child(format!("It answered in {elapsed_ms} ms, with no results for “WackCode”."))
            .into_any_element(),
        TestOutcome::Found { elapsed_ms, hits } => div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().typography(Type::Small).text_color(theme.success).child(format!("Answered in {elapsed_ms} ms")))
            .children(hits.into_iter().map(|(title, host)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().truncate().typography(Type::Ui).text_color(theme.text).child(title))
                    .child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(host))
            }))
            .into_any_element(),
    };
    card(Some("Test search"), None, None, theme).child(div().px(px(18.)).py(px(13.)).border_t_1().border_color(theme.border).child(body)).tag("search-test")
}

fn footer(editor: &EditorData, theme: &Theme) -> impl IntoElement {
    if editor.confirming_delete {
        return div()
            .p(px(14.))
            .rounded(px(12.))
            .bg(theme.window)
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().typography(Type::Ui).text_color(theme.text).child("Delete this provider and its saved key?"))
            .child(Button::new("search-keep", "Keep it", ButtonKind::Secondary).action(CancelDeleteSearchProvider))
            .child(Button::new("search-delete-confirm", "Delete", ButtonKind::Danger).action(ConfirmDeleteSearchProvider))
            .into_any_element();
    }
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .when(!editor.new, |bar| bar.child(Button::new("search-delete", "Delete", ButtonKind::Secondary).action(AskDeleteSearchProvider)))
        .child(
            div()
                .flex_1()
                .typography(Type::Small)
                .text_color(theme.danger)
                .children(editor.error.clone()),
        )
        .child(Button::new("search-test-button", if editor.testing { "Testing…" } else { "Test search" }, ButtonKind::Secondary).action(TestSearchProvider).enabled(!editor.testing))
        .child(Button::new("search-cancel", "Cancel", ButtonKind::Secondary).action(CloseSearchEditor))
        .child(Button::new("search-save", if editor.new { "Add provider" } else { "Save" }, ButtonKind::Primary).action(SaveSearchProvider))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use wack_core::{
        web::search::SearchProvider,
        web_usage::Counters,
    };

    use super::*;

    #[test]
    fn usage_reads_naturally_and_estimates_by_billed_requests() {
        let provider = SearchProvider {
            id: "s".into(),
            name: "DeepSeek".into(),
            kind: SearchKind::Deepseek,
            key_source: KeySource::Own,
            base_url: None,
            custom: None,
            price_search_per_1k: Some(0.75),
            price_fetch_per_1k: Some(0.75),
        };
        let mut summary = ProviderSummary { provider, active: true, problem: None, usage: Counters::default() };
        assert_eq!(usage_line(&summary), None);
        summary.usage = Counters { searches: 1000, fetches: 1, billed_searches: 1000, billed_fetches: 0, failed: 2 };
        assert_eq!(usage_line(&summary).as_deref(), Some("1000 searches · 1 page read · 2 failed · ≈ $0.75 this month"));
    }
}
