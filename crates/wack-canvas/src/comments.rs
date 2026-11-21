//! The user's comments on a canvas: a note pinned to an element of an artboard.
//!
//! A comment is made on the board, anchored to the element under the pointer from the artboard's
//! element map (`render::Element`), and reaches the agent with the user's next message
//! (`note`). The agent settles one with `resolve_comments` on a `canvas` call.
//!
//! Invariants:
//! - Comments live in the canvas's own folder (`comments.json`), saved whole and atomically,
//!   with the next id: an id is never reused, so one the model was told of names one comment.
//! - A draft goes to the model only with a message the user sends (`arm_drafts`, when that
//!   message starts a run), never in the middle of a turn. Armed comments are `untold` until
//!   the note carrying them is written (`take_untold`), so a relaunch before then loses none.
//! - The model sees only comments sent to it (`waiting`): drafts are the user's until sent.
//! - A comment's `rev` is the revision of the element map it was pinned on, so the next drawing
//!   re-anchors it.
//! - An anchor keeps what found its element (path, tag, id, classes, words) and where it was, so
//!   after a revision the pin finds the element again (`reanchor`) or stays at its last place,
//!   marked as detached; it never jumps to an unrelated element.
//! - Comment text is the user's own words; element words come from an agent's page and are cut
//!   short (`render`).

use serde::{Deserialize, Serialize};

use crate::{render::Element, store::{Store, write_atomic}};

const FILE: &str = "comments.json";
pub const MAX_COMMENT_CHARS: usize = 2000;
pub const MAX_COMMENTS: usize = 200;
/// How much of an element's markup a comment keeps.
pub const MAX_MARKUP_CHARS: usize = 1500;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Written, not sent yet.
    #[default]
    Draft,
    /// Sent with a message; waiting for the agent.
    Sent,
    Resolved,
}

/// The element a comment is pinned to.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub path: String,
    pub tag: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub classes: String,
    #[serde(default)]
    pub text: String,
    /// Its box, in the artboard's CSS pixels, when the comment was made or last re-anchored.
    pub rect: [f64; 4],
    /// Its markup, cut short, read from the page when the comment was made.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub markup: String,
}

impl Anchor {
    pub fn from_element(element: &Element) -> Self {
        Self {
            path: element.p.clone(),
            tag: element.t.clone(),
            id: element.i.clone(),
            classes: element.c.clone(),
            text: element.s.clone(),
            rect: [element.x, element.y, element.w, element.h],
            markup: String::new(),
        }
    }

