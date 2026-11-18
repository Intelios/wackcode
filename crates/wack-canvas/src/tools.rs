//! The agent's canvas tools: `canvas_guide`, `canvas` and `canvas_look`.
//!
//! All three are deferred, so their rules live in their descriptions and the guide. They are
//! never given to sub-agents (`wack_core::run` adds them after the sub-agents' share is taken),
//! and are offered only where a canvas can be drawn (`Canvases::available`).
//!
//! Invariants:
//! - `canvas` returns once the artboards it changed are drawn, with a report of each (size,
//!   overflow, console errors and warnings), so the model learns of a broken artboard from the
//!   call that broke it rather than from a screenshot it has to ask for.
//! - `canvas_look` draws only what has no picture of its current revision, and returns pictures
//!   only to a model that can see them; its text makes sense alone.
//! - Markup handed back is shortened where it carries image bytes (`collapse_data_urls`), so a
//!   picture inlined in an artboard never floods the context.
//! - Plan mode lets all three run: they change nothing in the project, only the chat's canvases
//!   (`wack_agent::plan`).

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use serde_json::{Value, json};
use wack_agent::{Tool, ToolContext, ToolOutput, ToolSpec, tool::Loading};
use wack_llm::Image;

use crate::{
    canvases::Canvases,
    comments,
    guide,
    model::{self, Canvas, MAX_ARTBOARDS_PER_CALL},
    render::Report,
};

pub const CANVAS: &str = "canvas";
pub const CANVAS_GUIDE: &str = "canvas_guide";
pub const CANVAS_LOOK: &str = "canvas_look";
pub const NAMES: [&str; 3] = [CANVAS_GUIDE, CANVAS, CANVAS_LOOK];
/// Artboards one `canvas_look` returns pictures of.
const LOOK_AT_ONCE: usize = 6;

/// The canvas tools for a chat's canvases; none where canvases can't be drawn.
pub fn tools(canvases: Canvases) -> Vec<Arc<dyn Tool>> {
    if !canvases.available() {
        return Vec::new();
    }
    vec![Arc::new(Guide), Arc::new(Draw { canvases: canvases.clone() }), Arc::new(Look { canvases })]
}

/// Calls the tool `name` as a run would, outside one: for the dev app's automation socket.
pub async fn call(canvases: Canvases, name: &str, input: Value) -> Result<ToolOutput, String> {
    let tool = tools(canvases).into_iter().find(|tool| tool.spec().name == name).ok_or_else(|| format!("There is no canvas tool {name}."))?;
    tool.run(input, &ToolContext::detached("/")).await
}

struct Guide;
struct Draw {
    canvases: Canvases,
}
struct Look {
    canvases: Canvases,
}

#[async_trait]
impl Tool for Guide {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: CANVAS_GUIDE.into(),
            description: "Read the canvas guide: how to draw artboards on a canvas, explore directions, make themes and tweaks, act on the user's comments, stay true to the project, and craft. Read it before your first canvas call in a chat; ask for sections to read just those.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sections": { "type": "array", "items": { "type": "string", "enum": guide::SECTIONS }, "description": "The sections to read. Omit for the whole guide." }
                }
            }),
            loading: Loading::Deferred,
            parallel_safe: true,
            hint: "the canvas guide: read it before drawing on a canvas".into(),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, _context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let sections: Vec<String> = input["sections"].as_array().map(|list| list.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        Ok(guide::guide(&sections).into())
    }
}

