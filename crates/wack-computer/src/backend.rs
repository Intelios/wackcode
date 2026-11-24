//! `Desktop`: the boundary between the tools and the Mac. The tools work on plain data
//! (`outline::Node`, `geometry::Rect`) through this trait, so every rule above it is tested
//! against [`fake::FakeDesktop`]; `native::desktop::Mac` is the real one.
//!
//! Invariants:
//! - A [`Tree`]'s node handles **are** ref numbers: the engine gives each element one number for
//!   as long as it exists (`native::ax::Registry`), so refs survive later looks.
//! - Window ids are window-server numbers (`CGWindowID`), as `computer_apps` lists them.
//! - Points handed to [`Desktop::act`] are global screen points; the tools turn picture
//!   coordinates into them (`geometry`), and find the element under a point in the last tree
//!   (`Target::At`), since the spike found Accessibility's own hit test unreliable in the
//!   background.

use std::path::PathBuf;

use async_trait::async_trait;

use crate::{
    geometry::Rect,
    keys::{Chord, Layout, Modifiers},
    ladder::{AppKind, Reason},
    native::capture::{Shot, Zoom},
    outline::Node,
    policy::{AppIdentity, OwnIdentity},
};

/// A running app.
#[derive(Debug, Clone, PartialEq)]
pub struct AppInfo {
    pub pid: i32,
    pub identity: AppIdentity,
    pub frontmost: bool,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Found {
    Running(AppInfo),
    Installed(AppIdentity),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowInfo {
    /// The window-server number.
    pub id: u32,
    pub title: String,
    /// Global points; `None` when the window reports none (minimized, say).
    pub frame: Option<Rect>,
    pub main: bool,
    pub minimized: bool,
    /// On the current Space and not hidden.
    pub on_screen: bool,
    /// AX subrole: `AXStandardWindow`, `AXDialog`, `AXFloatingWindow`…
    pub kind: Option<String>,
}

/// One window's accessibility tree, as the engine walked it.
#[derive(Debug, Clone, PartialEq)]
pub struct Tree {
    /// Handles are ref numbers.
    pub root: Node,
    pub truncated: bool,
    /// The window's frame when it was walked.
    pub window: Rect,
}

/// Where an action lands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// An element by ref.
    Ref(u32),
    /// A global point, with the deepest element the last tree placed under it, if any.
    At { point: (f64, f64), element: Option<u32> },
    /// Whatever has keyboard focus in the app.
    Focused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Click { target: Target, button: Button, count: u32, modifiers: Modifiers },
    /// `replace` writes over the field's whole text; otherwise text that would be replaced (a
    /// selection, or a field that takes only whole writes) is refused.
    Type { target: Target, text: String, replace: bool },
    Key { chord: Chord, times: u32 },
    Scroll { target: Target, dx: f64, dy: f64 },
    /// Global points, first is the press and last the release.
    Drag { path: Vec<(f64, f64)> },
    Menu { path: Vec<String> },
    Focus { target: Target },
}

/// How an action was carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// Through Accessibility, confirmed where it can be read back.
    Ax,
    /// Raw input posted to the app's process: unverified.
    Raw,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    /// What was acted on, in words: `pressed button "Save"`.
    pub what: String,
    pub via: Via,
    /// Anything the model should know: "typed as key events; check the result".
    pub note: Option<String>,
    /// Text a replace wrote over, at most 500 characters, when it could be read.
    pub replaced: Option<String>,
}

/// What an action changed, read before and after it (`change.rs` compares two).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fingerprint {
    /// The focused element, described.
    pub focused: Option<String>,
    /// The app's windows: id and title.
    pub windows: Vec<(u32, String)>,
    pub frontmost: Option<i32>,
    /// The acted-on element's value, when it shows one.
    pub value: Option<String>,
    /// The acted-on element's description.
    pub target: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Permissions {
    pub accessibility: bool,
    pub screen_recording: bool,
}