    /// The element as the agent is told of it: `<button#save.primary> at 0/2/1 (“Save”)`.
    pub fn describe(&self) -> String {
        let id = if self.id.is_empty() { String::new() } else { format!("#{}", self.id) };
        let classes: String = self.classes.split_whitespace().map(|c| format!(".{c}")).collect();
        let text = if self.text.is_empty() { String::new() } else { format!(" (“{}”)", self.text) };
        format!("<{}{id}{classes}> at {}{text}", self.tag, self.path)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub artboard: String,
    /// The artboard's revision it was made on.
    pub rev: u64,
    pub anchor: Anchor,
    pub body: String,
    #[serde(default)]
    pub state: State,
    /// The pin no longer finds its element on the artboard's current revision.
    #[serde(default)]
    pub detached: bool,
    /// Sent with a message whose note has not been written yet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub untold: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    /// The number the next comment's id takes.
    next: u32,
    comments: Vec<Comment>,
}

fn read(store: &Store, canvas: &str) -> Saved {
    let Ok(bytes) = std::fs::read(store.dir(canvas).join(FILE)) else { return Saved::default() };
    serde_json::from_slice::<Saved>(&bytes)
        .or_else(|_| serde_json::from_slice::<Vec<Comment>>(&bytes).map(|comments| Saved { next: 0, comments }))
        .unwrap_or_default()
}

fn write(store: &Store, canvas: &str, saved: &Saved) -> Result<(), String> {
    let dir = store.dir(canvas);
    if !dir.exists() {
        return Err("That canvas is gone.".into());
    }
    let bytes = serde_json::to_vec_pretty(saved).map_err(|_| "Could not save the comments.".to_string())?;
    write_atomic(&dir.join(FILE), &bytes)
}

/// Every comment on a canvas, resolved ones too, in the order they were made.
pub fn load(store: &Store, canvas: &str) -> Vec<Comment> {
    read(store, canvas).comments
}

/// Changes a canvas's comments, keeping its next id.
fn change<T>(store: &Store, canvas: &str, change: impl FnOnce(&mut Vec<Comment>) -> Result<(T, bool), String>) -> Result<T, String> {
    let mut saved = read(store, canvas);
    let (out, changed) = change(&mut saved.comments)?;
    if changed {
        write(store, canvas, &saved)?;
    }
    Ok(out)
}

/// The comments the model was sent and has not settled: what `canvas_look` shows it.
pub fn waiting(store: &Store, canvas: &str) -> Vec<Comment> {
    load(store, canvas).into_iter().filter(|c| c.state == State::Sent).collect()
}

/// Pins a new draft comment. Its id is short and never reused on the canvas.
pub fn add(store: &Store, canvas: &str, artboard: &str, rev: u64, anchor: Anchor, body: &str) -> Result<Comment, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err("A comment needs some words.".into());
    }
    if body.chars().count() > MAX_COMMENT_CHARS {
        return Err(format!("A comment can be at most {MAX_COMMENT_CHARS} characters."));
    }
    let mut saved = read(store, canvas);
    if saved.comments.len() >= MAX_COMMENTS {
        saved.comments.retain(|c| c.state != State::Resolved);
        if saved.comments.len() >= MAX_COMMENTS {
            return Err("This canvas has too many open comments. Resolve or delete some first.".into());
        }
    }
    let used = saved.comments.iter().filter_map(|c| c.id.strip_prefix('k')?.parse::<u32>().ok()).max().unwrap_or(0);
    let next = saved.next.max(used + 1).max(1);
    saved.next = next + 1;
    let comment = Comment {
        id: format!("k{next}"),
        artboard: artboard.to_string(),
        rev,
        anchor,
        body: body.to_string(),
        state: State::Draft,
        detached: false,
        untold: false,
    };
    saved.comments.push(comment.clone());
    write(store, canvas, &saved)?;
    Ok(comment)
}

/// Changes a comment's words, or deletes it (`None`).
pub fn edit(store: &Store, canvas: &str, id: &str, body: Option<&str>) -> Result<(), String> {
    change(store, canvas, |comments| {
        match body.map(str::trim) {
            Some("") => return Err("A comment needs some words.".into()),
            Some(body) if body.chars().count() > MAX_COMMENT_CHARS => return Err(format!("A comment can be at most {MAX_COMMENT_CHARS} characters.")),
            Some(body) => comments.iter_mut().find(|c| c.id == id).ok_or("That comment is gone.")?.body = body.to_string(),
            None => comments.retain(|c| c.id != id),
        }
        Ok(((), true))
    })
}

/// Keeps an element's markup with its comment, cut short.
pub fn set_markup(store: &Store, canvas: &str, id: &str, markup: &str) -> Result<(), String> {
    change(store, canvas, |comments| {
        let Some(comment) = comments.iter_mut().find(|c| c.id == id) else { return Ok(((), false)) };
        let mut kept: String = markup.chars().take(MAX_MARKUP_CHARS).collect();
        if kept.len() < markup.len() {
            kept.push('…');
        }
        comment.anchor.markup = kept;
        Ok(((), true))
    })
}

/// Marks drafts sent, with the message that starts a run, and returns them. They stay `untold`
/// until their note is written.
pub fn arm_drafts(store: &Store, canvas: &str) -> Result<Vec<Comment>, String> {
    change(store, canvas, |comments| {
        let mut armed = Vec::new();
        for comment in comments.iter_mut().filter(|c| c.state == State::Draft) {
            comment.state = State::Sent;
            comment.untold = true;
            armed.push(comment.clone());
        }
        let changed = !armed.is_empty();
        Ok((armed, changed))
    })
}

