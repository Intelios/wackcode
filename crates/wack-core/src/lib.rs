//! The engine: everything the app does that is not drawing.
//!
//! Projects, chats, settings, secrets, Git and PTYs live here, on Tokio. The UI calls typed async
//! commands and subscribes to typed event streams. Nothing in this crate (or below it) may depend
//! on GPUI: the UI is a viewport onto the engine, so the framework choice stays confined to
//! `wack-ui`.
//!
//! So far: the data folder, the secret store, connections, projects, chats, running the agent for
//! a chat (`run`), what changed in a project (`git`), terminals (`terminal`), how the app looks
//! (`appearance`), where each project's memory lives (`memory`), skills and slash commands
//! (`skills`), sub-agents' settings (`agents`), automatic chat titles (`title`), the usage ledger TokenTrail reads (`usage`), the
//! web tools' settings (`web`), the MCP servers (`mcp`), and the Chat area's `@` references
//! (`chat_refs`). A chat is a Code chat (a project's) or a Chat chat (the Chat area's:
//! `store::ChatKind`), and `run` builds a different agent for each.

pub mod agents;
pub mod appearance;
pub mod chat_refs;
pub mod clock;
pub mod computer;
pub mod git;
pub mod mcp;
pub mod memory;
pub mod preview;
pub mod prompts;
pub mod run;
pub mod runner;
pub mod secrets;
pub mod skills;
pub mod store;
pub mod terminal;
pub mod title;
pub mod usage;
pub mod web;
pub mod web_usage;

pub use appearance::{Appearance, Backdrop, Mode, Texture, ThemeName};
pub use git::GitConfig;
pub use prompts::{MAX_PROMPT_CHARS, PromptConfig, PromptGroup, PromptId};
pub use run::{ChatParts, Job, RunEvent, RunSetup, chat_agent, discard_plan, run, save_plan};
pub use title::AutoTitleConfig;
pub use runner::{CommandEvent, CommandInfo, CommandSession, CommandStatus, Runner};
pub use usage::{RunRecorder, UsageStatus, UsageWriter};
pub use store::{
    BrowserConfig, Chat, ChatKind, CommandKind, Connection, Model, ModelChoice, NewConnection, Project, Reasoning, Resolved, Store, SuggestionOrigin, Ui,
    title_from,
};

// What the UI needs from the layers below, so it depends on the engine alone.
pub use wack_agent::{AgentMode, Control, Event, HistoryEntry, Outcome, Recorder, TOOL_SEARCH, history};
pub use wack_agent::prompt::{WACKCODE_COAUTHOR_EMAIL, co_author_label, co_author_trailer};
pub use wack_llm::{Message, RetryNotice, Role, Usage, WireFormat};
pub use wack_web::{WEB_FETCH, WEB_SEARCH};

/// The MCP client the app owns one of, and what Settings shows of its connections.
pub use wack_mcp as mcp_client;
/// The user's login-shell environment, read once at launch for `bash` and MCP servers.
pub use wack_agent::shell_env;
pub use wack_agent::context;

/// Asking the user: the questions a tool raises and the answers that go back.
pub mod ask {
    pub use wack_agent::ask::{ASK_USER_QUESTION, Answer, MAX_ANSWER_LENGTH, Question, QuestionAnswer, QuestionOption, normalize, parse_answers};
}

/// Plan mode: the state the session saves and the names the transcript looks for.
pub mod plan {
    pub use wack_agent::plan::{
        PLAN_MODE_COMPLETE, PLAN_MODE_ENTER, PLAN_STATE, PlanState, normalize_plan,
        proposed::{Proposed, parse as parse_proposed},
    };
}

/// The todo list as the session saves it.
pub mod todo {
    pub use wack_agent::todo::{Status, TODO, TODO_STATE, Task, TodoList, clean};
}

pub mod suggest {
    pub use wack_agent::suggest::{SUGGESTIONS_STATE, SUGGEST_TASK};
}

/// Shell commands the agent runs, which outlive its runs: a chat owns one registry.
pub mod tasks {
    pub use wack_agent::tasks::{
        Kind, NOTE_PREFIX, STOP_TASK, now_millis, Signal, Status, StoppedBy, TASKS_STATE, Task, TaskInfo, Tasks, WAIT_FOR_TASKS, WATCH, Why, elapsed,
        first_line,
    };
}

/// The provider-neutral conversation model, for code that reads messages.
pub mod llm {
    pub use wack_llm::{Block, Image};
}

/// Pictures: recognising them, viewing copies and a chat's saved originals.
pub mod images {
    pub use wack_agent::images::{Images, reference, store::SHORT, view::{MAX_EDGE, decode, dimensions, encode, media_type}};
}
