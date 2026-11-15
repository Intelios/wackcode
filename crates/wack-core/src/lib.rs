//! The engine: everything the app does that is not drawing.
//!
//! Projects, chats, settings, secrets, Git and PTYs live here, on Tokio. The UI calls typed async
//! commands and subscribes to typed event streams. Nothing in this crate (or below it) may depend
//! on GPUI: the UI is a viewport onto the engine, so the framework choice stays confined to
//! `wack-ui`.
//!
//! So far: the data folder, the secret store, connections, projects, chats, running the agent for
//! a chat (`run`), what changed in a project (`git`), terminals (`terminal`), and how the app looks
//! (`appearance`).

pub mod appearance;
pub mod git;
pub mod run;
pub mod secrets;
pub mod store;
pub mod terminal;

pub use appearance::{Appearance, Backdrop, Mode, Texture};
pub use run::{Job, RunEvent, RunSetup, discard_plan, run, save_plan};
pub use store::{
    Chat, Connection, Model, ModelChoice, NewConnection, Project, Reasoning, Resolved, Store, Ui, title_from,
};

// What the UI needs from the layers below, so it depends on the engine alone.
pub use wack_agent::{AgentMode, Control, Event, HistoryEntry, Outcome, TOOL_SEARCH, history};
pub use wack_llm::{Message, RetryNotice, Role, Usage, WireFormat};

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

/// The provider-neutral conversation model, for code that reads messages.
pub mod llm {
    pub use wack_llm::Block;
}
