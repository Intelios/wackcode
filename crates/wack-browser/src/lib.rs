//! The browser engine: Chromium through CEF, a chat's tabs, and in-process DevTools.
//!
//! - `tabs`: `ChatTabs`, owned by each chat, and the `Backend` boundary to the engine.
//! - `state`: a tab's state folded from engine events (console and network history included).
//! - `policy`: which addresses the agent may open.
//! - `tools`: the agent's `browser_*` tools, which drive a chat's tabs over DevTools.
//! - `cef`: the engine itself: one CEF instance for the app, one shared profile, a page per tab.
//!
//! No GPUI here (docs/architecture.md): the UI hands the engine a raw NSView to host pages in and
//! positions them; everything else is plain data. Security rules are in docs/security.md
//! ("Network", "Credentials") and the CEF pin in docs/cef.md.

pub mod cef;
mod main_thread;
pub mod policy;
pub mod state;
pub mod tabs;
pub mod tools;

pub use state::{ConsoleEntry, Dialog, DialogKind, Level, NetworkEntry, TabState, Viewport};
pub use tabs::{Backend, ChatTabs, Nav, Owner, SavedTab, Signal, Tab, TabId, default_viewport, set_default_viewport};
