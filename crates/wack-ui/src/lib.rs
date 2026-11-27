//! The view layer: GPUI views, the pane tree and its layout solver, the theme and components.
//!
//! This is the only crate (with the app binaries) that depends on GPUI.

pub mod actions;
pub mod app;
pub mod attachments;
pub mod appearance_page;
#[cfg(feature = "automation")]
pub mod automation;
pub mod backdrop;
pub mod brand;
pub mod browser;
pub mod button;
pub mod canvas_card;
pub mod changes_card;
pub mod chat;
pub mod chat_area;
pub mod chat_transcript;
pub mod colour;
pub mod commands_page;
pub mod commit;
pub mod compaction_card;
pub mod subagents_page;
pub mod composer;
pub mod computer_card;
pub mod computer_page;
pub mod context_menu;
pub mod conversation;
pub mod copy;
pub mod edge_fade;
pub mod fixtures;
pub mod browser_page;
pub mod git_page;
pub mod glass;
pub mod highlight;
pub mod icon;
pub mod integrations_page;
pub mod markdown;
pub mod mcp_page;
pub mod memory_page;
pub mod model_catalog;
pub mod motion;
pub mod panes;
pub mod pictures;
pub mod pixel_scroll;
pub mod plan_card;
pub mod prompts_page;
pub mod question;
pub mod rewind;
pub mod ring;
pub mod run_button;
pub mod scenes;
pub mod settings;
pub mod settings_kit;
pub mod slash_menu;
pub mod slide;
pub mod shell;
pub mod sidebar;
pub mod stage;
pub mod skills_page;
pub mod specimen;
pub mod suggestion;
pub mod suggestions;
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
pub mod web_page;
pub mod window_state;
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
