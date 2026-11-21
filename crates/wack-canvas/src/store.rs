//! Where a chat's canvases live on disk, and the site that serves their pages to Chromium.
//!
//! ```text
//! v3/canvases/<chat>/<canvas-id>/
//!   canvas.json            the index: title, artboards (sizes, groups, variants, tweaks), order
//!   <artboard>.html        an artboard's markup (a base, or a plain artboard)
//!   <artboard>.css         a variant's css
//!   assets/<sha256>.<ext>  local images the agent named, copied in
//!   comments.json          the user's comments (comments.rs)
//!   view.json              where the board's camera was left: x, y and zoom (the pane's own)
//!   renders/<id>@<rev>.png the latest picture of each artboard, and .json its report and map
//! ```
//!
//! Invariants:
//! - Every write is atomic (a temporary file, then a rename) and private (files 0600, folders
//!   0700). A crash leaves the old file or the new one, never half of one.
//! - The folder belongs to its chat and goes with it (`remove_all`).
//! - A canvas's pages are served only from its own folder, through `CanvasSite`: the document an
//!   artboard shows (`model::resolve_html`) with the runtime and the artboard's tweaks first, the
//!   copied assets, nothing else. The CSP it sends is the second layer; the engine's native rule
//!   (`wack_browser::site`) is the first.
//! - Local images are copied in when a call is accepted, so an artboard keeps working after the
//!   file moves. Their bytes decide their type; a name only finds them.

use std::{
    collections::HashMap,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use wack_browser::site::{self, Served, Site};

use crate::model::{self, Artboard, Canvas};

const INDEX: &str = "canvas.json";
/// A thumbnail's long edge, in pixels.
pub const THUMB_EDGE: u32 = 640;
const RUNTIME: &str = include_str!("runtime.js");
pub const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_ASSETS_BYTES: u64 = 64 * 1024 * 1024;

/// What the site sends with every page and file. Nothing leaves the page: the engine refuses
/// every other origin before this is read, and this says the same to Chromium.
pub const CSP: &str = "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; media-src 'self' data: blob:; connect-src 'self'; form-action 'none'; base-uri 'none'; frame-src 'none'; worker-src 'none'; object-src 'none'; manifest-src 'none'";

/// One chat's canvases.
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    /// What each canvas's site serves, kept in memory so Chromium's IO thread never reads the
    /// index. Shared with the sites; replaced whenever a canvas is saved.
    live: Arc<RwLock<HashMap<String, Arc<Canvas>>>>,
}

