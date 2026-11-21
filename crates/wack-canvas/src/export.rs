//! Exports: an artboard as a picture, a standalone page or a PDF, for the user to keep.
//!
//! - A picture is the board's own (`renders/`), drawn again first if it is a revision behind.
//! - A standalone page is what the canvas site serves, with its copied images inlined and the
//!   user's tweak settings in force. It runs agent-written script in whatever browser opens it,
//!   so it carries its own CSP as a `<meta>`: no network, no frames, no forms, as on the canvas.
//! - A PDF is printed by the engine from the live page at the artboard's size, one page tall
//!   per artboard height.

use std::{path::Path, sync::OnceLock};

use base64::Engine;
use regex::Regex;

use crate::{model::{Artboard, Canvas}, store};

/// The CSP a standalone page carries: the canvas's, for a page with no origin of its own.
pub const STANDALONE_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; form-action 'none'; base-uri 'none'; frame-src 'none'; worker-src 'none'; object-src 'none'; manifest-src 'none'";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Html,
    Pdf,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Html => "html",
            Format::Pdf => "pdf",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Png => "PNG picture",
            Format::Html => "HTML page",
            Format::Pdf => "PDF",
        }
    }
}

/// A file name for an export: `<canvas>-<artboard>.<ext>`.
pub fn file_name(canvas: &Canvas, artboard: &Artboard, format: Format) -> String {
    format!("{}-{}.{}", canvas.id, artboard.id, format.extension())
}

/// The artboard as one self-contained page: images inlined from `dir`'s `assets/`, tweaks as
/// set, and the CSP first, before anything the agent wrote.
pub fn standalone(dir: &Path, canvas: &Canvas, artboard: &Artboard) -> String {
    static ASSET: OnceLock<Regex> = OnceLock::new();
    let asset = ASSET.get_or_init(|| Regex::new(r"assets/([A-Za-z0-9]+\.[A-Za-z0-9]+)").expect("asset pattern"));
    let page = store::page(canvas, artboard);
    let page = asset.replace_all(&page, |found: &regex::Captures| {
        let name = &found[1];
        match std::fs::read(dir.join("assets").join(name)) {
            Ok(bytes) => format!("data:{};base64,{}", wack_browser::site::mime_of(name), base64::engine::general_purpose::STANDARD.encode(bytes)),
            Err(_) => found[0].to_string(),
        }
    });
    let meta = format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{STANDALONE_CSP}\">");
    // `page` always opens with a doctype (`store::page`): the CSP goes straight after it.
    match page.find('>') {
        Some(end) if page[..end].to_ascii_lowercase().contains("<!doctype") => format!("{}{meta}{}", &page[..=end], &page[end + 1..]),
        _ => format!("{meta}{page}"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_standalone_page_inlines_its_images_and_leads_with_its_csp() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("assets/abc123.png"), [137, 80, 78, 71]).unwrap();
        let mut planned = crate::model::plan(&[], &json!({ "title": "T", "artboards": [
            { "name": "A", "width": 400, "height": 300, "html": "<!DOCTYPE html><html><body><img src=\"assets/abc123.png\"><img src=\"assets/gone.png\"></body></html>" }
        ] }))
        .unwrap();
        let canvas = &mut planned.canvas;
        let page = standalone(dir.path(), canvas, &canvas.artboards[0]);
        assert!(page.starts_with("<!DOCTYPE html><meta http-equiv=\"Content-Security-Policy\""), "{}", &page[..80]);
        assert!(page.contains("src=\"data:image/png;base64,iVBORw==\""));
        assert!(page.contains("assets/gone.png"), "a missing image is left as it was");
        assert!(!STANDALONE_CSP.contains("'self'") && STANDALONE_CSP.contains("connect-src 'none'"));
        assert_eq!(file_name(canvas, &canvas.artboards[0], Format::Pdf), "t-a.pdf");
    }
}
