//! macOS permissions for computer use: Accessibility (read and operate other apps) and Screen
//! Recording (capture their windows). Both belong to WackCode.app as a whole — which is why
//! the README warns that anything WackCode runs inherits them — and an ad-hoc-signed build
//! loses them whenever its code signature changes, so status reports the *effective* state and
//! Settings offers a reset.

use objc2::runtime::AnyClass;
use objc2_application_services::{kAXTrustedCheckOptionPrompt, AXIsProcessTrusted, AXIsProcessTrustedWithOptions};
use objc2_core_foundation::{CFBoolean, CFDictionary, CFString, CFType};
use objc2_core_graphics::{CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess};
use std::process::Command;

/// Computer use needs `SCScreenshotManager` (macOS 14+). ScreenCaptureKit is weak-linked
/// (`build.rs`), so this check is safe on older systems.
pub fn is_supported() -> bool {
    AnyClass::get(c"SCScreenshotManager").is_some()
}

pub fn accessibility() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Asks macOS to show its Accessibility prompt, which also lists WackCode in the pane.
pub fn request_accessibility() -> bool {
    let key: &CFString = unsafe { kAXTrustedCheckOptionPrompt };
    let value: &CFType = CFBoolean::new(true).as_ref();
    let options = CFDictionary::<CFString, CFType>::from_slices(&[key], &[value]);
    unsafe { AXIsProcessTrustedWithOptions(Some(options.as_opaque())) }
}

/// The cached Screen Recording answer. It can stay true after a grant stops applying, so
/// status also runs a live ScreenCaptureKit probe (`capture::probe`).
pub fn screen_recording_preflight() -> bool {
    CGPreflightScreenCaptureAccess()
}

/// Asks macOS to show its Screen Recording prompt, which also lists WackCode in the pane.
pub fn request_screen_recording() -> bool {
    CGRequestScreenCaptureAccess()
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Pane {
    Accessibility,
    ScreenRecording,
}

impl Pane {
    fn url(self) -> &'static str {
        match self {
            Self::Accessibility => "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            Self::ScreenRecording => "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
        }
    }

    fn tcc_service(self) -> &'static str {
        match self {
            Self::Accessibility => "Accessibility",
            Self::ScreenRecording => "ScreenCapture",
        }
    }
}

/// Opens the System Settings pane. Only these two fixed URLs are ever opened.
pub fn open_pane(pane: Pane) -> Result<(), String> {
    let status = Command::new("open").arg(pane.url()).status().map_err(|error| format!("System Settings couldn't be opened: {error}"))?;
    if status.success() { Ok(()) } else { Err("System Settings couldn't be opened.".into()) }
}

/// Removes WackCode's own entries from both panes (`tccutil reset`, scoped to its bundle id), so
/// a stale grant from an earlier build can be replaced by a fresh one.
pub fn reset(identifier: &str) -> Result<(), String> {
    for pane in [Pane::Accessibility, Pane::ScreenRecording] {
        let output = Command::new("/usr/bin/tccutil")
            .args(["reset", pane.tcc_service(), identifier])
            .output()
            .map_err(|error| format!("The permissions couldn't be reset: {error}"))?;
        if !output.status.success() {
            return Err(format!("The {} permission couldn't be reset.", pane.tcc_service()));
        }
    }
    Ok(())
}