impl Store {
    /// The chat's folder, `v3/canvases/<chat>`. Nothing is created until a canvas is saved.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), live: Arc::default() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn dir(&self, canvas: &str) -> PathBuf {
        self.root.join(canvas)
    }

    /// Every canvas the chat has, oldest first, with its markup loaded. A canvas whose index
    /// can't be read is skipped (its folder stays for a later look).
    pub fn load_all(&self) -> Vec<Canvas> {
        let Ok(entries) = std::fs::read_dir(&self.root) else { return Vec::new() };
        let mut found: Vec<(std::time::SystemTime, Canvas)> = Vec::new();
        for entry in entries.flatten() {
            let dir = entry.path();
            let Ok(text) = std::fs::read_to_string(dir.join(INDEX)) else { continue };
            let Ok(mut canvas) = serde_json::from_str::<Canvas>(&text) else { continue };
            if canvas.id != entry.file_name().to_string_lossy() {
                continue;
            }
            for artboard in &mut canvas.artboards {
                artboard.html = std::fs::read_to_string(dir.join(format!("{}.html", artboard.id))).ok();
                artboard.css = std::fs::read_to_string(dir.join(format!("{}.css", artboard.id))).ok();
            }
            let created = std::fs::metadata(dir.join(INDEX)).and_then(|m| m.created()).unwrap_or(std::time::UNIX_EPOCH);
            found.push((created, canvas));
        }
        found.sort_by_key(|(created, _)| *created);
        let canvases: Vec<Canvas> = found.into_iter().map(|(_, canvas)| canvas).collect();
        let mut live = self.live.write().unwrap_or_else(|e| e.into_inner());
        for canvas in &canvases {
            live.insert(canvas.id.clone(), Arc::new(canvas.clone()));
        }
        canvases
    }

    /// Saves a canvas a call changed: the markup of `changed` artboards, the index, and away
    /// with the files of `removed` ones.
    pub fn save(&self, canvas: &Canvas, changed: &[String], removed: &[String]) -> Result<(), String> {
        let dir = self.dir(&canvas.id);
        private_dir(&self.root)?;
        private_dir(&dir)?;
        for artboard in canvas.artboards.iter().filter(|a| changed.contains(&a.id)) {
            write_or_remove(&dir.join(format!("{}.html", artboard.id)), artboard.html.as_deref())?;
            write_or_remove(&dir.join(format!("{}.css", artboard.id)), artboard.css.as_deref())?;
        }
        for id in removed {
            let _ = std::fs::remove_file(dir.join(format!("{id}.html")));
            let _ = std::fs::remove_file(dir.join(format!("{id}.css")));
            self.drop_renders(&canvas.id, id);
        }
        let index = serde_json::to_vec_pretty(canvas).map_err(|_| "Could not write the canvas's index.".to_string())?;
        write_atomic(&dir.join(INDEX), &index)?;
        self.live.write().unwrap_or_else(|e| e.into_inner()).insert(canvas.id.clone(), Arc::new(canvas.clone()));
        Ok(())
    }

    /// Saves only the index (a tweak the user set, say), leaving every artboard's markup alone.
    pub fn save_index(&self, canvas: &Canvas) -> Result<(), String> {
        self.save(canvas, &[], &[])
    }

    /// Removes every canvas of the chat: its folder and all in it.
    pub fn remove_all(&self) {
        let _ = std::fs::remove_dir_all(&self.root);
        self.live.write().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// The site a canvas's pages load from.
    pub fn site(&self, canvas: &str) -> Arc<dyn Site> {
        Arc::new(CanvasSite { name: site_name(&self.root, canvas), canvas: canvas.to_string(), dir: self.dir(canvas), live: self.live.clone() })
    }

    /// The address of an artboard's page; `rev` keeps a stale page from being reused.
    pub fn address(&self, canvas: &str, artboard: &Artboard) -> String {
        site::address(&site_name(&self.root, canvas), &format!("{}.html?rev={}", artboard.id, artboard.rev))
    }

    pub fn renders(&self, canvas: &str) -> PathBuf {
        self.dir(canvas).join("renders")
    }

    /// Where an artboard's picture of revision `rev` lives (`.png`, and `.json` its report).
    pub fn render_path(&self, canvas: &str, id: &str, rev: u64, extension: &str) -> PathBuf {
        self.renders(canvas).join(format!("{id}@{rev}.{extension}"))
    }

    /// Keeps a picture and its report, dropping the artboard's older ones.
    pub fn save_render(&self, canvas: &str, id: &str, rev: u64, png: &[u8], report: &Value) -> Result<(), String> {
        let renders = self.renders(canvas);
        private_dir(&self.dir(canvas))?;
        private_dir(&renders)?;
        self.drop_renders(canvas, id);
        write_atomic(&self.render_path(canvas, id, rev, "png"), png)?;
        let report = serde_json::to_vec(report).map_err(|_| "Could not write a canvas report.".to_string())?;
        write_atomic(&self.render_path(canvas, id, rev, "json"), &report)
    }

    /// A small copy of a picture, for the board at a distance and the transcript's card: drawn
    /// from the full one, its long edge `THUMB_EDGE`.
    pub fn save_thumbnail(&self, canvas: &str, id: &str, rev: u64, png: &[u8]) -> Result<(), String> {
        let picture = image::load_from_memory(png).map_err(|_| "Could not read a canvas picture.".to_string())?;
        let small = picture.thumbnail(THUMB_EDGE, THUMB_EDGE);
        let mut bytes = Vec::new();
        small.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).map_err(|_| "Could not make a canvas thumbnail.".to_string())?;
        write_atomic(&self.render_path(canvas, id, rev, "thumb.png"), &bytes)
    }

    fn drop_renders(&self, canvas: &str, id: &str) {
        let Ok(entries) = std::fs::read_dir(self.renders(canvas)) else { return };
        let prefix = format!("{id}@");
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    /// The saved report of an artboard's revision `rev`, if it has been drawn.
    pub fn render_report(&self, canvas: &str, id: &str, rev: u64) -> Option<Value> {
        let text = std::fs::read(self.render_path(canvas, id, rev, "json")).ok()?;
        serde_json::from_slice(&text).ok()
    }

    /// Copies the local images an artboard's markup or css names by absolute path into the
    /// canvas's `assets/`, and points the markup at the copies. A missing, oversized or
    /// non-image file refuses the call, naming the path.
    pub fn localize_images(&self, canvas: &str, artboard: &mut Artboard) -> Result<(), String> {
        let assets = self.dir(canvas).join("assets");
        let mut failures = Vec::new();
        for field in [&mut artboard.html, &mut artboard.css] {
            let Some(text) = field.as_ref() else { continue };
            let references = image_references(text);
            if references.is_empty() {
                continue;
            }
            let mut copies: HashMap<String, String> = HashMap::new();
            for path in references.iter().map(|r| r.2.clone()).collect::<std::collections::BTreeSet<_>>() {
                match copy_image(&assets, Path::new(&path)) {
                    Ok(name) => {
                        copies.insert(path, format!("assets/{name}"));
                    }
                    Err(reason) => failures.push(format!("{path}: {reason}")),
                }
            }
            if !failures.is_empty() {
                continue;
            }
            let mut out = String::with_capacity(text.len());
            let mut offset = 0;
            for (start, end, path) in &references {
                out.push_str(&text[offset..*start]);
                out.push_str(&copies[path]);
                offset = *end;
            }
            out.push_str(&text[offset..]);
            *field = Some(out);
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(format!("Could not include local images. Fix or remove these paths:\n{}", failures.join("\n")))
        }
    }
}

