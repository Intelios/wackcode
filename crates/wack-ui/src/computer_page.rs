//! Settings › Computer use: the agent working in the user's other Mac apps.
//!
//! - **Computer use**: the one switch, off until the user turns it on. Code chats only.
//! - **Permissions**: Accessibility and Screen Recording, each with its state as macOS reports it
//!   now and Allow…, which renews a grant a rebuild left stale (`tccutil reset` of WackCode's own
//!   entry only) before macOS asks.
//! - **Always allowed**: apps saved from the agent's card; Ask again takes one back (and every
//!   chat's allow of it, `Computer::set_settings`). Apps are added only from the card.
//! - **Never allowed**: what is refused whatever the user says, then the apps they refused from
//!   the card, each of which may be asked about again.

use gpui::{SharedString, div, prelude::*, px};

use crate::{
    actions::{AskComputerAgain, ForgetNeverAllowed, RequestComputerPermission, ToggleComputerUse},
    icon::Icon,
    settings_kit::{card, dispatch, head, pill, row},
    tags::Tag,
    theme::Theme,
    tools_page::switch,
    typography::{Type, Typography},
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComputerData {
    pub enabled: bool,
    pub accessibility: bool,
    pub screen_recording: bool,
    /// Always allowed: (name, path).
    pub always: Vec<(SharedString, SharedString)>,
    /// The user's own never-allowed bundle ids.
    pub never: Vec<SharedString>,
    pub error: Option<SharedString>,
}

fn permission(label: &'static str, about: &'static str, granted: bool, screen: bool, theme: &Theme) -> gpui::Div {
    let tag = if screen { "computer-allow-screen" } else { "computer-allow-accessibility" };
    let control = if granted {
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(Icon::Check.view(px(13.), theme.success))
            .child(div().typography(Type::Small).text_color(theme.text_soft).child("Allowed"))
            .into_any_element()
    } else {
        pill(tag, "Allow…", None, false, theme).on_click(dispatch(RequestComputerPermission { screen })).tag(tag).into_any_element()
    };
    row(label, Some(about)).control(control).view(theme)
}

/// A line of small print in a card, under a hairline.
fn words(text: &'static str, theme: &Theme) -> gpui::Div {
    div().px(px(18.)).py(px(13.)).border_t_1().border_color(theme.border).typography(Type::Small).text_color(theme.text_dim).child(text)
}

pub fn page(data: ComputerData, theme: &Theme) -> impl IntoElement {
    let toggle = div().id("computer-enabled").cursor_pointer().on_click(dispatch(ToggleComputerUse)).child(switch(data.enabled, theme)).tag("computer-enabled");
    let main = card(None, None, None, theme)
        .child(
            row(
                "Let the agent use your Mac’s apps",
                Some("In Code chats, never in sub-agents. The first time a chat needs an app, a card above the composer asks you. The agent works in the app’s windows in the background, without taking your pointer or keyboard."),
            )
            .first()
            .control(toggle)
            .view(theme),
        )
        .when_some(data.error.clone(), |card, error| card.child(div().px(px(18.)).pb(px(14.)).typography(Type::Ui).text_color(theme.danger).child(error)));

    let permissions = card(Some("macOS permissions"), None, None, theme)
        .child(permission("Accessibility", "To read apps’ windows and work in them.", data.accessibility, false, theme))
        .child(permission("Screen Recording", "To see apps’ windows as pictures. Without it the agent reads windows as text only.", data.screen_recording, true, theme))
        .child(words("macOS may forget these after WackCode is rebuilt: Allow… asks again. Screen Recording takes effect once WackCode is reopened.", theme));

    let mut always = card(Some("Always allowed"), None, None, theme);
    if data.always.is_empty() {
        always = always.child(words("Apps you choose Always allow for on the agent’s card show here.", theme));
    }
    for (index, (name, path)) in data.always.iter().enumerate() {
        let again = pill(SharedString::from(format!("computer-ask-again-{index}")), "Ask again", None, false, theme)
            .on_click(dispatch(AskComputerAgain { path: path.to_string() }))
            .tag("computer-ask-again");
        always = always.child(row(name.clone(), None).about(path.clone()).control(again).view(theme));
    }

    let mut never = card(Some("Never allowed"), None, None, theme).child(words(
        "Always refused: terminals and script runners, installers, password managers and Keychain, security tools, system UI and System Settings, and WackCode itself. Web browsers are look-only: the agent works on the web in its own browser.",
        theme,
    ));
    if data.never.is_empty() {
        never = never.child(words("Apps you choose Never allow for on the agent’s card show here too.", theme));
    }
    for (index, bundle_id) in data.never.iter().enumerate() {
        let forget = pill(SharedString::from(format!("computer-forget-never-{index}")), "Let it ask", None, false, theme)
            .on_click(dispatch(ForgetNeverAllowed { bundle_id: bundle_id.to_string() }))
            .tag("computer-let-ask");
        never = never.child(row(bundle_id.clone(), None).control(forget).view(theme));
    }

    div()
        .flex()
        .flex_col()
        .child(head("Computer use", "The agent working in your other Mac apps, through Accessibility, while you keep using your Mac. It asks before it uses each one.", None, theme))
        .child(main)
        .child(div().pt(px(18.)).child(permissions))
        .child(div().pt(px(18.)).child(always))
        .child(div().pt(px(18.)).child(never))
}