#[async_trait]
impl Tool for Draw {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: CANVAS.into(),
            description: format!(
                "Add, revise or remove artboards on a canvas: a board of whole HTML mockups the user pans, zooms, plays, comments on and tweaks in the Canvas pane beside the chat. Use it to explore UI directions side by side; a single chart or diagram doesn't need one. Read canvas_guide before your first call in a chat.\n\n\
                 Each artboard is a self-contained, offline HTML page at a fixed width (and a fixed height, or as tall as its content). Start a canvas with a title; later calls pass its id. Revise an artboard by its id with only the fields that change, and prefer `edits` (exact find-and-replace on the current markup) to resending it. Up to {MAX_ARTBOARDS_PER_CALL} artboards a call.\n\n\
                 The result reports every artboard it drew: its size, overflow, and console errors and warnings. Fix what it shows, then keep your reply short and ask which direction to take."
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "canvas": { "type": "string", "description": "An existing canvas's id. Omit to start a new canvas (then give a title)." },
                    "title": { "type": "string", "description": "The canvas's title, 1–60 characters. Required to start one; on an existing canvas it renames it." },
                    "artboards": {
                        "type": "array",
                        "maxItems": MAX_ARTBOARDS_PER_CALL,
                        "description": "Artboards to add (no id) or revise (by id; only the fields given change).",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string", "description": "Revise this artboard." },
                                "name": { "type": "string", "description": "A short name the user will say out loud. Required to add." },
                                "group": { "type": "string", "description": "The row it sits in, e.g. a direction or a screen." },
                                "width": { "type": "number", "description": "CSS pixels, 200–2560: 1440 desktop, 1280 laptop, 834 tablet, 390 phone." },
                                "height": { "type": "number", "description": "CSS pixels, 200–8000. Omit to fit the content." },
                                "html": { "type": "string", "description": "The whole page: markup with its <style> and <script>. Offline: no URLs, web fonts or CDNs." },
                                "edits": { "type": "array", "description": "Changes to the current markup (a variant's: its css), each old string matching exactly once.", "items": { "type": "object", "properties": { "old": { "type": "string" }, "new": { "type": "string" } }, "required": ["old", "new"] } },
                                "basedOn": { "type": "string", "description": "Make this a variant of another artboard (id or name): its markup, then this css." },
                                "css": { "type": "string", "description": "CSS appended after the markup; a variant's own styles." },
                                "notes": { "type": "string", "description": "One line on what it bets on." },
                                "tweaks": { "type": "array", "description": "Levers the user moves without asking you (canvas_guide › tweaks). Replaces the list.", "items": { "type": "object" } }
                            }
                        }
                    },
                    "remove": { "type": "array", "items": { "type": "string" }, "description": "Artboard ids to remove." },
                    "resolve_comments": { "type": "array", "items": { "type": "string" }, "description": "Ids of the user's comments this call settles." }
                }
            }),
            loading: Loading::Deferred,
            parallel_safe: false,
            hint: "draw UI mockups as artboards on a canvas beside the chat, to compare directions (read canvas_guide first)".into(),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, _context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let canvases = self.canvases.clone();
        let planned = tokio::task::spawn_blocking({
            let canvases = canvases.clone();
            move || canvases.apply(&input)
        })
        .await
        .map_err(|_| "The canvas could not be saved.".to_string())??;
        let resolved = comments::resolve(canvases.store(), &planned.canvas.id, &planned.resolve_comments)?;
        let stale = model::stale(&planned);
        let drawn = canvases.draw(&planned.canvas.id, &stale).await;
        Ok(summary(&planned, &drawn, &resolved).into())
    }
}

/// What a `canvas` call tells the model.
fn summary(planned: &model::Planned, drawn: &[(String, Result<Report, String>)], resolved: &[String]) -> String {
    let canvas = &planned.canvas;
    let mut lines = Vec::new();
    let verb = if planned.created { "Started" } else { "Updated" };
    lines.push(format!("{verb} canvas “{}” (canvas: \"{}\").", canvas.title, canvas.id));
    let added: Vec<&str> = planned.applied.iter().filter(|a| a.op == model::Op::Add).map(|a| a.id.as_str()).collect();
    let revised: Vec<&str> = planned.applied.iter().filter(|a| a.op == model::Op::Update).map(|a| a.id.as_str()).collect();
    if !added.is_empty() {
        lines.push(format!("Added: {}.", added.join(", ")));
    }
    if !revised.is_empty() {
        lines.push(format!("Revised: {}.", revised.join(", ")));
    }
    if !planned.removed.is_empty() {
        lines.push(format!("Removed: {}.", planned.removed.join(", ")));
    }
    if !resolved.is_empty() {
        lines.push(format!("Resolved comments: {}.", resolved.join(", ")));
    }
    if !drawn.is_empty() {
        lines.push(String::new());
        lines.push("Drawn:".into());
        for (id, result) in drawn {
            let Some(artboard) = canvas.artboards.iter().find(|a| &a.id == id) else { continue };
            lines.push(format!("- {}", report_line(artboard, result)));
        }
    }
    lines.push(String::new());
    lines.push(format!("Artboards on “{}”:", canvas.title));
    for artboard in &canvas.artboards {
        lines.push(format!("- {}", model::describe(artboard)));
    }
    lines.push(String::new());
    lines.push("The user sees the canvas now. Fix what the report shows; then keep your reply short and ask which direction to take.".into());
    lines.join("\n")
}

