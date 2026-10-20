//! Visual replies, host side: the staged documents behind the `show_visual` tool, and the
//! `wcvisual:` protocol the desktop loads them through.
//!
//! A visual is model-authored HTML and JavaScript, so it runs in its own sandboxed browsing
//! context, never in the renderer:
//!
//! - The renderer hands the document to [`stage_visual`], which keeps it in [`VisualStore`] (in
//!   memory only, bounded) and returns a `wcvisual://localhost/<token>` URL.
//! - [`protocol_response`] serves that URL with the wrapper in `visual_frame.html` and a CSP of
//!   `default-src 'none'`, so the frame has no network, no fonts, no frames and no forms. The
//!   parent CSP allows the scheme through `frame-src`.
//! - The iframe carries `sandbox="allow-scripts"` (see `VisualCard.tsx`), which gives it an
//!   opaque origin: no access to the app's DOM, storage or `__TAURI_INTERNALS__` (Tauri's init
//!   scripts are main-frame-only, and the IPC layer also refuses a request without the
//!   per-launch invoke key). Its only way out is `postMessage`: a content height, a link the
//!   user clicked, and a bounded `wackcode.send` text the renderer gates further.
//! - [`navigation_plugin`] refuses every navigation the app does not need, so a script inside a
//!   visual cannot assign `location.href` and turn its frame into a remote page — the one way a
//!   CSP cannot close by itself, because a policy travels with the document that leaves.
//!
//! A canvas artboard (`kind: artboard`) is the same document in the same frame with a different
//! bridge: it paints its own background, never sends a message or opens a link, and reports its
//! script errors so `canvas_look` can show them (`canvas_render.rs`).
//!
//! Documents have an audience. The renderer's (`stage_visual`) are served to the main webview
//! only; the ones `canvas_look` stages for its offscreen `canvas-render` webview are served to
//! that webview only, together with the small shell page that frames them
//! (`canvas_render_shell.html`). Neither webview can fetch the other's documents.
//!
//! Nothing here touches disk, paths or credentials, and the asset protocol's scope is untouched.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use tauri::{http, Manager, State};

/// The scheme the wrapper is served through. The app's CSP names it in `frame-src`.
pub const VISUAL_SCHEME: &str = "wcvisual";
/// The webview allowed to fetch a staged document: the app's own main webview.
pub const MAIN_WEBVIEW: &str = "main";
/// The offscreen webview `canvas_look` renders artboards in (`canvas_render.rs`).
pub const RENDER_WEBVIEW: &str = "canvas-render";
/// A visual is capped at 64 KiB by the tool (`worker/src/builtin/visual.ts`); this is the host's
/// own backstop for anything that reaches the command another way.
pub const MAX_STAGE_BYTES: usize = 256 * 1024;
const MAX_ENTRIES: usize = 128;
const MAX_STORE_BYTES: usize = 16 * 1024 * 1024;
/// Long enough for any colour, font stack or length, short enough that a value cannot carry a
/// second declaration.
const MAX_VALUE_CHARS: usize = 200;

/// The theme the renderer sends along with a staged document: the handful of CSS variables the
/// wrapper defines before the model's markup runs.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualTheme {
    pub bg: String,
    pub surface: String,
    pub text: String,
    pub text_soft: String,
    pub text_dim: String,
    pub border: String,
    pub accent: String,
    pub accent_text: String,
    pub on_accent: String,
    pub font: String,
    pub mono: String,
    pub radius: String,
    /// `light` or `dark`, so form controls and scrollbars follow the app.
    pub scheme: String,
}

