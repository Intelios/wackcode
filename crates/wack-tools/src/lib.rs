//! The core tools every coding chat starts with: read, write, edit, bash, grep, find and ls, plus
//! the agent's own tools for commands in the background (stop_task, wait_for_tasks, watch).
//!
//! They are always loaded (never deferred) and need nothing outside this process: searching uses
//! the `ignore`, `globset` and `regex` crates rather than `rg` or `fd` binaries. No tool here asks
//! for approval; that is out of scope for WackCode by design.

mod bash;
mod edit;
mod find;
mod grep;
mod ls;
mod read;
mod text;
mod walk;
mod write;

use std::sync::Arc;

use wack_agent::{
    Tool,
    tasks::{StopTask, WaitForTasks, Watch},
};

pub use bash::{Bash, SECRET_ENV};
pub use edit::Edit;
pub use find::Find;
pub use grep::Grep;
pub use ls::Ls;
pub use read::Read;
pub use write::Write;

/// The ten core tools, in the order the model is shown them: the seven here, and the three for
/// commands in the background (`wack_agent::tasks`).
pub fn core_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(Read),
        Arc::new(Write),
        Arc::new(Edit),
        Arc::new(Bash),
        Arc::new(StopTask),
        Arc::new(WaitForTasks),
        Arc::new(Watch),
        Arc::new(Grep),
        Arc::new(Find),
        Arc::new(Ls),
    ]
}
