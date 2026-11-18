//! Pages WackCode serves itself: a canvas's artboards, laid out by Chromium with no network.
//!
//! A *site* is a set of files the app owns: a canvas's folder, say. Its pages load from a reserved
//! origin, `https://<name>.canvas.invalid/`, which the engine answers in the browser process
//! (`cef.rs`, `WackSiteResources`). `.invalid` never resolves (RFC 6761), and the engine maps it
//! to nowhere too (`host-resolver-rules`), so a request the engine did not answer reaches no
//! network either. A real `https` origin, rather than a custom scheme, needs nothing registered in
//! Chromium's renderer processes (the helper), and gives each canvas its own storage.
//!
//! Invariants (docs/security.md, "Canvas"):
//! - A site's tab loads only its own origin. Every main-frame or frame navigation elsewhere, every
//!   subresource elsewhere (an image, a font, a `fetch`), every popup and every download is
//!   refused in native code, whatever the page's CSP says; the CSP the site sends is a second
//!   layer. `data:`, `blob:` and `about:` pass: they reach no network.
//! - A site's tab is never a chat's browser tab: it is not in `ChatTabs`, never saved and never
//!   reachable by the agent's `browser_*` tools.
//! - The rule here is pure and tested; `cef.rs` only applies it.

use url::Url;

/// The reserved domain every site's origin is under.
pub const SITE_DOMAIN: &str = "canvas.invalid";

/// One response a site gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Served {
    pub status: u16,
    pub mime: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Served {
    pub fn ok(mime: &str, body: Vec<u8>) -> Self {
        Self { status: 200, mime: mime.to_string(), headers: Vec::new(), body }
    }

    pub fn not_found() -> Self {
        Self::ok("text/plain", b"Not found".to_vec()).with_status(404)
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

/// Files a site serves. Called on Chromium's IO thread, so it must be quick and never block on
/// the main thread.
pub trait Site: Send + Sync {
    /// The site's name: its origin is `https://<name>.canvas.invalid`. Lower-case letters, digits
    /// and `-` only (`valid_name`).
    fn name(&self) -> &str;
    /// The file at `path` (no leading `/`, no query), or a 404.
    fn serve(&self, path: &str) -> Served;
}

/// True for a name a site's origin can carry: one DNS label of `a-z0-9-`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The site's origin, with no trailing slash.
pub fn origin(name: &str) -> String {
    format!("https://{name}.{SITE_DOMAIN}")
}

/// The address of `path` on the site.
pub fn address(name: &str, path: &str) -> String {
    format!("{}/{}", origin(name), path.trim_start_matches('/'))
}

/// What a request from a site's page gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    /// The site answers it with this path (decoded, no leading `/`).
    Serve(String),
    /// It reaches no network: `data:`, `blob:`, `about:`.
    Pass,
    /// Anything else, with why, for the page's console.
    Refuse(String),
}

/// The rule for a request (or navigation) to `raw` from a page of the site called `name`.
pub fn rule(name: &str, raw: &str) -> Rule {
    let Ok(url) = Url::parse(raw.trim()) else {
        return Rule::Refuse(format!("Not a valid address: {}", raw.trim()));
    };
    match url.scheme() {
        "data" | "blob" | "about" => return Rule::Pass,
        "https" => {}
        _ => return Rule::Refuse(refused(raw)),
    }
    let ours = url.host_str() == Some(&format!("{name}.{SITE_DOMAIN}"))
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    if !ours {
        return Rule::Refuse(refused(raw));
    }
    let Some(path) = decode(url.path()) else {
        return Rule::Refuse(format!("Not a valid path: {}", url.path()));
    };
    let path = path.trim_start_matches('/');
    if path.split('/').any(|part| part == ".." || part == "." || part.contains('\\')) {
        return Rule::Refuse(format!("Not a valid path: {path}"));
    }
    Rule::Serve(path.to_string())
}

fn refused(raw: &str) -> String {
    let shown: String = raw.chars().take(200).collect();
    format!("Canvas pages are offline: {shown} was not loaded.")
}

/// Percent-decodes a URL path; `None` when it is not UTF-8 or decodes to a NUL.
fn decode(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let text = String::from_utf8(out).ok()?;
    (!text.contains('\0')).then_some(text)
}

/// The type a site gives a file, from its extension. Anything unknown is served as bytes, so a
/// page cannot have a file it named run as script.
pub fn mime_of(path: &str) -> &'static str {
    let extension = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match extension.as_str() {
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_sites_own_origin_is_served() {
        assert_eq!(rule("c1", "https://c1.canvas.invalid/main.html"), Rule::Serve("main.html".into()));
        assert_eq!(rule("c1", "https://c1.canvas.invalid/assets/a%20b.png?x=1#f"), Rule::Serve("assets/a b.png".into()));
        assert_eq!(rule("c1", "https://c1.canvas.invalid/"), Rule::Serve(String::new()));
        for refused in [
            "https://c2.canvas.invalid/main.html",
            "https://canvas.invalid/main.html",
            "http://c1.canvas.invalid/main.html",
            "https://c1.canvas.invalid:8443/main.html",
            "https://user@c1.canvas.invalid/main.html",
            "https://example.com/x.png",
            "http://localhost:3000/",
            "file:///etc/passwd",
            "wss://c1.canvas.invalid/socket",
            "chrome://settings",
            "javascript:alert(1)",
            "not a url",
        ] {
            assert!(matches!(rule("c1", refused), Rule::Refuse(_)), "{refused}");
        }
    }

    #[test]
    fn requests_that_reach_no_network_pass() {
        for passing in ["data:image/png;base64,AAAA", "blob:https://c1.canvas.invalid/1234", "about:blank"] {
            assert_eq!(rule("c1", passing), Rule::Pass, "{passing}");
        }
    }

    #[test]
    fn paths_cannot_climb_out() {
        // The URL parser already folds `.` and `..` segments (and their escapes) inside the root.
        assert_eq!(rule("c1", "https://c1.canvas.invalid/../secrets.json"), Rule::Serve("secrets.json".into()));
        assert_eq!(rule("c1", "https://c1.canvas.invalid/a/%2e%2e/b"), Rule::Serve("b".into()));
        // An escaped slash or backslash would smuggle one back in after decoding.
        for path in ["/a%2F..%2F..%2Fb", "/a%2F.%2Fb", "/a%5cb", "/a%00b", "/%ff"] {
            let url = format!("https://c1.canvas.invalid{path}");
            assert!(matches!(rule("c1", &url), Rule::Refuse(_)), "{url}");
        }
    }

    #[test]
    fn names_are_one_dns_label() {
        assert!(valid_name("c-3f2a"));
        assert!(!valid_name(""));
        assert!(!valid_name("-a"));
        assert!(!valid_name("A"));
        assert!(!valid_name("a.b"));
        assert!(!valid_name(&"a".repeat(64)));
        assert_eq!(address("c1", "/main.html"), "https://c1.canvas.invalid/main.html");
    }

    #[test]
    fn unknown_files_are_bytes() {
        assert_eq!(mime_of("main.HTML"), "text/html");
        assert_eq!(mime_of("assets/x.woff2"), "font/woff2");
        assert_eq!(mime_of("notes.exe"), "application/octet-stream");
        assert_eq!(mime_of("noext"), "application/octet-stream");
    }
}
