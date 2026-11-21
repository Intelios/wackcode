//! A chat's canvases, live: what the agent's tools change and the board draws.
//!
//! `Canvases` is owned by the chat, like its `Tasks`, `Agents` and `ChatTabs`: made when the chat
//! loads, handed to every run and to the Canvas pane, and outliving both. Views only read it and
//! ask it for changes.
//!
//! Invariants:
//! - Every change goes through here: `apply` (the agent's calls), `set_tweak` (the user), `draw`.
//!   Each saves to the store before it tells the observer, so a view never shows what a crash
//!   would lose.
//! - Pictures are drawn one at a time, in the order asked, and a picture is saved only for the
//!   revision it was drawn from: a stale drawing that finishes late never replaces a newer one.
//! - Without a browser engine (the CLI, the bare binary) there are no canvases to offer: the
//!   tools are not offered at all (`available`).

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

use serde_json::Value;
use wack_browser::Backend;

use crate::{
    model::{self, Canvas, Planned},
    render::{Drawn, Job, Renderer, Report},
    store::Store,
};

/// What changed, for the chat's observer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signal {
    /// A canvas was made or changed (artboards, title, tweaks).
    Changed { canvas: String, created: bool },
    /// An artboard's picture is being drawn, or is done.
    Drawing { canvas: String, artboard: String },
    Drawn { canvas: String, artboard: String, rev: u64 },
}

type Observer = Arc<dyn Fn(Signal) + Send + Sync>;

struct Inner {
    store: Store,
    renderer: Option<Arc<Renderer>>,
    canvases: Mutex<Option<Vec<Canvas>>>,
    observer: Mutex<Option<Observer>>,
    /// Artboards waiting for, or in, a drawing.
    drawing: Mutex<HashSet<(String, String)>>,
}

/// The chat's canvases. Clones share them.
#[derive(Clone)]
pub struct Canvases {
    inner: Arc<Inner>,
}

impl Canvases {
    /// `root` is the chat's canvas folder, `v3/canvases/<chat>`; `backend` is `None` where there
    /// is no browser.
    pub fn new(root: impl Into<PathBuf>, backend: Option<Arc<dyn Backend>>) -> Self {
        Self {
            inner: Arc::new(Inner {
                store: Store::new(root),
                renderer: backend.map(|backend| Arc::new(Renderer::new(backend))),
                canvases: Mutex::new(None),
                observer: Mutex::new(None),
                drawing: Mutex::default(),
            }),
        }
    }

    /// True when canvases can be drawn here.
    pub fn available(&self) -> bool {
        self.inner.renderer.is_some()
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    pub fn renderer(&self) -> Option<&Arc<Renderer>> {
        self.inner.renderer.as_ref()
    }

    /// One observer at a time; it is told of every change and every picture.
    pub fn set_observer(&self, observer: impl Fn(Signal) + Send + Sync + 'static) {
        *lock(&self.inner.observer) = Some(Arc::new(observer));
    }

    fn signal(&self, signal: Signal) {
        let observer = lock(&self.inner.observer).clone();
        if let Some(observer) = observer {
            observer(signal);
        }
    }

    fn loaded(&self) -> MutexGuard<'_, Option<Vec<Canvas>>> {
        let mut canvases = lock(&self.inner.canvases);
        if canvases.is_none() {
            *canvases = Some(self.inner.store.load_all());
        }
        canvases
    }

    /// The chat has no canvas.
    pub fn is_empty(&self) -> bool {
        self.loaded().as_ref().is_none_or(Vec::is_empty)
    }

    /// Every canvas, oldest first.
    pub fn list(&self) -> Vec<Canvas> {
        self.loaded().clone().unwrap_or_default()
    }

    pub fn get(&self, id: &str) -> Option<Canvas> {
        self.loaded().as_ref()?.iter().find(|c| c.id == id).cloned()
    }

    /// Applies one `canvas` call: checks it, copies its local images in, saves it.
    pub fn apply(&self, input: &Value) -> Result<Planned, String> {
        let mut canvases = self.loaded();
        let list = canvases.get_or_insert_with(Vec::new);
        let mut planned = model::plan(list, input)?;
        let applied: Vec<String> = planned.applied.iter().map(|a| a.id.clone()).collect();
        for artboard in planned.canvas.artboards.iter_mut().filter(|a| applied.contains(&a.id)) {
            self.inner.store.localize_images(&planned.canvas.id, artboard)?;
        }
        self.inner.store.save(&planned.canvas, &applied, &planned.removed)?;
        match list.iter_mut().find(|c| c.id == planned.canvas.id) {
            Some(slot) => *slot = planned.canvas.clone(),
            None => list.push(planned.canvas.clone()),
        }
        drop(canvases);
        self.signal(Signal::Changed { canvas: planned.canvas.id.clone(), created: planned.created });
        Ok(planned)
    }

