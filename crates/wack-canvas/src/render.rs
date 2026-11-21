//! Drawing an artboard: Chromium lays its page out at the artboard's CSS size, offscreen, and
//! hands back a picture and a report. The one renderer for the board's pictures, `canvas_look`
//! and export, so what the agent is told is what the user sees (version 2 measured 600 px on the
//! board and 900 in its look).
//!
//! Invariants:
//! - One artboard at a time per renderer, each in a page of its own that is closed afterwards:
//!   nothing from one artboard's page reaches the next.
//! - Every wait is bounded (`SETTLE`): a page whose script never yields is reported as stuck, not
//!   waited on.
//! - Pictures are PNGs at up to twice the artboard's size, and never more than `MAX_PIXELS`: a
//!   large artboard is drawn at a smaller scale instead.
//! - An auto-height artboard is measured from its content (`runtime.js`), then laid out at that
//!   height and measured again, so a page that got shorter is drawn shorter.
//! - What the page reports is data from an agent's page: sizes are clamped, text is cut short.

use std::{sync::Arc, time::Duration};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wack_browser::{Backend, Level, Tab, site::Site};

use crate::model::{MAX_ARTBOARD_HEIGHT, MIN_ARTBOARD_SIZE};

/// How long a page may take to load and settle before it is reported as stuck.
const SETTLE: Duration = Duration::from_secs(8);
/// The height an auto-height artboard is first laid out at, before it is measured.
pub const FIRST_HEIGHT: u32 = 800;
/// The most pixels one picture may have (8 MP).
pub const MAX_PIXELS: f64 = 8_000_000.;
const MAX_CONSOLE_LINES: usize = 10;
const MAX_CONSOLE_CHARS: usize = 300;

/// One element of an artboard's page, as `runtime.js` maps it: its child-index path from
/// `<body>`, tag, id, first classes, a few words and its box in CSS pixels.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub p: String,
    pub t: String,
    #[serde(default)]
    pub i: String,
    #[serde(default)]
    pub c: String,
    #[serde(default)]
    pub s: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// What drawing an artboard found, saved beside its picture.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub rev: u64,
    /// The size it was drawn at, in CSS pixels.
    pub width: u32,
    pub height: u32,
    /// How tall its content is.
    pub content: u32,
    /// How far content runs past the right edge (horizontal scroll), and past the bottom of a
    /// fixed-height artboard (or past the tallest an auto one may be).
    pub overflow_x: u32,
    pub overflow_y: u32,
    /// Picture pixels per CSS pixel.
    pub scale: f64,
    /// Errors and warnings from its console, uncaught errors included.
    pub console: Vec<String>,
    /// It never settled: a script may be stuck.
    #[serde(default)]
    pub stuck: bool,
    #[serde(default)]
    pub elements: Vec<Element>,
}

impl Report {
    pub fn errors(&self) -> impl Iterator<Item = &String> {
        self.console.iter().filter(|line| line.starts_with("error"))
    }
}

/// A finished drawing: the report and its PNG.
pub struct Drawn {
    pub report: Report,
    pub png: Vec<u8>,
}

/// What a page is drawn from.
pub struct Job {
    pub site: Arc<dyn Site>,
    pub address: String,
    pub rev: u64,
    pub width: u32,
    /// `None`: as tall as its content.
    pub height: Option<u32>,
}

pub struct Renderer {
    backend: Arc<dyn Backend>,
    turn: tokio::sync::Mutex<()>,
}

/// The picture scale for an artboard: twice its size, less for one too large for `MAX_PIXELS`.
pub fn scale_for(width: u32, height: u32) -> f64 {
    let fit = (MAX_PIXELS / (f64::from(width.max(1)) * f64::from(height.max(1)))).sqrt();
    (fit.min(2.) * 100.).floor() / 100.
}

impl Renderer {
    pub fn new(backend: Arc<dyn Backend>) -> Self {
        Self { backend, turn: tokio::sync::Mutex::new(()) }
    }

    pub fn backend(&self) -> &Arc<dyn Backend> {
        &self.backend
    }

    /// Draws one artboard. Waits its turn behind any other drawing.
    pub async fn draw(&self, job: Job) -> Result<Drawn, String> {
        let _turn = self.turn.lock().await;
        let tab = Tab::for_site(job.site.clone());
        self.backend.create(tab.clone(), job.address.clone()).await?;
        let page = Page { backend: self.backend.clone(), tab };
        let drawn = page.draw(&job).await;
        self.backend.close(page.tab.id);
        drawn
    }

