//! The view layer: GPUI views, the pane tree and its layout solver, the theme and components.
//!
//! This is the only crate (with the app binaries) that depends on GPUI.

#[cfg(feature = "automation")]
pub mod automation;
pub mod colour;
pub mod composer;
pub mod fixtures;
pub mod icon;
pub mod scenes;
pub mod shell;
pub mod sidebar;
pub mod specimen;
pub mod tags;
pub mod theme;
pub mod transcript;
pub mod typography;

use gpui::App;

/// Sets up everything the views need before the first window opens.
pub fn init(cx: &mut App) {
    theme::init(cx);
}
