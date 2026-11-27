//! Chromium's startup (`SetupSignalHandlers`) resets SIGCHLD to its default action in the app's
//! process. Tokio learns that a child exited through its own SIGCHLD handler, so once that handler
//! is gone a task's command can finish without the task ever ending. This runs in its own process
//! (an integration test) because it changes the process's signal state.

use std::time::Duration;

use wack_agent::tasks::{Kind, Status, Tasks};

fn reset_sigchld() {
    // SAFETY: what Chromium does: the default action for SIGCHLD, no flags, an empty mask.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = libc::SIG_DFL;
        assert_eq!(libc::sigaction(libc::SIGCHLD, &action, std::ptr::null_mut()), 0);
    }
}

#[tokio::test]
async fn a_task_ends_after_something_resets_the_sigchld_handler() {
    let folder = tempfile::tempdir().unwrap();
    let tasks = Tasks::new(folder.path().join("tasks"));
    // The first command makes tokio install its SIGCHLD handler, as the app's first command does.
    let first = tasks.start(folder.path(), "true", "", Kind::Shell, None).unwrap();
    tokio::time::timeout(Duration::from_secs(5), first.ended()).await.expect("ends before the reset");

    reset_sigchld();

    let second = tasks.start(folder.path(), "echo done", "", Kind::Shell, None).unwrap();
    tokio::time::timeout(Duration::from_secs(5), second.ended()).await.expect("a command that exits ends, whatever SIGCHLD's handler");
    assert_eq!(second.info().status, Status::Finished);
    assert_eq!(second.output(), "done\n");
}
