import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NormalizedMessage, SubagentStreamFrame, SubagentTranscript } from "./protocol.js";
import { normalizeMessage } from "./message-normalization.js";
import type { ThinkingDurations } from "./run-timing.js";
import {
  MAX_TOOL_OUTPUT_CHARS,
  SubagentStreams,
  TRIMMED_NOTE,
  capTranscript,
  clip,
  isSubagentTranscript,
  sanitizeMessage
} from "./subagent-stream.js";

type RawBlock = { type: string; text?: string; thinking?: string; name?: string; id?: string; arguments?: unknown };
type RawMessage = { role: string; content: RawBlock[]; toolCallId?: string; toolName?: string };

/** The shared worker normalizer, with shorter positional ids for these fixtures. */
function normalize(raw: unknown, position: number, thinking?: ThinkingDurations, starts?: Array<number | null>): NormalizedMessage | undefined {
  const message = normalizeMessage(raw, position, thinking, () => ({ type: "image" }), starts);
  return message ? { ...message, id: `${message.role}-${position}` } : undefined;
}

const assistant = (text: string): RawMessage => ({ role: "assistant", content: [{ type: "text", text }] });
const call = (id: string, name: string, args: unknown): RawMessage => ({ role: "assistant", content: [{ type: "toolCall", id, name, arguments: args }] });
const result = (id: string, name: string, text: string): RawMessage => ({ role: "toolResult", toolCallId: id, toolName: name, content: [{ type: "text", text }] });

function message(id: string, role: NormalizedMessage["role"], blocks: NormalizedMessage["blocks"]): NormalizedMessage {
  return { id, role, blocks };
}

describe("clip", () => {
  it("keeps the head or the tail, saying how much went", () => {
    expect(clip("short", 10, "head")).toBe("short");
    expect(clip("abcdefghij", 4, "head")).toBe("abcd\n… 6 more characters");
    expect(clip("abcdefghij", 4, "tail")).toBe("… 6 earlier characters\nghij");
  });
});

describe("sanitizeMessage", () => {
  const redact = (text: string) => text.split("sk-secret").join("[credential redacted]");

  it("redacts every place a child's words or a tool's output appear", () => {
    const clean = sanitizeMessage(message("a", "assistant", [
      { type: "thinking", text: "the key is sk-secret", startedAt: 1_000 },
      { type: "text", text: "using sk-secret" },
      { type: "tool-call", toolName: "bash", toolCallId: "c1", arguments: { command: "echo sk-secret", nested: ["sk-secret", 3] } },
      { type: "image", imageId: "image-1", thumbnail: "data:image/png;base64,AAAA" }
    ]), redact);
    expect(JSON.stringify(clean)).not.toContain("sk-secret");
    expect(clean.blocks.map((block) => block.type)).toEqual(["thinking", "text", "tool-call"]);
    expect(clean.blocks[0].startedAt).toBe(1_000);
    expect(clean.blocks[2].arguments).toEqual({ command: "echo [credential redacted]", nested: ["[credential redacted]", 3] });
  });

  it("keeps a tool's output tail and an edit's diff, and drops other details", () => {
    const long = `${"x".repeat(MAX_TOOL_OUTPUT_CHARS)}END sk-secret`;
    const clean = sanitizeMessage(message("t", "tool", [
      { type: "tool-result", toolName: "edit", toolCallId: "c1", text: long, details: { diff: "+sk-secret", fullOutputPath: "/tmp/out" } }
    ]), redact);
    const block = clean.blocks[0];
    expect(block.text?.endsWith("END [credential redacted]")).toBe(true);
    expect(block.text?.startsWith("… ")).toBe(true);
    expect(block.details).toEqual({ diff: "+[credential redacted]" });
    const plain = sanitizeMessage(message("t", "tool", [{ type: "tool-result", text: "ok", details: { truncation: {} } }]), redact);
    expect(plain.blocks[0]).not.toHaveProperty("details");
  });
});

