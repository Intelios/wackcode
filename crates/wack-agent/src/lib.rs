//! The agent: the turn loop, the tool registry (with deferred loading), prompt assembly, sessions
//! and compaction.
//!
//! This crate is the replacement for Pi. It has no UI dependency and runs headless (see `wack-cli`),
//! which is how its behaviour is tested. Features hook in through one small compiled-in trait
//! rather than a dynamic extension runtime: so far the question tool (`ask`), the todo list
//! (`todo`) and Plan mode (`plan`), assembled for a run by `kit`.

pub mod agent;
pub mod ask;
pub mod compaction;
pub mod kit;
pub mod plan;
pub mod prompt;
pub mod registry;
pub mod session;
pub mod todo;
pub mod tool;

pub use agent::{Agent, Control, Event, EventSink, Feature, Outcome, Profile, Settings, Verdict};
pub use kit::Kit;
pub use plan::AgentMode;
pub use registry::{Registry, TOOL_SEARCH};
pub use session::{HistoryEntry, Note, Session, history};
pub use tool::{Loading, Tool, ToolContext, ToolSpec, User};
