//! The engine: everything the app does that is not drawing.
//!
//! Projects, chats, settings, secrets, Git and PTYs live here, on Tokio. The UI calls typed async
//! commands and subscribes to typed event streams. Nothing in this crate (or below it) may depend
//! on GPUI: the UI is a viewport onto the engine, so the framework choice stays confined to
//! `wack-ui`.
//!
//! So far: the data folder, the secret store, connections, projects, chats, running the agent for
//! a chat (`run`), what changed in a project (`git`), and terminals (`terminal`).

pub mod git;
pub mod run;
pub mod secrets;
pub mod store;
pub mod terminal;

pub use run::{Job, RunEvent, RunSetup, run};
pub use store::{
    Chat, Connection, Model, ModelChoice, NewConnection, Project, Resolved, Store, Ui, title_from,
};

// What the UI needs from the layers below, so it depends on the engine alone.
pub use wack_agent::{Control, Event, HistoryEntry, Outcome, TOOL_SEARCH, history};
pub use wack_llm::{Effort, Message, RetryNotice, Role, Usage, WireFormat};

/// The provider-neutral conversation model, for code that reads messages.
pub mod llm {
    pub use wack_llm::Block;
}