describe("capTranscript", () => {
  it("leaves a transcript within budget untouched", () => {
    const messages = [message("a", "assistant", [{ type: "text", text: "hi" }])];
    const capped = capTranscript(messages, 10_000);
    expect(capped).toEqual({ messages, truncated: false });
    expect(capped.messages).toBe(messages);
  });

  it("trims the oldest tool output first and keeps untouched messages as they were", () => {
    const answer = message("answer", "assistant", [{ type: "text", text: "All done." }]);
    const calls = [0, 1, 2].map((index) => message(`call-${index}`, "assistant", [{ type: "tool-call", toolName: "read", toolCallId: `c${index}`, arguments: { path: `f${index}.ts` } }]));
    const outputs = [0, 1, 2].map((index) => message(`out-${index}`, "tool", [{ type: "tool-result", toolName: "read", toolCallId: `c${index}`, text: "y".repeat(3_000) }]));
    const messages = [calls[0], outputs[0], calls[1], outputs[1], calls[2], outputs[2], answer];
    const size = JSON.stringify(messages).length;
    const capped = capTranscript(messages, size - 4_000);
    expect(capped.truncated).toBe(true);
    expect(capped.messages).toHaveLength(messages.length);
    expect(capped.messages[1].blocks[0].text).toBe(TRIMMED_NOTE);
    expect(capped.messages[3].blocks[0].text).toBe(TRIMMED_NOTE);
    expect(capped.messages[5]).toBe(outputs[2]);
    expect(capped.messages[6]).toBe(answer);
    expect(JSON.stringify(capped.messages).length).toBeLessThanOrEqual(size - 4_000);
  });

  it("drops the oldest messages as a last resort, never the answer", () => {
    const messages = [0, 1, 2, 3].map((index) => message(`m${index}`, "assistant", [{ type: "text", text: `${"z".repeat(1_000)} ${index}` }]));
    const capped = capTranscript(messages, 1_200);
    expect(capped.truncated).toBe(true);
    expect(capped.messages.at(-1)?.id).toBe("m3");
    expect(capped.messages.length).toBeLessThan(messages.length);
  });
});

describe("isSubagentTranscript", () => {
  it("accepts only the saved shape", () => {
    expect(isSubagentTranscript({ v: 1, messages: [{ id: "a", role: "assistant", blocks: [] }] })).toBe(true);
    expect(isSubagentTranscript({ v: 1, messages: [] })).toBe(true);
    expect(isSubagentTranscript({ v: 2, messages: [] })).toBe(false);
    expect(isSubagentTranscript({ v: 1, messages: [{ id: 3 }] })).toBe(false);
    expect(isSubagentTranscript(undefined)).toBe(false);
  });
});

