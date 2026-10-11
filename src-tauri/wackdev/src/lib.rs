//! The macOS automation core WackCode's computer use and the dev-app driver (`wackdev-helper`,
//! `scripts/wackdev/`) share. Nothing here knows about Tauri, chats or grants — those live in
//! the app's `computer_use` module, which re-exports this crate.
//!
//! Security rules that live here because both callers need them: secure text fields are never
//! read (`ax`), and keys/text only ever go to the target process (`input`), never the system's
//! event stream. Driver-specific targeting (aim only at the dev app) is in `driver`.

pub mod apps;
pub mod ax;
pub mod capture;
pub mod driver;
pub mod geometry;
pub mod input;
pub mod keys;
pub mod outline;
pub mod permissions;
pub mod policy;
