/**
 * Sub-agent transcripts for the side panel. A child runs as an in-memory Pi session inside this
 * worker (`subagent-runner.ts`); this registry turns its messages into the transcript the panel
 * renders — live while it works, and saved with the call's result once it ends.
 *
 * - Everything a child wrote passes through the worker's credential redactor, like its final
 *   answer does, and is capped per block (`sanitizeMessage`), so a `read` of a huge file never
 *   ships whole. A saved transcript is capped again as a whole (`capTranscript`). The same
 *   per-block caps apply live, so the panel looks the same before and after a reload.
 * - The child's task (its one user message) is already on the call's result, and Pi's system
 *   declarations carry no text, so a transcript holds only what the child answered and ran.
 * - Only the watched child streams (`watch`): at most one frame per `UPDATE_INTERVAL_MS`,
 *   diffed against the last frame by message identity, so a frame costs O(changes). Finished
 *   messages keep one normalized object for the life of the worker, which is what makes the
 *   diff work.
 * - Watching is sticky: a child that hasn't started yet (queued behind its siblings) begins
 *   streaming the moment it does.
 */
import { UPDATE_INTERVAL_MS } from "./builtin/subagents/details.js";
import { diffMessages } from "./delta.js";
import type { NormalizedBlock, NormalizedMessage, SubagentStreamFrame, SubagentTarget, SubagentTranscript } from "./protocol.js";
import type { ThinkingDurations } from "./run-timing.js";
import type { ChildTranscriptHooks } from "./subagent-runner.js";
import { ThinkingClock } from "./thinking-timing.js";

/** One text block of the child's own answer. */
export const MAX_TEXT_CHARS = 48 * 1024;
/** One thinking block. */
export const MAX_THINKING_CHARS = 24 * 1024;
/** One tool's output: the tail, roughly what the panel's tool row shows expanded. */
export const MAX_TOOL_OUTPUT_CHARS = 8 * 1024;
/** One string inside a tool call's arguments (a `write`'s content, a long command). */
export const MAX_ARGUMENT_CHARS = 8 * 1024;
/** An edit's diff. */
export const MAX_DIFF_CHARS = 24 * 1024;
/** A saved transcript, all told: it rides the chat's session file. */
export const MAX_TRANSCRIPT_CHARS = 200 * 1024;
/** Finished children kept in memory, so a watch never falls in the gap before their call's
 *  result reaches the session. */
const RECENT_CHILDREN = 16;
/** What trimmed output reads as in the panel. */
export const TRIMMED_NOTE = "[Trimmed to keep this chat's history small.]";

export type Redact = (text: string) => string;

export interface SubagentStreamsOptions {
  /** Send one frame of the watched child's transcript to the host. */
  emit(frame: SubagentStreamFrame): void;
  /** The worker's own message normalizer, so the panel renders a child exactly like the chat. */
  normalize(raw: unknown, position: number, thinking?: ThinkingDurations): NormalizedMessage | undefined;
  /** A redactor over the worker's current secrets. Taken once per frame, not once per string. */
  redactor(): Redact;
  /** A finished call's saved transcript, read from the chat's session. */
  saved(target: SubagentTarget): SubagentTranscript | undefined;
  intervalMs?: number;
}

/** The hooks one child's runner reports through, and its saved transcript once it has run. */
export interface ChildStream extends ChildTranscriptHooks {
  /** The child's run is over: its transcript for the call's result (none if it never started). */
  finish(): SubagentTranscript | undefined;
}

interface Child {
  target: SubagentTarget;
  key: string;
  /** Reads the child's messages while it runs; set once its session exists. */
  read?: () => readonly unknown[];
  /** Everything the child said, captured just before its session was disposed. */
  ended?: readonly unknown[];
  /** The assistant message it is writing right now. */
  streaming?: unknown;
  clock: ThinkingClock;
}

interface Watch {
  target: SubagentTarget;
  key: string;
  /** The last frame's rev; -1 until the first (reset) frame. */
  rev: number;
  sent?: NormalizedMessage[];
  partialJson: string;
  live: boolean;
  truncated: boolean;
  missing: boolean;
}