impl VisualTheme {
    /// The app's default dark theme (`:root` in `styles.css`, `visualThemeVars` in `theme.ts`),
    /// for a render the renderer has not themed yet this launch. Mirrored by hand, like
    /// `DEFAULT_BACKGROUND` in `glass.rs`.
    pub fn fallback() -> Self {
        Self {
            bg: "#111310".into(),
            surface: "#161914".into(),
            text: "#e8ece4".into(),
            text_soft: "#adb5a7".into(),
            text_dim: "#757e70".into(),
            border: "#2b3028".into(),
            accent: "#c2ee4a".into(),
            accent_text: "#c2ee4a".into(),
            on_accent: "#172000".into(),
            font: "Inter, ui-sans-serif, -apple-system, BlinkMacSystemFont, \"SF Pro Text\", \"Segoe UI\", sans-serif".into(),
            mono: "\"SFMono-Regular\", ui-monospace, Consolas, monospace".into(),
            radius: "14px".into(),
            scheme: "dark".into(),
        }
    }

    /// `(name, value)` pairs as the wrapper writes them. A value that is not a plain CSS value
    /// is refused: it is spliced into a `<style>` element.
    fn variables(&self) -> Result<Vec<(&'static str, &str)>, String> {
        let pairs = [
            ("--bg", self.bg.as_str()),
            ("--surface", self.surface.as_str()),
            ("--text", self.text.as_str()),
            ("--text-soft", self.text_soft.as_str()),
            ("--text-dim", self.text_dim.as_str()),
            ("--border", self.border.as_str()),
            ("--accent", self.accent.as_str()),
            ("--accent-text", self.accent_text.as_str()),
            ("--on-accent", self.on_accent.as_str()),
            ("--font", self.font.as_str()),
            ("--mono", self.mono.as_str()),
            ("--radius", self.radius.as_str()),
        ];
        for (name, value) in pairs {
            if !safe_css_value(value) {
                return Err(format!("{name} is not a plain CSS value."));
            }
        }
        if self.scheme != "light" && self.scheme != "dark" {
            return Err("scheme must be light or dark.".into());
        }
        Ok(pairs.to_vec())
    }

    /// The `:root{…}` body the wrapper's stylesheet starts with.
    fn declarations(&self) -> Result<String, String> {
        let pairs = self.variables()?;
        let mut css = String::with_capacity(320);
        for (name, value) in pairs {
            css.push_str(name);
            css.push(':');
            css.push_str(value);
            css.push(';');
        }
        css.push_str("color-scheme:");
        css.push_str(&self.scheme);
        css.push(';');
        Ok(css)
    }
}

/// Letters, digits and the punctuation real values use (`#111310`, `rgb(17 19 16 / 50%)`,
/// `Inter, ui-sans-serif, "SF Pro Text", sans-serif`, `8px`). No `<`, `>`, `;`, `{`, `}`,
/// `:` or `\`, so a value cannot end the declaration or the style element.
fn safe_css_value(value: &str) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= MAX_VALUE_CHARS
        && value.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, ' ' | '#' | '(' | ')' | ',' | '.' | '%' | '/' | '\'' | '"' | '-' | '+' | '_')
        })
}

/// What a staged document is: a visual reply in the transcript, or an artboard on a canvas.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VisualKind {
    #[default]
    Reply,
    Artboard,
}

impl VisualKind {
    fn as_str(self) -> &'static str {
        match self {
            VisualKind::Reply => "reply",
            VisualKind::Artboard => "artboard",
        }
    }
}

/// Who may fetch a staged document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Audience {
    /// The app's renderer (`stage_visual`).
    App,
    /// `canvas_look`'s offscreen render webview.
    Render,
}

impl Audience {
    fn of(webview_label: &str) -> Option<Self> {
        match webview_label {
            MAIN_WEBVIEW => Some(Audience::App),
            RENDER_WEBVIEW => Some(Audience::Render),
            _ => None,
        }
    }
}

/// One staged document, with the theme it was staged under.
#[derive(Debug, Clone)]
struct Staged {
    html: String,
    theme: VisualTheme,
    kind: VisualKind,
    audience: Audience,
}

