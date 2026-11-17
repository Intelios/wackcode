//! The agent: the turn loop, the tool registry (with deferred loading), prompt assembly, sessions
//! and compaction.
//!
//! This crate is the replacement for Pi. It has no UI dependency and runs headless (see `wack-cli`),
//! which is how its behaviour is tested. Features hook in through one small compiled-in trait
//! rather than a dynamic extension runtime: so far the question tool (`ask`), the todo list
//! (`todo`), Plan mode (`plan`), memory (`memory`) and skills (`skills`), assembled for a run by
//! `kit`, and sub-agents (`agents`). Slash commands (`commands`) expand before a run starts.

pub mod agent;
pub mod agents;
pub mod ask;
pub mod commands;
pub mod compaction;
pub mod kit;
pub mod memory;
pub mod plan;
pub mod prompt;
pub mod registry;
pub mod session;
pub mod shell_env;
pub mod skills;
pub mod tasks;
pub mod todo;
pub mod tool;
pub mod usage;

pub use agent::{Agent, Control, Event, EventSink, Feature, Outcome, Profile, Settings, Verdict};
pub use agents::{AgentDef, Agents, AgentsSetup, ChildModel, Delegation};
pub use kit::Kit;
pub use memory::Memory;
pub use commands::CommandUse;
pub use plan::AgentMode;
pub use registry::{Registry, TOOL_SEARCH};
pub use session::{HistoryEntry, Note, Session, history};
pub use tasks::{Tasks, SECRET_ENV};
pub use tool::{Loading, Tool, ToolContext, ToolSpec, User};
pub use usage::{CallOutcome, ModelCall, Purpose, Recorder};