/// What one `canvas` call did, read back from its result: the transcript keeps only the text,
/// so its card is built from this. Kept beside `summary`, which writes what it reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub created: bool,
    pub canvas: String,
    pub title: String,
    pub added: Vec<String>,
    pub revised: Vec<String>,
    pub removed: Vec<String>,
    /// The drawn artboards whose report named a problem: an error, overflow, a stuck page.
    pub problems: Vec<String>,
}

pub fn outcome(result: &str) -> Option<Outcome> {
    let mut lines = result.lines();
    let first = lines.next()?;
    let created = first.starts_with("Started canvas “");
    let rest = first.strip_prefix("Started canvas “").or_else(|| first.strip_prefix("Updated canvas “"))?;
    let split = rest.rfind("” (canvas: \"")?;
    let title = rest[..split].to_string();
    let canvas = rest[split + "” (canvas: \"".len()..].strip_suffix("\").")?.to_string();
    let ids = |line: &str, label: &str| -> Option<Vec<String>> {
        let list = line.strip_prefix(label)?.strip_suffix('.')?;
        Some(list.split(", ").map(str::to_string).filter(|id| !id.is_empty()).collect())
    };
    let mut outcome = Outcome { created, canvas, title, ..Outcome::default() };
    let mut drawn = false;
    for line in lines {
        if let Some(list) = ids(line, "Added: ") {
            outcome.added = list;
        } else if let Some(list) = ids(line, "Revised: ") {
            outcome.revised = list;
        } else if let Some(list) = ids(line, "Removed: ") {
            outcome.removed = list;
        } else if line == "Drawn:" {
            drawn = true;
        } else if line.is_empty() {
            drawn = false;
        } else if drawn && let Some(entry) = line.strip_prefix("- ") {
            let id = entry.split(':').next().unwrap_or_default();
            // Errors, overflow, a stuck page or no picture: what the board badges. Warnings alone are not.
            let errors = entry.contains("Console: error:") || entry.contains(" | error:");
            let problem = errors || entry.contains("could not be drawn") || entry.contains("overflows") || entry.contains("cut off") || entry.contains("never settled");
            if problem && !id.is_empty() {
                outcome.problems.push(id.to_string());
            }
        }
    }
    Some(outcome)
}

fn report_line(artboard: &model::Artboard, result: &Result<Report, String>) -> String {
    let report = match result {
        Ok(report) => report,
        Err(error) => return format!("{}: could not be drawn. {error}", artboard.id),
    };
    let mut parts = vec![format!("{}: drawn {}×{}.", artboard.id, report.width, report.height)];
    if report.stuck {
        parts.push("Its page never settled: a script may be stuck in a loop.".into());
    }
    if report.overflow_y > 0 {
        parts.push(if artboard.height.is_some() {
            format!("Content overflows the bottom by {} px.", report.overflow_y)
        } else {
            format!("Content is cut off {} px below {} px.", report.overflow_y, model::MAX_ARTBOARD_HEIGHT)
        });
    }
    if report.overflow_x > 0 {
        parts.push(format!("Content overflows the right edge by {} px (horizontal scroll).", report.overflow_x));
    }
    if report.console.is_empty() {
        parts.push("No console errors.".into());
    } else {
        parts.push(format!("Console: {}", report.console.join(" | ")));
    }
    parts.join(" ")
}