#[async_trait]
pub trait Desktop: Send + Sync {
    fn own(&self) -> OwnIdentity;
    /// The keyboard layout shortcuts are pressed on.
    fn layout(&self) -> Layout;
    fn permissions(&self) -> Permissions;
    async fn running(&self) -> Vec<AppInfo>;
    async fn installed(&self, query: &str, limit: usize) -> Vec<AppIdentity>;
    async fn resolve(&self, query: &str) -> Result<Found, String>;
    /// Launches `app` (or opens `file` with it) without bringing it forward; its pid.
    async fn launch(&self, app: &AppIdentity, file: Option<PathBuf>) -> Result<i32, String>;
    fn kind(&self, app: &AppIdentity) -> AppKind;
    async fn windows(&self, pid: i32) -> Result<Vec<WindowInfo>, String>;
    /// Walks `window`, or only the element `from` and what is inside it.
    async fn tree(&self, pid: i32, window: u32, from: Option<u32>) -> Result<Tree, String>;
    /// A quick walk of `window` for the change report: cheaper limits than `tree`, and `None`
    /// rather than an error. The fake's is its tree.
    async fn glance(&self, pid: i32, window: u32) -> Option<Node> {
        self.tree(pid, window, None).await.ok().map(|tree| tree.root)
    }
    /// Titles under the menu bar (`path` empty) or the menu at `path`.
    async fn menus(&self, pid: i32, path: Vec<String>) -> Result<Vec<String>, String>;
    /// A picture of `window`; `region` zooms (see `capture::WindowShot`).
    async fn picture(&self, window: u32, scale: f64, region: Option<Zoom>) -> Result<Shot, String>;
    async fn act(&self, pid: i32, window: u32, act: Act) -> Result<Done, Reason>;
    async fn fingerprint(&self, pid: i32, target: Option<u32>) -> Fingerprint;
    fn frontmost(&self) -> Option<i32>;
    /// Brings `pid` forward (the user's app, after a target took the front).
    async fn bring_forward(&self, pid: i32) -> bool;
    fn seconds_since_typing(&self) -> f64;
    async fn icon(&self, app: &AppIdentity) -> Option<Vec<u8>>;
    async fn clipboard_read(&self) -> Result<Option<String>, String>;
    async fn clipboard_write(&self, text: &str) -> Result<(), String>;
}

pub mod fake {
    //! A desktop in memory for tests: apps, windows and trees the test sets up, and a log of
    //! every action. Typing into a node sets its value; pressing one runs the test's hook.

    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    pub struct State {
        pub apps: Vec<AppInfo>,
        pub installed: Vec<AppIdentity>,
        pub windows: Vec<(i32, WindowInfo)>,
        /// The tree of each window, by window id.
        pub trees: Vec<(u32, Node)>,
        pub acts: Vec<(i32, u32, Act)>,
        pub launched: Vec<(AppIdentity, Option<PathBuf>)>,
        pub pictures: usize,
        pub frontmost: Option<i32>,
        pub clipboard: Option<String>,
        /// Answers for the next actions, in order; an action with none left succeeds through AX.
        pub outcomes: Vec<Result<Done, Reason>>,
        pub permissions: Permissions,
        pub chromium: Vec<String>,
    }

    #[derive(Clone, Default)]
    pub struct FakeDesktop {
        pub state: Arc<Mutex<State>>,
    }

    impl FakeDesktop {
        pub fn new() -> Self {
            let desktop = Self::default();
            desktop.state.lock().unwrap().permissions = Permissions { accessibility: true, screen_recording: true };
            desktop
        }

        pub fn identity(name: &str, bundle_id: &str) -> AppIdentity {
            AppIdentity { pid: None, name: name.into(), bundle_id: Some(bundle_id.into()), path: Some(PathBuf::from(format!("/Applications/{name}.app"))) }
        }