#[derive(Debug, Default)]
struct Inner {
    /// Oldest first, so eviction is a `pop_front`.
    order: VecDeque<String>,
    entries: HashMap<String, Staged>,
    bytes: usize,
    /// The theme the renderer last staged under or announced (`set_visual_theme`), for documents
    /// the host stages itself (`canvas_render.rs`).
    theme: Option<VisualTheme>,
}

/// Staged visuals for this launch. In memory only: a visual is re-staged whenever it is drawn,
/// so nothing has to survive a restart (the model's HTML lives in the chat's own session file).
#[derive(Debug, Default)]
pub struct VisualStore {
    inner: Mutex<Inner>,
}

impl VisualStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keep `html` under a fresh token for the renderer, evicting the oldest documents past the caps.
    pub fn stage(&self, html: String, theme: VisualTheme, kind: VisualKind) -> Result<String, String> {
        self.keep(html, theme, kind, Audience::App)
    }

    /// Keep an artboard for `canvas_look`'s render webview, in the app's current theme.
    pub fn stage_for_render(&self, html: String) -> Result<String, String> {
        let theme = self.current_theme();
        self.keep(html, theme, VisualKind::Artboard, Audience::Render)
    }

    /// Drop a document nobody needs any more (a finished render's).
    pub fn forget(&self, token: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(removed) = inner.entries.remove(token) {
                inner.bytes = inner.bytes.saturating_sub(removed.html.len());
                inner.order.retain(|kept| kept != token);
            }
        }
    }

    fn keep(&self, html: String, theme: VisualTheme, kind: VisualKind, audience: Audience) -> Result<String, String> {
        if html.trim().is_empty() {
            return Err("A visual needs an HTML body.".into());
        }
        if html.len() > MAX_STAGE_BYTES {
            return Err("That visual is too large to draw.".into());
        }
        // Validates the theme before anything is kept.
        theme.declarations()?;
        let token = format!("{:x}", uuid::Uuid::new_v4().simple());
        let bytes = html.len();
        let mut inner = self.inner.lock().map_err(|_| "The visual store was poisoned.".to_string())?;
        if audience == Audience::App {
            inner.theme = Some(theme.clone());
        }
        if let Some(previous) = inner.entries.insert(token.clone(), Staged { html, theme, kind, audience }) {
            inner.bytes = inner.bytes.saturating_sub(previous.html.len());
        }
        inner.order.push_back(token.clone());
        inner.bytes += bytes;
        while inner.order.len() > MAX_ENTRIES || inner.bytes > MAX_STORE_BYTES {
            let Some(oldest) = inner.order.pop_front() else { break };
            if let Some(evicted) = inner.entries.remove(&oldest) {
                inner.bytes = inner.bytes.saturating_sub(evicted.html.len());
            }
        }
        Ok(token)
    }

    /// The ready-to-serve document for a renderer token, or `None` once it has been evicted.
    #[cfg(test)]
    fn document(&self, token: &str) -> Option<String> {
        self.document_for(token, Audience::App)
    }

    fn document_for(&self, token: &str, audience: Audience) -> Option<String> {
        let inner = self.inner.lock().ok()?;
        let staged = inner.entries.get(token).filter(|staged| staged.audience == audience)?;
        wrapper(&staged.html, &staged.theme, staged.kind).ok()
    }

    fn staged_for(&self, token: &str, audience: Audience) -> bool {
        self.inner.lock().map(|inner| inner.entries.get(token).is_some_and(|staged| staged.audience == audience)).unwrap_or(false)
    }

    /// Remember the app's current theme without staging anything.
    pub fn set_theme(&self, theme: VisualTheme) -> Result<(), String> {
        theme.declarations()?;
        let mut inner = self.inner.lock().map_err(|_| "The visual store was poisoned.".to_string())?;
        inner.theme = Some(theme);
        Ok(())
    }

    /// The theme a host-staged document should wear: the app's latest, or its default.
    pub fn current_theme(&self) -> VisualTheme {
        self.inner.lock().ok().and_then(|inner| inner.theme.clone()).unwrap_or_else(VisualTheme::fallback)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner.lock().map(|inner| inner.entries.len()).unwrap_or(0)
    }
}

