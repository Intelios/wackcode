//! The agent: the turn loop, the tool registry (with deferred loading), prompt assembly, sessions
//! and compaction.
//!
//! This crate is the replacement for Pi. It has no UI dependency and runs headless (see `wack-cli`),
//! which is how its behaviour is tested. Features hook in through one small compiled-in trait
//! rather than a dynamic extension runtime.

pub mod agent;
pub mod compaction;
pub mod prompt;
pub mod registry;
pub mod session;
pub mod tool;

pub use agent::{Agent, Control, Event, EventSink, Feature, Outcome, Profile, Settings, Verdict};
pub use registry::{Registry, TOOL_SEARCH};
pub use session::{HistoryEntry, Session, history};
pub use tool::{Loading, Tool, ToolContext, ToolSpec};
