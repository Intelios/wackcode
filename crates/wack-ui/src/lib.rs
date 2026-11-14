//! The view layer: GPUI views, the pane tree and its layout solver, the theme and components.
//!
//! This is the only crate (with the app binaries) that depends on GPUI.

pub mod actions;
pub mod app;
#[cfg(feature = "automation")]
pub mod automation;
pub mod chat;
pub mod colour;
pub mod composer;
pub mod conversation;
pub mod fixtures;
pub mod highlight;
pub mod icon;
pub mod markdown;
pub mod scenes;
pub mod settings;
pub mod shell;
pub mod sidebar;
pub mod specimen;
pub mod tags;
pub mod text_field;
pub mod theme;
pub mod transcript;
pub mod typography;
pub mod work;

use gpui::App;

/// Sets up everything the views need before the first window opens.
pub fn init(cx: &mut App) {
    theme::init(cx);
    text_field::init(cx);
    actions::init(cx);
    gpui_tokio::init(cx);
}