#[async_trait]
impl Tool for Look {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: CANVAS_LOOK.into(),
            description: format!(
                "Look at artboards as the canvas shows them: a screenshot of each (for models that can see images), its drawn size, overflow and console errors, the user's tweak settings and open comments, and optionally its exact markup. Up to {LOOK_AT_ONCE} artboards a call; omit artboards for the ones changed last. Use html: true to get an artboard's markup back, to edit it or to build a chosen design."
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "canvas": { "type": "string", "description": "The canvas's id. May be omitted when the chat has one canvas." },
                    "artboards": { "type": "array", "items": { "type": "string" }, "description": "Artboard ids or names. Omit for the ones changed last." },
                    "html": { "type": "boolean", "description": "Also return each artboard's markup." }
                }
            }),
            loading: Loading::Deferred,
            parallel_safe: true,
            hint: "see artboards on a canvas: screenshots, sizes, console errors, the user's tweaks and comments, markup".into(),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, _context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let canvases = self.canvases.clone();
        let all = tokio::task::spawn_blocking({
            let canvases = canvases.clone();
            move || canvases.list()
        })
        .await
        .map_err(|_| "The canvases could not be read.".to_string())?;
        let canvas = pick_canvas(&all, input["canvas"].as_str())?;
        let wanted: Vec<String> = match input["artboards"].as_array() {
            Some(list) if !list.is_empty() => {
                let mut ids = Vec::new();
                for value in list {
                    let name = value.as_str().ok_or("artboards must be a list of ids or names.")?;
                    let found = canvas
                        .artboards
                        .iter()
                        .find(|a| a.id == name.trim() || a.name.eq_ignore_ascii_case(name.trim()))
                        .ok_or_else(|| format!("There is no artboard “{name}” on “{}”.", canvas.title))?;
                    if !ids.contains(&found.id) {
                        ids.push(found.id.clone());
                    }
                }
                ids
            }
            _ if !canvas.last_changed.is_empty() => canvas.last_changed.clone(),
            _ => canvas.artboards.iter().map(|a| a.id.clone()).collect(),
        };
        let (now, later) = wanted.split_at(wanted.len().min(LOOK_AT_ONCE));
        let show_html = input["html"].as_bool().unwrap_or(false);
        let open = comments::open(canvases.store(), &canvas.id);

        let mut text = vec![format!("Canvas “{}” (canvas: \"{}\").", canvas.title, canvas.id)];
        let mut images = Vec::new();
        for id in now {
            let Some(artboard) = canvas.artboards.iter().find(|a| &a.id == id) else { continue };
            // A picture of the current revision is reused; anything else is drawn now.
            let report = match canvases.report(&canvas.id, id).filter(|r| r.rev == artboard.rev) {
                Some(report) => Ok(report),
                None => canvases.draw(&canvas.id, std::slice::from_ref(id)).await.remove(0).1,
            };
            text.push(String::new());
            text.push(format!("{} — {}", model::describe(artboard), report_line(artboard, &report)));
            for tweak in &artboard.tweaks {
                let set = artboard.tweak_values.get(&tweak.id);
                let value = tweak.css_value(tweak.value(&artboard.tweak_values));
                text.push(match set {
                    Some(_) => format!("Tweak {} ({}): the user set {value}.", tweak.id, tweak.label),
                    None => format!("Tweak {} ({}): at its default, {value}.", tweak.id, tweak.label),
                });
            }
            for comment in open.iter().filter(|c| &c.artboard == id) {
                text.push(format!("Open comment {}: “{}” on {}.", comment.id, comment.body, comment.anchor.describe()));
            }
            if report.is_ok()
                && let Some((path, rev)) = canvases.picture(&canvas.id, id)
                && rev == artboard.rev
                && let Ok(bytes) = std::fs::read(&path)
            {
                text.push(format!("Screenshot of {id} follows."));
                let data = base64::engine::general_purpose::STANDARD.encode(bytes);
                images.push(Image::new("image/png", data).labelled(format!("canvas {} artboard {id}", canvas.id)));
            }
            if show_html {
                let markup = match &artboard.based_on {
                    Some(base) => format!("(a variant of {base}; its css:)\n{}", artboard.css.as_deref().unwrap_or("")),
                    None => artboard.html.clone().unwrap_or_default(),
                };
                text.push(format!("```html\n{}\n```", collapse_data_urls(&markup)));
            }
        }
        if !later.is_empty() {
            text.push(String::new());
            text.push(format!("Not looked at in this call (look at them next): {}.", later.join(", ")));
        }
        Ok(ToolOutput::with_images(text.join("\n"), images))
    }
}

fn pick_canvas<'a>(all: &'a [Canvas], wanted: Option<&str>) -> Result<&'a Canvas, String> {
    match wanted.map(str::trim).filter(|w| !w.is_empty()) {
        Some(id) => all.iter().find(|c| c.id == id).ok_or_else(|| {
            let known: Vec<&str> = all.iter().map(|c| c.id.as_str()).collect();
            format!("There is no canvas “{id}” in this chat.{}", if known.is_empty() { String::new() } else { format!(" Canvases: {}.", known.join(", ")) })
        }),
        None => match all {
            [] => Err("This chat has no canvas yet. Start one with the canvas tool.".into()),
            [only] => Ok(only),
            many => Err(format!("Name the canvas: {}.", many.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join(", "))),
        },
    }
}

