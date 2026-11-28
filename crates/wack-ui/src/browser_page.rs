//! Settings › Browser: what WackCode's own browser does for the agent, and its data.
//!
//! - **The agent's tabs**: whether the Browser pane opens when the agent opens a tab.
//! - **New tabs**: the size they open at. Tabs already open keep theirs.
//! - **Browsing data**: one action, behind a confirm: the shared profile (every site's cookies,
//!   storage, cache and history) is deleted. A running browser can't start again in the same
//!   process, so then cookies and saved site logins go at once and the rest at the next launch
//!   (`wack_browser::cef::clear_browsing_data`). The page says which happened.
//! - **Engine**: the bundled Chromium's version (it has no updater: the bundled Chromium is
//!   fixed at the CEF pin in the root Cargo.toml, bumped only as its own task).

use gpui::{SharedString, div, prelude::*, px};
use wack_browser::Viewport;

use crate::{
    actions::{AskClearBrowsingData, CancelClearBrowsingData, ChooseDefaultViewport, ConfirmClearBrowsingData, ToggleAgentOpensPane},
    button::{Button, Kind as ButtonKind},
    icon::Icon,
    settings_kit::{card, dispatch, pill, row, segmented, staged_head},
    tags::Tag,
    theme::Theme,
    tools_page::switch,
    typography::{Type, Typography},
};

/// Where clearing the browsing data stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Clearing {
    #[default]
    Idle,
    /// Asked to clear, waiting for the confirm.
    Confirming,
    /// Everything is gone.
    Cleared,
    /// Cookies are gone; the rest goes when WackCode next starts.
    Pending,
    Failed(SharedString),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BrowserData {
    pub agent_opens_pane: bool,
    pub default_viewport: Viewport,
    pub clearing: Clearing,
    /// "Chromium 156.0.8078.96 (CEF 156.0.30)".
    pub version: SharedString,
    pub error: Option<SharedString>,
}

impl Default for BrowserData {
    fn default() -> Self {
        Self { agent_opens_pane: true, default_viewport: Viewport::Desktop, clearing: Clearing::Idle, version: "Chromium".into(), error: None }
    }
}

const SIZES: [(Viewport, &str); 4] = [(Viewport::Fit, "Fit"), (Viewport::Phone, "Phone"), (Viewport::Tablet, "Tablet"), (Viewport::Desktop, "Desktop")];

pub fn page(data: BrowserData, theme: &Theme) -> impl IntoElement {
    let on = data.agent_opens_pane;
    let opens = div()
        .id("browser-agent-opens-pane")
        .cursor_pointer()
        .on_click(dispatch(ToggleAgentOpensPane))
        .child(switch(on, theme))
        .tag("browser-agent-opens-pane");
    let agent = card(Some("The agent’s tabs"), None, None, theme)
        .child(
            row(
                "Show the Browser pane when the agent opens a tab",
                Some("Off, the agent still browses: its tabs wait in the pane until you open it (⌘⇧B). A tab it opens in a chat you aren’t looking at never takes you there."),
            )
            .control(opens)
            .view(theme),
        )
        .when_some(data.error, |card, error| card.child(div().px(px(18.)).pb(px(14.)).typography(Type::Ui).text_color(theme.danger).child(error)));

    let sizes = card(Some("New tabs"), None, None, theme).child(
        row(
            "Open at",
            Some("Fit follows the pane. Phone, tablet and desktop lay the page out at that device’s size and fit it in the pane. Tabs already open keep theirs."),
        )
        .control(segmented("browser-viewport", &SIZES, data.default_viewport, |viewport| Box::new(ChooseDefaultViewport { viewport }), theme))
        .stacked()
        .view(theme),
    );

    let clear_control = match &data.clearing {
        Clearing::Confirming => div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().child(Button::new("browser-clear-keep", "Keep it", ButtonKind::Secondary).action(CancelClearBrowsingData)).tag("browser-clear-keep"))
            .child(div().child(Button::new("browser-clear-confirm", "Clear", ButtonKind::Danger).action(ConfirmClearBrowsingData)).tag("browser-clear-confirm"))
            .into_any_element(),
        _ => pill("browser-clear", "Clear…", Some(Icon::Trash), false, theme).on_click(dispatch(AskClearBrowsingData)).tag("browser-clear").into_any_element(),
    };
    let outcome: Option<(SharedString, bool)> = match &data.clearing {
        Clearing::Idle => None,
        Clearing::Confirming => Some(("What every site stored here is deleted, sign-ins included. This can’t be undone.".into(), false)),
        Clearing::Cleared => Some(("Cleared: no site has anything stored in WackCode’s browser.".into(), false)),
        Clearing::Pending => Some((
            "Cookies are gone, which signs you out of most sites. The rest (site storage, cache, history) is deleted when WackCode next starts, with anything you sign in to before then; a site that keeps its sign-in in storage stays signed in until then."
                .into(),
            false,
        )),
        Clearing::Failed(message) => Some((message.clone(), true)),
    };
    let data_card = card(Some("Browsing data"), None, None, theme)
        .child(
            row(
                "Clear browsing data",
                Some("Deletes every site’s cookies, storage, cache and history from WackCode’s browser, which every chat shares. Tabs stay open, and chats keep their addresses."),
            )
            .control(clear_control)
            .view(theme),
        )
        .when_some(outcome, |card, (words, failed)| {
            card.child(
                div()
                    .px(px(18.))
                    .pb(px(14.))
                    .typography(Type::Small)
                    .text_color(if failed { theme.danger } else { theme.text_soft })
                    .child(words)
                    .tag("browser-clear-outcome"),
            )
        });

    let engine = card(Some("Engine"), None, None, theme).child(
        row(
            data.version.clone(),
            Some("WackCode’s own copy of Chromium, with no Google services: no sync, no Safe Browsing, no updates of its own. It is updated with WackCode."),
        )
        .control(div().child(Icon::Globe.view(px(18.), theme.text_dim)))
        .view(theme),
    );

    div()
        .flex()
        .flex_col()
        .child(staged_head(crate::settings::Section::Browser, "Browser", "The browser in the Browser pane, which you and the agent share. Its sign-ins are kept in WackCode’s own data folder, apart from your other browsers.", None, theme))
        .child(agent)
        .child(div().pt(px(18.)).child(sizes))
        .child(div().pt(px(18.)).child(data_card))
        .child(div().pt(px(18.)).child(engine))
}
