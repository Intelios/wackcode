//! The engine: everything the app does that is not drawing.
//!
//! Projects, chats, settings, secrets, Git and PTYs live here, on Tokio. The UI calls typed async
//! commands and subscribes to typed event streams. Nothing in this crate (or below it) may depend
//! on GPUI: the UI is a viewport onto the engine, so the framework choice stays confined to
//! `wack-ui`.
//!
//! So far: the data folder, the secret store, connections, projects, chats, and running the agent
//! for a chat (`run`).

pub mod run;
pub mod secrets;
pub mod store;

pub use run::{Job, RunEvent, RunSetup, run};
pub use store::{
    Chat, Connection, Model, ModelChoice, NewConnection, Project, Resolved, Store, title_from,
};