interface CachedMessage {
  message: NormalizedMessage;
  /** The durations it was normalized with: a message cached before its thinking was clocked is
   *  rebuilt once the clock has them. */
  thinking: ThinkingDurations | undefined;
}

export function targetKey(target: SubagentTarget): string {
  return `${target.toolCallId}#${target.index}`;
}

/** A saved transcript as found in a session file: checked, since the file is only JSON. */
export function isSubagentTranscript(value: unknown): value is SubagentTranscript {
  if (!value || typeof value !== "object") return false;
  const transcript = value as { v?: unknown; messages?: unknown };
  return transcript.v === 1 && Array.isArray(transcript.messages)
    && transcript.messages.every((message) => {
      const candidate = message as { id?: unknown; role?: unknown; blocks?: unknown } | null;
      return typeof candidate?.id === "string" && typeof candidate.role === "string" && Array.isArray(candidate.blocks);
    });
}

/** At most `max` characters, keeping the head (prose, arguments) or the tail (tool output). */
export function clip(text: string, max: number, keep: "head" | "tail"): string {
  if (text.length <= max) return text;
  const dropped = text.length - max;
  return keep === "head"
    ? `${text.slice(0, max)}\n… ${dropped} more characters`
    : `… ${dropped} earlier characters\n${text.slice(-max)}`;
}

function redactValue(value: unknown, redact: Redact, depth = 0): unknown {
  if (typeof value === "string") return clip(redact(value), MAX_ARGUMENT_CHARS, "head");
  if (!value || typeof value !== "object" || depth > 8) return value;
  if (Array.isArray(value)) return value.map((item) => redactValue(item, redact, depth + 1));
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, redactValue(item, redact, depth + 1)]));
}

/**
 * One child message as the panel may hold it: redacted everywhere a child's words or a tool's
 * output can appear, capped per block, and without the images or tool details the panel never
 * renders (an edit's diff is the one detail it shows).
 */
export function sanitizeMessage(message: NormalizedMessage, redact: Redact): NormalizedMessage {
  const blocks = message.blocks.flatMap((block): NormalizedBlock[] => {
    if (block.type === "image") return [];
    if (block.type === "text") return [{ ...block, text: clip(redact(block.text ?? ""), MAX_TEXT_CHARS, "head") }];
    if (block.type === "thinking") return [{ ...block, text: clip(redact(block.text ?? ""), MAX_THINKING_CHARS, "head") }];
    if (block.type === "tool-call") return [{ ...block, arguments: redactValue(block.arguments, redact) }];
    const { details, ...rest } = block;
    const diff = (details as { diff?: unknown } | undefined)?.diff;
    return [{
      ...rest,
      text: clip(redact(block.text ?? ""), MAX_TOOL_OUTPUT_CHARS, "tail"),
      ...(typeof diff === "string" && diff ? { details: { diff: clip(redact(diff), MAX_DIFF_CHARS, "head") } } : {})
    }];
  });
  return {
    ...message,
    blocks,
    ...(message.errorMessage ? { errorMessage: redact(message.errorMessage) } : {})
  };
}

function sizeOf(message: NormalizedMessage): number {
  return JSON.stringify(message).length;
}

type Trim = (message: NormalizedMessage, isLast: boolean) => NormalizedMessage | undefined;

/** Tool output, oldest first: the part of a transcript the panel can best do without. */
const trimToolOutput: Trim = (message) =>
  message.role === "tool" && message.blocks.some((block) => block.type === "tool-result" && block.text !== TRIMMED_NOTE)
    ? { ...message, blocks: message.blocks.map((block) => block.type === "tool-result" ? { type: block.type, toolName: block.toolName, toolCallId: block.toolCallId, isError: block.isError, text: TRIMMED_NOTE } : block) }
    : undefined;

const trimThinking: Trim = (message) =>
  message.blocks.some((block) => block.type === "thinking" && block.text !== TRIMMED_NOTE)
    ? { ...message, blocks: message.blocks.map((block) => block.type === "thinking" ? { ...block, text: TRIMMED_NOTE } : block) }
    : undefined;

