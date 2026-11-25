//! The glow that shows the screen is taken: an accent edge around every display and, at the
//! top of the main one, a label saying how to take the screen back.
//!
//! Invariants:
//! - Main thread only; [`show`] and [`hide`] post there from anywhere.
//! - The panels take no input (moving the mouse is how the user takes the screen back, so a
//!   Stop button would be one move too late), are invisible to Accessibility, are never
//!   captured, and are left out of hit tests ([`window_numbers`]).
//! - Still: there is nothing to animate, so Reduce Motion has nothing to change.

use std::{
    cell::RefCell,
    sync::{Mutex, atomic::AtomicBool},
};

use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{
    NSBackingStoreType, NSBezierPath, NSColor, NSFont, NSFontAttributeName, NSFontWeightMedium, NSForegroundColorAttributeName, NSPanel, NSScreen,
    NSStringDrawing, NSView, NSWindowCollectionBehavior, NSWindowSharingType, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSDictionary, NSPoint, NSRect, NSSize, NSString};

use super::main_thread;
use crate::cursor::{Appearance, Rgb};

/// Above menus and the Dock, below the system's shields.
const LEVEL: isize = 101;
const EDGE: f64 = 3.0;
const LABEL: &str = "WackCode is using your Mac  ·  Move the mouse or press ⌃⌥⌘. to take it back";

static NUMBERS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
static SHOWN: AtomicBool = AtomicBool::new(false);

/// The glow's window numbers while it shows: hit tests look through them.
pub fn window_numbers() -> Vec<u32> {
    NUMBERS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[derive(Clone)]
struct Drawing {
    appearance: Appearance,
    label: bool,
    /// Debug frames only: a ground to judge contrast against.
    backdrop: Option<Rgb>,
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "WackCodeScreenGlowView"]
    #[ivars = RefCell<Option<Drawing>>]
    struct GlowView;

    impl GlowView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }
        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool { false }
        #[unsafe(method(accessibilityHitTest:))]
        fn accessibility_hit_test(&self, _point: NSPoint) -> *mut AnyObject { std::ptr::null_mut() }
        #[unsafe(method_id(accessibilityChildren))]
        fn accessibility_children(&self) -> Retained<NSArray<AnyObject>> { NSArray::new() }
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            if let Some(drawing) = self.ivars().borrow().as_ref() { draw(drawing, self.bounds()); }
        }
    }
);

thread_local! { static PANELS: RefCell<Vec<Retained<NSPanel>>> = const { RefCell::new(Vec::new()) }; }

fn colour(rgb: Rgb, alpha: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(rgb[0], rgb[1], rgb[2], alpha)
}

fn draw(drawing: &Drawing, bounds: NSRect) {
    let accent = drawing.appearance.accent;
    if let Some(backdrop) = drawing.backdrop {
        colour(backdrop, 1.0).setFill();
        NSBezierPath::fillRect(bounds);
    }
    // The edge: a crisp line with a soft wash inside it.
    for (inset, width, alpha) in [(EDGE / 2.0, EDGE, 0.95), (EDGE + 3.0, 6.0, 0.22), (EDGE + 8.0, 6.0, 0.1)] {
        let rect = NSRect::new(NSPoint::new(bounds.origin.x + inset, bounds.origin.y + inset), NSSize::new(bounds.size.width - inset * 2.0, bounds.size.height - inset * 2.0));
        let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 10.0, 10.0);
        colour(accent, alpha).setStroke();
        path.setLineWidth(width);
        path.stroke();
    }
    if !drawing.label {
        return;
    }
    let font: Retained<AnyObject> = NSFont::systemFontOfSize_weight(12.0, unsafe { NSFontWeightMedium }).into();
    let ink: Retained<AnyObject> = colour(drawing.appearance.text, 1.0).into();
    let attributes = unsafe { NSDictionary::from_slices(&[NSFontAttributeName, NSForegroundColorAttributeName], &[&*font, &*ink]) };
    let text = NSString::from_str(LABEL);
    let size = unsafe { text.sizeWithAttributes(Some(&attributes)) };
    let (width, height) = (size.width + 40.0, 30.0);
    // Under the menu bar, centred.
    let pill = NSRect::new(NSPoint::new(bounds.origin.x + (bounds.size.width - width) / 2.0, bounds.origin.y + 34.0), NSSize::new(width, height));
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(pill, height / 2.0, height / 2.0);
    colour(drawing.appearance.surface, 0.97).setFill();
    path.fill();
    colour(accent, 0.9).setStroke();
    path.setLineWidth(1.5);
    path.stroke();
    // The dot: the accent, as on the agent's cursor.
    colour(accent, 1.0).setFill();
    NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::new(pill.origin.x + 14.0, pill.origin.y + height / 2.0 - 3.5), NSSize::new(7.0, 7.0))).fill();
    unsafe { text.drawAtPoint_withAttributes(NSPoint::new(pill.origin.x + 28.0, pill.origin.y + (height - size.height) / 2.0), Some(&attributes)) };
}