/// The canvas's origin name: one DNS label, unique to the chat and canvas, so no two canvases
/// share storage.
fn site_name(root: &Path, canvas: &str) -> String {
    let digest = Sha256::digest(format!("{}\0{canvas}", root.display()).as_bytes());
    let hex: String = digest.iter().take(10).map(|b| format!("{b:02x}")).collect();
    format!("c{hex}")
}

struct CanvasSite {
    name: String,
    canvas: String,
    dir: PathBuf,
    live: Arc<RwLock<HashMap<String, Arc<Canvas>>>>,
}

impl Site for CanvasSite {
    fn name(&self) -> &str {
        &self.name
    }

    fn serve(&self, path: &str) -> Served {
        let served = self.answer(path);
        served.with_header("Content-Security-Policy", CSP).with_header("Cache-Control", "no-store").with_header("X-Content-Type-Options", "nosniff")
    }
}

impl CanvasSite {
    fn answer(&self, path: &str) -> Served {
        if let Some(name) = path.strip_prefix("assets/") {
            if name.contains('/') || name.starts_with('.') {
                return Served::not_found();
            }
            return match std::fs::read(self.dir.join("assets").join(name)) {
                Ok(bytes) => Served::ok(site::mime_of(name), bytes),
                Err(_) => Served::not_found(),
            };
        }
        let Some(id) = path.strip_suffix(".html") else { return Served::not_found() };
        let canvas = self.live.read().unwrap_or_else(|e| e.into_inner()).get(&self.canvas).cloned();
        let Some(canvas) = canvas else { return Served::not_found() };
        let Some(artboard) = canvas.artboards.iter().find(|a| a.id == id) else { return Served::not_found() };
        Served::ok("text/html", page(&canvas, artboard).into_bytes())
    }
}