/// The sent comments whose note is still to be written, marked told.
pub fn take_untold(store: &Store, canvas: &str) -> Result<Vec<Comment>, String> {
    change(store, canvas, |comments| {
        let mut told = Vec::new();
        for comment in comments.iter_mut().filter(|c| c.untold) {
            comment.untold = false;
            if comment.state == State::Sent {
                told.push(comment.clone());
            }
        }
        let changed = !told.is_empty();
        Ok((told, changed))
    })
}

/// Resolves the comments the agent named; returns the ids it knew. Unknown ids are left out.
pub fn resolve(store: &Store, canvas: &str, ids: &[String]) -> Result<Vec<String>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    change(store, canvas, |comments| {
        let mut resolved = Vec::new();
        // Only what the model was sent: a draft is still the user's.
        for comment in comments.iter_mut().filter(|c| ids.contains(&c.id) && c.state == State::Sent) {
            comment.state = State::Resolved;
            comment.untold = false;
            resolved.push(comment.id.clone());
        }
        let changed = !resolved.is_empty();
        Ok((resolved, changed))
    })
}

/// Finds an anchor's element in an artboard's newer map: the same path and tag, else the same id,
/// else the same tag and words. `None`: it is gone, and the pin stays where it was.
pub fn reanchor<'a>(anchor: &Anchor, elements: &'a [Element]) -> Option<&'a Element> {
    elements
        .iter()
        .find(|e| e.p == anchor.path && e.t == anchor.tag)
        .or_else(|| (!anchor.id.is_empty()).then(|| elements.iter().find(|e| e.i == anchor.id)).flatten())
        .or_else(|| (!anchor.text.is_empty()).then(|| elements.iter().find(|e| e.t == anchor.tag && e.s == anchor.text)).flatten())
}

/// Moves every pin of `artboard` onto its element in the newest map, marking those that lost it.
pub fn follow(store: &Store, canvas: &str, artboard: &str, rev: u64, elements: &[Element]) -> Result<(), String> {
    change(store, canvas, |comments| {
        let mut changed = false;
        for comment in comments.iter_mut().filter(|c| c.artboard == artboard && c.state != State::Resolved && c.rev < rev) {
            match reanchor(&comment.anchor, elements) {
                Some(found) => {
                    let markup = std::mem::take(&mut comment.anchor.markup);
                    comment.anchor = Anchor { markup, ..Anchor::from_element(found) };
                    comment.detached = false;
                }
                None => comment.detached = true,
            }
            comment.rev = rev;
            changed = true;
        }
        Ok(((), changed))
    })
}

/// The smallest element under a point (artboard CSS pixels): what a click pins to.
pub fn element_at(elements: &[Element], x: f64, y: f64) -> Option<&Element> {
    elements
        .iter()
        .filter(|e| x >= e.x && x <= e.x + e.w && y >= e.y && y <= e.y + e.h)
        .min_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h)).then(b.p.len().cmp(&a.p.len())))
}

/// The smallest element that covers a whole region: what a drag pins to.
pub fn element_covering(elements: &[Element], rect: [f64; 4]) -> Option<&Element> {
    let [x, y, w, h] = rect;
    elements
        .iter()
        .filter(|e| e.x <= x && e.y <= y && e.x + e.w >= x + w && e.y + e.h >= y + h)
        .min_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h)))
}

