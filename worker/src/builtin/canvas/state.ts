/**
 * A canvas's contents, derived from the session branch: every accepted `canvas` call, in order.
 *
 * The call's arguments carry the markup (Pi saves them), and its result's `details` say which
 * artboard each argument became (`applied`) and what was removed. Folding the two pairs back
 * together is the whole state, so a rewind, a fork or a restart needs nothing else — the same
 * record `suggest-task.ts` keeps. The renderer folds the snapshot the same way
 * (`src/canvas-state.ts`); keep the two rules identical (`state.test.ts` and
 * `canvas-state.test.ts` pin the same cases).
 *
 * Resolution: a variant (`basedOn`) takes its base's current markup, then its own `css` is
 * appended as a `<style>`, so revising a base restyles every variant. A base is never itself a
 * variant, and it can't be removed while a variant depends on it. A new variant that gives no
 * width takes its base's size at that moment; `applied` records it, since the arguments don't.
 */
export const CANVAS_TOOL_NAME = "canvas";
export const MAX_CANVAS_TITLE = 60;
export const MAX_ARTBOARD_NAME = 48;
export const MAX_ARTBOARD_GROUP = 40;
export const MAX_ARTBOARD_NOTES = 200;
export const MAX_ARTBOARD_HTML_BYTES = 64 * 1024;
export const MAX_ARTBOARD_CSS_BYTES = 8 * 1024;
export const MAX_ARTBOARDS_PER_CALL = 6;
export const MAX_CALL_BYTES = 192 * 1024;
export const MAX_ARTBOARDS_PER_CANVAS = 24;
export const MIN_ARTBOARD_SIZE = 200;
export const MAX_ARTBOARD_WIDTH = 2560;
export const MAX_ARTBOARD_HEIGHT = 4000;
const MAX_ID_CHARS = 32;

export interface Artboard {
  id: string;
  name: string;
  group?: string;
  width: number;
  /** Absent: the artboard is as tall as its content (measured when drawn). */
  height?: number;
  html?: string;
  basedOn?: string;
  css?: string;
  notes?: string;
}

export interface Canvas {
  id: string;
  title: string;
  /** Creation order; an update keeps an artboard's place. */
  artboards: Artboard[];
  /** What the latest accepted call added or revised, which `canvas_look` checks by default. */
  lastChanged: string[];
}

export interface CanvasApplied {
  /** Position in the call's `artboards` argument. */
  index: number;
  id: string;
  op: "add" | "update";
  /** The base's id, when the argument named it by its name. */
  basedOn?: string;
  /** A new variant that gave no width takes its base's size, recorded here because its arguments lack it. */
  width?: number;
  height?: number;
}

export interface CanvasDetails {
  /** Versioned result shape, like the other WackCode cards. */
  v: 1;
  canvas: string;
  title: string;
  applied: CanvasApplied[];
  removed: string[];
}

export function isCanvasDetails(value: unknown): value is CanvasDetails {
  if (!value || typeof value !== "object") return false;
  const details = value as Record<string, unknown>;
  return details.v === 1 && typeof details.canvas === "string" && typeof details.title === "string"
    && Array.isArray(details.applied) && Array.isArray(details.removed);
}

type Fields = Partial<Omit<Artboard, "id">>;

const ARTBOARD_FIELDS = ["name", "group", "width", "height", "html", "basedOn", "css", "notes"] as const;

/** The fields one argument entry sets; anything else in it is ignored. */
function fieldsOf(raw: unknown): Fields {
  if (!raw || typeof raw !== "object") return {};
  const input = raw as Record<string, unknown>;
  const fields: Fields = {};
  for (const key of ARTBOARD_FIELDS) {
    const value = input[key];
    if (key === "width" || key === "height") {
      if (typeof value === "number" && Number.isFinite(value)) fields[key] = Math.round(value);
    } else if (typeof value === "string") {
      // Labels are one line; markup and styles are kept exactly as written.
      fields[key] = key === "name" || key === "group" || key === "notes" ? clean(value) : value;
    }
  }
  return fields;
}

/** Applies one entry: new markup makes an artboard plain, a new base makes it a variant. */
function merge(existing: Artboard | undefined, id: string, fields: Fields): Artboard {
  const next: Artboard = existing ? { ...existing } : { id, name: id, width: 1440 };
  for (const key of ARTBOARD_FIELDS) {
    const value = fields[key];
    if (value !== undefined) (next as unknown as Record<string, unknown>)[key] = value;
  }
  if (fields.html !== undefined && fields.basedOn === undefined) delete next.basedOn;
  if (fields.basedOn !== undefined && fields.html === undefined) delete next.html;
  return next;
}