/// The page an artboard's site serves: its document with the runtime first.
pub fn page(canvas: &Canvas, artboard: &Artboard) -> String {
    let html = model::resolve_html(canvas, artboard);
    let tweaks: Vec<Value> = artboard
        .tweaks
        .iter()
        .map(|t| json!({ "var": t.var, "attr": t.attr, "value": t.css_value(t.value(&artboard.tweak_values)) }))
        .collect();
    let config = json!({ "tweaks": tweaks }).to_string().replace("</", "<\\/");
    let head = format!("<script>window.__wackCanvas={config};\n{RUNTIME}</script>");
    let (doctype, rest) = split_doctype(&html);
    match doctype {
        Some(doctype) => format!("{doctype}{head}{rest}"),
        // A fragment gets a standards-mode document, as version 2's frame gave it.
        None => format!("<!doctype html><meta charset=\"utf-8\">{head}{rest}"),
    }
}

/// The page's `<!doctype …>` (with anything before it) and the rest.
fn split_doctype(html: &str) -> (Option<&str>, &str) {
    let trimmed = html.trim_start_matches(['\u{feff}', ' ', '\n', '\r', '\t']);
    let lead = html.len() - trimmed.len();
    if trimmed.len() >= 9 && trimmed[..9].eq_ignore_ascii_case("<!doctype")
        && let Some(end) = trimmed.find('>')
    {
        let cut = lead + end + 1;
        return (Some(&html[..cut]), &html[cut..]);
    }
    (None, html)
}

/// Absolute image paths in quoted attributes and CSS `url(…)`: `(start, end, path)`.
fn image_references(text: &str) -> Vec<(usize, usize, String)> {
    static PATTERNS: std::sync::OnceLock<[Regex; 2]> = std::sync::OnceLock::new();
    let [attributes, urls] = PATTERNS.get_or_init(|| {
        let ext = r"(?:png|jpe?g|gif|webp|avif|bmp|ico|svg)";
        let dq = format!(r#""(/[^/"\r\n][^"\r\n]*?\.{ext})""#);
        let sq = format!(r"'(/[^/'\r\n][^'\r\n]*?\.{ext})'");
        [
            Regex::new(&format!(r"(?i)\b[\w:-]+\s*=\s*(?:{dq}|{sq})")).expect("attribute pattern"),
            Regex::new(&format!(r#"(?i)url\(\s*(?:{dq}|{sq}|(/[^/"'()\r\n][^"'()\r\n]*?\.{ext}))\s*\)"#)).expect("url pattern"),
        ]
    });
    let mut found: Vec<(usize, usize, String)> = Vec::new();
    for pattern in [attributes, urls] {
        for captures in pattern.captures_iter(text) {
            if let Some(m) = (1..=3).filter_map(|i| captures.get(i)).next()
                && !found.iter().any(|f| f.0 == m.start())
            {
                found.push((m.start(), m.end(), m.as_str().to_string()));
            }
        }
    }
    found.sort_by_key(|f| f.0);
    found
}

/// The type of an image from its bytes; `None` for anything that isn't one.
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]) {
        return Some("png");
    }
    if bytes.starts_with(&[255, 216, 255]) {
        return Some("jpg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("gif");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("webp");
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" && (&bytes[8..12] == b"avif" || &bytes[8..12] == b"avis") {
        return Some("avif");
    }
    if bytes.starts_with(b"BM") && bytes.len() >= 14 {
        return Some("bmp");
    }
    if bytes.len() >= 6 && bytes[..4] == [0, 0, 1, 0] {
        return Some("ico");
    }
    let text = std::str::from_utf8(&bytes[..bytes.len().min(4096)]).unwrap_or("").trim_start_matches('\u{feff}').trim_start();
    let text = text.strip_prefix("<?xml").map(|rest| rest.split_once("?>").map_or("", |(_, after)| after).trim_start()).unwrap_or(text);
    let mut text = text;
    while let Some(rest) = text.strip_prefix("<!--") {
        text = rest.split_once("-->").map_or("", |(_, after)| after).trim_start();
    }
    let text = text.strip_prefix("<!DOCTYPE svg").or_else(|| text.strip_prefix("<!doctype svg")).map(|rest| rest.split_once('>').map_or("", |(_, a)| a).trim_start()).unwrap_or(text);
    (text.starts_with("<svg") && text[4..].starts_with([' ', '>', '\n', '\t', '\r', '/'])).then_some("svg")
}

fn copy_image(assets: &Path, path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|_| "no such file".to_string())?;
    if !meta.is_file() {
        return Err("is not a regular image file".into());
    }
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(format!("is over the {} image limit", model::size_of(MAX_IMAGE_BYTES as usize)));
    }
    let bytes = std::fs::read(path).map_err(|_| "could not be read".to_string())?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err("changed while reading; try again".into());
    }
    let kind = image_type(&bytes).ok_or("is not a supported image (PNG, JPEG, GIF, WebP, AVIF, BMP, ICO or SVG)")?;
    let digest = Sha256::digest(&bytes);
    let name = format!("{}.{kind}", digest.iter().take(16).map(|b| format!("{b:02x}")).collect::<String>());
    let target = assets.join(&name);
    if !target.exists() {
        let used: u64 = std::fs::read_dir(assets).map(|d| d.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0);
        if used + bytes.len() as u64 > MAX_ASSETS_BYTES {
            return Err(format!("the canvas already holds {} of images", model::size_of(used as usize)));
        }
        if let Some(parent) = assets.parent() {
            private_dir(parent)?;
        }
        private_dir(assets)?;
        write_atomic(&target, &bytes)?;
    }
    Ok(name)
}

