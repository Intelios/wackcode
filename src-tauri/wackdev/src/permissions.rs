//! macOS permissions for computer use: Accessibility (read and operate other apps) and Screen
//! Recording (capture their windows). Both belong to WackCode.app as a whole — which is why
//! the README warns that anything WackCode runs inherits them — and an ad-hoc-signed build
//! loses them whenever its code signature changes. System Settings may still show the old
//! approval as on; a user-initiated request renews only the denied permission before prompting
//! for this build. Status and tool requests always use macOS's effective answer.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
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

/// `AXIsProcessTrustedWithOptions` alone doesn't replace the code requirement in a stale TCC
/// entry. Renew the requested service before prompting, only when its effective check denied
/// access and the user clicked Allow. Never reset on a status poll or a computer tool call,
/// and never clear the other permission as part of requesting this one.
pub fn request(pane: Pane, identifier: &str, granted: bool) -> Result<(), String> {
    request_if_denied(pane, granted, |pane| reset_pane(identifier, pane), |pane| {
        match pane {
            Pane::Accessibility => { request_accessibility(); }
            Pane::ScreenRecording => { request_screen_recording(); }
        }
    })?;
    open_pane(pane)
}

fn request_if_denied(
    pane: Pane,
    granted: bool,
    reset: impl FnOnce(Pane) -> Result<(), String>,
    prompt: impl FnOnce(Pane),
) -> Result<(), String> {
    if !granted {
        reset(pane)?;
        prompt(pane);
    }
    Ok(())
}

fn reset_pane(identifier: &str, pane: Pane) -> Result<(), String> {
    let output = Command::new("/usr/bin/tccutil")
        .args(["reset", pane.tcc_service(), identifier])
        .output()
        .map_err(|error| format!("The permissions couldn't be reset: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("The {} permission couldn't be reset.", pane.tcc_service()))
    }
}

/// Removes WackCode's own entries from both panes (`tccutil reset`, scoped to its bundle id), so
/// a stale grant from an earlier build can be replaced by a fresh one.
pub fn reset(identifier: &str) -> Result<(), String> {
    for pane in [Pane::Accessibility, Pane::ScreenRecording] {
        reset_pane(identifier, pane)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn denied_permission_is_reset_before_prompting_and_the_other_is_untouched() {
        for pane in [Pane::Accessibility, Pane::ScreenRecording] {
            let calls = RefCell::new(Vec::new());
            request_if_denied(pane, false, |pane| {
                calls.borrow_mut().push(("reset", pane));
                Ok(())
            }, |pane| calls.borrow_mut().push(("prompt", pane))).unwrap();
            assert_eq!(*calls.borrow(), [("reset", pane), ("prompt", pane)]);
        }
    }

    #[test]
    fn a_working_permission_is_not_reset_or_prompted() {
        for pane in [Pane::Accessibility, Pane::ScreenRecording] {
            request_if_denied(pane, true,
                |_| panic!("a working permission must not be reset"),
                |_| panic!("a working permission must not be prompted"),
            ).unwrap();
        }
    }

    #[test]
    fn a_failed_reset_is_reported_without_prompting_against_the_stale_entry() {
        let result = request_if_denied(Pane::Accessibility, false,
            |_| Err("The Accessibility permission couldn't be reset.".into()),
            |_| panic!("a failed reset must not prompt"),
        );
        assert_eq!(result.unwrap_err(), "The Accessibility permission couldn't be reset.");
    }
}