fn panel(mtm: MainThreadMarker, frame: NSRect, drawing: Drawing) -> Option<Retained<NSPanel>> {
    let panel: Option<Retained<NSPanel>> = unsafe {
        msg_send![NSPanel::alloc(mtm), initWithContentRect: frame,
            styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            backing: NSBackingStoreType::Buffered, defer: false]
    };
    let panel = panel?;
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setTitle(&NSString::from_str(&format!("{} · glow", super::input::OVERLAY_TITLE)));
    panel.setExcludedFromWindowsMenu(true);
    panel.setIgnoresMouseEvents(true);
    panel.setHidesOnDeactivate(false);
    panel.setLevel(LEVEL);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::FullScreenAuxiliary | NSWindowCollectionBehavior::Stationary | NSWindowCollectionBehavior::IgnoresCycle,
    );
    // Debug builds can let screenshots include it, as the cursor does.
    let capturable = cfg!(debug_assertions) && std::env::var_os("WACKCODE_CURSOR_CAPTURABLE").is_some_and(|value| !value.is_empty());
    panel.setSharingType(if capturable { NSWindowSharingType::ReadOnly } else { NSWindowSharingType::None });
    let view = GlowView::alloc(mtm).set_ivars(RefCell::new(Some(drawing)));
    let view: Option<Retained<GlowView>> = unsafe { msg_send![super(view), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), frame.size)] };
    panel.setContentView(Some(&*view?));
    Some(panel)
}

/// Shows the glow on every display, in the theme's colours.
pub fn show(appearance: Appearance) {
    SHOWN.store(true, std::sync::atomic::Ordering::SeqCst);
    main_thread::post(move || {
        let Some(mtm) = MainThreadMarker::new() else { return };
        // A hide posted after this show wins.
        if !SHOWN.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        close_all();
        let mut numbers = Vec::new();
        PANELS.with(|panels| {
            let mut panels = panels.borrow_mut();
            for (index, screen) in NSScreen::screens(mtm).iter().enumerate() {
                let drawing = Drawing { appearance: appearance.clone(), label: index == 0, backdrop: None };
                if let Some(panel) = panel(mtm, screen.frame(), drawing) {
                    panel.orderFrontRegardless();
                    numbers.push(u32::try_from(panel.windowNumber()).unwrap_or(0));
                    panels.push(panel);
                }
            }
        });
        *NUMBERS.lock().unwrap_or_else(|e| e.into_inner()) = numbers;
    });
}

/// Takes the glow down.
pub fn hide() {
    SHOWN.store(false, std::sync::atomic::Ordering::SeqCst);
    main_thread::post(close_all);
}

fn close_all() {
    PANELS.with(|panels| {
        for panel in panels.borrow_mut().drain(..) {
            panel.orderOut(None);
            panel.close();
        }
    });
    NUMBERS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Debug builds: draws the top of the main display's glow, over a dark and a light ground, to
/// PNGs in `dir` (`WACKCODE_CURSOR_FRAMES`, with the cursor's frames).
#[cfg(debug_assertions)]
pub(super) fn render_frames(dir: &std::path::Path, appearance: &Appearance) {
    use objc2_app_kit::NSBitmapImageFileType;
    let Some(mtm) = MainThreadMarker::new() else { return };
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(900.0, 120.0));
    for (ground, rgb) in [("on-dark", [0.125, 0.129, 0.141]), ("on-light", [0.95, 0.95, 0.95])] {
        let drawing = Drawing { appearance: appearance.clone(), label: true, backdrop: Some(rgb) };
        let view = GlowView::alloc(mtm).set_ivars(RefCell::new(Some(drawing)));
        let view: Option<Retained<GlowView>> = unsafe { msg_send![super(view), initWithFrame: frame] };
        let Some(view) = view else { return };
        let bounds = view.bounds();
        let Some(bitmap) = view.bitmapImageRepForCachingDisplayInRect(bounds) else { continue };
        view.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
        if let Some(png) = unsafe { bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) } {
            let _ = std::fs::write(dir.join(format!("glow-{ground}.png")), png.to_vec());
        }
    }
}
