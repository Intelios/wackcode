//! CEF's `CefAppProtocol` on the app's NSApplication, added at runtime.
//!
//! Chromium requires the application object to report whether it is inside `sendEvent:`
//! (`CrAppProtocol`, `CrAppControlProtocol`). GPUI owns the NSApplication subclass, so rather than
//! fork GPUI this adds the methods to whatever class the shared application is, before CEF starts.
//! Proven in the Phase 0 spike (apps/browser-spike/SPIKE.md on the `browser-spike` branch).

use std::sync::atomic::{AtomicBool, Ordering};

use objc2::{
    ClassType, msg_send,
    runtime::{AnyClass, AnyObject, AnyProtocol, Bool, Imp, Sel},
    sel,
};
use objc2_app_kit::{NSApplication, NSEvent};

static HANDLING_SEND_EVENT: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);

extern "C-unwind" fn is_handling_send_event(_this: &AnyObject, _sel: Sel) -> Bool {
    Bool::new(HANDLING_SEND_EVENT.load(Ordering::Relaxed))
}

extern "C-unwind" fn set_handling_send_event(_this: &AnyObject, _sel: Sel, value: Bool) {
    HANDLING_SEND_EVENT.store(value.as_bool(), Ordering::Relaxed);
}

extern "C-unwind" fn send_event(this: &NSApplication, _sel: Sel, event: &NSEvent) {
    let was = HANDLING_SEND_EVENT.swap(true, Ordering::Relaxed);
    // SAFETY: calls NSApplication's own `sendEvent:` on the shared application.
    unsafe {
        let _: () = msg_send![super(this, NSApplication::class()), sendEvent: event];
    }
    if !was {
        HANDLING_SEND_EVENT.store(false, Ordering::Relaxed);
    }
}

/// Adds the protocol's methods to the shared application's class. Main thread, before CEF starts.
pub fn install() {
    if INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }
    // SAFETY: `sharedApplication` exists once AppKit has started, which it has when the UI asks
    // for a page; the methods added have the type encodings given.
    unsafe {
        let app: *mut AnyObject = msg_send![NSApplication::class(), sharedApplication];
        let Some(app) = app.as_ref() else { return };
        let class = app.class() as *const AnyClass as *mut AnyClass;
        let add = |sel: Sel, imp: Imp, types: &std::ffi::CStr| {
            objc2::ffi::class_addMethod(class, sel, imp, types.as_ptr());
        };
        add(
            sel!(isHandlingSendEvent),
            std::mem::transmute::<extern "C-unwind" fn(&AnyObject, Sel) -> Bool, Imp>(
                is_handling_send_event,
            ),
            c"B@:",
        );
        add(
            sel!(setHandlingSendEvent:),
            std::mem::transmute::<extern "C-unwind" fn(&AnyObject, Sel, Bool), Imp>(
                set_handling_send_event,
            ),
            c"v@:B",
        );
        add(
            sel!(sendEvent:),
            std::mem::transmute::<extern "C-unwind" fn(&NSApplication, Sel, &NSEvent), Imp>(
                send_event,
            ),
            c"v@:@",
        );
        for name in [c"CrAppProtocol", c"CrAppControlProtocol", c"CefAppProtocol"] {
            if let Some(protocol) = AnyProtocol::get(name) {
                objc2::ffi::class_addProtocol(class, protocol);
            }
        }
    }
}
