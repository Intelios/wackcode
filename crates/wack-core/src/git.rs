//! Git, through the system `git` CLI (never libgit2), so it behaves exactly as the user's own
//! `git` does, with their configuration.
//!
//! Every call is blocking: run it off the UI thread.

use std::{path::Path, process::Command};

/// The branch checked out in `folder`, a short commit hash when the head is detached, or `None`
/// outside a repository (or when Git is missing).
pub fn branch(folder: &Path) -> Option<String> {
    let run = |args: &[&str]| {
        let output = Command::new("git").arg("-C").arg(folder).args(args).output().ok()?;
        let text = String::from_utf8(output.stdout).ok()?;
        let text = text.trim();
        (output.status.success() && !text.is_empty()).then(|| text.to_string())
    };
    run(&["symbolic-ref", "--quiet", "--short", "HEAD"]).or_else(|| run(&["rev-parse", "--short", "HEAD"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(folder: &Path, args: &[&str]) {
        let status = Command::new("git").arg("-C").arg(folder).args(args).output().unwrap().status;
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn names_the_branch_or_nothing() {
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(branch(folder.path()), None);

        git(folder.path(), &["init", "--quiet", "--initial-branch", "trunk"]);
        // An unborn branch still has a name.
        assert_eq!(branch(folder.path()).as_deref(), Some("trunk"));
    }
}