/// What the agent is told of comments sent with a message.
pub fn note(canvas_title: &str, canvas: &str, comments: &[Comment]) -> String {
    let mut lines = vec![format!("The user left {} on canvas “{canvas_title}” (canvas: \"{canvas}\"):", plural(comments.len(), "comment"))];
    for comment in comments {
        lines.push(String::new());
        let detached = if comment.detached { " (the element has since changed; its last place is kept)" } else { "" };
        lines.push(format!("Comment {} on artboard {}, at {}{detached}:", comment.id, comment.artboard, comment.anchor.describe()));
        lines.push(format!("“{}”", comment.body));
        if !comment.anchor.markup.is_empty() {
            lines.push(format!("Its markup: ```html\n{}\n```", comment.anchor.markup));
        }
    }
    lines.push(String::new());
    lines.push("Change what each asks on the canvas (edits on that artboard), and settle each with resolve_comments on the same canvas call. Leave one open, and say why, if it needs the user's decision.".into());
    lines.join("\n")
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(p: &str, t: &str, rect: [f64; 4], text: &str) -> Element {
        Element { p: p.into(), t: t.into(), s: text.into(), x: rect[0], y: rect[1], w: rect[2], h: rect[3], ..Element::default() }
    }

    #[test]
    fn a_click_pins_to_the_smallest_element_under_it() {
        let map = vec![element("0", "main", [0., 0., 400., 300.], ""), element("0/1", "button", [10., 10., 80., 30.], "Save")];
        assert_eq!(element_at(&map, 20., 20.).unwrap().t, "button");
        assert_eq!(element_at(&map, 200., 200.).unwrap().t, "main");
        assert!(element_at(&map, 500., 20.).is_none());
        assert_eq!(element_covering(&map, [5., 5., 100., 40.]).unwrap().t, "main");
    }

    #[test]
    fn pins_follow_their_element_or_stay_put() {
        let anchor = Anchor::from_element(&element("0/1", "button", [10., 10., 80., 30.], "Save"));
        let moved = vec![element("0/2", "button", [10., 50., 80., 30.], "Save"), element("0/1", "p", [0., 0., 10., 10.], "")];
        assert_eq!(reanchor(&anchor, &moved).unwrap().p, "0/2", "the same words, moved");
        assert!(reanchor(&anchor, &[element("0/1", "div", [0., 0., 1., 1.], "")]).is_none());
    }

    #[test]
    fn comments_are_drafted_sent_and_resolved() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        std::fs::create_dir_all(store.dir("t")).unwrap();
        let anchor = Anchor::from_element(&element("0/1", "button", [10., 10., 80., 30.], "Save"));
        let first = add(&store, "t", "a", 1, anchor.clone(), "  Make this the primary action ").unwrap();
        assert_eq!((first.id.as_str(), first.body.as_str()), ("k1", "Make this the primary action"));
        assert!(add(&store, "t", "a", 1, anchor.clone(), " ").is_err());
        let second = add(&store, "t", "a", 1, anchor, "Bigger").unwrap();
        assert_eq!(second.id, "k2");
        assert!(waiting(&store, "t").is_empty(), "drafts are the user's");
        assert!(resolve(&store, "t", &["k1".into()]).unwrap().is_empty(), "a draft can't be resolved");
        let sent = arm_drafts(&store, "t").unwrap();
        assert_eq!(sent.len(), 2);
        assert!(arm_drafts(&store, "t").unwrap().is_empty());
        assert_eq!(take_untold(&store, "t").unwrap().len(), 2);
        assert!(take_untold(&store, "t").unwrap().is_empty(), "told once");
        assert_eq!(resolve(&store, "t", &["k1".into(), "k9".into()]).unwrap(), ["k1"]);
        assert_eq!(waiting(&store, "t").len(), 1);
        let note = note("T", "t", &sent);
        assert!(note.contains("Comment k1 on artboard a, at <button> at 0/1 (“Save”):\n“Make this the primary action”"), "{note}");
    }

    #[test]
    fn an_id_is_never_reused() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        std::fs::create_dir_all(store.dir("t")).unwrap();
        let anchor = Anchor::from_element(&element("0", "main", [0., 0., 10., 10.], ""));
        add(&store, "t", "a", 1, anchor.clone(), "One").unwrap();
        let two = add(&store, "t", "a", 1, anchor.clone(), "Two").unwrap();
        edit(&store, "t", &two.id, None).unwrap();
        assert_eq!(add(&store, "t", "a", 1, anchor, "Three").unwrap().id, "k3");
        set_markup(&store, "t", "k1", &"x".repeat(MAX_MARKUP_CHARS + 10)).unwrap();
        assert_eq!(load(&store, "t")[0].anchor.markup.chars().count(), MAX_MARKUP_CHARS + 1);
    }
}
