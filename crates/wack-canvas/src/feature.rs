//! What the canvas tells the model between requests: a note naming the tweaks the user moved
//! since it last heard, with their values now, so "keep that accent" means something and a
//! revision doesn't undo a setting the user chose; and a note of the comments the user sent with
//! their message (`comments::note`), which comes before that message.
//!
//! - Told once per change: `Canvases::take_tweaked` clears each artboard's `tweaks_untold` as it
//!   is read (kept in the index, so it survives a relaunch), and the note is saved in the session
//!   like any other the app writes (tag `canvas-tweaks`).
//! - Values are read when the note is written, so many moves of one slider make one line.

use wack_agent::{Feature, Note, Session};

use crate::{Canvas, Canvases, model::Artboard};

pub const TWEAKS_TAG: &str = "canvas-tweaks";
pub const COMMENTS_TAG: &str = "canvas-comments";

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
        let mut notes = self.tweaks_note().into_iter().collect::<Vec<_>>();
        // Comments sent with the user's message, armed as it started the run (the app's send).
        for (canvas, comments) in self.canvases.take_untold_comments() {
            notes.push(Note { tag: COMMENTS_TAG.into(), text: crate::comments::note(&canvas.title, &canvas.id, &comments) });
        }
        notes
    }
}

impl CanvasFeature {
    fn tweaks_note(&self) -> Option<Note> {
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
            return None;
        }
        Some(Note {
            tag: TWEAKS_TAG.into(),
            text: format!(
                "The user moved tweaks on the canvas. These are the values they see now; keep them when you revise these artboards unless they ask otherwise:\n{}",
                lines.join("\n")
            ),
        })
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

    #[test]
    fn comments_reach_the_model_once_with_the_message_that_sent_them() {
        let dir = tempfile::tempdir().unwrap();
        let canvases = Canvases::new(dir.path(), None);
        canvases.apply(&json!({ "title": "Settings", "artboards": [{ "name": "Calm", "width": 400, "height": 300, "html": "<p/>" }] })).unwrap();
        let anchor = crate::comments::Anchor { path: "0".into(), tag: "p".into(), ..Default::default() };
        canvases.add_comment("settings", "calm", 1, anchor, "Warmer").unwrap();
        assert_eq!(canvases.draft_count(), 1);
        assert!(canvases.take_untold_comments().is_empty(), "a draft waits for a message");
        assert_eq!(canvases.arm_drafts().len(), 1);
        // Armed, then the app quit before the request: the note still goes.
        let relaunched = Canvases::new(dir.path(), None);
        let told = relaunched.take_untold_comments();
        assert_eq!((told.len(), told[0].1[0].id.as_str()), (1, "k1"));
        assert!(relaunched.take_untold_comments().is_empty(), "told once");
        assert_eq!(relaunched.draft_count(), 0);
    }
}