fn private_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|_| format!("Could not create {}.", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(|_| format!("Could not protect {}.", path.display()))
}

fn write_or_remove(path: &Path, text: Option<&str>) -> Result<(), String> {
    match text {
        Some(text) => write_atomic(path, text.as_bytes()),
        None => {
            let _ = std::fs::remove_file(path);
            Ok(())
        }
    }
}

/// Writes a private file whole or not at all.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let failed = || format!("Could not save {}.", path.display());
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).ok_or_else(failed)?;
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temporary).map_err(|_| failed())?;
    file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|_| failed())?;
    drop(file);
    std::fs::rename(&temporary, path).map_err(|_| {
        let _ = std::fs::remove_file(&temporary);
        failed()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::plan;

    fn canvas() -> Canvas {
        plan(&[], &json!({ "title": "T", "artboards": [
            { "name": "A", "width": 400, "html": "<!DOCTYPE html><html><body><p>a</p></body></html>",
              "tweaks": [{ "id": "accent", "label": "Accent", "kind": "color", "default": "#123456", "var": "--accent" }] },
            { "name": "B", "basedOn": "a", "css": "p{color:red}" },
            { "name": "C", "width": 300, "html": "<p>fragment</p>" }
        ] }))
        .unwrap()
        .canvas
    }

    #[test]
    fn a_saved_canvas_loads_back_whole_and_privately() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("chat"));
        let canvas = canvas();
        let ids: Vec<String> = canvas.artboards.iter().map(|a| a.id.clone()).collect();
        store.save(&canvas, &ids, &[]).unwrap();
        assert_eq!(store.load_all(), vec![canvas.clone()]);
        let mode = std::fs::metadata(store.dir("t").join("a.html")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(!store.dir("t").join("b.html").exists(), "a variant has only its css");

        let mut next = canvas.clone();
        next.artboards.retain(|a| a.id != "c");
        store.save(&next, &[], &["c".into()]).unwrap();
        assert!(!store.dir("t").join("c.html").exists());
        store.remove_all();
        assert!(!dir.path().join("chat").exists());
    }

    #[test]
    fn pages_get_the_runtime_first_and_keep_their_doctype() {
        let canvas = canvas();
        let full = page(&canvas, &canvas.artboards[0]);
        // Key order depends on serde_json's features in the build, so the keys are checked apart.
        assert!(full.starts_with("<!DOCTYPE html><script>window.__wackCanvas={\"tweaks\":[{"), "{}", &full[..120]);
        let config = &full[..full.find("};").unwrap()];
        assert!(config.contains("\"var\":\"--accent\"") && config.contains("\"value\":\"#123456\""), "{config}");
        assert!(full.ends_with("<html><body><p>a</p></body></html>"));
        let variant = page(&canvas, &canvas.artboards[1]);
        assert!(variant.ends_with("</html>\n<style>p{color:red}</style>"));
        let fragment = page(&canvas, &canvas.artboards[2]);
        assert!(fragment.starts_with("<!doctype html><meta charset=\"utf-8\"><script>"));
        assert!(fragment.ends_with("</script><p>fragment</p>"));
    }

    #[test]
    fn tweak_values_cannot_close_the_script() {
        let mut canvas = canvas();
        canvas.artboards[0].tweaks[0].kind = crate::model::TweakKind::Choice;
        canvas.artboards[0].tweak_values.insert("accent".into(), json!("</script><script>alert(1)</script>"));
        let page = page(&canvas, &canvas.artboards[0]);
        assert_eq!(page.matches("</script>").count(), 1);
    }

    #[test]
    fn the_site_serves_only_its_canvas() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("chat"));
        let canvas = canvas();
        store.save(&canvas, &["a".into()], &[]).unwrap();
        let site = store.site("t");
        assert!(site::valid_name(site.name()));
        assert_ne!(site.name(), Store::new(dir.path().join("other")).site("t").name(), "another chat's canvas has its own origin");
        let served = site.serve("a.html");
        assert_eq!((served.status, served.mime.as_str()), (200, "text/html"));
        assert!(served.headers.iter().any(|(n, v)| n == "Content-Security-Policy" && v == CSP));
        for missing in ["nope.html", "canvas.json", "a.css", "assets/../canvas.json", "renders/a@1.png", ""] {
            assert_eq!(site.serve(missing).status, 404, "{missing}");
        }
        assert!(store.address("t", &canvas.artboards[0]).starts_with(&format!("https://{}.canvas.invalid/a.html?rev=1", site.name())));
    }

    #[test]
    fn local_images_are_copied_in_and_named_by_their_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("shot.png");
        std::fs::write(&png, [137, 80, 78, 71, 13, 10, 26, 10, 0, 0]).unwrap();
        let text_file = dir.path().join("notes.png");
        std::fs::write(&text_file, "not an image").unwrap();
        let store = Store::new(dir.path().join("chat"));
        let mut artboard = Artboard {
            html: Some(format!("<img src=\"{0}\"><div style=\"background:url('{0}')\"></div><a href=\"https://x/y.png\">", png.display())),
            css: Some(format!("body{{background:url({})}}", png.display())),
            ..Artboard::default()
        };
        store.localize_images("t", &mut artboard).unwrap();
        let html = artboard.html.unwrap();
        assert_eq!(html.matches("assets/").count(), 2, "{html}");
        assert!(html.contains("https://x/y.png"), "only absolute paths are local");
        assert!(artboard.css.unwrap().starts_with("body{background:url(assets/"));
        let assets: Vec<_> = std::fs::read_dir(store.dir("t").join("assets")).unwrap().collect();
        assert_eq!(assets.len(), 1, "the same bytes are copied once");

        let mut bad = Artboard { html: Some(format!("<img src=\"{}\"><img src=\"/no/such.png\">", text_file.display())), ..Artboard::default() };
        let error = store.localize_images("t", &mut bad).unwrap_err();
        assert!(error.contains("notes.png: is not a supported image") && error.contains("/no/such.png: no such file"), "{error}");
    }

    #[test]
    fn svg_is_known_by_its_root_element() {
        assert_eq!(image_type(b"<?xml version=\"1.0\"?>\n<!-- x --><svg xmlns=\"\"></svg>"), Some("svg"));
        assert_eq!(image_type(b"<html><svg></svg></html>"), None);
        assert_eq!(image_type(b"<svgfoo>"), None);
    }

    #[test]
    fn renders_keep_only_the_latest_revision() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("chat"));
        store.save_render("t", "a", 1, b"one", &json!({"content": 1})).unwrap();
        store.save_render("t", "a", 2, b"two", &json!({"content": 2})).unwrap();
        assert!(!store.render_path("t", "a", 1, "png").exists());
        assert_eq!(store.render_report("t", "a", 2), Some(json!({"content": 2})));
        assert_eq!(store.render_report("t", "a", 1), None);
    }
}
