//! Walking a project the way a developer expects: `.gitignore` is respected, dotfiles are
//! included, and `.git` itself is skipped.

use std::path::Path;

use ignore::{Walk, WalkBuilder};

pub fn walk(root: &Path) -> Walk {
    WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .sort_by_file_name(|a, b| a.cmp(b))
        .build()
}