/** One accepted call folded into `canvases`, which it updates in place. */
export function applyCall(canvases: Map<string, Canvas>, args: unknown, details: CanvasDetails): void {
  const input = (args && typeof args === "object" ? args : {}) as { artboards?: unknown };
  const entries = Array.isArray(input.artboards) ? input.artboards : [];
  const canvas = canvases.get(details.canvas) ?? { id: details.canvas, title: details.title, artboards: [], lastChanged: [] };
  canvas.title = details.title;
  for (const applied of details.applied) {
    if (!applied || typeof applied.id !== "string" || typeof applied.index !== "number") continue;
    const fields = fieldsOf(entries[applied.index]);
    if (typeof applied.basedOn === "string") fields.basedOn = applied.basedOn;
    if (typeof applied.width === "number") fields.width = applied.width;
    if (typeof applied.height === "number") fields.height = applied.height;
    const at = canvas.artboards.findIndex((artboard) => artboard.id === applied.id);
    const next = merge(at >= 0 ? canvas.artboards[at] : undefined, applied.id, fields);
    if (at >= 0) canvas.artboards[at] = next;
    else canvas.artboards.push(next);
  }
  const removed = new Set(details.removed);
  canvas.artboards = canvas.artboards.filter((artboard) => !removed.has(artboard.id));
  canvas.lastChanged = details.applied.map((applied) => applied.id).filter((id) => !removed.has(id));
  canvases.set(canvas.id, canvas);
}

type BranchEntry = {
  type?: string;
  message?: {
    role?: string;
    content?: unknown;
    toolCallId?: string;
    toolName?: string;
    isError?: boolean;
    details?: unknown;
  };
};

/** Every canvas on this branch, in creation order. Refused calls carry no details and drop out. */
export function canvasesFromBranch(branch: Iterable<unknown>): Map<string, Canvas> {
  const args = new Map<string, unknown>();
  const canvases = new Map<string, Canvas>();
  for (const raw of branch) {
    const entry = raw as BranchEntry;
    if (entry?.type !== "message" || !entry.message) continue;
    const message = entry.message;
    if (message.role === "assistant" && Array.isArray(message.content)) {
      for (const block of message.content as Array<{ type?: unknown; name?: unknown; id?: unknown; arguments?: unknown }>) {
        if (block?.type === "toolCall" && block.name === CANVAS_TOOL_NAME && typeof block.id === "string") args.set(block.id, block.arguments);
      }
    } else if (message.role === "toolResult" && message.toolName === CANVAS_TOOL_NAME && !message.isError && isCanvasDetails(message.details)) {
      applyCall(canvases, args.get(message.toolCallId ?? ""), message.details);
    }
  }
  return canvases;
}

/** An artboard's document: its own markup, or its base's, with its stylesheet after it. */
export function resolveArtboardHtml(canvas: Canvas, artboard: Artboard): string {
  const base = artboard.basedOn ? canvas.artboards.find((candidate) => candidate.id === artboard.basedOn) : undefined;
  const html = (base ? base.html : artboard.html) ?? "";
  return artboard.css?.trim() ? `${html}\n<style>${artboard.css}</style>` : html;
}

/** A lowercase id from a name: letters, digits and single hyphens. */
export function slugify(value: string): string {
  const slug = value.normalize("NFKD").replace(/[\u0300-\u036f]/g, "").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, MAX_ID_CHARS).replace(/-+$/, "");
  return slug || "artboard";
}

function uniqueId(base: string, taken: (id: string) => boolean): string {
  if (!taken(base)) return base;
  for (let suffix = 2; ; suffix += 1) {
    const candidate = `${base.slice(0, MAX_ID_CHARS - String(suffix).length - 1)}-${suffix}`;
    if (!taken(candidate)) return candidate;
  }
}

