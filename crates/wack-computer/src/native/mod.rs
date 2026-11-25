//! The macOS side of computer use: Accessibility, ScreenCaptureKit, synthetic input, apps and
//! permissions. Everything above this module works on plain data (`outline::Node`,
//! `geometry::Rect`), so the tools can be tested against a fake desktop (`backend.rs`).

pub mod apps;
pub mod ax;
pub mod capture;
pub mod clipboard;
pub mod cursor;
pub mod desktop;
pub mod glow;
pub mod input;
pub mod keyboard;
pub mod main_thread;
pub mod permissions;
pub mod screen;