/** Long tool arguments (a `write`'s whole file) down to what names the call. */
const trimArguments: Trim = (message) => {
  const long = (value: unknown) => typeof value === "string" && value.length > 512;
  if (!message.blocks.some((block) => block.type === "tool-call" && block.arguments && typeof block.arguments === "object"
    && Object.values(block.arguments).some(long))) return undefined;
  return {
    ...message,
    blocks: message.blocks.map((block) => block.type === "tool-call" && block.arguments && typeof block.arguments === "object"
      ? { ...block, arguments: Object.fromEntries(Object.entries(block.arguments).map(([key, value]) => [key, long(value) ? clip(value as string, 512, "head") : value])) }
      : block)
  };
};

/** Earlier prose, never the final answer. */
const trimText: Trim = (message, isLast) =>
  !isLast && message.blocks.some((block) => block.type === "text" && (block.text?.length ?? 0) > 2048)
    ? { ...message, blocks: message.blocks.map((block) => block.type === "text" ? { ...block, text: clip(block.text ?? "", 2048, "head") } : block) }
    : undefined;

/**
 * A saved transcript within `max` characters of JSON. Trims, oldest first and one kind at a time,
 * whatever the panel can best do without: tool output, then reasoning, then long tool arguments,
 * then earlier prose; as a last resort it drops the oldest messages, always keeping the answer.
 * Untrimmed messages keep their objects.
 */
export function capTranscript(messages: NormalizedMessage[], max = MAX_TRANSCRIPT_CHARS): { messages: NormalizedMessage[]; truncated: boolean } {
  const sizes = messages.map(sizeOf);
  let size = sizes.reduce((total, value) => total + value, 0);
  if (size <= max) return { messages, truncated: false };
  const out = [...messages];
  for (const trim of [trimToolOutput, trimThinking, trimArguments, trimText]) {
    for (let index = 0; index < out.length && size > max; index += 1) {
      const trimmed = trim(out[index], index === out.length - 1);
      if (!trimmed) continue;
      const next = sizeOf(trimmed);
      size += next - sizes[index];
      sizes[index] = next;
      out[index] = trimmed;
    }
    if (size <= max) return { messages: out, truncated: true };
  }
  while (out.length > 1 && size > max) {
    size -= sizes.shift() ?? 0;
    out.shift();
  }
  return { messages: out, truncated: true };
}

export class SubagentStreams {
  private readonly children = new Map<string, Child>();
  private readonly recent = new Map<string, SubagentTranscript>();
  private readonly cache = new WeakMap<object, CachedMessage>();
  private watching: Watch | undefined;
  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor(private readonly options: SubagentStreamsOptions) {}

  /** A child about to run: the hooks its runner reports through. */
  child(target: SubagentTarget): ChildStream {
    const key = targetKey(target);
    const child: Child = { target, key, clock: new ThinkingClock() };
    this.children.set(key, child);
    return {
      started: (read) => {
        child.read = read;
        this.changed(key);
      },
      event: (event) => {
        // Pi awaits its listeners inside the child's loop: nothing here may throw into it.
        try {
          this.track(child, event);
        } catch {
          /* A transcript frame is never worth failing the child. */
        }
      },
      ended: (messages) => {
        child.ended = messages;
        child.streaming = undefined;
      },
      finish: () => this.finish(child)
    };
  }

  /** Stream `target` from now on (a reset frame goes out at once), or stop with null. */
  watch(target: SubagentTarget | null): void {
    this.clearTimer();
    if (!target) {
      this.watching = undefined;
      return;
    }
    this.watching = { target, key: targetKey(target), rev: -1, partialJson: "", live: false, truncated: false, missing: false };
    this.flush();
  }

  /** Stop streaming and forget every child: the worker is going away. */
  dispose(): void {
    this.clearTimer();
    this.watching = undefined;
    this.children.clear();
    this.recent.clear();
  }

  private track(child: Child, event: Record<string, unknown>): void {
    const type = event.type;
    const message = event.message as { role?: unknown } | undefined;
    if (message?.role === "assistant") {
      if (type === "message_start") {
        child.clock.begin();
        child.streaming = message;
      } else if (type === "message_update") {
        child.clock.update(event.assistantMessageEvent);
        child.streaming = message;
      } else if (type === "message_end") {
        child.clock.end(message);
        child.streaming = undefined;
      }
    }
    if (type === "message_start" || type === "message_update" || type === "message_end" || type === "tool_execution_end") {
      this.changed(child.key);
    }
  }

