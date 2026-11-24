//! Computer use: the agent working in the user's other Mac apps (docs/security.md › Computer
//! use).
//!
//! - `policy`: which apps it may touch and how far (blocked, look-only, full), and a chat's grants.
//! - `geometry`: picture sizes, coordinates and the hit test.
//! - `keys`: key chords on the user's keyboard layout.
//! - `outline`: an accessibility tree as the outline and element list the model reads.
//! - `ladder`: what an action may try after Accessibility can't do it.
//! - `change`: what an action changed, from fingerprints before and after.
//! - `computer`: the app's `Computer` and each chat's `ChatComputer`: grants and access cards.
//! - `tools`: the agent's `computer_*` tools; `feature`: what the loop keeps across compactions.
//! - `backend`: the `Desktop` trait the tools work through, and a fake one for tests.
//! - `engine`: one thread per app, where its Accessibility calls run.
//! - `native`: Accessibility, ScreenCaptureKit, synthetic input, apps, permissions and the real
//!   `Desktop` (`native::desktop::Mac`).
//!
//! No GPUI here (docs/architecture.md). Security rules, in one place:
//! - Off by default, Code chats only, never sub-agents.
//! - No WackCode build is ever pictured, read or driven; terminals, script runners, installers,
//!   password managers, the system's security and chrome UI and System Settings never are either.
//!   Browsers are look-only.
//! - An app is used only after the user allows it on the access card; nothing a tool does can
//!   answer the card.
//! - Secure text fields are never read or typed into.
//! - In the background, input goes to the target process only, never the user's pointer.

//!
//! What works in the background (the Phase 0 spike, 2026-11-24, macOS 27, against TextEdit and
//! Calculator with another app in front; the front app never changed):
//! - Window pictures through ScreenCaptureKit, covered or not: 75–175 ms; zooms at full density.
//! - The AX walk with one message per element: about twice as fast as one per attribute.
//! - `AXPress` on controls, typing through `AXSelectedText` (confirmed by reading the value back),
//!   keys and Unicode text posted to the process, and scrolling by moving the AX scroll bar.
//! - App-level menu commands (File › New).
//!
//! What does not, and so climbs the ladder (`ladder.rs`) or asks for the screen:
//! - Mouse clicks and wheel scrolls posted to the process: AppKit and SwiftUI apps ignore them in
//!   the background (Claude Desktop reaches them through private SkyLight calls; we don't). A click
//!   is therefore an `AXPress` of the element the last walk places under the point, or the caret
//!   placed through `AXRangeForPosition`, or it needs the screen.
//! - Commands on the front document (File › Save) and their shortcuts: they read disabled and are
//!   held until the app comes forward, so they need the screen.
//! - AX hit tests (`AXUIElementCopyElementAtPosition`) on a background window answered with the
//!   menu bar or nothing: hits are found in the last walk's frames instead.
//! - `NSWorkspace`'s app list and ScreenCaptureKit need the main run loop and a window-server
//!   connection, which the app has (GPUI runs both).

pub mod backend;
pub mod change;
pub mod computer;
pub mod engine;
pub mod feature;
pub mod geometry;
pub mod keys;
pub mod ladder;
pub mod native;
pub mod outline;
pub mod policy;
pub mod tools;