    /// The live page for an artboard (Play, export), laid out at `width` × `height` CSS pixels
    /// and shown scaled by `display`. Not behind the drawing turn: it is its own page. The
    /// caller closes it (`Backend::close`).
    pub async fn open(&self, job: &Job, height: u32, display: f64) -> Result<Arc<Tab>, String> {
        let tab = Tab::for_site(job.site.clone());
        self.backend.create(tab.clone(), job.address.clone()).await?;
        let page = Page { backend: self.backend.clone(), tab: tab.clone() };
        if let Err(error) = page.lay_out(job.width, height, 0., display).await {
            self.backend.close(tab.id);
            return Err(error);
        }
        Ok(tab)
    }

    /// Prints an artboard to PDF from its live page, settled, at `width` × `height`.
    pub async fn pdf(&self, job: &Job, height: u32) -> Result<Vec<u8>, String> {
        let _turn = self.turn.lock().await;
        let tab = Tab::for_site(job.site.clone());
        self.backend.create(tab.clone(), job.address.clone()).await?;
        let page = Page { backend: self.backend.clone(), tab: tab.clone() };
        let printed = async {
            page.lay_out(job.width, height, 1., 1.).await?;
            page.settle().await?;
            // CSS pixels are 1/96 inch; a page per artboard height, edge to edge.
            let printed = page
                .cdp(
                    "Page.printToPDF",
                    json!({
                        "printBackground": true,
                        "paperWidth": f64::from(job.width) / 96.,
                        "paperHeight": f64::from(height) / 96.,
                        "marginTop": 0, "marginBottom": 0, "marginLeft": 0, "marginRight": 0,
                        "preferCSSPageSize": false,
                    }),
                )
                .await?;
            let data = printed["data"].as_str().ok_or("The page printed nothing.")?;
            base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| "The printed PDF could not be read.".to_string())
        }
        .await;
        self.backend.close(tab.id);
        printed
    }

    /// The markup of the element at `path` (a child-index path from `<body>`) in a page of the
    /// artboard, read from a page opened for it and closed again.
    pub async fn markup(&self, job: &Job, height: u32, path: &str) -> Result<String, String> {
        let _turn = self.turn.lock().await;
        let tab = Tab::for_site(job.site.clone());
        self.backend.create(tab.clone(), job.address.clone()).await?;
        let page = Page { backend: self.backend.clone(), tab: tab.clone() };
        let read = async {
            page.lay_out(job.width, height, 1., 1.).await?;
            page.settle().await?;
            let path = serde_json::to_string(path).map_err(|_| "That element can't be found.".to_string())?;
            let found = page.evaluate(&format!("window.__wack ? window.__wack.markup({path}, {}) : ''", crate::comments::MAX_MARKUP_CHARS + 1)).await?;
            Ok(found.as_str().unwrap_or_default().to_string())
        }
        .await;
        self.backend.close(tab.id);
        read
    }

    /// Runs one expression in an open live page, answering its value.
    pub async fn evaluate(&self, tab: &Arc<Tab>, expression: &str) -> Result<Value, String> {
        Page { backend: self.backend.clone(), tab: tab.clone() }.evaluate(expression).await
    }

    /// Shows an open live page at another scale, its layout unchanged.
    pub async fn rescale(&self, tab: &Arc<Tab>, width: u32, height: u32, display: f64) -> Result<(), String> {
        Page { backend: self.backend.clone(), tab: tab.clone() }.lay_out(width, height, 0., display).await
    }

}

struct Page {
    backend: Arc<dyn Backend>,
    tab: Arc<Tab>,
}

impl Page {
    async fn cdp(&self, method: &str, params: Value) -> Result<Value, String> {
        self.backend.cdp(self.tab.id, method.to_string(), params).await
    }

    async fn lay_out(&self, width: u32, height: u32, scale: f64, display: f64) -> Result<(), String> {
        self.cdp(
            "Emulation.setDeviceMetricsOverride",
            json!({ "width": width, "height": height, "deviceScaleFactor": scale, "mobile": false, "scale": display }),
        )
        .await
        .map(|_| ())
    }