    /// Sets one of an artboard's tweaks to `value` (`None`: back to its default), as the user
    /// moves it in the canvas. The artboard's revision moves on, so its picture is redrawn.
    pub fn set_tweak(&self, canvas: &str, artboard: &str, tweak: &str, value: Option<Value>) -> Result<u64, String> {
        let mut canvases = self.loaded();
        let list = canvases.get_or_insert_with(Vec::new);
        let found = list.iter_mut().find(|c| c.id == canvas).ok_or("That canvas is gone.")?;
        let board = found.artboards.iter_mut().find(|a| a.id == artboard).ok_or("That artboard is gone.")?;
        let declared = board.tweaks.iter().find(|t| t.id == tweak).ok_or("That tweak is gone.")?;
        match value {
            Some(value) => {
                declared.accepts(&value)?;
                board.tweak_values.insert(tweak.to_string(), value);
            }
            None => {
                board.tweak_values.remove(tweak);
            }
        }
        board.rev += 1;
        board.tweaks_untold = true;
        let rev = board.rev;
        let snapshot = found.clone();
        self.inner.store.save_index(&snapshot)?;
        drop(canvases);
        self.signal(Signal::Changed { canvas: canvas.to_string(), created: false });
        Ok(rev)
    }

    /// Whether an artboard is waiting for, or in, a drawing.
    pub fn is_drawing(&self, canvas: &str, artboard: &str) -> bool {
        lock(&self.inner.drawing).contains(&(canvas.to_string(), artboard.to_string()))
    }

    /// Draws the artboards named, one after another, saving each picture for the revision it
    /// was drawn from. An artboard someone else is drawing already (the board and the agent's
    /// call both ask) is waited for, not drawn twice. Each result is the artboard's report, or
    /// why it could not be drawn.
    pub async fn draw(&self, canvas: &str, artboards: &[String]) -> Vec<(String, Result<Report, String>)> {
        let Some(renderer) = self.inner.renderer.clone() else {
            return artboards.iter().map(|id| (id.clone(), Err(NO_ENGINE.to_string()))).collect();
        };
        let mut results = Vec::new();
        for id in artboards {
            let key = (canvas.to_string(), id.clone());
            let mine = lock(&self.inner.drawing).insert(key.clone());
            let result = if mine {
                // Taken back however this ends, a dropped call included, so it never stays "drawing".
                let _done = Drawing { canvases: self.clone(), key: key.clone() };
                self.signal(Signal::Drawing { canvas: canvas.to_string(), artboard: id.clone() });
                let result = self.draw_one(&renderer, canvas, id).await;
                if let Ok(report) = &result {
                    self.signal(Signal::Drawn { canvas: canvas.to_string(), artboard: id.clone(), rev: report.rev });
                }
                result
            } else {
                while lock(&self.inner.drawing).contains(&key) {
                    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                }
                let rev = self.get(canvas).and_then(|c| c.artboards.into_iter().find(|a| &a.id == id)).map(|a| a.rev);
                self.report(canvas, id).filter(|report| Some(report.rev) == rev).ok_or_else(|| "It could not be drawn.".to_string())
            };
            results.push((id.clone(), result));
        }
        results
    }

    async fn draw_one(&self, renderer: &Renderer, canvas: &str, id: &str) -> Result<Report, String> {
        let job = self.job(canvas, id)?;
        let rev = job.rev;
        let Drawn { report, png } = renderer.draw(job).await?;
        // Saved only if the artboard is still at that revision.
        let current = self.get(canvas).and_then(|c| c.artboards.into_iter().find(|a| a.id == id)).map(|a| a.rev);
        if current == Some(rev) {
            let saved = serde_json::to_value(&report).map_err(|_| "Could not keep the report.".to_string())?;
            let store = self.inner.store.clone();
            let (canvas, id) = (canvas.to_string(), id.to_string());
            tokio::task::spawn_blocking(move || {
                store.save_render(&canvas, &id, rev, &png, &saved)?;
                store.save_thumbnail(&canvas, &id, rev, &png)?;
                crate::comments::follow(&store, &canvas, &id, rev, &serde_json::from_value::<Report>(saved).map(|r| r.elements).unwrap_or_default())
            })
            .await
            .map_err(|_| "Could not keep the picture.".to_string())??;
        }
        Ok(report)
    }

