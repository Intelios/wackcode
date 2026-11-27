//! How many frames the window keeps in flight.
//!
//! zui gives a window's Metal layer two drawables: one on screen and one being drawn. With two, a
//! frame can only start once the one before it is on screen, so when the CPU's draw and the GPU's
//! work together run past one refresh, the next frame waits a whole refresh and motion drops to
//! half rate in bursts. A third lets the CPU draw the next frame while the GPU and the window
//! server finish the last, as Zed does, for one more window-sized surface (about 28 MB at
//! 1680×1050, 2x). On an M1 it cut late frames while scrolling by about a quarter (2026-11-27);
//! the rest was the GPU's own cost (docs/gpui.md, Costs worth knowing).
//!
//! Invariants:
//! - Call [`keep_three`] once the window has its view. GPUI sets the count only when it makes
//!   the layer, so nothing later puts it back.
//! - Only the main window's layer is changed; the browser's overlay plane keeps its own.

use gpui::Window;
use objc2::runtime::AnyObject;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Lets the window's Metal layer hold three drawables instead of GPUI's two.
pub fn keep_three(window: &Window) {
    // SAFETY: `layer` is the window's live CAMetalLayer, and the count is an NSUInteger in 2...3.
    with_layer(window, |layer| unsafe {
        let _: () = objc2::msg_send![layer, setMaximumDrawableCount: 3usize];
    });
}

/// How many drawables the window's Metal layer holds, for the automation socket's frame report.
pub fn drawables(window: &Window) -> Option<usize> {
    // SAFETY: `layer` is the window's live CAMetalLayer.
    with_layer(window, |layer| unsafe { objc2::msg_send![layer, maximumDrawableCount] })
}

fn with_layer<R>(window: &Window, act: impl FnOnce(&AnyObject) -> R) -> Option<R> {
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw() else { return None };
    // SAFETY: `ns_view` is the window's live content view, whose backing layer GPUI sets to its
    // CAMetalLayer (`setLayer:` in gpui_macos); `layer` is an NSView selector. The layer is asked
    // whether it takes CAMetalLayer's selectors before the caller sends them (the Metal HUD, for
    // one, puts a layer of its own class there).
    unsafe {
        let view = &*(handle.ns_view.as_ptr() as *const AnyObject);
        let layer: *const AnyObject = objc2::msg_send![view, layer];
        let layer = layer.as_ref()?;
        let metal: bool = objc2::msg_send![layer, respondsToSelector: objc2::sel!(setMaximumDrawableCount:)];
        metal.then(|| act(layer))
    }
}

