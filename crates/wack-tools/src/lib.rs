//! The core tools every coding chat starts with: read, write, edit, bash, grep, find and ls.
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

use wack_agent::Tool;

pub use bash::Bash;
pub use edit::Edit;
pub use find::Find;
pub use grep::Grep;
pub use ls::Ls;
pub use read::Read;
pub use write::Write;

/// The seven core tools, in the order the model is shown them.
pub fn core_tools() -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(Read), Arc::new(Write), Arc::new(Edit), Arc::new(Bash), Arc::new(Grep), Arc::new(Find), Arc::new(Ls)]
}