  private finish(child: Child): SubagentTranscript | undefined {
    if (this.children.get(child.key) === child) this.children.delete(child.key);
    if (!child.read && !child.ended) {
      this.changed(child.key, true);
      return undefined;
    }
    const capped = capTranscript(this.transcriptOf(child, this.options.redactor()));
    const transcript: SubagentTranscript = { v: 1, messages: capped.messages, ...(capped.truncated ? { truncated: true } : {}) };
    this.recent.delete(child.key);
    this.recent.set(child.key, transcript);
    while (this.recent.size > RECENT_CHILDREN) this.recent.delete(this.recent.keys().next().value as string);
    // The panel learns the child stopped right away, not a throttle interval later.
    this.changed(child.key, true);
    return transcript;
  }

  private transcriptOf(child: Child, redact: Redact): NormalizedMessage[] {
    const raw = child.ended ?? child.read?.() ?? [];
    const messages: NormalizedMessage[] = [];
    raw.forEach((message, position) => {
      if (!message || typeof message !== "object") return;
      const role = (message as { role?: unknown }).role;
      if (role !== "assistant" && role !== "toolResult") return;
      const thinking = child.clock.durations(message);
      let cached = this.cache.get(message);
      if (!cached || (cached.thinking === undefined && thinking !== undefined)) {
        const normalized = this.options.normalize(message, position, thinking);
        if (!normalized) return;
        cached = { message: sanitizeMessage(normalized, redact), thinking };
        this.cache.set(message, cached);
      }
      messages.push(cached.message);
    });
    return messages;
  }

  private partialOf(child: Child, redact: Redact): NormalizedMessage | null {
    const streaming = child.streaming;
    if (!streaming || child.ended) return null;
    const normalized = this.options.normalize(streaming, child.read?.().length ?? 0, child.clock.live(streaming));
    return normalized ? sanitizeMessage(normalized, redact) : null;
  }

  private view(watch: Watch) {
    const child = this.children.get(watch.key);
    if (child && (child.read || child.ended)) {
      const redact = this.options.redactor();
      return { messages: this.transcriptOf(child, redact), partial: this.partialOf(child, redact), live: true, truncated: false, missing: false };
    }
    const finished = this.recent.get(watch.key) ?? this.options.saved(watch.target);
    if (finished) return { messages: finished.messages, partial: null, live: false, truncated: finished.truncated === true, missing: false };
    return { messages: [] as NormalizedMessage[], partial: null, live: false, truncated: false, missing: true };
  }

  private changed(key: string, now = false): void {
    if (this.watching?.key !== key) return;
    if (now) {
      this.flush();
      return;
    }
    if (this.timer) return;
    this.timer = setTimeout(() => {
      this.timer = undefined;
      this.flush();
    }, this.options.intervalMs ?? UPDATE_INTERVAL_MS);
  }

  private flush(): void {
    this.clearTimer();
    const watch = this.watching;
    if (!watch) return;
    const view = this.view(watch);
    const partialJson = view.partial ? JSON.stringify(view.partial) : "";
    const diff = watch.sent ? diffMessages(watch.sent, view.messages) : undefined;
    if (diff && diff.upserts.length === 0 && diff.removed.length === 0 && partialJson === watch.partialJson
      && view.live === watch.live && view.truncated === watch.truncated && view.missing === watch.missing) return;
    watch.rev += 1;
    watch.sent = view.messages;
    watch.partialJson = partialJson;
    watch.live = view.live;
    watch.truncated = view.truncated;
    watch.missing = view.missing;
    this.options.emit({
      ...watch.target,
      rev: watch.rev,
      // The first frame, and any change a delta can't express, replaces the host's copy whole.
      ...(diff ? {} : { reset: true as const }),
      upserts: diff ? diff.upserts : view.messages,
      removed: diff ? diff.removed : [],
      partial: view.partial,
      live: view.live,
      ...(view.truncated ? { truncated: true } : {}),
      ...(view.missing ? { missing: true } : {})
    });
  }

  private clearTimer(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = undefined;
  }
}
