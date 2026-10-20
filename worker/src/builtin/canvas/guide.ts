/**
 * The canvas guide: the long authoring rules behind the `canvas` tool, served on demand by
 * `canvas_guide` (and in full by `/canvas`) instead of riding in every system prompt. The tool's
 * own `promptGuidelines` only say when to reach for the canvas and to read this first, which
 * keeps the coding guide inside its budget (`code-prompt/prompt.test.ts`).
 *
 * Original WackCode text. Keep each section self-contained: the agent may read one alone.
 */
import {
  MAX_ARTBOARDS_PER_CALL, MAX_ARTBOARDS_PER_CANVAS, MAX_ARTBOARD_HEIGHT, MAX_ARTBOARD_WIDTH, MIN_ARTBOARD_SIZE,
} from "./state.js";

export const CANVAS_GUIDE_SECTIONS = ["basics", "explore", "themes", "fidelity", "craft", "look", "example"] as const;
export type CanvasGuideSection = typeof CANVAS_GUIDE_SECTIONS[number];

const SECTIONS: Record<CanvasGuideSection, string> = {
  basics: `## Basics

A canvas is a board the user pans and zooms beside the chat, holding artboards: full mockups drawn side by side so they can be compared. It opens in the side panel and can expand over the whole window while the user keeps chatting with you.

- **Start one** with \`canvas\` and a \`title\` (e.g. "Settings page directions"). Later calls pass the returned \`canvas\` id instead. A chat can hold several canvases; start a new one only for a genuinely different subject.
- **Artboards** have a \`name\` the user will say out loud ("Dusk", "Compact list"), a \`width\`, an optional \`height\` and an \`html\` body. Omit \`height\` for a component or long page that should be as tall as its content; give one for a fixed screen.
- **Groups** are rows. Artboards with the same \`group\` sit in one labelled row, left to right in the order you add them; groups stack top to bottom in the order they first appear. Group by direction ("A · Calm", "B · Dense") or by screen ("Inbox", "Detail"), whichever the user will compare.
- **Revise in place.** Pass an existing artboard's \`id\` to change only the fields you give; the user sees it update where it stands. Prefer revising to adding near-duplicates.
- **Variants.** \`basedOn\` makes an artboard take another artboard's markup, and its \`css\` is appended after it. Change the base and every variant follows. A new variant that gives no \`width\` takes its base's whole size; give a \`width\` to show the same markup at another device size. A variant can't be the base of another variant.
- **Remove** artboards with \`remove: [ids]\`. A base can't be removed while variants depend on it.
- **Limits:** ${MAX_ARTBOARDS_PER_CALL} artboards per call, ${MAX_ARTBOARDS_PER_CANVAS} per canvas, 64 KiB of markup per artboard, 192 KiB per call. Widths ${MIN_ARTBOARD_SIZE}–${MAX_ARTBOARD_WIDTH} px, heights up to ${MAX_ARTBOARD_HEIGHT} px.

**Sizes:** Desktop 1440×900, Laptop 1280×800, Tablet 834×1194, Phone 390×844. A single component sits well at 480–800 wide with no height.

**The document.** Each artboard is its own sandboxed page sized exactly to the artboard, so \`100vh\` and \`100vw\` are the artboard's. It has no network at all: inline every style, script, image and icon (inline SVG, data: URLs) and never reference a URL, a web font or a CDN. Use system font stacks (\`-apple-system, "SF Pro Text", Inter, system-ui, sans-serif\`; \`ui-monospace, "SF Mono", Menlo, monospace\`). Scripts run, so hover states, tabs and small interactions work when the user steps into an artboard, but nothing can send a message or open a link from a canvas.

**Background.** An artboard is a screen, not a chat card: paint its own background on \`body\` and size it to fill. The app's theme variables (\`--bg\`, \`--surface\`, \`--text\`, \`--accent\`…) are defined if you are mocking up WackCode itself; for the user's own product, use that product's colours.

**Exploring changes nothing in the project.** Everything lives on the canvas: don't edit, write or create project files while exploring, even to fix something you noticed in a mockup — fix it on the artboard. Implement a direction only once the user picks one and asks for it.

**After drawing,** call \`canvas_look\` on what you changed and fix what it shows before you hand over. Then keep your reply short: name the artboards, say what distinguishes them, and ask which way to go. Don't describe what the user can already see.`,

  explore: `## Exploring directions

The canvas earns its keep when the user can compare real alternatives side by side.

1. **Pin down the brief first**, briefly: the screen or component, who uses it, what must be on it, and any constraint (brand, platform, existing design system). If the project has a UI, read it before drawing (see "fidelity").
2. **Diverge on purpose.** Draw 3–5 directions that differ on axes that matter to the decision — density, hierarchy, navigation model, colour temperature, type personality, shape language — not five recolours of one layout. Give each a short evocative name and one line of \`notes\` saying what it is betting on.
3. **Keep the content identical** across directions: the same realistic data, labels and states, so the comparison is about design, not copy. Real-looking content always (plausible names, numbers, dates); never lorem ipsum.
4. **Show the hard parts.** Include the state that breaks naive layouts: a long name, an empty list, an error, a busy table. One extra artboard per direction for its weakest state beats five happy paths.
5. **Converge on feedback.** When the user picks or mixes ("B's layout with D's colours"), revise in place or add a new group such as "Round 2" so the history stays readable. Remove dead ends only when the user agrees or the canvas is getting crowded.
6. **Cover the platforms that matter.** If the product ships on phones, put a Phone artboard beside the Desktop one in the same group.

Draw in batches: the first call can carry the first ${MAX_ARTBOARDS_PER_CALL} artboards, and later calls add the rest, so the user sees the canvas fill in rather than waiting for everything.`,

  themes: `## Theme explorations

For several themes of the same UI, write the markup once and vary only tokens:

1. Draw one **base** artboard whose styles read everything from CSS custom properties: \`:root { --bg: …; --surface: …; --text: …; --muted: …; --accent: …; --radius: …; --font: …; }\`, with every colour, radius, shadow and font in the rules using \`var(--…)\`.
2. Add one artboard per theme with \`basedOn\` set to the base's id and \`css\` that overrides \`:root { … }\`. Each variant costs a few hundred bytes, and revising the base updates every theme at once.
3. Vary more than the hue: a theme is a temperature, a contrast level, a radius and shadow language, sometimes a type pairing. Keep at least one quiet, neutral option among the bold ones.
4. Check contrast: body text at least 4.5:1 against its surface, large text and icons 3:1. Muted text is where themes usually fail.
5. Show light and dark if the product supports both, as separate variants in the same group.
6. Name themes by feel ("Fog", "Ember", "Pine") and group them in one row so they line up for comparison.`,

  fidelity: `## Fidelity to the project

When the chat has a project with a UI, the mockups should look like they belong in it unless the user asked for something new.

- **Read before drawing:** the stylesheet or theme file (colour tokens, spacing scale, radii, shadows, fonts), one or two representative components, and any design notes in the repository. Reuse its token names in your CSS so the user can map a mockup back to code.
- **Mirror the real shell:** the actual navigation, header and density, so a new screen is judged in context. Reproduce the parts you need in plain HTML and CSS; you can't load the project's files from inside an artboard.
- **Don't invent a design system** when one exists. Exploring beyond it is fine when asked; say which artboards depart from the current system.
- When the user picks a direction, the markup you wrote is a reference, not production code: implement it with the project's own components and conventions. \`canvas_look\` with \`html: true\` returns an artboard's exact markup if it has left your context.`,

  craft: `## Craft

- **Hierarchy first:** one clear primary action per screen, a type scale with real steps (e.g. 12/14/16/20/28), and weight and colour doing the work before size does.
- **A spacing scale** (4/8/12/16/24/32/48) applied consistently; align to a grid. Uneven gaps read as unfinished.
- **Real interface details:** focus and hover states, disabled buttons, badges with counts, truncation with ellipses, numbers that align (\`font-variant-numeric: tabular-nums\`).
- **Icons** as small inline SVGs in one consistent style, stroke weight and size. Never emoji as UI icons, and never icon fonts.
- **Avoid the generic look:** no gratuitous gradients, glassmorphism or drop shadows on everything; no hero sections on app screens; no centred-everything layouts for tools. Give each direction a point of view.
- **States:** empty, loading (skeletons, not spinners everywhere), error and success are part of the design; show at least one when it matters.
- Keep markup lean: shared styles in one \`<style>\` block, no frameworks, no base64 images larger than they need to be.`,

  look: `## Checking your work (canvas_look)

\`canvas_look\` renders artboards the way the canvas does and reports back: a screenshot of each one (when your model can see images), its rendered size, how far content overflows a fixed-height artboard, and any script errors.

- Look at what you just drew or changed (4 artboards per call, and it names any others to look at next; omit \`artboards\` for the ones changed last), fix real defects — clipped text, overflow, broken layout, a script error, poor contrast — and look once more if you changed a lot. Don't loop on cosmetic nits.
- Overflow on a fixed-height screen usually means the content is too tall for the device: tighten it or omit \`height\` if the artboard is meant to be a long page.
- \`html: true\` also returns each artboard's resolved markup, for implementing a chosen design after the original call has left your context.`,

  example: `## Example

A base and two theme variants in one call:

\`\`\`json
{
  "title": "Billing settings — themes",
  "artboards": [
    { "name": "Base", "group": "Themes", "width": 1280, "height": 800,
      "notes": "Neutral reference",
      "html": "<style>:root{--bg:#f6f6f4;--surface:#fff;--text:#1d1d1b;--muted:#6b6b66;--accent:#2f6df6;--radius:10px}body{margin:0;background:var(--bg);color:var(--text);font:14px/1.5 -apple-system,system-ui,sans-serif}…</style><main>…</main>" },
    { "name": "Ember", "group": "Themes", "width": 1280, "height": 800, "basedOn": "base",
      "notes": "Warm, high contrast",
      "css": ":root{--bg:#1b1410;--surface:#261c16;--text:#f6eee8;--muted:#b8a598;--accent:#ff7a3d;--radius:6px}" },
    { "name": "Pine", "group": "Themes", "width": 1280, "height": 800, "basedOn": "base",
      "css": ":root{--bg:#eef3ef;--surface:#fbfdfb;--text:#13241a;--muted:#5d7466;--accent:#1f7a4d;--radius:14px}" }
  ]
}
\`\`\`

Then \`canvas_look\` with the returned canvas id, a short reply naming the three themes, and a question about which to take forward.`,
};

