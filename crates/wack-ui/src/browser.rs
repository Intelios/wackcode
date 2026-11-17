//! The app's side of the browser engine (`wack-browser`): making it, and lending it the window.
//!
//! The engine never sees GPUI. This module hands it two raw views of the window: GPUI's content
//! view, which pages live in, and GPUI's overlay view, which they stay beneath so menus, cards and
//! tooltips draw over a page (`Window::enable_scene_overlay`, docs/gpui.md).
//!
//! Invariants:
//! - Making the engine starts nothing: CEF starts the first time a tab is opened.
//! - The overlay plane is turned on only when a page is first shown, so the app draws exactly as
//!   before until the browser is used.

use std::sync::Arc;

use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use wack_browser::{Backend, cef::EngineConfig};
use wack_core::Store;

pub fn engine(store: &Store) -> Arc<dyn Backend> {
    wack_browser::cef::backend(EngineConfig { root: store.browser_folder() })
}

fn content_view(window: &Window) -> Option<*mut std::ffi::c_void> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::AppKit(handle) => Some(handle.ns_view.as_ptr()),
        _ => None,
    }
}

/// Lends the engine the window's view, for pages to live in while parked. Call when the window
/// opens. Without the overlay yet: a parked page is off screen, so nothing needs to draw over it.
pub fn attach(window: &Window) {
    if let Some(view) = content_view(window) {
        wack_browser::cef::set_host(view, std::ptr::null_mut());
    }
}

/// Turns on GPUI's overlay plane and keeps pages beneath it. Call before a page is first shown.
pub fn enable_overlay(window: &Window) -> Result<(), String> {
    window.enable_scene_overlay().map_err(|error| format!("Could not prepare the window for the browser: {error}"))?;
    let view = content_view(window).ok_or("The window has no view for the browser.")?;
    // SAFETY: `view` is the window's live content view; its subviews are read, not changed.
    let overlay = unsafe {
        let view = &*(view as *const objc2_app_kit::NSView);
        view.subviews()
            .iter()
            .find(|sub| sub.class().name().to_str() == Ok("GPUIOverlayView"))
            .map(|sub| objc2::rc::Retained::as_ptr(&sub) as *mut std::ffi::c_void)
    };
    wack_browser::cef::set_host(view, overlay.unwrap_or(std::ptr::null_mut()));
    Ok(())
}
