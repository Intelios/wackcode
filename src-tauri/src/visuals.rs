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
//! Nothing here touches disk, paths or credentials, and the asset protocol's scope is untouched.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use tauri::{http, Manager, State};

/// The scheme the wrapper is served through. The app's CSP names it in `frame-src`.
pub const VISUAL_SCHEME: &str = "wcvisual";
/// The webview allowed to fetch a staged document: the app's own main webview.
pub const MAIN_WEBVIEW: &str = "main";
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

/// One staged document, with the theme it was staged under.
#[derive(Debug, Clone)]
struct Staged {
    html: String,
    theme: VisualTheme,
}

#[derive(Debug, Default)]
struct Inner {
    /// Oldest first, so eviction is a `pop_front`.
    order: VecDeque<String>,
    entries: HashMap<String, Staged>,
    bytes: usize,
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

    /// Keep `html` under a fresh token, evicting the oldest documents past the caps.
    pub fn stage(&self, html: String, theme: VisualTheme) -> Result<String, String> {
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
        if let Some(previous) = inner.entries.insert(token.clone(), Staged { html, theme }) {
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

    /// The ready-to-serve document for a token, or `None` once it has been evicted.
    pub fn document(&self, token: &str) -> Option<String> {
        let inner = self.inner.lock().ok()?;
        let staged = inner.entries.get(token)?;
        wrapper(&staged.html, &staged.theme).ok()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner.lock().map(|inner| inner.entries.len()).unwrap_or(0)
    }
}

fn wrapper(html: &str, theme: &VisualTheme) -> Result<String, String> {
    let declarations = theme.declarations()?;
    Ok(include_str!("visual_frame.html")
        .replace("/*__THEME__*/", &declarations)
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

/// Stage a visual and return the URL the card loads it from./// Stage a visual and return the URL the card loads it from.
#[tauri::command]
pub fn stage_visual(store: State<'_, VisualStore>, html: String, theme: VisualTheme) -> Result<String, String> {
    let token = store.stage(html, theme)?;
    Ok(format!("{VISUAL_SCHEME}://localhost/{token}"))
}

/// `GET wcvisual://localhost/<token>` → the wrapped document. Everything else, and every
/// evicted token, is a 404: nothing else is served through this scheme.
pub fn protocol_response(store: &VisualStore, request: &http::Request<Vec<u8>>, webview_label: &str) -> http::Response<Vec<u8>> {
    let not_found = || {
        http::Response::builder()
            .status(http::StatusCode::NOT_FOUND)
            .header(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(http::header::CACHE_CONTROL, "no-store")
            .body(b"Not found".to_vec())
            .expect("a static 404 response is valid")
    };
    if request.method() != http::Method::GET || webview_label != MAIN_WEBVIEW {
        return not_found();
    }
    let token = request.uri().path().trim_start_matches('/');
    if token.is_empty() || token.len() > 64 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
        return not_found();
    }
    let Some(document) = store.document(token) else { return not_found() };
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(http::header::CACHE_CONTROL, "no-store")
        .header("Content-Security-Policy", FRAME_CSP)
        .header("X-Content-Type-Options", "nosniff")
        .body(document.into_bytes())
        .expect("a static 200 response is valid")
}

/// The visual's own policy: no network of any kind, inline scripts and styles only (the model's
/// own), and no forms or base URLs. Images must be inline (`data:`) or blob references.
pub const FRAME_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; form-action 'none'; base-uri 'none'";

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
        let token = store.stage("<p>hi</p>".into(), theme()).unwrap();
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
        let document = store.document(&store.stage("<p/>".into(), theme()).unwrap()).unwrap();
        for name in ["--wc-accent:", "--wc-accent-line:", "--wc-toggle-on:", "--wc-knob:", "--wc-radius-card:", "--border-bright:", "--danger:"] {
            assert!(document.contains(name), "{name} is missing from the frame's stylesheet");
        }
        // The frame is still served with its own policy.
        assert_eq!(FRAME_CSP.split(';').count(), 9);
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
        assert!(store.stage("<p/>".into(), broken).is_err());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn evicts_the_oldest_documents_past_the_entry_cap() {
        let store = VisualStore::new();
        let mut tokens = Vec::new();
        for index in 0..(MAX_ENTRIES + 3) {
            tokens.push(store.stage(format!("<p>{index}</p>"), theme()).unwrap());
        }
        assert_eq!(store.len(), MAX_ENTRIES);
        assert!(store.document(&tokens[0]).is_none());
        assert!(store.document(&tokens[3]).is_some());
        assert!(store.document(tokens.last().unwrap()).is_some());
    }

    #[test]
    fn refuses_empty_and_oversized_documents() {
        let store = VisualStore::new();
        assert!(store.stage("   ".into(), theme()).is_err());
        let huge = "x".repeat(MAX_STAGE_BYTES + 1);
        assert!(store.stage(huge, theme()).is_err());
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
        let token = store.stage("<p>hi</p>".into(), theme()).unwrap();
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