        /// Adds a running app with one window holding `tree`; its window id is `pid * 10`.
        pub fn add_app(&self, pid: i32, name: &str, bundle_id: &str, tree: Node) -> u32 {
            let mut state = self.state.lock().unwrap();
            let mut identity = Self::identity(name, bundle_id);
            identity.pid = Some(pid);
            state.apps.push(AppInfo { pid, identity, frontmost: false, hidden: false });
            let id = pid as u32 * 10;
            let frame = tree.frame.unwrap_or(Rect::new(100.0, 100.0, 800.0, 600.0));
            state.windows.push((pid, WindowInfo { id, title: format!("{name} window"), frame: Some(frame), main: true, minimized: false, on_screen: true, kind: Some("AXStandardWindow".into()) }));
            state.trees.push((id, tree));
            id
        }

        pub fn acts(&self) -> Vec<(i32, u32, Act)> {
            self.state.lock().unwrap().acts.clone()
        }

        fn node_mut(node: &mut Node, handle: usize) -> Option<&mut Node> {
            if node.handle == handle {
                return Some(node);
            }
            node.children.iter_mut().find_map(|child| Self::node_mut(child, handle))
        }
    }

    #[async_trait]
    impl Desktop for FakeDesktop {
        fn own(&self) -> OwnIdentity {
            OwnIdentity { pid: 1, executable: Some(PathBuf::from("/Applications/WackCode.app/Contents/MacOS/wackcode")), bundle: Some(PathBuf::from("/Applications/WackCode.app")) }
        }

        fn layout(&self) -> Layout {
            Layout::ansi()
        }

        fn permissions(&self) -> Permissions {
            self.state.lock().unwrap().permissions
        }

        async fn running(&self) -> Vec<AppInfo> {
            self.state.lock().unwrap().apps.clone()
        }

        async fn installed(&self, query: &str, limit: usize) -> Vec<AppIdentity> {
            let lower = query.to_lowercase();
            self.state.lock().unwrap().installed.iter().filter(|app| app.name.to_lowercase().contains(&lower)).take(limit).cloned().collect()
        }

        async fn resolve(&self, query: &str) -> Result<Found, String> {
            let state = self.state.lock().unwrap();
            let matches = |identity: &AppIdentity| identity.name.eq_ignore_ascii_case(query) || identity.bundle_id.as_deref().is_some_and(|id| id.eq_ignore_ascii_case(query));
            if let Some(app) = state.apps.iter().find(|app| matches(&app.identity)) {
                return Ok(Found::Running(app.clone()));
            }
            if let Some(app) = state.installed.iter().find(|app| matches(app)) {
                return Ok(Found::Installed(app.clone()));
            }
            Err(format!("No app called “{query}” is running or installed."))
        }

        async fn launch(&self, app: &AppIdentity, file: Option<PathBuf>) -> Result<i32, String> {
            let mut state = self.state.lock().unwrap();
            let pid = 901 + state.launched.len() as i32;
            let mut identity = app.clone();
            identity.pid = Some(pid);
            let title = file.as_ref().and_then(|file| file.file_name()).map_or_else(|| "Untitled".to_string(), |name| name.to_string_lossy().into_owned());
            state.launched.push((app.clone(), file));
            state.apps.push(AppInfo { pid, identity, frontmost: false, hidden: false });
            let frame = Rect::new(200.0, 200.0, 600.0, 400.0);
            state.windows.push((pid, WindowInfo { id: pid as u32 * 10, title, frame: Some(frame), main: true, minimized: false, on_screen: true, kind: Some("AXStandardWindow".into()) }));
            Ok(pid)
        }

        fn kind(&self, app: &AppIdentity) -> AppKind {
            let state = self.state.lock().unwrap();
            if app.bundle_id.as_ref().is_some_and(|id| state.chromium.contains(id)) { AppKind::Chromium } else { AppKind::Native }
        }

        async fn windows(&self, pid: i32) -> Result<Vec<WindowInfo>, String> {
            let state = self.state.lock().unwrap();
            if !state.apps.iter().any(|app| app.pid == pid) {
                return Err("That app isn't running any more.".into());
            }
            Ok(state.windows.iter().filter(|(owner, _)| *owner == pid).map(|(_, window)| window.clone()).collect())
        }

