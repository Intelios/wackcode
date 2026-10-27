//! Shared renderer for `canvas_look`, visual previews and reply height measurements: draws
//! documents offscreen, the way the app shows them, and
//! reports back what came out — a screenshot (only when the chat's model can see images), the
//! rendered size, how far content runs past it, and the artboard's script errors.
//!
//! One lazily created child webview (`visuals::RENDER_WEBVIEW`) parked far offscreen in the main
//! window, like the browser preview's hidden pages, so WebKit keeps rendering it without it ever
//! covering the app. It is incognito, may navigate only to `wcvisual:` and `about:blank`, never
//! downloads or opens a window, and has its Tauri globals removed.
//!
//! The model's markup never runs in that webview's main frame. Each artboard is staged for the
//! render webview alone (`VisualStore::stage_for_render`) and loaded inside the shell page
//! (`canvas_render_shell.html`), which frames it with the same `sandbox="allow-scripts"`, opaque
//! origin and `FRAME_CSP` it has on the canvas, then reads the frame's own size and error reports.
//! Renders run one at a time, each artboard's document is dropped once it is drawn, and the
//! webview closes after two idle minutes.
//!
//! The Canvas tab's Download menu uses the same renderer for a PNG (`export_artboard`): the user
//! picks where it goes in the native save panel, so the renderer process never names a path.
use crate::visuals::{VisualKind, VisualStore, RENDER_WEBVIEW, VISUAL_SCHEME};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::Deserialize;
use tauri_plugin_dialog::DialogExt;
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tauri::{
    webview::{NewWindowResponse, WebviewBuilder},
    AppHandle, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl,
};
use tokio::sync::Mutex as AsyncMutex;

/// `canvas_look` asks for at most four; this is the host's own backstop.
const MAX_ARTBOARDS: usize = 4;
const MIN_SIZE: f64 = 200.0;
const MAX_WIDTH: f64 = 2560.0;
const MAX_HEIGHT: f64 = 4000.0;
const MAX_PREVIEW_HEIGHT: f64 = 20_000.0;
/// What an auto-height artboard is laid out at before it reports its own height.
const AUTO_START_HEIGHT: f64 = 900.0;
const READY_TIMEOUT: Duration = Duration::from_secs(8);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// A resized auto-height artboard gets this long to lay out again before its snapshot.
const RELAYOUT: Duration = Duration::from_millis(150);
const IDLE_TEARDOWN: Duration = Duration::from_secs(120);
const PARKED: (f64, f64) = (-10_000.0, -10_000.0);

/// The shell needs no IPC, so the injected Tauri globals go, as on the browser preview's pages.
const INIT_SCRIPT: &str = "(() => { try { delete window.__TAURI__; delete window.__TAURI_INTERNALS__; } catch (_) {} })();";

/// What the shell exposes as `window.__wcRender`, read before each snapshot.
const READ_SCRIPT: &str = "(() => { const s = window.__wcRender; return s ? { token: s.token, ready: s.ready, height: s.height, width: s.width, errors: s.errors.slice(0, 5) } : null; })()";