fn wrapper(html: &str, theme: &VisualTheme, kind: VisualKind) -> Result<String, String> {
    let declarations = theme.declarations()?;
    // Both markers are filled before the model's markup goes in, so markup that happens to
    // contain one is never rewritten.
    Ok(include_str!("visual_frame.html")
        .replace("/*__THEME__*/", &declarations)
        .replace("__KIND__", kind.as_str())
        .replace("<!--__BODY__-->", html))
}

/// Whether a navigation may proceed: the app itself, a visual's frame, or a blank one.
///
/// The app is a single-page webview, so it never needs to leave its own origin, and a visual's
/// frame must never leave `wcvisual:`. Without this, a script inside a visual could assign
/// `location.href` and turn its sandboxed document into a remote page — a network request the
/// user never asked for. The frame's own CSP cannot forbid that (a policy belongs to the
/// document that leaves, not the one that arrives), so the refusal happens natively, before the
/// request is made. Pure, so the rule is testable without a webview.
pub fn navigation_allowed(url: &tauri::Url, app_origins: &[tauri::Url]) -> bool {
    match url.scheme() {
        // The bundled app's own protocol.
        "tauri" => true,
        // A staged visual document (and, through it, the frame's own reloads).
        VISUAL_SCHEME => true,
        // An empty frame, which any webview may create.
        "about" => url.path() == "blank",
        // The dev server, or a frontendDist served over HTTP(S).
        "http" | "https" => app_origins.iter().any(|origin| origin.origin() == url.origin()),
        _ => false,
    }
}

/// The origins the app itself is served from this run: the dev server, or a remote
/// `frontendDist`. A bundled build is `tauri://localhost`, which needs no origin list.
fn app_origins<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Vec<tauri::Url> {
    let config = app.config();
    let mut origins = Vec::new();
    if let Some(dev) = config.build.dev_url.clone() {
        origins.push(dev);
    }
    if let Some(tauri::utils::config::FrontendDist::Url(remote)) = config.build.frontend_dist.clone() {
        origins.push(remote);
    }
    origins
}

/// Refuses every navigation outside the app's own origins and a visual's frame
/// (`navigation_allowed`). Registered as a plugin, because Tauri only attaches a navigation
/// handler where one is built, and this window comes from `tauri.conf.json`.
pub fn navigation_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("wackcode-navigation")
        .on_navigation(|webview, url| navigation_allowed(url, &app_origins(webview.app_handle())))
        .build()
}

/// Stage a visual (or a canvas artboard) and return the URL its frame loads it from.
#[tauri::command]
pub fn stage_visual(store: State<'_, VisualStore>, html: String, theme: VisualTheme, kind: Option<VisualKind>) -> Result<String, String> {
    let token = store.stage(html, theme, kind.unwrap_or_default())?;
    Ok(format!("{VISUAL_SCHEME}://localhost/{token}"))
}

/// The app's theme, announced at launch and on every appearance change, so a document the host
/// stages itself (`canvas_look`'s renders) looks like the one on screen.
#[tauri::command]
pub fn set_visual_theme(store: State<'_, VisualStore>, theme: VisualTheme) -> Result<(), String> {
    store.set_theme(theme)
}

fn valid_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= 64 && token.chars().all(|c| c.is_ascii_hexdigit())
}