        async fn tree(&self, pid: i32, window: u32, from: Option<u32>) -> Result<Tree, String> {
            let state = self.state.lock().unwrap();
            let frame = state.windows.iter().find(|(owner, info)| *owner == pid && info.id == window).and_then(|(_, info)| info.frame).ok_or("That window is gone.")?;
            let root = state.trees.iter().find(|(id, _)| *id == window).map(|(_, tree)| tree.clone()).ok_or("That window is gone.")?;
            let root = match from {
                Some(handle) => root.find(handle as usize).cloned().ok_or("That element is gone: take a new computer_snapshot.")?,
                None => root,
            };
            Ok(Tree { root, truncated: false, window: frame })
        }

        async fn menus(&self, _pid: i32, path: Vec<String>) -> Result<Vec<String>, String> {
            Ok(if path.is_empty() { vec!["File".into(), "Edit".into()] } else { vec!["New".into(), "Save…".into()] })
        }

        async fn picture(&self, window: u32, scale: f64, region: Option<Zoom>) -> Result<Shot, String> {
            let mut state = self.state.lock().unwrap();
            state.pictures += 1;
            let bounds = state.windows.iter().find(|(_, info)| info.id == window).and_then(|(_, info)| info.frame).ok_or("That window can't be pictured right now.")?;
            let (picture, frame) = match region {
                None => {
                    let frame = crate::geometry::budget(bounds.width, bounds.height);
                    (crate::geometry::scaled(frame, scale), frame)
                }
                Some(Zoom { rect: [x0, y0, x1, y1], .. }) => {
                    let size = (((x1 - x0) * 2.0) as u32, ((y1 - y0) * 2.0) as u32);
                    (size, size)
                }
            };
            // A one-pixel JPEG is enough for tests: the tools never decode pictures.
            Ok(Shot { jpeg: vec![0xFF, 0xD8, 0xFF, 0xD9], picture, frame, bounds })
        }

        async fn act(&self, pid: i32, window: u32, act: Act) -> Result<Done, Reason> {
            let mut state = self.state.lock().unwrap();
            state.acts.push((pid, window, act.clone()));
            if !state.outcomes.is_empty() {
                return state.outcomes.remove(0);
            }
            let what = match &act {
                Act::Type { target: Target::Ref(handle), text, replace } => {
                    if let Some((_, tree)) = state.trees.iter_mut().find(|(id, _)| *id == window)
                        && let Some(node) = Self::node_mut(tree, *handle as usize)
                    {
                        let old = node.value.clone().unwrap_or_default();
                        node.value = Some(if *replace { text.clone() } else { format!("{old}{text}") });
                    }
                    format!("typed into e{handle}")
                }
                Act::Click { target: Target::Ref(handle), .. } => format!("pressed e{handle}"),
                other => format!("{other:?}"),
            };
            Ok(Done { what, via: Via::Ax, note: None, replaced: None })
        }

        async fn fingerprint(&self, pid: i32, target: Option<u32>) -> Fingerprint {
            let state = self.state.lock().unwrap();
            let windows = state.windows.iter().filter(|(owner, _)| *owner == pid).map(|(_, info)| (info.id, info.title.clone())).collect();
            let value = target.and_then(|handle| state.trees.iter().find_map(|(_, tree)| tree.find(handle as usize)).and_then(|node| node.value.clone()));
            Fingerprint { focused: None, windows, frontmost: state.frontmost, value, target: None }
        }

        fn frontmost(&self) -> Option<i32> {
            self.state.lock().unwrap().frontmost
        }

        async fn bring_forward(&self, pid: i32) -> bool {
            self.state.lock().unwrap().frontmost = Some(pid);
            true
        }

        fn seconds_since_typing(&self) -> f64 {
            60.0
        }

        async fn icon(&self, _app: &AppIdentity) -> Option<Vec<u8>> {
            None
        }

        async fn clipboard_read(&self) -> Result<Option<String>, String> {
            Ok(self.state.lock().unwrap().clipboard.clone())
        }

        async fn clipboard_write(&self, text: &str) -> Result<(), String> {
            self.state.lock().unwrap().clipboard = Some(text.to_string());
            Ok(())
        }
    }
}