/** The sections asked for, in the guide's own order; all of them when none are named. */
export function canvasGuide(sections?: readonly string[]): string {
  const wanted = sections?.length
    ? CANVAS_GUIDE_SECTIONS.filter((name) => sections.includes(name))
    : CANVAS_GUIDE_SECTIONS;
  return ["# Canvas guide", ...wanted.map((name) => SECTIONS[name])].join("\n\n");
}

export const CANVAS_MAX_REQUEST_CHARS = 20_000;

/** What `/canvas` runs: the whole guide, then the user's request (or an invitation to give one). */
export function buildCanvasPrompt(request: string): string {
  const trimmed = request.trim().slice(0, CANVAS_MAX_REQUEST_CHARS);
  const task = trimmed
    ? `The user's request:\n\n${trimmed}`
    : "The user gave no further request, so ask what they want to explore on the canvas — the screen or component, and what kind of alternatives — before drawing anything.";
  return `The user started a canvas exploration with /canvas. Work on the canvas: draw the alternatives as artboards with the canvas tool, check them with canvas_look, and keep your reply short. Read the project as much as you need, but change none of its files: the exploration lives on the canvas until the user picks a direction and asks you to build it. The guide below is everything you need, so you don't need to call canvas_guide in this chat.

${task}

${canvasGuide()}`;
}
