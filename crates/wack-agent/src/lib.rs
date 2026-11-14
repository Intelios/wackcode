//! The agent: the turn loop, the tool registry (with deferred loading), prompt assembly, sessions
//! and compaction.
//!
//! This crate is the replacement for Pi. It has no UI dependency and runs headless (see `wack-cli`),
//! which is how its behaviour is tested. Features hook in through one small compiled-in trait
//! rather than a dynamic extension runtime.
