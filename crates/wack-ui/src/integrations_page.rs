//! Settings › Integrations: the apps that read what WackCode writes on this Mac. For now one,
//! TokenTrail, which turns the usage ledger into cost estimates. The switch is the ledger's
//! published off switch; the card shows the ledger's health, read live from the writer.

use std::{path::PathBuf, time::Duration};

use gpui::{div, prelude::*, px};
use wack_core::{UsageStatus, tasks::elapsed};

use crate::{
    actions::{RevealUsageFolder, ToggleUsageRecording},
    icon::Icon,
    settings_kit::{card, dispatch, head, pill, row},
    tags::Tag,
    theme::Theme,
    tools_page::switch,
    typography::{Type, Typography},
};

/// Everything the page shows.
#[derive(Clone, Debug, Default)]
pub struct IntegrationsData {
    pub status: UsageStatus,
}

/// What every record holds, and what never enters one, condensed from the contract
/// (`docs/wackcode-usage-v1.md`).
const RECORDED: [&str; 4] =
    ["Token counts and timing", "Model, provider and purpose", "Project and workspace paths", "Outcome — completed, failed, cancelled"];
const NEVER_RECORDED: [&str; 3] =
    ["Prompts, responses and chat titles", "Credentials and endpoint URLs", "Tool arguments"];

pub fn page(data: IntegrationsData, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .child(head(
            "Integrations",
            "Apps that work alongside WackCode by reading files it writes on this Mac. Nothing is sent anywhere.",
            None,
            theme,
        ))
        .child(token_trail(data.status, theme))
        .child(div().pt(px(18.)).child(scope(theme)))
}

/// TokenTrail's card: the recording switch, the ledger's live health, and the folder it reads.
fn token_trail(status: UsageStatus, theme: &Theme) -> impl IntoElement {
    let UsageStatus { enabled, path, has_history, error, pending, dropped, last_written } = status;
    let state = if error.is_some() {
        "Recording needs attention"
    } else if !enabled {
        "Recording paused"
    } else if last_written.is_some() {
        "Recording usage locally"
    } else {
        "Ready to record usage"
    };
    let last = last_written
        .filter(|_| enabled && error.is_none())
        .map(|written| {
            let ago = crate::chat::now_millis().saturating_sub(written as u64);
            format!("last record {} ago", elapsed(Duration::from_millis(ago)))
        });

    let control = div()
        .id("usage-recording")
        .cursor_pointer()
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleUsageRecording), cx))
        .child(switch(enabled, theme))
        .tag("usage-recording");

    let folder = div()
        .flex_none()
        .when(!has_history, |wrap| wrap.opacity(0.5))
        .when(has_history, |wrap| {
            wrap.child(pill("usage-folder", "Usage folder", Some(Icon::Folder), false, theme).on_click(dispatch(RevealUsageFolder {
                path: PathBuf::from(path.clone()),
            })))
        });

    card(Some("TokenTrail"), Some("local".into()), None, theme)
        .child(
            row(
                "Record TokenTrail usage",
                Some("Token counts, model, timing and the outcome of every request WackCode makes, in a ledger only TokenTrail reads."),
            )
            .control(control)
            .view(theme),
        )
        .child(
            div()
                .px(px(18.))
                .py(px(13.))
                .flex()
                .items_center()
                .gap(px(18.))
                .border_t_1()
                .border_color(theme.border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().typography(Type::UiStrong).text_color(theme.text).child(state))
                        .children(last.map(|last| div().typography(Type::Small).text_color(theme.text_dim).child(last)))
                        .tag("usage-state"),
                )
                .children(has_history.then_some(folder)),
        )
        .children((pending > 0).then(|| note(&format!("{pending} records waiting to be saved."), theme.text_dim, None)))
        .children((dropped > 0).then(|| note(&format!("{dropped} records could not be retained."), theme.danger, Some(Icon::Close))))
        .children(error.map(|error| note(&error, theme.danger, Some(Icon::Close))))
        .child(
            div()
                .px(px(18.))
                .py(px(13.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .border_t_1()
                .border_color(theme.border)
                .child(
                    div()
                        .typography(Type::Small)
                        .text_color(theme.text_dim)
                        .child("Ledger · kept indefinitely, read-only for TokenTrail"),
                )
                .child(div().typography(Type::Mono).text_color(theme.text_soft).child(path))
                .tag("usage-path"),
        )
}

/// One line the card adds below its rows: a queue draining, a count of losses, a failure.
fn note(text: &str, colour: gpui::Hsla, icon: Option<Icon>) -> gpui::Div {
    div()
        .px(px(18.))
        .pb(px(13.))
        .flex()
        .items_center()
        .gap(px(6.))
        .children(icon.map(|icon| icon.view(px(11.), colour)))
        .typography(Type::Small)
        .text_color(colour)
        .child(text.to_owned())
}

/// The scope card: what a record holds, and what never enters one.
fn scope(theme: &Theme) -> impl IntoElement {
    card(Some("What TokenTrail can see"), None, None, theme)
        .child(group("In each record", &RECORDED, true, theme))
        .child(group("Never recorded", &NEVER_RECORDED, false, theme))
}

/// One titled list of the scope card.
fn group(title: &'static str, entries: &[&'static str], recorded: bool, theme: &Theme) -> gpui::Div {
    let mut block = div()
        .px(px(18.))
        .py(px(13.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .border_t_1()
        .border_color(theme.border)
        .child(div().typography(Type::Label).text_color(theme.text_dim).child(title));
    for entry in entries {
        let (icon, colour) = if recorded { (Icon::Check, theme.text_dim) } else { (Icon::Close, theme.text_faint) };
        block = block.child(
            div().flex().items_center().gap(px(8.)).child(icon.view(px(11.), colour)).child(
                div().typography(Type::Small).text_color(theme.text_soft).child(*entry),
            ),
        );
    }
    block
}
