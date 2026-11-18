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
//! - A page is on screen only while a Browser pane painted its place in the frame that reached
//!   the screen. Anything else (the pane closed, another chat, Settings) parks it, still rendering
//!   for the agent. The bookkeeping runs in presentation callbacks, never in paint, because a
//!   cached view replays its callbacks without painting again: `begin_frame` is painted first in
//!   the window, each pane's `show` marks its page, and `end_frame`, painted last, parks the rest.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use gpui::{App, Bounds, FocusHandle, Pixels, WeakFocusHandle, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use wack_browser::{
    Backend, TabId,
    cef::{EngineConfig, Frame2},
};
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
///
/// It also gives a pressed page's pane the app's focus, so the app's keys (⌘W) act on that pane
/// and the page gets the ones it handles (⌘A): GPUI never hears a press the page handles itself.
/// The engine calls in from inside AppKit's event dispatch, so the focus moves in a task, which
/// runs before the next event.
pub fn attach(window: &Window, cx: &App) {
    if let Some(view) = content_view(window) {
        wack_browser::cef::set_host(view, std::ptr::null_mut());
    }
    let handle = window.window_handle();
    let cx = cx.to_async();
    wack_browser::cef::on_page_press(move |tab| {
        let Some(focus) = PRESENTED.with(|presented| presented.borrow().panes.get(&tab).and_then(WeakFocusHandle::upgrade)) else {
            return;
        };
        cx.spawn(async move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                // The pane itself, not a field in it (the address): that field would take ⌘A.
                if !focus.is_focused(window) {
                    window.focus(&focus, cx);
                }
            });
        })
        .detach();
    });
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

#[derive(Default)]
struct Presented {
    /// Pages placed in the frame being presented.
    marked: HashSet<TabId>,
    /// The focus of the pane that last showed each page.
    panes: HashMap<TabId, WeakFocusHandle>,
    /// Pages on screen after the last frame.
    shown: HashSet<TabId>,
    /// Whether the overlay plane is on, or why it could not be turned on.
    overlay: Option<Result<(), String>>,
}

thread_local! {
    static PRESENTED: RefCell<Presented> = RefCell::default();
    /// A pane tab is being dragged: pages park, so the drop zones under them show.
    static TAB_DRAG: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The shell says whether a pane tab is being dragged, as it renders.
pub fn set_tab_drag(on: bool) {
    TAB_DRAG.with(|drag| drag.set(on));
}

/// Paint first in the window: starts the frame's bookkeeping.
pub fn begin_frame(window: &mut Window) {
    window.on_present(|| PRESENTED.with(|presented| presented.borrow_mut().marked.clear()));
}

/// Paint last in the window: parks every page that was on screen and was not placed this frame.
pub fn end_frame(window: &mut Window) {
    window.on_present(|| {
        let parked: Vec<TabId> = PRESENTED.with(|presented| {
            let mut presented = presented.borrow_mut();
            let parked = presented.shown.difference(&presented.marked).copied().collect();
            presented.shown = presented.marked.clone();
            parked
        });
        for id in parked {
            wack_browser::cef::place(id, None);
        }
    });
}

/// Puts `tab`'s page at `bounds` (window points) when this frame reaches the screen. Call from a
/// paint pass. Errs when the window could not be prepared for pages; the page stays parked then.
pub fn show(window: &mut Window, tab: TabId, pane: &FocusHandle, bounds: Bounds<Pixels>, corner_radius: f32) -> Result<(), String> {
    PRESENTED.with(|presented| presented.borrow_mut().panes.insert(tab, pane.downgrade()));
    let overlay = PRESENTED.with(|presented| presented.borrow().overlay.clone());
    let overlay = match overlay {
        Some(overlay) => overlay,
        None => {
            let overlay = enable_overlay(window);
            PRESENTED.with(|presented| presented.borrow_mut().overlay = Some(overlay.clone()));
            overlay
        }
    };
    overlay?;
    let frame = Frame2 {
        x: f64::from(f32::from(bounds.origin.x)),
        y: f64::from(f32::from(bounds.origin.y)),
        width: f64::from(f32::from(bounds.size.width)),
        height: f64::from(f32::from(bounds.size.height)),
        corner_radius: f64::from(corner_radius),
    };
    window.on_present(move || {
        // Read when the frame is presented, not painted: a cached pane replays this unpainted.
        if TAB_DRAG.with(std::cell::Cell::get) {
            return;
        }
        PRESENTED.with(|presented| presented.borrow_mut().marked.insert(tab));
        wack_browser::cef::place(tab, Some(frame));
    });
    Ok(())
}

/// Takes the keyboard back from a page, before the app moves its own focus to a field. (A press
/// outside the page does this by itself, in the engine.)
pub fn release_page_focus() {
    wack_browser::cef::release_focus();
}

/// Hands keyboard focus to a shown page.
pub fn focus_page(tab: TabId) {
    wack_browser::cef::focus(tab);
}
