//! The engine: everything the app does that is not drawing.
//!
//! Projects, chats, settings, secrets, Git and PTYs live here, on Tokio. The UI calls typed async
//! commands and subscribes to typed event streams. Nothing in this crate (or below it) may depend
//! on GPUI: the UI is a viewport onto the engine, so the framework choice stays confined to
//! `wack-ui`.
//!
//! So far it holds what the headless agent needs: the data folder, the secret store and the
//! provider connections.

pub mod secrets;
pub mod store;

pub use store::{Connection, Model, NewConnection, Store};