    /// What drawing an artboard's current revision takes.
    /// The height an artboard's live page is laid out at: its own, or for an auto one the height
    /// it was last drawn at.
    pub fn live_height(&self, canvas: &str, id: &str) -> Result<u32, String> {
        let job = self.job(canvas, id)?;
        Ok(job.height.or_else(|| self.report(canvas, id).map(|report| report.height)).unwrap_or(crate::render::FIRST_HEIGHT))
    }

    /// A live page of an artboard (Play), shown scaled by `display`, and the revision it shows.
    /// The caller closes it with `close_live`.
    pub async fn open_live(&self, canvas: &str, id: &str, display: f64) -> Result<(Arc<wack_browser::Tab>, u64), String> {
        let renderer = self.inner.renderer.clone().ok_or(NO_ENGINE)?;
        let job = self.job(canvas, id)?;
        let height = self.live_height(canvas, id)?;
        let tab = renderer.open(&job, height, display).await?;
        Ok((tab, job.rev))
    }

    pub async fn rescale_live(&self, tab: &Arc<wack_browser::Tab>, canvas: &str, id: &str, display: f64) -> Result<(), String> {
        let renderer = self.inner.renderer.clone().ok_or(NO_ENGINE)?;
        let job = self.job(canvas, id)?;
        let height = self.live_height(canvas, id)?;
        renderer.rescale(tab, job.width, height, display).await
    }

    /// An artboard exported as `format`: the bytes to save.
    pub async fn export(&self, canvas: &str, id: &str, format: crate::export::Format) -> Result<Vec<u8>, String> {
        use crate::export::Format;
        let found = self.get(canvas).ok_or("That canvas is gone.")?;
        let artboard = found.artboards.iter().find(|a| a.id == id).ok_or("That artboard is gone.")?.clone();
        match format {
            Format::Html => Ok(crate::export::standalone(&self.inner.store.dir(canvas), &found, &artboard).into_bytes()),
            Format::Png => {
                if self.picture(canvas, id).is_none_or(|(_, rev)| rev != artboard.rev)
                    && let Some((_, Err(error))) = self.draw(canvas, &[id.to_string()]).await.into_iter().next()
                {
                    return Err(error);
                }
                let (path, _) = self.picture(canvas, id).ok_or("The artboard has no picture yet.")?;
                std::fs::read(path).map_err(|_| "The artboard's picture could not be read.".to_string())
            }
            Format::Pdf => {
                let renderer = self.inner.renderer.clone().ok_or(NO_ENGINE)?;
                let job = self.job(canvas, id)?;
                let height = self.live_height(canvas, id)?;
                renderer.pdf(&job, height).await
            }
        }
    }

    /// Sets one tweak on a live page at once (Play), as the runtime would on load. The value is
    /// checked against the declaration first; `None` puts the default back.
    pub async fn apply_live_tweak(&self, tab: &Arc<wack_browser::Tab>, canvas: &str, artboard: &str, tweak: &str, value: Option<Value>) -> Result<(), String> {
        let renderer = self.inner.renderer.clone().ok_or(NO_ENGINE)?;
        let found = self.get(canvas).ok_or("That canvas is gone.")?;
        let board = found.artboards.iter().find(|a| a.id == artboard).ok_or("That artboard is gone.")?;
        let declared = board.tweaks.iter().find(|t| t.id == tweak).ok_or("That tweak is gone.")?;
        let value = match value {
            Some(value) => {
                declared.accepts(&value)?;
                value
            }
            None => declared.default.clone(),
        };
        let argument = serde_json::json!({ "var": declared.var, "attr": declared.attr, "value": declared.css_value(&value) });
        renderer.evaluate(tab, &format!("window.__wack && window.__wack.setTweak({argument})")).await.map(|_| ())
    }

    /// The artboards whose tweaks the user moved since this was last asked, each once.
    pub fn take_tweaked(&self) -> Vec<(String, String)> {
        let mut canvases = self.loaded();
        let mut told = Vec::new();
        for canvas in canvases.iter_mut().flatten() {
            let before = told.len();
            for board in canvas.artboards.iter_mut().filter(|a| a.tweaks_untold) {
                board.tweaks_untold = false;
                told.push((canvas.id.clone(), board.id.clone()));
            }
            if told.len() > before {
                // Unsaved, the model is told again after a relaunch: better than never.
                let _ = self.inner.store.save_index(canvas);
            }
        }
        told
    }