describe("SubagentStreams", () => {
  let frames: SubagentStreamFrame[];
  let saved: Map<string, SubagentTranscript>;
  let streams: SubagentStreams;

  beforeEach(() => {
    vi.useFakeTimers();
    frames = [];
    saved = new Map();
    streams = new SubagentStreams({
      emit: (frame) => frames.push(frame),
      normalize,
      redactor: () => (text) => text.split("sk-secret").join("[credential redacted]"),
      saved: (target) => saved.get(`${target.toolCallId}#${target.index}`),
      intervalMs: 10
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  const target = { toolCallId: "call-1", index: 1 };

  it("says a child is missing until it starts, then streams it and reports when it stops", () => {
    streams.watch(target);
    expect(frames).toEqual([{ ...target, rev: 0, reset: true, upserts: [], removed: [], partial: null, live: false, missing: true }]);

    const messages: RawMessage[] = [{ role: "user", content: [{ type: "text", text: "the task" }] }, { role: "system", content: [] }];
    const child = streams.child(target);
    child.started(() => messages);
    vi.advanceTimersByTime(10);
    // The task and Pi's system declaration are not part of the transcript.
    expect(frames[1]).toMatchObject({ rev: 1, upserts: [], removed: [], partial: null, live: true });
    expect(frames[1]).not.toHaveProperty("reset");
    expect(frames[1]).not.toHaveProperty("missing");

    const streaming = { role: "assistant", content: [{ type: "text", text: "Looking at sk-secret" }] };
    child.event({ type: "message_start", message: streaming });
    child.event({ type: "message_update", message: streaming, assistantMessageEvent: { type: "text_delta", contentIndex: 0 } });
    vi.advanceTimersByTime(10);
    expect(frames).toHaveLength(3);
    expect(frames[2].partial?.blocks).toEqual([{ type: "text", text: "Looking at [credential redacted]" }]);

    const first = call("t1", "ls", { path: "." });
    messages.push(first);
    child.event({ type: "message_end", message: first });
    messages.push(result("t1", "ls", "a.ts"));
    child.event({ type: "tool_execution_end" });
    vi.advanceTimersByTime(10);
    expect(frames[3].partial).toBeNull();
    expect(frames[3].upserts.map((entry) => entry.id)).toEqual(["assistant-2", "tool-3"]);

    // Unchanged messages are not sent again.
    const answer = assistant("Found it.");
    messages.push(answer);
    child.event({ type: "message_end", message: answer });
    vi.advanceTimersByTime(10);
    expect(frames[4].upserts.map((entry) => entry.id)).toEqual(["assistant-4"]);

    child.ended([...messages]);
    const transcript = child.finish();
    expect(transcript?.messages.map((entry) => entry.id)).toEqual(["assistant-2", "tool-3", "assistant-4"]);
    // Stopping goes out at once, not an interval later.
    expect(frames.at(-1)).toMatchObject({ rev: 5, live: false, upserts: [], removed: [] });

    // Watched again, it comes back whole from memory.
    streams.watch(target);
    expect(frames.at(-1)).toMatchObject({ rev: 0, reset: true, live: false });
    expect(frames.at(-1)?.upserts).toBe(transcript?.messages);
  });

  it("streams stable thinking starts, drops them on close, and saves only durations", () => {
    vi.setSystemTime(1_000);
    const messages: RawMessage[] = [];
    const child = streams.child(target);
    child.started(() => messages);
    streams.watch(target);
    const first: RawMessage = { role: "assistant", content: [{ type: "thinking", thinking: "sk-secret reasoning" }] };
    child.event({ type: "message_start", message: first });
    child.event({ type: "message_update", message: first, assistantMessageEvent: { type: "thinking_start", contentIndex: 0 } });
    vi.advanceTimersByTime(10);
    const open = frames.at(-1)?.partial;
    expect(open?.blocks).toEqual([{ type: "thinking", text: "[credential redacted] reasoning", startedAt: 1_000 }]);

    vi.setSystemTime(1_200);
    child.event({ type: "message_update", message: first, assistantMessageEvent: { type: "thinking_delta", contentIndex: 0 } });
    child.event({ type: "message_update", message: first, assistantMessageEvent: { type: "thinking_start", contentIndex: 0 } });
    // Identical starts and text do not emit redundant frames.
    const count = frames.length;
    vi.advanceTimersByTime(10);
    expect(frames).toHaveLength(count);

    vi.setSystemTime(1_500);
    first.content.push({ type: "text", text: "Answer" });
    child.event({ type: "message_update", message: first, assistantMessageEvent: { type: "text_start", contentIndex: 1 } });
    vi.advanceTimersByTime(10);
    expect(frames.at(-1)?.partial?.blocks[0]).toEqual({ type: "thinking", text: "[credential redacted] reasoning", durationMs: 500 });
    messages.push(first);
    child.event({ type: "message_end", message: first });
    vi.advanceTimersByTime(10);
    expect(frames.at(-1)?.partial).toBeNull();
    expect(frames.at(-1)?.upserts[0].blocks[0]).toEqual({ type: "thinking", text: "[credential redacted] reasoning", durationMs: 500 });

    vi.setSystemTime(2_000);
    const next: RawMessage = { role: "assistant", content: [{ type: "thinking", thinking: "More" }] };
    child.event({ type: "message_start", message: next });
    child.event({ type: "message_update", message: next, assistantMessageEvent: { type: "thinking_delta", contentIndex: 0 } });
    vi.advanceTimersByTime(10);
    expect(frames.at(-1)?.partial?.blocks).toEqual([{ type: "thinking", text: "More", startedAt: 2_000 }]);
    expect(frames.at(-1)?.upserts).toEqual([]);
    // The previous frame was not mutated as the reasoning ended or the next message began.
    expect(open?.blocks[0].startedAt).toBe(1_000);

    vi.setSystemTime(2_300);
    messages.push(next);
    child.event({ type: "message_end", message: next });
    child.ended([...messages]);
    const transcript = child.finish();
    expect(frames.at(-1)).toMatchObject({ partial: null, live: false });
    expect(transcript?.messages.map((message) => message.blocks[0].durationMs)).toEqual([500, 300]);
    expect(transcript?.messages.flatMap((message) => message.blocks).every((block) => !Object.hasOwn(block, "startedAt"))).toBe(true);
  });

  it("serves a saved transcript, and only ever streams the watched child", () => {
    const transcript: SubagentTranscript = { v: 1, messages: [message("a", "assistant", [{ type: "text", text: "saved" }])], truncated: true };
    saved.set("call-1#1", transcript);
    streams.watch(target);
    expect(frames).toEqual([{ ...target, rev: 0, reset: true, upserts: transcript.messages, removed: [], partial: null, live: false, truncated: true }]);

    const other = streams.child({ toolCallId: "call-2", index: 0 });
    other.started(() => [assistant("elsewhere")]);
    other.event({ type: "message_end", message: assistant("elsewhere") });
    vi.advanceTimersByTime(50);
    expect(frames).toHaveLength(1);

    streams.watch(null);
    const again = streams.child(target);
    again.started(() => [assistant("rerun")]);
    vi.advanceTimersByTime(50);
    expect(frames).toHaveLength(1);
  });

  it("has nothing to save for a child that never started, and never throws into a child's loop", () => {
    const child = streams.child(target);
    expect(() => child.event({ type: "message_update", message: { role: "assistant", content: "not an array" }, assistantMessageEvent: null })).not.toThrow();
    expect(child.finish()).toBeUndefined();
  });
});