#[derive(Default)]
pub struct CanvasRenderer {
    view: AsyncMutex<Option<Webview>>,
    /// Bumped by every render, so an idle teardown never closes a view a later render is using.
    generation: AtomicU64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenderRequest {
    op: String,
    #[serde(default)]
    images: bool,
    appearance: Option<RenderAppearance>,
    artboards: Vec<ArtboardRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RenderAppearance {
    Light,
    Dark,
}

impl RenderAppearance {
    fn scheme(&self) -> &str {
        match self { Self::Light => "light", Self::Dark => "dark" }
    }
}

#[derive(Debug, Deserialize)]
struct ArtboardRequest {
    id: String,
    #[serde(default = "default_render_kind")]
    kind: VisualKind,
    html: String,
    width: f64,
    height: Option<f64>,
}

fn default_render_kind() -> VisualKind {
    VisualKind::Artboard
}

impl ArtboardRequest {
    fn max_height(&self) -> f64 {
        if self.kind == VisualKind::Preview { MAX_PREVIEW_HEIGHT } else { MAX_HEIGHT }
    }
}

#[derive(Debug, Default, Deserialize)]
struct ShellReport {
    #[serde(default)]
    token: String,
    #[serde(default)]
    ready: bool,
    #[serde(default)]
    height: f64,
    #[serde(default)]
    width: f64,
    #[serde(default)]
    errors: Vec<String>,
}

fn validate(request: &RenderRequest) -> Result<(), String> {
    if request.op != "render" {
        return Err("That canvas operation isn't supported.".into());
    }
    if request.artboards.is_empty() {
        return Err("There are no artboards to render.".into());
    }
    if request.artboards.len() > MAX_ARTBOARDS {
        return Err(format!("Render at most {MAX_ARTBOARDS} artboards at a time."));
    }
    for artboard in &request.artboards {
        if artboard.kind == VisualKind::Reply {
            return Err("Render an artboard or a visual preview.".into());
        }
        if artboard.id.is_empty() || artboard.id.len() > 64 {
            return Err("An artboard id is missing or too long.".into());
        }
        let (min_width, max_width) = if artboard.kind == VisualKind::Preview { (240.0, 1600.0) } else { (MIN_SIZE, MAX_WIDTH) };
        if !(min_width..=max_width).contains(&artboard.width) {
            return Err(format!("“{}” must be {min_width}–{max_width} pixels wide.", artboard.id));
        }
        let max_height = artboard.max_height();
        if artboard.height.is_some_and(|height| !(MIN_SIZE..=max_height).contains(&height)) {
            return Err(format!("“{}” must be {MIN_SIZE}–{max_height} pixels tall.", artboard.id));
        }
    }
    Ok(())
}

/// Runs one `canvas_request` from a worker and answers with every artboard's report. An artboard
/// that fails to draw reports why in its own `errors`; only a malformed request fails the call.
pub async fn execute_agent_request(app: AppHandle, _task_id: String, _request_id: String, request: Value) -> Result<Value, String> {
    let request: RenderRequest = serde_json::from_value(request).map_err(|_| "That canvas request was malformed.".to_string())?;
    validate(&request)?;
    let renderer = app.state::<CanvasRenderer>();
    let mut slot = renderer.view.lock().await;
    let generation = renderer.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let view = current_view(&app, &mut slot)?;
    let mut results = Vec::with_capacity(request.artboards.len());
    for artboard in &request.artboards {
        results.push(render_one(&app, &view, artboard, request.images, request.appearance.as_ref().map(RenderAppearance::scheme)).await);
    }
    if let Ok(blank) = "about:blank".parse() {
        let _ = view.navigate(blank);
    }
    drop(slot);
    schedule_teardown(app.clone(), generation);
    Ok(json!({ "artboards": results }))
}

fn current_view(app: &AppHandle, slot: &mut Option<Webview>) -> Result<Webview, String> {
    if let Some(view) = slot.as_ref() {
        return Ok(view.clone());
    }
    let view = create(app)?;
    *slot = Some(view.clone());
    Ok(view)
}

/// What the Canvas tab's Download menu saves: one artboard's resolved document at its real size.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtboardExport {
    name: String,
    html: String,
    width: f64,
    height: f64,
    format: ExportFormat,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum ExportFormat {
    Png,
    Html,
}

/// A resolved artboard is its markup plus a variant's stylesheet; the worker caps both well below this.
const MAX_EXPORT_HTML: usize = 512 * 1024;

fn validate_export(export: &ArtboardExport) -> Result<(), String> {
    if export.html.len() > MAX_EXPORT_HTML {
        return Err("That artboard is too large to save.".into());
    }
    if !(MIN_SIZE..=MAX_WIDTH).contains(&export.width) || !(MIN_SIZE..=MAX_HEIGHT).contains(&export.height) {
        return Err("That artboard's size is outside what the canvas draws.".into());
    }
    Ok(())
}

/// The artboard's name as a file name: letters, digits, spaces, dashes and underscores only.
fn export_file_stem(name: &str) -> String {
    let kept: String = name.chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')).take(80).collect();
    // Dropping a separator such as "·" leaves its spaces behind; keep one.
    let stem = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    if stem.is_empty() { "Artboard".into() } else { stem }
}

/// A standalone page around an artboard's body markup, so it opens in any browser.
fn export_html_document(name: &str, html: &str) -> String {
    let title = name.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{title}</title>\n<style>html, body {{ margin: 0; }}</style>\n</head>\n<body>\n{html}\n</body>\n</html>\n")
}

/// Saves one artboard as a PNG (drawn offscreen at the screen's scale) or a standalone HTML page,
/// wherever the user picks in the native save panel. `false` when they cancel.
#[tauri::command]
pub async fn export_artboard(app: AppHandle, artboard: ArtboardExport) -> Result<bool, String> {
    validate_export(&artboard)?;
    let (extension, label) = if artboard.format == ExportFormat::Png { ("png", "PNG image") } else { ("html", "HTML page") };
    let Some(picked) = app
        .dialog()
        .file()
        .set_title("Save artboard")
        .set_file_name(format!("{}.{extension}", export_file_stem(&artboard.name)))
        .add_filter(label, &[extension])
        .blocking_save_file()
    else {
        return Ok(false);
    };
    let path = picked.into_path().map_err(|_| "That location can't be saved to.".to_string())?;
    let bytes = match artboard.format {
        ExportFormat::Html => export_html_document(&artboard.name, &artboard.html).into_bytes(),
        ExportFormat::Png => render_png(&app, &artboard).await?,
    };
    std::fs::write(&path, bytes).map_err(|error| format!("Couldn't save the artboard: {error}"))?;
    Ok(true)
}

async fn render_png(app: &AppHandle, export: &ArtboardExport) -> Result<Vec<u8>, String> {
    let request = ArtboardRequest { id: "export".into(), kind: VisualKind::Artboard, html: export.html.clone(), width: export.width, height: Some(export.height.round()) };
    let renderer = app.state::<CanvasRenderer>();
    let mut slot = renderer.view.lock().await;
    let generation = renderer.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let view = current_view(app, &mut slot)?;
    let store = app.state::<VisualStore>();
    let outcome = match store.stage_for_render(request.html.clone(), request.kind, None) {
        Ok(token) => {
            let rendered = render_token(&view, &token, &request, true).await;
            store.forget(&token);
            rendered
        }
        Err(error) => Err(error),
    };
    if let Ok(blank) = "about:blank".parse() {
        let _ = view.navigate(blank);
    }
    drop(slot);
    schedule_teardown(app.clone(), generation);
    let data = outcome?
        .get("image")
        .and_then(|image| image.get("data"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "The artboard couldn't be drawn.".to_string())?;
    BASE64.decode(data).map_err(|_| "The artboard couldn't be drawn.".to_string())
}

fn create(app: &AppHandle) -> Result<Webview, String> {
    let window = app.get_window("main").ok_or_else(|| "The main window is unavailable.".to_string())?;
    let blank = "about:blank".parse().map_err(|_| "Could not start the canvas renderer.".to_string())?;
    let builder = WebviewBuilder::new(RENDER_WEBVIEW, WebviewUrl::External(blank))
        .incognito(true)
        .initialization_script(INIT_SCRIPT)
        .on_navigation(|url| url.scheme() == VISUAL_SCHEME || url.as_str() == "about:blank")
        .on_download(|_, _| false)
        .on_new_window(|_, _| NewWindowResponse::Deny);
    let view = window
        .add_child(builder, LogicalPosition::new(PARKED.0, PARKED.1), LogicalSize::new(1440.0, AUTO_START_HEIGHT))
        .map_err(|error| format!("Could not start the canvas renderer: {error}"))?;
    view.show().map_err(|error| error.to_string())?;
    Ok(view)
}

fn resize(view: &Webview, width: f64, height: f64) -> Result<(), String> {
    view.set_position(LogicalPosition::new(PARKED.0, PARKED.1)).map_err(|error| error.to_string())?;
    view.set_size(LogicalSize::new(width, height)).map_err(|error| error.to_string())
}

fn failure(artboard: &ArtboardRequest, message: String) -> Value {
    json!({ "id": artboard.id, "width": artboard.width, "height": artboard.height.unwrap_or(AUTO_START_HEIGHT), "errors": [message] })
}

async fn render_one(app: &AppHandle, view: &Webview, artboard: &ArtboardRequest, images: bool, appearance: Option<&str>) -> Value {
    let store = app.state::<VisualStore>();
    let token = match store.stage_for_render(artboard.html.clone(), artboard.kind, appearance) {
        Ok(token) => token,
        Err(error) => return failure(artboard, error),
    };
    let outcome = render_token(view, &token, artboard, images).await;
    store.forget(&token);
    outcome.unwrap_or_else(|error| failure(artboard, error))
}

async fn render_token(view: &Webview, token: &str, artboard: &ArtboardRequest, images: bool) -> Result<Value, String> {
    let width = artboard.width.round();
    let fixed = artboard.height.map(f64::round);
    let mut height = fixed.unwrap_or(AUTO_START_HEIGHT);
    resize(view, width, height)?;
    let url = format!("{VISUAL_SCHEME}://localhost/render/{token}");
    view.navigate(url.parse().map_err(|_| "Could not load the artboard.".to_string())?)
        .map_err(|error| format!("Could not load the artboard: {error}"))?;
    let mut report = wait_ready(view, token).await?;
    if fixed.is_none() {
        let measured = report.height.clamp(MIN_SIZE, artboard.max_height()).round();
        if (measured - height).abs() >= 1.0 {
            height = measured;
            resize(view, width, height)?;
            tokio::time::sleep(RELAYOUT).await;
            report = wait_ready(view, token).await.unwrap_or(report);
        }
    }
    let image = if images {
        let (bytes, pixels_wide, pixels_high) = crate::browser::screenshot(view).await?;
        Some(json!({ "data": BASE64.encode(bytes), "mimeType": "image/png", "width": pixels_wide, "height": pixels_high }))
    } else {
        None
    };
    Ok(json!({
        "id": artboard.id,
        "width": width,
        "height": height,
        "contentWidth": report.width,
        "contentHeight": report.height,
        "errors": report.errors,
        "image": image,
    }))
}

/// Polls the shell until this token's frame has loaded and gone quiet. Reads that fail while
/// the page is still navigating are just "not yet".
async fn wait_ready(view: &Webview, token: &str) -> Result<ShellReport, String> {
    let started = Instant::now();
    let mut last = ShellReport::default();
    while started.elapsed() < READY_TIMEOUT {
        if let Ok(value) = crate::browser::eval_page_value(view, READ_SCRIPT).await {
            if let Ok(report) = serde_json::from_value::<ShellReport>(value) {
                if report.token == token {
                    if report.ready {
                        return Ok(report);
                    }
                    last = report;
                }
            }
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    let mut message = "The artboard didn't finish drawing within 8 seconds; a script may be stuck in a loop.".to_string();
    if let Some(error) = last.errors.first() {
        message.push_str(&format!(" Its first error: {error}"));
    }
    Err(message)
}

fn schedule_teardown(app: AppHandle, generation: u64) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(IDLE_TEARDOWN).await;
        let renderer = app.state::<CanvasRenderer>();
        if renderer.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        let mut slot = renderer.view.lock().await;
        if renderer.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Some(view) = slot.take() {
            let _ = view.close();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(artboards: Value) -> RenderRequest {
        serde_json::from_value(json!({ "op": "render", "images": true, "artboards": artboards })).unwrap()
    }

    #[test]
    fn accepts_up_to_four_artboards_within_the_canvas_sizes() {
        assert!(validate(&request(json!([{ "id": "a", "html": "<p/>", "width": 1440, "height": 900 }]))).is_ok());
        assert!(validate(&request(json!([{ "id": "a", "html": "<p/>", "width": 390 }]))).is_ok());
        assert!(validate(&request(json!([]))).is_err());
        let five: Vec<Value> = (0..5).map(|index| json!({ "id": format!("a{index}"), "html": "<p/>", "width": 400 })).collect();
        assert!(validate(&request(Value::Array(five))).is_err());
        assert!(validate(&request(json!([{ "id": "a", "html": "<p/>", "width": 100 }]))).is_err());
        assert!(validate(&request(json!([{ "id": "a", "html": "<p/>", "width": 400, "height": 9000 }]))).is_err());
        assert!(validate(&request(json!([{ "id": "", "html": "<p/>", "width": 400 }]))).is_err());
        let mut other = request(json!([{ "id": "a", "html": "<p/>", "width": 400 }]));
        other.op = "navigate".into();
        assert!(validate(&other).is_err());
    }

    #[test]
    fn defaults_to_artboard_and_accepts_preview_sizes_without_widening_artboards() {
        let default = request(json!([{ "id": "a", "html": "<p/>", "width": 720 }]));
        assert_eq!(default.artboards[0].kind, VisualKind::Artboard);
        let preview = request(json!([{ "id": "p", "kind": "preview", "html": "<p/>", "width": 720, "height": 20_000 }]));
        assert!(validate(&preview).is_ok());
        assert_eq!(preview.artboards[0].kind, VisualKind::Preview);
        for (width, height) in [(239, 900), (1601, 900), (720, 20_001)] {
            assert!(validate(&request(json!([{ "id": "p", "kind": "preview", "html": "<p/>", "width": width, "height": height }]))).is_err());
        }
        assert!(validate(&request(json!([{ "id": "p", "kind": "reply", "html": "<p/>", "width": 720 }]))).is_err());
        assert!(validate(&request(json!([{ "id": "a", "html": "<p/>", "width": 720, "height": 20_000 }]))).is_err());
        assert!(serde_json::from_value::<RenderRequest>(json!({ "op": "render", "artboards": [{ "id": "p", "kind": "remote", "html": "<p/>", "width": 720 }] })).is_err());
    }

    #[test]
    fn accepts_only_explicit_light_or_dark_render_appearances() {
        for appearance in ["light", "dark"] {
            let request: RenderRequest = serde_json::from_value(json!({ "op": "render", "appearance": appearance, "artboards": [{ "id": "p", "kind": "preview", "html": "<p/>", "width": 720 }] })).unwrap();
            assert_eq!(request.appearance.as_ref().unwrap().scheme(), appearance);
            assert!(validate(&request).is_ok());
        }
        assert!(serde_json::from_value::<RenderRequest>(json!({ "op": "render", "appearance": "auto", "artboards": [] })).is_err());
    }

    #[test]
    fn names_exports_safely_and_wraps_html_as_a_page() {
        assert_eq!(export_file_stem("Ticket screen (interactive)"), "Ticket screen interactive");
        assert_eq!(export_file_stem("../../etc/passwd"), "etcpasswd");
        assert_eq!(export_file_stem("  ✦  "), "Artboard");
        assert_eq!(export_file_stem("Base · Phone"), "Base Phone");
        let page = export_html_document("A <b>", "<main>Hi</main>");
        assert!(page.starts_with("<!doctype html>"));
        assert!(page.contains("<title>A &lt;b&gt;</title>"));
        assert!(page.contains("<main>Hi</main>"));
    }

    #[test]
    fn refuses_exports_outside_the_canvas_sizes() {
        let export = |width: f64, height: f64| ArtboardExport { name: "A".into(), html: String::new(), width, height, format: ExportFormat::Png };
        assert!(validate_export(&export(390.0, 844.0)).is_ok());
        assert!(validate_export(&export(100.0, 844.0)).is_err());
        assert!(validate_export(&export(390.0, 99_999.0)).is_err());
    }

    #[test]
    fn reads_the_shell_report_and_tolerates_missing_fields() {
        let report: ShellReport = serde_json::from_value(json!({ "token": "ab", "ready": true, "height": 1234.5, "width": 1440, "errors": ["TypeError: x"] })).unwrap();
        assert!(report.ready);
        assert_eq!(report.errors, vec!["TypeError: x".to_string()]);
        let empty: ShellReport = serde_json::from_value(json!({})).unwrap();
        assert!(!empty.ready && empty.token.is_empty());
    }

    #[test]
    fn the_apps_capability_reaches_its_own_webview_and_no_child_of_its_window() {
        // Tauri treats a registered scheme (`wcvisual:`) as a local origin, and a capability
        // matches a webview by its own label or its window's. Naming only the app's webview keeps
        // the render shell — a child of the main window — without any permission.
        let capability: Value = serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        assert_eq!(capability["webviews"], json!([crate::visuals::MAIN_WEBVIEW]));
        assert!(capability.get("windows").is_none());
        assert_ne!(RENDER_WEBVIEW, crate::visuals::MAIN_WEBVIEW);
    }

    #[test]
    fn the_shell_reports_its_own_token_so_a_stale_page_is_never_read() {
        let shell = include_str!("canvas_render_shell.html");
        assert!(shell.contains("token: \"__TOKEN__\""));
        assert!(shell.contains("sandbox=\"allow-scripts\""));
    }
}