/// Image bytes in markup never echo back into the model's context whole.
pub fn collapse_data_urls(html: &str) -> String {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN.get_or_init(|| regex::Regex::new(r"(?i)data:([a-z0-9.+-]+/[a-z0-9.+-]+);base64,([A-Za-z0-9+/=]{257,})").expect("data url pattern"));
    pattern.replace_all(html, |c: &regex::Captures| format!("data:{};base64,…({} characters)", &c[1], c[2].len())).into_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn what_summary_writes_outcome_reads() {
        let planned = model::plan(&[], &json!({ "title": "Sign-up, “v2”", "artboards": [
            { "name": "Calm", "width": 400, "height": 600, "html": "<p/>" },
            { "name": "Loud", "width": 400, "height": 600, "html": "<p/>" },
            { "name": "Quiet", "width": 400, "height": 600, "html": "<p/>" }
        ] }))
        .unwrap();
        let report = |console: &[&str], overflow_y| Report { width: 400, height: 600, overflow_y, console: console.iter().map(|s| s.to_string()).collect(), ..Report::default() };
        let drawn = vec![
            ("calm".to_string(), Ok(report(&["warning: a deprecated API (calm.html:3)"], 0))),
            ("loud".to_string(), Ok(report(&["warning: w", "error: Uncaught x (loud.html:9)"], 0))),
            ("quiet".to_string(), Ok(report(&[], 40))),
        ];
        let text = summary(&planned, &drawn, &[]);
        let read = outcome(&text).unwrap();
        assert!(read.created);
        assert_eq!((read.canvas.as_str(), read.title.as_str()), (planned.canvas.id.as_str(), "Sign-up, “v2”"));
        assert_eq!(read.added, ["calm", "loud", "quiet"]);
        assert_eq!(read.problems, ["loud", "quiet"], "{text}");
    }

    #[test]
    fn a_result_reads_back_as_what_the_call_did() {
        let text = "Started canvas “Settings, “tidy” edition” (canvas: \"settings\").\nAdded: calm, dense.\nRevised: bold.\n\nDrawn:\n- calm: drawn 600×400. No console errors.\n- dense: drawn 600×400. Content overflows the bottom by 20 px. No console errors.\n- bold: could not be drawn. The page failed.\n\nArtboards on “Settings”:\n- calm — “Calm” 600×400\n\nThe user sees the canvas now.";
        let read = outcome(text).unwrap();
        assert!(read.created);
        assert_eq!((read.canvas.as_str(), read.title.as_str()), ("settings", "Settings, “tidy” edition"));
        assert_eq!(read.added, ["calm", "dense"]);
        assert_eq!(read.revised, ["bold"]);
        assert_eq!(read.problems, ["dense", "bold"]);
        assert!(outcome("Updated canvas “T” (canvas: \"t\").\nRemoved: x.").unwrap().removed == ["x"]);
        assert!(outcome("There is no artboard “ws”.").is_none());
    }

    use super::*;

    #[test]
    fn reports_say_what_is_wrong() {
        let artboard = model::Artboard { id: "a".into(), name: "A".into(), width: 400, height: Some(300), ..Default::default() };
        let report = Report { width: 400, height: 300, overflow_y: 120, console: vec!["error: x is not defined (a.html:3)".into()], ..Default::default() };
        let line = report_line(&artboard, &Ok(report));
        assert_eq!(line, "a: drawn 400×300. Content overflows the bottom by 120 px. Console: error: x is not defined (a.html:3)");
        assert!(report_line(&artboard, &Err("boom".into())).contains("could not be drawn"));
    }

    #[test]
    fn data_urls_are_shortened() {
        let long = "A".repeat(300);
        assert_eq!(collapse_data_urls(&format!("<img src=\"data:image/png;base64,{long}\">")), "<img src=\"data:image/png;base64,…(300 characters)\">");
        assert_eq!(collapse_data_urls("data:image/png;base64,AAAA"), "data:image/png;base64,AAAA");
    }

    #[test]
    fn a_lone_canvas_needs_no_name() {
        let one = vec![Canvas { id: "t".into(), title: "T".into(), ..Default::default() }];
        assert_eq!(pick_canvas(&one, None).unwrap().id, "t");
        assert!(pick_canvas(&[], None).unwrap_err().contains("no canvas yet"));
        assert!(pick_canvas(&one, Some("x")).unwrap_err().contains("Canvases: t."));
    }
}
