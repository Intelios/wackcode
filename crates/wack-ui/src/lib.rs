//! The view layer: GPUI views, the pane tree and its layout solver, the theme and components.
//!
//! This is the only crate (with the app binaries) that depends on GPUI.

pub mod actions;
pub mod app;
pub mod appearance_page;
#[cfg(feature = "automation")]
pub mod automation;
pub mod backdrop;
pub mod button;
pub mod chat;
pub mod colour;
pub mod commands_page;
pub mod composer;
pub mod conversation;
pub mod edge_fade;
pub mod fixtures;
pub mod glass;
pub mod highlight;
pub mod icon;
pub mod integrations_page;
pub mod markdown;
pub mod memory_page;
pub mod motion;
pub mod panes;
pub mod plan_card;
pub mod question;
pub mod run_button;
pub mod scenes;
pub mod settings;
pub mod settings_kit;
pub mod slash_menu;
pub mod slide;
pub mod shell;
pub mod sidebar;
pub mod skills_page;
pub mod specimen;
pub mod tags;
pub mod text_field;
pub mod theme;
pub mod titles_card;
pub mod todo_panel;
pub mod tools_page;
pub mod tooltip;
pub mod transcript;
pub mod typography;
pub mod ultra;
pub mod work;

use gpui::App;

/// Sets up everything the views need before the first window opens.
pub fn init(cx: &mut App) {
    theme::init(cx);
    text_field::init(cx);
    actions::init(cx);
    question::init(cx);
    gpui_tokio::init(cx);
}