    pub fn close_live(&self, tab: wack_browser::TabId) {
        if let Some(renderer) = &self.inner.renderer {
            renderer.backend().close(tab);
        }
    }

    pub fn job(&self, canvas: &str, id: &str) -> Result<Job, String> {
        let found = self.get(canvas).ok_or("That canvas is gone.")?;
        let artboard = found.artboards.iter().find(|a| a.id == id).ok_or_else(|| format!("There is no artboard “{id}” on “{}”.", found.title))?;
        Ok(Job {
            site: self.inner.store.site(canvas),
            address: self.inner.store.address(canvas, artboard),
            rev: artboard.rev,
            width: artboard.width,
            height: artboard.height,
        })
    }

    /// The newest picture of an artboard, and the revision it shows (which may be older than the
    /// artboard's while a new one is drawn).
    pub fn picture(&self, canvas: &str, id: &str) -> Option<(PathBuf, u64)> {
        let renders = self.inner.store.renders(canvas);
        let prefix = format!("{id}@");
        std::fs::read_dir(renders)
            .ok()?
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                let rev = name.strip_prefix(&prefix)?.strip_suffix(".png")?.parse::<u64>().ok()?;
                Some((entry.path(), rev))
            })
            .max_by_key(|(_, rev)| *rev)
    }

    /// The small copy of an artboard's newest picture, if it has one yet.
    pub fn thumbnail(&self, canvas: &str, id: &str) -> Option<PathBuf> {
        let (_, rev) = self.picture(canvas, id)?;
        let path = self.inner.store.render_path(canvas, id, rev, "thumb.png");
        path.exists().then_some(path)
    }

    /// The report saved with an artboard's newest picture.
    pub fn report(&self, canvas: &str, id: &str) -> Option<Report> {
        let (_, rev) = self.picture(canvas, id)?;
        serde_json::from_value(self.inner.store.render_report(canvas, id, rev)?).ok()
    }

    /// Removes every canvas of the chat, with the chat.
    pub fn remove_all(&self) {
        self.inner.store.remove_all();
        *lock(&self.inner.canvases) = Some(Vec::new());
    }
}

/// Takes an artboard off the drawing list when its drawing ends, however it ends.
struct Drawing {
    canvases: Canvases,
    key: (String, String),
}

impl Drop for Drawing {
    fn drop(&mut self) {
        lock(&self.canvases.inner.drawing).remove(&self.key);
    }
}

const NO_ENGINE: &str = "Canvases need the browser engine, which this build of WackCode does not have. It needs the app bundle (`cargo dev`).";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn calls_are_saved_before_the_observer_hears_and_tweaks_move_the_revision() {
        let dir = tempfile::tempdir().unwrap();
        let canvases = Canvases::new(dir.path().join("chat"), None);
        assert!(!canvases.available());
        let heard = Arc::new(Mutex::new(Vec::new()));
        let log = heard.clone();
        let root = dir.path().join("chat");
        canvases.set_observer(move |signal| {
            assert!(root.join("t/canvas.json").exists(), "saved first");
            log.lock().unwrap().push(signal);
        });
        canvases
            .apply(&json!({ "title": "T", "artboards": [{ "name": "A", "width": 400, "html": "<p/>",
                "tweaks": [{ "id": "dark", "label": "Dark", "kind": "toggle", "default": false, "attr": "theme" }] }] }))
            .unwrap();
        assert_eq!(*heard.lock().unwrap(), vec![Signal::Changed { canvas: "t".into(), created: true }]);
        assert_eq!(canvases.set_tweak("t", "a", "dark", Some(json!(true))).unwrap(), 2);
        assert!(canvases.set_tweak("t", "a", "dark", Some(json!("yes"))).is_err());
        let reloaded = Canvases::new(dir.path().join("chat"), None);
        assert_eq!(reloaded.get("t").unwrap().artboards[0].tweak_values["dark"], json!(true));
        assert_eq!(reloaded.get("t").unwrap().artboards[0].html.as_deref(), Some("<p/>"));
    }

    #[tokio::test]
    async fn without_an_engine_nothing_is_drawn() {
        let dir = tempfile::tempdir().unwrap();
        let canvases = Canvases::new(dir.path(), None);
        let drawn = canvases.draw("t", &["a".into()]).await;
        assert!(drawn[0].1.as_ref().unwrap_err().contains("browser engine"));
    }
}
