//! Native window backdrop: Liquid Glass (`NSGlassEffectView`, macOS 26+) behind the webview,
//! and the window's own opacity. Ported from Media Logger's glass module.
//!
//! The main window is created `transparent` (see `tauri.conf.json`), because WKWebView can
//! only stop drawing its background at creation. Everything here keeps it opaque, painted in
//! the user's background colour, unless Liquid Glass is on *and* the window has focus: a
//! transparent window costs WindowServer 25–40% GPU even while idle and unfocused.
use objc2::runtime::AnyClass;
use objc2::{msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSGlassEffectView, NSGlassEffectViewStyle, NSView, NSWindow,
    NSWindowOrderingMode,
};
use tauri::{AppHandle, Manager};

use crate::models::{AppearanceConfig, BackdropMode, GlassStyleSetting};

/// `DEFAULT_BACKGROUND` in `src/theme.ts`.
const DEFAULT_BACKGROUND: (u8, u8, u8) = (0x11, 0x13, 0x10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlassStyle {
    Frosted,
    Clear,
}

impl GlassStyle {
    /// Values of the private `variant` selector. Frosted is the sidebar material (16).
    fn variant(self) -> isize {
        match self {
            Self::Clear => 1,
            Self::Frosted => 16,
        }
    }
}

/// What the native window should look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeBackdrop {
    /// `None` removes any glass view.
    pub glass: Option<GlassStyle>,
    /// sRGB fill for the opaque window: the user's background colour.
    pub background: (u8, u8, u8),
}

impl NativeBackdrop {
    /// The renderer stores the background it displays (already darkened for readability), so
    /// the native fill matches the page painted over it.
    pub fn from_config(config: &AppearanceConfig) -> Self {
        let glass = (config.backdrop == BackdropMode::Glass).then_some(match config.glass_style {
            GlassStyleSetting::Frosted => GlassStyle::Frosted,
            GlassStyleSetting::Clear => GlassStyle::Clear,
        });
        let background = config.background.as_deref().and_then(parse_hex).unwrap_or(DEFAULT_BACKGROUND);
        Self { glass, background }
    }
}

/// `#rrggbb` to channels.
pub fn parse_hex(value: &str) -> Option<(u8, u8, u8)> {
    let digits = value.strip_prefix('#')?;
    if digits.len() != 6 || !digits.chars().all(|character| character.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |index: usize| u8::from_str_radix(&digits[index..index + 2], 16).ok();
    Some((channel(0)?, channel(2)?, channel(4)?))
}

/// Applies `config` to the main window, asking it whether it has focus.
pub fn sync(app: &AppHandle, config: &AppearanceConfig) -> Result<(), String> {
    let focused = app.get_window("main").and_then(|window| window.is_focused().ok()).unwrap_or(true);
    apply(app, NativeBackdrop::from_config(config), focused)
}

pub fn is_supported() -> bool {
    AnyClass::get(c"NSGlassEffectView").is_some()
}

/// Brings the main window in line with `backdrop`. `focused` decides whether glass shows
/// (transparent window) or the window is painted opaque in the background colour.
pub fn apply(app: &AppHandle, backdrop: NativeBackdrop, focused: bool) -> Result<(), String> {
    // Browser previews make this a multi-webview window. The single-webview
    // convenience lookup stops resolving it after the first preview is created.
    let window = app.get_window("main").ok_or_else(|| "The main window is unavailable".to_string())?;
    let ns_window = window.ns_window().map_err(|error| error.to_string())?;
    if ns_window.is_null() {
        return Err("The native macOS window is unavailable".to_string());
    }
    let backdrop = NativeBackdrop { glass: backdrop.glass.filter(|_| is_supported()), ..backdrop };

    // Raw AppKit objects are main-thread-only. Pass only the pointer address into Tauri's
    // Send closure, then recreate the typed borrow on main.
    let address = ns_window as usize;
    if let Some(mtm) = MainThreadMarker::new() {
        return unsafe { apply_on_main(address, backdrop, focused, mtm) };
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    window
        .run_on_main_thread(move || {
            let result = MainThreadMarker::new()
                .ok_or_else(|| "Tauri did not run the window update on the main thread".to_string())
                .and_then(|mtm| unsafe { apply_on_main(address, backdrop, focused, mtm) });
            let _ = sender.send(result);
        })
        .map_err(|error| error.to_string())?;
    receiver.recv().map_err(|error| format!("The window update did not finish: {error}"))?
}

unsafe fn apply_on_main(address: usize, backdrop: NativeBackdrop, focused: bool, mtm: MainThreadMarker) -> Result<(), String> {
    // SAFETY: Tauri supplied this pointer for the live main window, and the caller proves this
    // function is executing on AppKit's main thread.
    let ns_window = unsafe { &*(address as *const NSWindow) };
    let content_view = ns_window.contentView().ok_or_else(|| "The native macOS window has no content view".to_string())?;

    let opaque = backdrop.glass.is_none() || !focused;
    let existing = content_view.subviews().iter().find_map(|subview| subview.downcast::<NSGlassEffectView>().ok());
    match (backdrop.glass, existing) {
        (Some(style), Some(glass)) => {
            unsafe { set_variant(&glass, style) };
            // Hidden while opaque: otherwise its grey inactive material covers the background colour.
            glass.setHidden(opaque);
        }
        (Some(style), None) => {
            let glass = NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), content_view.bounds());
            glass.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
            unsafe { set_variant(&glass, style) };
            glass.setHidden(opaque);
            content_view.addSubview_positioned_relativeTo(&glass, NSWindowOrderingMode::Below, None::<&NSView>);
        }
        (None, Some(glass)) => glass.removeFromSuperview(),
        (None, None) => {}
    }

    let color = if opaque {
        let (red, green, blue) = backdrop.background;
        NSColor::colorWithSRGBRed_green_blue_alpha(red as f64 / 255.0, green as f64 / 255.0, blue as f64 / 255.0, 1.0)
    } else {
        NSColor::clearColor()
    };
    ns_window.setBackgroundColor(Some(&color));
    ns_window.setOpaque(opaque);
    Ok(())
}

unsafe fn set_variant(view: &NSGlassEffectView, style: GlassStyle) {
    let variant = style.variant();

    // `variant` is an undocumented selector. Keep Media Logger's known-working probe order so
    // Frosted (16) stays visually identical to it.
    let responds: bool = unsafe { msg_send![view, respondsToSelector: sel!(set_variant:)] };
    if responds {
        let _: () = unsafe { msg_send![view, set_variant: variant] };
        return;
    }
    let responds: bool = unsafe { msg_send![view, respondsToSelector: sel!(setVariant:)] };
    if responds {
        let _: () = unsafe { msg_send![view, setVariant: variant] };
        return;
    }

    // If Apple removes the private selector, keep the public Clear style and degrade Frosted
    // to the public Regular style without crashing.
    view.setStyle(match style {
        GlassStyle::Clear => NSGlassEffectViewStyle::Clear,
        GlassStyle::Frosted => NSGlassEffectViewStyle::Regular,
    });
}
