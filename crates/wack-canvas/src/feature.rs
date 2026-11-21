//! What the canvas tells the model between requests: a note naming the tweaks the user moved
//! since it last heard, with their values now, so "keep that accent" means something and a
//! revision doesn't undo a setting the user chose.
//!
//! - Told once per change: `Canvases::take_tweaked` clears each artboard's `tweaks_untold` as it
//!   is read (kept in the index, so it survives a relaunch), and the note is saved in the session
//!   like any other the app writes (tag `canvas-tweaks`).
//! - Values are read when the note is written, so many moves of one slider make one line.

use wack_agent::{Feature, Note, Session};

use crate::{Canvas, Canvases, model::Artboard};

pub const TWEAKS_TAG: &str = "canvas-tweaks";

pub struct CanvasFeature {
    canvases: Canvases,
}

impl CanvasFeature {
    pub fn new(canvases: Canvases) -> Self {
        Self { canvases }
    }
}

impl Feature for CanvasFeature {
    fn notes(&self, _session: &Session) -> Vec<Note> {
        let tweaked = self.canvases.take_tweaked();
        let lines: Vec<String> = tweaked
            .iter()
            .filter_map(|(canvas, artboard)| {
                let found = self.canvases.get(canvas)?;
                let board = found.artboards.iter().find(|a| &a.id == artboard)?;
                Some(tweak_line(&found, board))
            })
            .collect();
        if lines.is_empty() {
            return Vec::new();
        }
        vec![Note {
            tag: TWEAKS_TAG.into(),
            text: format!(
                "The user moved tweaks on the canvas. These are the values they see now; keep them when you revise these artboards unless they ask otherwise:\n{}",
                lines.join("\n")
            ),
        }]
    }
}

/// One artboard's tweaks as the user left them: "Accent #e85d2f (default #2f6df6), Dark on".
pub fn tweak_line(canvas: &Canvas, artboard: &Artboard) -> String {
    let settings: Vec<String> = artboard
        .tweaks
        .iter()
        .map(|tweak| {
            let value = tweak.css_value(tweak.value(&artboard.tweak_values));
            if artboard.tweak_values.contains_key(&tweak.id) {
                format!("{} {value} (default {})", tweak.label, tweak.css_value(&tweak.default))
            } else {
                format!("{} {value} (its default)", tweak.label)
            }
        })
        .collect();
    format!("- “{}” (canvas: \"{}\"), artboard {}: {}", canvas.title, canvas.id, artboard.id, settings.join(", "))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_note_names_each_moved_tweak_once_with_its_value_now() {
        let dir = tempfile::tempdir().unwrap();
        let canvases = Canvases::new(dir.path(), None);
        canvases
            .apply(&json!({ "title": "Settings", "artboards": [{ "name": "Calm", "width": 400, "height": 300, "html": "<p/>", "tweaks": [
                { "id": "accent", "label": "Accent", "kind": "color", "default": "#2f6df6", "var": "--accent" },
                { "id": "dark", "label": "Dark", "kind": "toggle", "default": false, "attr": "theme" }
            ] }] }))
            .unwrap();
        canvases.set_tweak("settings", "calm", "accent", Some(json!("#e85d2f"))).unwrap();
        canvases.set_tweak("settings", "calm", "accent", Some(json!("#111111"))).unwrap();
        let found = canvases.get("settings").unwrap();
        let line = tweak_line(&found, &found.artboards[0]);
        assert_eq!(line, "- “Settings” (canvas: \"settings\"), artboard calm: Accent #111111 (default #2f6df6), Dark off (its default)");
        assert_eq!(canvases.take_tweaked(), vec![("settings".to_string(), "calm".to_string())]);
        assert!(canvases.take_tweaked().is_empty(), "told once");
        // A move not yet told outlives a relaunch.
        canvases.set_tweak("settings", "calm", "dark", Some(json!(true))).unwrap();
        let relaunched = Canvases::new(dir.path(), None);
        assert_eq!(relaunched.take_tweaked(), vec![("settings".to_string(), "calm".to_string())]);
        assert!(Canvases::new(dir.path(), None).take_tweaked().is_empty(), "telling is saved");
    }
}