    /// Waits for the page to load its document, then for the runtime to say it has settled.
    async fn settle(&self) -> Result<Value, String> {
        // The page starts blank: wait for the artboard's own document, not the blank one.
        let origin = self.tab.site().map(|site| wack_browser::site::origin(site.name())).unwrap_or_default();
        let wait = async {
            loop {
                let ready = {
                    let state = self.tab.state();
                    state.url.starts_with(&origin) && !state.loading
                };
                if ready {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            self.evaluate("window.__wack ? window.__wack.settle() : null").await
        };
        match tokio::time::timeout(SETTLE, wait).await {
            Ok(result) => result,
            Err(_) => Err("stuck".into()),
        }
    }

    async fn measure(&self) -> Result<Value, String> {
        let wait = self.evaluate("new Promise(r => requestAnimationFrame(() => requestAnimationFrame(() => r(window.__wack ? window.__wack.measure() : null))))");
        tokio::time::timeout(SETTLE, wait).await.map_err(|_| "stuck".to_string())?
    }

    async fn evaluate(&self, expression: &str) -> Result<Value, String> {
        let result = self.cdp("Runtime.evaluate", json!({ "expression": expression, "awaitPromise": true, "returnByValue": true })).await?;
        if let Some(details) = result.get("exceptionDetails") {
            let text = details["exception"]["description"].as_str().or_else(|| details["text"].as_str()).unwrap_or("The page threw.");
            return Err(cut(text, MAX_CONSOLE_CHARS));
        }
        Ok(result["result"]["value"].clone())
    }

    async fn draw(&self, job: &Job) -> Result<Drawn, String> {
        let mut height = job.height.unwrap_or(FIRST_HEIGHT);
        let mut scale = scale_for(job.width, height);
        self.lay_out(job.width, height, scale, 1.).await?;
        let mut stuck = false;
        let mut measured = match self.settle().await {
            Ok(value) => value,
            Err(error) if error == "stuck" => {
                stuck = true;
                Value::Null
            }
            Err(error) => return Err(error),
        };
        if job.height.is_none() && !stuck {
            let content = content_of(&measured);
            let fitted = content.clamp(MIN_ARTBOARD_SIZE, MAX_ARTBOARD_HEIGHT);
            if fitted != height {
                height = fitted;
                scale = scale_for(job.width, height);
                self.lay_out(job.width, height, scale, 1.).await?;
                measured = self.measure().await.unwrap_or(measured);
            }
        }
        let shot = self
            .cdp(
                "Page.captureScreenshot",
                json!({ "format": "png", "fromSurface": true, "captureBeyondViewport": false, "clip": { "x": 0, "y": 0, "width": job.width, "height": height, "scale": 1 } }),
            )
            .await?;
        let png = base64::engine::general_purpose::STANDARD
            .decode(shot["data"].as_str().unwrap_or_default())
            .map_err(|_| "The page's picture could not be read.".to_string())?;
        let content = content_of(&measured);
        let scroll_width = measured["scroll"][0].as_f64().unwrap_or(f64::from(job.width)).clamp(0., 100_000.) as u32;
        let elements: Vec<Element> = serde_json::from_value(measured["elements"].clone()).unwrap_or_default();
        let report = Report {
            rev: job.rev,
            width: job.width,
            height,
            content,
            overflow_x: scroll_width.saturating_sub(job.width),
            overflow_y: content.saturating_sub(height),
            scale,
            console: console_lines(&self.tab),
            stuck,
            elements: elements.into_iter().take(2000).map(clean_element).collect(),
        };
        Ok(Drawn { report, png })
    }
}

fn content_of(measured: &Value) -> u32 {
    measured["content"].as_f64().unwrap_or(0.).clamp(0., 100_000.) as u32
}

fn clean_element(mut element: Element) -> Element {
    element.p = cut(&element.p, 64);
    element.t = cut(&element.t, 24);
    element.i = cut(&element.i, 60);
    element.c = cut(&element.c, 80);
    element.s = cut(&element.s, 60);
    for value in [&mut element.x, &mut element.y, &mut element.w, &mut element.h] {
        *value = if value.is_finite() { value.clamp(-100_000., 100_000.) } else { 0. };
    }
    element
}

/// The page's errors and warnings, newest last, as the report gives them.
fn console_lines(tab: &Tab) -> Vec<String> {
    let state = tab.state();
    let lines: Vec<String> = state
        .console
        .iter()
        .filter(|(_, entry)| matches!(entry.level, Level::Error | Level::Warning))
        // Chromium asks every page for a favicon; an artboard has none, and that is no error of its.
        .filter(|(_, entry)| !entry.source.as_deref().unwrap_or("").contains("/favicon.ico"))
        .map(|(_, entry)| {
            let source = entry.source.as_ref().map(|s| format!(" ({s})")).unwrap_or_default();
            cut(&format!("{}: {}{source}", entry.level.name(), entry.text), MAX_CONSOLE_CHARS)
        })
        .collect();
    let skip = lines.len().saturating_sub(MAX_CONSOLE_LINES);
    lines.into_iter().skip(skip).collect()
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_stay_under_the_pixel_budget() {
        assert_eq!(scale_for(1440, 900), 2.);
        let tall = scale_for(2560, 8000);
        assert!(tall < 1. && f64::from(2560) * f64::from(8000) * tall * tall <= MAX_PIXELS, "{tall}");
        assert_eq!(scale_for(390, 844), 2.);
    }

    #[test]
    fn page_text_is_cut_short() {
        assert_eq!(cut("abcdef", 4), "abc…");
        let element = clean_element(Element { s: "x".repeat(500), x: f64::NAN, y: 1e9, ..Element::default() });
        assert_eq!(element.s.chars().count(), 60);
        assert_eq!((element.x, element.y), (0., 100_000.));
    }
}