function clean(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function bytes(value: string | undefined): number {
  return value ? Buffer.byteLength(value, "utf8") : 0;
}

function kib(count: number): string {
  return `${Math.round(count / 1024)} KiB`;
}

export type PlannedCall =
  | { ok: true; canvas: Canvas; details: CanvasDetails; created: boolean }
  | { ok: false; error: string };

/**
 * Validates one `canvas` call against the branch's canvases and works out what it does. Nothing
 * is kept here: the call's result carries `details`, and the branch is the record.
 */
export function planCall(canvases: ReadonlyMap<string, Canvas>, input: unknown): PlannedCall {
  if (typeof input !== "object" || input === null || Array.isArray(input)) {
    return { ok: false, error: "Expected an object with a title or a canvas id, and artboards." };
  }
  const params = input as Record<string, unknown>;
  const fail = (error: string): PlannedCall => ({ ok: false, error });

  let title: string | undefined;
  if (params.title !== undefined) {
    if (typeof params.title !== "string" || !clean(params.title)) return fail("title must be a short non-empty string.");
    title = clean(params.title);
    if (title.length > MAX_CANVAS_TITLE) return fail(`title must be at most ${MAX_CANVAS_TITLE} characters.`);
  }

  let base: Canvas | undefined;
  if (params.canvas !== undefined) {
    if (typeof params.canvas !== "string" || !params.canvas.trim()) return fail("canvas must be the id of a canvas in this chat.");
    base = canvases.get(params.canvas.trim());
    if (!base) {
      const known = [...canvases.keys()];
      return fail(`There is no canvas “${params.canvas.trim()}” in this chat.${known.length ? ` Canvases: ${known.join(", ")}.` : " Omit canvas to start one."}`);
    }
  } else if (!title) {
    return fail("Give a title to start a new canvas, or the id of an existing one in canvas.");
  }

  const entries = params.artboards ?? [];
  if (!Array.isArray(entries)) return fail("artboards must be a list.");
  if (entries.length > MAX_ARTBOARDS_PER_CALL) {
    return fail(`One call can add or change at most ${MAX_ARTBOARDS_PER_CALL} artboards. Split the rest into another call.`);
  }
  const removals = params.remove ?? [];
  if (!Array.isArray(removals) || removals.some((value) => typeof value !== "string")) return fail("remove must be a list of artboard ids.");
  if (!base && removals.length) return fail("A new canvas has nothing to remove.");
  if (entries.length === 0 && removals.length === 0 && (!base || !title || title === base.title)) {
    return fail(base ? "Nothing to change: add or revise artboards, remove some, or give a new title." : "A new canvas needs at least one artboard.");
  }

  const id = base?.id ?? uniqueId(slugify(title!), (candidate) => canvases.has(candidate));
  const canvas: Canvas = { id, title: title ?? base!.title, artboards: (base?.artboards ?? []).map((artboard) => ({ ...artboard })), lastChanged: [] };
  // An id or an exact name, case aside: the model may refer to an artboard it just added.
  const find = (ref: string) => {
    const wanted = ref.trim();
    return canvas.artboards.find((artboard) => artboard.id === wanted)
      ?? canvas.artboards.find((artboard) => artboard.name.toLowerCase() === wanted.toLowerCase());
  };

  const applied: CanvasApplied[] = [];
  let callBytes = 0;
  for (const [index, raw] of entries.entries()) {
    const at = `artboards[${index}]`;
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) return fail(`${at} must be an object.`);
    const entry = raw as Record<string, unknown>;
    for (const key of ["id", "name", "group", "html", "basedOn", "css", "notes"]) {
      if (entry[key] !== undefined && typeof entry[key] !== "string") return fail(`${at}.${key} must be a string.`);
    }
    for (const key of ["width", "height"]) {
      if (entry[key] !== undefined && (typeof entry[key] !== "number" || !Number.isFinite(entry[key]))) return fail(`${at}.${key} must be a number of pixels.`);
    }
    const ref = typeof entry.id === "string" && entry.id.trim() ? entry.id.trim() : undefined;
    const existing = ref ? canvas.artboards.find((artboard) => artboard.id === ref) : undefined;
    if (ref && !existing) return fail(`There is no artboard “${ref}” on “${canvas.title}”. Omit id to add a new one.`);
    if (existing && applied.some((item) => item.id === existing.id)) return fail(`${at} changes “${existing.id}” a second time in one call.`);

    const fields = fieldsOf(entry);
    if (fields.name !== undefined) {
      if (!fields.name) return fail(`${at}.name must not be empty.`);
      if (fields.name.length > MAX_ARTBOARD_NAME) return fail(`${at}.name must be at most ${MAX_ARTBOARD_NAME} characters.`);
    }
    if (fields.group !== undefined && fields.group.length > MAX_ARTBOARD_GROUP) {
      return fail(`${at}.group must be at most ${MAX_ARTBOARD_GROUP} characters.`);
    }
    if (fields.notes !== undefined && fields.notes.length > MAX_ARTBOARD_NOTES) {
      return fail(`${at}.notes must be at most ${MAX_ARTBOARD_NOTES} characters.`);
    }
    if (fields.width !== undefined && (fields.width < MIN_ARTBOARD_SIZE || fields.width > MAX_ARTBOARD_WIDTH)) {
      return fail(`${at}.width must be between ${MIN_ARTBOARD_SIZE} and ${MAX_ARTBOARD_WIDTH} pixels.`);
    }
    if (fields.height !== undefined && (fields.height < MIN_ARTBOARD_SIZE || fields.height > MAX_ARTBOARD_HEIGHT)) {
      return fail(`${at}.height must be between ${MIN_ARTBOARD_SIZE} and ${MAX_ARTBOARD_HEIGHT} pixels, or omitted to fit the content.`);
    }
    if (fields.html !== undefined && !fields.html.trim()) return fail(`${at}.html must not be empty.`);
    if (fields.html !== undefined && fields.basedOn !== undefined) return fail(`${at} gives both html and basedOn. A variant takes its base's markup; give it css instead.`);
    if (bytes(fields.html) > MAX_ARTBOARD_HTML_BYTES) {
      return fail(`${at}.html is ${kib(bytes(fields.html))}, over the ${kib(MAX_ARTBOARD_HTML_BYTES)} limit. Draw it with less markup, or move shared styles into a base artboard.`);
    }
    if (bytes(fields.css) > MAX_ARTBOARD_CSS_BYTES) return fail(`${at}.css is over the ${kib(MAX_ARTBOARD_CSS_BYTES)} limit.`);
    callBytes += bytes(fields.html) + bytes(fields.css);

    if (!existing) {
      if (fields.name === undefined) return fail(`${at} needs a name.`);
      if (fields.width === undefined && fields.basedOn === undefined) return fail(`${at} needs a width, e.g. 1440 for desktop or 390 for a phone.`);
      if (fields.html === undefined && fields.basedOn === undefined) return fail(`${at} needs html, or basedOn to make it a variant of another artboard.`);
    }
    const artboardId = existing?.id ?? uniqueId(slugify(fields.name!), (candidate) => canvas.artboards.some((artboard) => artboard.id === candidate));
    if (fields.basedOn !== undefined) {
      const target = find(fields.basedOn);
      if (!target) return fail(`${at}.basedOn names “${fields.basedOn}”, which is not on “${canvas.title}”.`);
      if (target.id === artboardId) return fail(`${at} can't be a variant of itself.`);
      if (target.basedOn) return fail(`${at}.basedOn names “${target.id}”, which is itself a variant. Base it on “${target.basedOn}” instead.`);
      if (canvas.artboards.some((artboard) => artboard.basedOn === artboardId)) {
        return fail(`“${artboardId}” is the base of other variants, so it can't become a variant itself.`);
      }
      fields.basedOn = target.id;
    }
    // A new variant that gives no width is drawn at its base's whole size; one that gives a width
    // is another device size, and its height is its own (fit the content when omitted).
    const inherited: Pick<CanvasApplied, "width" | "height"> = {};
    const base = fields.basedOn !== undefined ? canvas.artboards.find((artboard) => artboard.id === fields.basedOn) : undefined;
    if (!existing && base && fields.width === undefined) {
      inherited.width = fields.width = base.width;
      if (fields.height === undefined && base.height !== undefined) inherited.height = fields.height = base.height;
    }
    const next = merge(existing, artboardId, fields);
    const position = canvas.artboards.findIndex((artboard) => artboard.id === artboardId);
    if (position >= 0) canvas.artboards[position] = next;
    else canvas.artboards.push(next);
    applied.push({ index, id: artboardId, op: existing ? "update" : "add", ...(fields.basedOn !== undefined ? { basedOn: fields.basedOn } : {}), ...inherited });
  }
  if (callBytes > MAX_CALL_BYTES) {
    return fail(`This call carries ${kib(callBytes)} of markup, over the ${kib(MAX_CALL_BYTES)} limit. Split it into smaller calls.`);
  }

  const removed: string[] = [];
  for (const ref of removals as string[]) {
    const target = find(ref);
    if (!target) return fail(`remove names “${ref}”, which is not on “${canvas.title}”.`);
    if (applied.some((item) => item.id === target.id)) return fail(`“${target.id}” is both changed and removed in one call.`);
    if (!removed.includes(target.id)) removed.push(target.id);
  }
  canvas.artboards = canvas.artboards.filter((artboard) => !removed.includes(artboard.id));
  const orphan = canvas.artboards.find((artboard) => artboard.basedOn && removed.includes(artboard.basedOn));
  if (orphan) return fail(`“${orphan.basedOn}” can't be removed while “${orphan.id}” is a variant of it. Remove or rebase that variant too.`);
  if (canvas.artboards.length > MAX_ARTBOARDS_PER_CANVAS) {
    return fail(`A canvas holds at most ${MAX_ARTBOARDS_PER_CANVAS} artboards. Remove some, or start a new canvas for the next round.`);
  }

  canvas.lastChanged = applied.map((item) => item.id);
  return {
    ok: true,
    canvas,
    created: !base,
    details: { v: 1, canvas: canvas.id, title: canvas.title, applied, removed },
  };
}

/** How the result and `canvas_look` name an artboard to the model. */
export function describeArtboard(artboard: Artboard): string {
  const size = `${artboard.width}×${artboard.height ?? "auto"}`;
  const variant = artboard.basedOn ? `, variant of ${artboard.basedOn}` : "";
  return `${artboard.id} — “${artboard.name}”${artboard.group ? ` [${artboard.group}]` : ""} ${size}${variant}`;
}