/// `GET wcvisual://localhost/<token>` → the wrapped document, to the webview it was staged for;
/// `GET wcvisual://localhost/render/<token>` → the shell that frames a render token, to the
/// render webview only. Everything else, and every evicted token, is a 404: nothing else is
/// served through this scheme.
pub fn protocol_response(store: &VisualStore, request: &http::Request<Vec<u8>>, webview_label: &str) -> http::Response<Vec<u8>> {
    let not_found = || {
        http::Response::builder()
            .status(http::StatusCode::NOT_FOUND)
            .header(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(http::header::CACHE_CONTROL, "no-store")
            .body(b"Not found".to_vec())
            .expect("a static 404 response is valid")
    };
    let Some(audience) = Audience::of(webview_label) else { return not_found() };
    if request.method() != http::Method::GET {
        return not_found();
    }
    let path = request.uri().path().trim_start_matches('/');
    let (document, policy) = if let Some(token) = path.strip_prefix("render/") {
        if audience != Audience::Render || !valid_token(token) || !store.staged_for(token, Audience::Render) {
            return not_found();
        }
        (include_str!("canvas_render_shell.html").replace("__TOKEN__", token), SHELL_CSP)
    } else {
        if !valid_token(path) {
            return not_found();
        }
        let Some(document) = store.document_for(path, audience) else { return not_found() };
        (document, FRAME_CSP)
    };
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(http::header::CACHE_CONTROL, "no-store")
        .header("Content-Security-Policy", policy)
        .header("X-Content-Type-Options", "nosniff")
        .body(document.into_bytes())
        .expect("a static 200 response is valid")
}

/// The visual's own policy: no network of any kind, inline scripts and styles only (the model's
/// own), and no forms or base URLs. Images must be inline (`data:`) or blob references.
pub const FRAME_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; form-action 'none'; base-uri 'none'";

/// The render shell's policy: its own inline script and style, and the one `wcvisual:` frame.
pub const SHELL_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; frame-src wcvisual:; connect-src 'none'; form-action 'none'; base-uri 'none'";

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> VisualTheme {
        VisualTheme {
            bg: "#111310".into(),
            surface: "#161914".into(),
            text: "#e8ece4".into(),
            text_soft: "#adb5a7".into(),
            text_dim: "#8a9384".into(),
            border: "#2b3028".into(),
            accent: "#d0f664".into(),
            accent_text: "#cce98a".into(),
            on_accent: "#172000".into(),
            font: "Inter, ui-sans-serif, -apple-system, \"SF Pro Text\", sans-serif".into(),
            mono: "\"SFMono-Regular\", Consolas, monospace".into(),
            radius: "8px".into(),
            scheme: "dark".into(),
        }
    }

    #[test]
    fn stages_and_serves_a_wrapped_document() {
        let store = VisualStore::new();
        let token = store.stage("<p>hi</p>".into(), theme(), VisualKind::Reply).unwrap();
        assert_eq!(token.len(), 32);
        let document = store.document(&token).expect("staged");
        assert!(document.contains("<p>hi</p>"));
        assert!(document.contains("--accent:#d0f664;"));
        assert!(document.contains("color-scheme:dark;"));
        assert!(!document.contains("/*__THEME__*/"));
        assert!(!document.contains("<!--__BODY__-->"));
    }

    #[test]
    fn answers_the_apps_own_token_names_inside_the_frame() {
        // A visual written against WackCode's styles (a Code chat can read them) uses `--wc-*`:
        // the frame must answer those names, or the model's literal fallbacks take over.
        let store = VisualStore::new();
        let document = store.document(&store.stage("<p/>".into(), theme(), VisualKind::Reply).unwrap()).unwrap();
        for name in ["--wc-accent:", "--wc-accent-line:", "--wc-toggle-on:", "--wc-knob:", "--wc-radius-card:", "--border-bright:", "--danger:"] {
            assert!(document.contains(name), "{name} is missing from the frame's stylesheet");
        }
        // The frame is still served with its own policy.
        assert_eq!(FRAME_CSP.split(';').count(), 9);
    }

    #[test]
    fn marks_an_artboard_and_leaves_markup_that_names_a_marker_alone() {
        let store = VisualStore::new();
        let reply = store.document(&store.stage("<p>__KIND__</p>".into(), theme(), VisualKind::Reply).unwrap()).unwrap();
        assert!(reply.contains("data-kind=\"reply\""));
        assert!(reply.contains("<p>__KIND__</p>"), "the model's markup is never rewritten");
        let artboard = store.document(&store.stage("<p/>".into(), theme(), VisualKind::Artboard).unwrap()).unwrap();
        assert!(artboard.contains("data-kind=\"artboard\""));
        assert!(!artboard.contains("__KIND__"));
    }

    #[test]
    fn remembers_the_latest_theme_and_falls_back_to_the_default() {
        let store = VisualStore::new();
        assert_eq!(store.current_theme().bg, "#111310");
        assert!(VisualTheme::fallback().declarations().is_ok());
        let mut light = theme();
        light.bg = "#f4f4f1".into();
        light.scheme = "light".into();
        store.set_theme(light).unwrap();
        assert_eq!(store.current_theme().bg, "#f4f4f1");
        store.stage("<p/>".into(), theme(), VisualKind::Artboard).unwrap();
        assert_eq!(store.current_theme().bg, "#111310");
        let mut broken = theme();
        broken.text = "red;}".into();
        assert!(store.set_theme(broken).is_err());
    }

    #[test]
    fn refuses_values_that_could_break_out_of_the_style_element() {
        let mut broken = theme();
        broken.accent = "#fff}</style><script>alert(1)</script>".into();
        assert!(broken.declarations().is_err());
        let mut bad_scheme = theme();
        bad_scheme.scheme = "dark; --x: y".into();
        assert!(bad_scheme.declarations().is_err());
        let store = VisualStore::new();
        assert!(store.stage("<p/>".into(), broken, VisualKind::Reply).is_err());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn evicts_the_oldest_documents_past_the_entry_cap() {
        let store = VisualStore::new();
        let mut tokens = Vec::new();
        for index in 0..(MAX_ENTRIES + 3) {
            tokens.push(store.stage(format!("<p>{index}</p>"), theme(), VisualKind::Reply).unwrap());
        }
        assert_eq!(store.len(), MAX_ENTRIES);
        assert!(store.document(&tokens[0]).is_none());
        assert!(store.document(&tokens[3]).is_some());
        assert!(store.document(tokens.last().unwrap()).is_some());
    }

    #[test]
    fn refuses_empty_and_oversized_documents() {
        let store = VisualStore::new();
        assert!(store.stage("   ".into(), theme(), VisualKind::Reply).is_err());
        let huge = "x".repeat(MAX_STAGE_BYTES + 1);
        assert!(store.stage(huge, theme(), VisualKind::Reply).is_err());
    }

    #[test]
    fn serves_each_webview_only_its_own_documents_and_the_shell_only_to_the_renderer() {
        let store = VisualStore::new();
        let app_token = store.stage("<p>app</p>".into(), theme(), VisualKind::Reply).unwrap();
        let render_token = store.stage_for_render("<p>render</p>".into()).unwrap();
        let get = |path: String| http::Request::builder().method("GET").uri(format!("{VISUAL_SCHEME}://localhost{path}")).body(Vec::new()).unwrap();
        let status = |path: String, label: &str| protocol_response(&store, &get(path), label).status();
        assert_eq!(status(format!("/{app_token}"), MAIN_WEBVIEW), http::StatusCode::OK);
        assert_eq!(status(format!("/{app_token}"), RENDER_WEBVIEW), http::StatusCode::NOT_FOUND);
        assert_eq!(status(format!("/{render_token}"), RENDER_WEBVIEW), http::StatusCode::OK);
        assert_eq!(status(format!("/{render_token}"), MAIN_WEBVIEW), http::StatusCode::NOT_FOUND);
        assert_eq!(status(format!("/render/{render_token}"), MAIN_WEBVIEW), http::StatusCode::NOT_FOUND);
        assert_eq!(status(format!("/render/{app_token}"), RENDER_WEBVIEW), http::StatusCode::NOT_FOUND);
        assert_eq!(status("/render/not-hex".into(), RENDER_WEBVIEW), http::StatusCode::NOT_FOUND);
        assert_eq!(status(format!("/{render_token}"), "browser-task"), http::StatusCode::NOT_FOUND);
        let shell = protocol_response(&store, &get(format!("/render/{render_token}")), RENDER_WEBVIEW);
        assert_eq!(shell.headers()["Content-Security-Policy"], SHELL_CSP);
        let body = String::from_utf8(shell.body().clone()).unwrap();
        assert!(body.contains(&format!("src=\"wcvisual://localhost/{render_token}\"")));
        assert!(body.contains("sandbox=\"allow-scripts\""));
        // A render document never becomes the theme the renderer's documents wear.
        let rendered = protocol_response(&store, &get(format!("/{render_token}")), RENDER_WEBVIEW);
        assert!(String::from_utf8(rendered.body().clone()).unwrap().contains("data-kind=\"artboard\""));
        store.forget(&render_token);
        assert_eq!(status(format!("/{render_token}"), RENDER_WEBVIEW), http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn allows_the_app_and_a_visual_frame_and_refuses_everything_else() {
        let origins = [tauri::Url::parse("http://localhost:1420").unwrap()];
        let allows = |url: &str| navigation_allowed(&tauri::Url::parse(url).unwrap(), &origins);
        // The app itself, in a bundle and under the dev server.
        assert!(allows("tauri://localhost/index.html"));
        assert!(allows("http://localhost:1420/chats/1"));
        // A staged document, and the blank frames a webview may create.
        assert!(allows("wcvisual://localhost/abc123"));
        assert!(allows("about:blank"));
        // A visual trying to leave: no request may be made.
        assert!(!allows("https://example.com"));
        assert!(!allows("http://127.0.0.1:1420/"));
        assert!(!allows("data:text/html,<script>alert(1)</script>"));
        assert!(!allows("file:///etc/passwd"));
        assert!(!allows("about:srcdoc"));
    }

    #[test]
    fn does_not_allow_a_lookalike_dev_server() {
        let origins = [tauri::Url::parse("http://localhost:1420").unwrap()];
        assert!(!navigation_allowed(&tauri::Url::parse("http://localhost:1421").unwrap(), &origins));
        assert!(!navigation_allowed(&tauri::Url::parse("https://localhost:1420").unwrap(), &origins));
    }

    #[test]
    fn serves_only_get_requests_for_hex_tokens_from_the_main_webview() {
        let store = VisualStore::new();
        let token = store.stage("<p>hi</p>".into(), theme(), VisualKind::Reply).unwrap();
        let get = |path: &str| {
            http::Request::builder()
                .method("GET")
                .uri(format!("{VISUAL_SCHEME}://localhost{path}"))
                .body(Vec::new())
                .unwrap()
        };
        let ok = protocol_response(&store, &get(&format!("/{token}")), MAIN_WEBVIEW);
        assert_eq!(ok.status(), http::StatusCode::OK);
        assert_eq!(ok.headers()["Content-Security-Policy"], FRAME_CSP);
        assert_eq!(ok.headers()[http::header::CACHE_CONTROL], "no-store");
        assert_eq!(protocol_response(&store, &get("/deadbeef"), MAIN_WEBVIEW).status(), http::StatusCode::NOT_FOUND);
        assert_eq!(protocol_response(&store, &get(&format!("/{token}")), "other").status(), http::StatusCode::NOT_FOUND);
        assert_eq!(protocol_response(&store, &get("/"), MAIN_WEBVIEW).status(), http::StatusCode::NOT_FOUND);
        assert_eq!(protocol_response(&store, &get("/../secrets.json"), MAIN_WEBVIEW).status(), http::StatusCode::NOT_FOUND);
        let post = http::Request::builder()
            .method("POST")
            .uri(format!("{VISUAL_SCHEME}://localhost/{token}"))
            .body(Vec::new())
            .unwrap();
        assert_eq!(protocol_response(&store, &post, MAIN_WEBVIEW).status(), http::StatusCode::NOT_FOUND);
    }
}
