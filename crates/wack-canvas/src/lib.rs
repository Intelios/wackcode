//! Canvas: boards of whole HTML mockups the agent draws and the user compares, comments on and
//! tweaks (docs/architecture.md, "Canvas").
//!
//! - `canvases`: a chat's canvases, live: what the tools change and the board draws.
//! - `comments`: the user's comments, pinned to an artboard's elements.
//! - `guide`, `tools`: the agent's `canvas_guide`, `canvas` and `canvas_look`.
//! - `model`: a canvas and its artboards, and `plan`, the one rule for changing them.
//! - `store`: the chat's canvas folder on disk, and the offline site Chromium loads pages from.
//! - `render`: drawing an artboard offscreen into a picture and a report.
//! - `runtime.js`: what every artboard's page runs first (tweaks, measuring, the element map).
//!
//! No GPUI here: the board is drawn by `wack-ui`, which reads the store and the pictures.

pub mod canvases;
pub mod comments;
pub mod guide;
pub mod model;
pub mod render;
pub mod store;
pub mod tools;

pub use canvases::{Canvases, Signal};
pub use model::{Artboard, Canvas, Tweak, TweakKind};
pub use render::{Element, Report};
pub use store::Store;
