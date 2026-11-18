//! The canvas guide: the long authoring rules behind the `canvas` tool, served on demand by
//! `canvas_guide` instead of riding in every system prompt.
//!
//! Ported from version 2's guide (`worker/src/builtin/canvas/guide.ts`), with what version 3
//! adds (edits, tweaks, comments, the report every call returns) and craft points learned from
//! other design canvases: honest variations on named axes, stable option names, pages that hold
//! at phone width, accessibility as drawn. Original WackCode text. Keep each section
//! self-contained: the agent may read one alone.

use crate::model::{
    MAX_ARTBOARD_HEIGHT, MAX_ARTBOARD_HTML_BYTES, MAX_ARTBOARD_WIDTH, MAX_ARTBOARDS_PER_CALL, MAX_ARTBOARDS_PER_CANVAS, MAX_CALL_BYTES,
    MAX_TWEAKS_PER_ARTBOARD, MIN_ARTBOARD_SIZE, size_of,
};

pub const SECTIONS: &[&str] = &["basics", "explore", "themes", "tweaks", "comments", "fidelity", "craft", "example"];

fn section(name: &str) -> String {
    match name {
        "basics" => format!(
            r##"## Basics

A canvas is a board of artboards: whole HTML mockups drawn side by side so they can be compared. It opens in the Canvas pane beside the chat, where the user pans, zooms, plays an artboard live, presents them one at a time, leaves comments on them, moves their tweaks and exports them. The chat shows a card for each turn that changed it.

- **Start one** with `canvas` and a `title` ("Settings page directions"). Later calls pass the returned `canvas` id instead. A chat can hold several canvases; start another only for a genuinely different subject.
- **Artboards** have a `name` the user will say out loud ("Dusk", "Compact list"), a `width`, an optional `height` and an `html` body. Omit `height` for a component or long page that should be as tall as its content; give one for a fixed screen.
- **Groups** are rows. Artboards with the same `group` sit in one labelled row, left to right in the order you add them; groups stack in the order they first appear. Group by direction ("A · Calm", "B · Dense") or by screen ("Inbox", "Detail"), whichever the user will compare.
- **Revise in place** by passing an artboard's `id`: only the fields you give change, and it updates where it stands. For a small change send `edits: [{{"old": "…", "new": "…"}}]`: each `old` must match the current markup exactly once, so include enough around it. Send the whole `html` only when most of it changes.
- **Variants.** `basedOn` makes an artboard take another's markup, with its own `css` appended. Change the base and every variant follows. A new variant with no `width` takes its base's whole size; give one to show the same markup at another device size. A variant's `edits` change its css. A variant can't be the base of another.
- **Remove** artboards with `remove: [ids]`. A base can't be removed while variants depend on it.
- **Limits:** {per_call} artboards per call, {per_canvas} per canvas, {html} of markup per artboard, {call} per call. Widths {min}–{max_w} px, heights up to {max_h} px.

**Sizes:** Desktop 1440×900, Laptop 1280×800, Tablet 834×1194, Phone 390×844. A web page or app screen 1280–1440 wide. A single component sits well at 480–800 wide with no height.

**The document.** Each artboard is its own page, sized exactly to the artboard, so `100vw` and `100vh` are the artboard's. It is offline: inline every style, script and icon (inline SVG), and never reference a URL, a web font or a CDN; they are refused. Use system font stacks (`-apple-system, "SF Pro Text", Inter, system-ui, sans-serif`; `ui-monospace, "SF Mono", Menlo, monospace`). Scripts run, so hover states, tabs and small interactions work when the user plays an artboard; a link to another artboard of the same canvas (`href="other-id.html"`) works too, for a clickable prototype. Nothing can open a window, download a file or reach the network.

**Images.** Local image files may be named by absolute path in quoted attributes (`src="/Users/you/project/shot.png"`) and CSS `url(…)`. They are copied into the canvas when the call is accepted, so the artboard keeps working after the file moves (PNG, JPEG, GIF, WebP, AVIF, BMP, ICO, SVG; at most 8 MiB each). A missing or unreadable image refuses the whole call, naming the path. A picture already in the chat can be reached through `list_images` and `read_image`.

**Background.** An artboard is a screen, not a card: paint its own background on `body` and fill it.

**Exploring changes nothing in the project.** Everything lives on the canvas: don't edit or create project files while exploring, even to fix something you noticed in a mockup. Fix it on the artboard. Build a direction only once the user picks one and asks for it.

**Every call is checked.** The `canvas` result reports each artboard it drew: its size, how far content overflows, and its console's errors and warnings. Fix what it shows before you hand over. `canvas_look` returns screenshots and the exact markup when you need to see one. Then keep your reply short: name the artboards, say what sets each apart and its main tradeoff, and ask which way to go. Don't describe what the user can already see."##,
            per_call = MAX_ARTBOARDS_PER_CALL,
            per_canvas = MAX_ARTBOARDS_PER_CANVAS,
            html = size_of(MAX_ARTBOARD_HTML_BYTES),
            call = size_of(MAX_CALL_BYTES),
            min = MIN_ARTBOARD_SIZE,
            max_w = MAX_ARTBOARD_WIDTH,
            max_h = MAX_ARTBOARD_HEIGHT,
        ),
        "explore" => format!(
            r##"## Exploring directions

The canvas earns its keep when the user can compare real alternatives side by side.

1. **Pin down the brief first**, briefly: the screen or component, who uses it, what must be on it, and any constraint (brand, platform, existing design system). If two or more decisions that change the result are still open, ask once before drawing; otherwise decide, and say what you assumed in a line. If the project has a UI, read it first (see "fidelity").
2. **Diverge on purpose.** Draw 3–5 directions that differ on an axis you can name — density, hierarchy, navigation model, colour temperature, type personality, shape language — not five shades of one layout. Low-fidelity sketches are enough to choose a direction; polish comes after.
3. **Make an honest case for each.** Give each a short evocative name and one line of `notes` saying what it bets on. In your reply, give each its main tradeoff too: a set where only your favourite gets a case is a rigged vote.
4. **Keep names stable.** Once an artboard is "B · Dense" it keeps that name and id; never renumber or rename options across turns.
5. **Keep the content identical** across directions: the same realistic data, labels and states, so the comparison is about design. Real-looking content always; never lorem ipsum or invented statistics (use a placeholder like [PRICE] for a fact you don't have).
6. **Show the hard parts.** The state that breaks naive layouts: a long name, an empty list, an error, a busy table. One artboard for a direction's weakest state beats five happy paths.
7. **Converge on feedback.** When the user picks or mixes ("B's layout with D's colours"), revise in place or add a group such as "Round 2" so the history stays readable. Remove dead ends only when the user agrees or the canvas is crowded.
8. **Cover the platforms that matter.** If the product ships on phones, put a Phone artboard beside the Desktop one in the same group.

Draw in batches: the first call can carry {per_call} artboards and later calls add the rest, so the user sees the canvas fill in rather than waiting."##,
            per_call = MAX_ARTBOARDS_PER_CALL,
        ),
        "themes" => r##"## Theme explorations

For several themes of the same UI, write the markup once and vary only tokens:

1. Draw one **base** artboard whose styles read everything from custom properties: `:root { --bg: …; --surface: …; --text: …; --muted: …; --accent: …; --radius: …; --font: …; }`, every colour, radius, shadow and font in the rules using `var(--…)`.
2. Add one artboard per theme with `basedOn` set to the base's id and `css` that overrides `:root { … }`. Each costs a few hundred bytes, and revising the base updates every theme.
3. Vary more than the hue: a theme is a temperature, a contrast level, a radius and shadow language, sometimes a type pairing. Keep one quiet, neutral option among the bold ones.
4. Check contrast: body text at least 4.5:1 against its surface, large text and icons 3:1. Muted text is where themes usually fail.
5. Show light and dark if the product supports both, as variants in the same group.
6. Name themes by feel ("Fog", "Ember", "Pine") and keep them in one row."##
            .to_string(),
        "tweaks" => format!(
            r##"## Tweaks

Tweaks are levers the user moves on the canvas without asking you: the artboard redraws at once. Declare a few on an artboard with `tweaks` (at most {max}):

```json
"tweaks": [
  {{ "id": "accent", "label": "Accent", "kind": "color", "default": "#2f6df6", "var": "--accent" }},
  {{ "id": "density", "label": "Density", "kind": "choice", "default": "cosy", "options": ["cosy", "compact"], "attr": "density" }},
  {{ "id": "radius", "label": "Corners", "kind": "number", "default": 10, "min": 0, "max": 24, "step": 1, "unit": "px", "var": "--radius" }},
  {{ "id": "dark", "label": "Dark", "kind": "toggle", "default": false, "attr": "theme" }}
]
```

- A `var` tweak sets that custom property on `:root`; write the css to read it (`background: var(--accent)`). An `attr` tweak sets `data-<attr>` on `<html>` (a toggle as `on` or `off`); write rules for it (`html[data-density=compact] .row {{ padding: 6px 12px }}`, `html[data-theme=on] {{ … }}`).
- `kind` is `color` (hex), `number` (with `min`, `max`, `step` and a `unit`: px, rem, em, %, ms, s or deg), `toggle` or `choice` (2–8 `options`).
- Make levers, not copy: a behaviour switch (dark, density, a variant), or a value that cuts across the design (one accent, a spacing or radius scale). Never a tweak for text, and never one for a colour used in one place.
- Giving `tweaks` again replaces the list; the user's settings are kept for tweaks that remain. `canvas_look` reports where the user has set each one: carry those values into the design when they ask you to keep them."##,
            max = MAX_TWEAKS_PER_ARTBOARD,
        ),
        "comments" => r##"## Comments

The user can point at an element on an artboard and leave a comment. Comments reach you with their next message, each naming its artboard, the element (its path from `<body>` as child indices like `0/2/1`, its tag, id and classes, a few of its words), the element's markup and the comment; with a picture of it when you can see images.

- Find the element in the artboard's markup by that path and its words, change what the comment asks with `edits`, and pass the comment's id in `resolve_comments` on the same `canvas` call, so the user sees it settled.
- A comment you can't act on yet (it needs the user's decision): leave it open and say why in your reply.
- Comments are about the canvas, not the project's files."##
            .to_string(),
        "fidelity" => r##"## Fidelity to the project

When the chat has a project with a UI, the mockups should look like they belong in it unless the user asked for something new.

- **Read before drawing:** the stylesheet or theme file (colour tokens, spacing scale, radii, shadows, fonts), one or two representative components, and any design notes in the repository. Reuse its token names in your css so the user can map a mockup back to code.
- **Mirror the real shell:** the actual navigation, header and density, so a new screen is judged in context. Reproduce the parts you need in plain HTML and CSS; an artboard can't load the project's files.
- **Don't invent a design system** when one exists. Exploring beyond it is fine when asked; say which artboards depart from it.
- When the user picks a direction, the markup is a reference, not production code: build it with the project's own components and conventions. `canvas_look` with `html: true` returns an artboard's exact markup if it has left your context."##
            .to_string(),
        "craft" => r##"## Craft

- **Hierarchy first:** one clear primary action per screen, a type scale with real steps (e.g. 12/14/16/20/28), weight and colour doing the work before size does.
- **A spacing scale** (4/8/12/16/24/32/48) applied consistently, aligned to a grid. Lay out groups of siblings with flex or grid and `gap`, not margins on each.
- **Real interface details:** focus and hover states, disabled buttons, badges with counts, truncation with ellipses, aligned numbers (`font-variant-numeric: tabular-nums`). Touch targets at least 44 px on phone artboards.
- **Accessible as drawn:** real `<button>`, `<a href>` and `<input>` with `<label>`, never a clickable `<div>`; `aria-label` on icon-only buttons; text at 4.5:1 contrast, colours that must be told apart differing in lightness, not hue alone.
- **Pages hold at phone width:** a web page or app screen uses a fluid root (max-width containers, %, rem, fr) so menus stack and toolbars wrap; wide tables scroll in their own box.
- **Icons** as small inline SVGs in one consistent style and stroke. Never emoji as UI icons, never icon fonts.
- **Avoid the generic look:** no gradient washes, glassmorphism or shadows on everything; no cards with a coloured left border; no hero sections on app screens; no centred-everything layouts for tools. Give each direction a point of view.
- **States** — empty, loading (skeletons, not spinners everywhere), error, success — are part of the design; show one when it matters.
- Keep markup lean: shared styles in one `<style>` block, no frameworks, no base64 blobs (name local images by path instead)."##
            .to_string(),
        "example" => r##"## Example

A base and two theme variants in one call:

```json
{
  "title": "Billing settings — themes",
  "artboards": [
    { "name": "Base", "group": "Themes", "width": 1280, "height": 800,
      "notes": "Neutral reference",
      "html": "<style>:root{--bg:#f6f6f4;--surface:#fff;--text:#1d1d1b;--muted:#6b6b66;--accent:#2f6df6;--radius:10px}body{margin:0;background:var(--bg);color:var(--text);font:14px/1.5 -apple-system,system-ui,sans-serif}…</style><main>…</main>",
      "tweaks": [{ "id": "accent", "label": "Accent", "kind": "color", "default": "#2f6df6", "var": "--accent" }] },
    { "name": "Ember", "group": "Themes", "basedOn": "base",
      "notes": "Warm, high contrast",
      "css": ":root{--bg:#1b1410;--surface:#261c16;--text:#f6eee8;--muted:#b8a598;--accent:#ff7a3d;--radius:6px}" },
    { "name": "Pine", "group": "Themes", "basedOn": "base",
      "css": ":root{--bg:#eef3ef;--surface:#fbfdfb;--text:#13241a;--muted:#5d7466;--accent:#1f7a4d;--radius:14px}" }
  ]
}
```

Then fix what the report shows, and give a short reply naming the three themes, each one's tradeoff, and a question about which to take forward. A later tweak to the base's heading is one edit: `{ "canvas": "billing-settings-themes", "artboards": [{ "id": "base", "edits": [{ "old": "<h1>Billing</h1>", "new": "<h1>Plan &amp; billing</h1>" }] }] }`."##
            .to_string(),
        _ => String::new(),
    }
}

/// The sections asked for, in the guide's own order; all of them when none are named.
pub fn guide(sections: &[String]) -> String {
    let wanted: Vec<&str> = if sections.is_empty() {
        SECTIONS.to_vec()
    } else {
        SECTIONS.iter().copied().filter(|name| sections.iter().any(|s| s == name)).collect()
    };
    let mut parts = vec!["# Canvas guide".to_string()];
    parts.extend(wanted.into_iter().map(section));
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_has_its_heading_and_the_whole_guide_is_in_order() {
        for name in SECTIONS {
            assert!(section(name).starts_with("## "), "{name}");
        }
        let whole = guide(&[]);
        let positions: Vec<usize> = SECTIONS.iter().map(|name| whole.find(&section(name)).unwrap()).collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(guide(&["tweaks".into(), "nope".into()]), format!("# Canvas guide\n\n{}", section("tweaks")));
    }
}
