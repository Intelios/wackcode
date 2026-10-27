import { describe, expect, it, vi } from "vitest";
import type { ModelRuntime } from "./model-runtime.js";
import { instrumentGoogleThoughtSignatures } from "./thought-signatures.js";

const VERTEX = "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/p/locations/us-central1/endpoints/openapi";
const GEMINI = "https://generativelanguage.googleapis.com/v1beta/openai";
const SIG = { google: { thought_signature: "AySignature/==" } };
const STORED = JSON.stringify({ extra_content: SIG });

function fakeEventStream(events: unknown[], result: unknown) {
  return {
    async *[Symbol.asyncIterator]() {
      for (const event of events) yield event;
    },
    result: () => Promise.resolve(result),
  };
}

function fixture(events: unknown[] = [], result: unknown = {}) {
  const stream = vi.fn(() => fakeEventStream(events, result));
  const runtime = { stream, streamSimple: stream, streamDeferred: stream } as unknown as ModelRuntime;
  instrumentGoogleThoughtSignatures(runtime);
  return { runtime, stream };
}

const chunkWithSignature = {
  choices: [{ delta: { role: "assistant", tool_calls: [
    { index: 0, id: "call_1", type: "function", function: { name: "grep", arguments: "{}" }, extra_content: SIG },
    { index: 1, id: "call_2", type: "function", function: { name: "read", arguments: "{}" } },
  ] }, index: 0 }],
};

describe("Google thought signatures on openai-completions", () => {
  it("replays a stored tool-call signature as extra_content on the next payload", async () => {
    const { runtime, stream } = fixture();
    const context = { messages: [
      { role: "assistant", content: [
        { type: "text", text: "checking" },
        { type: "toolCall", id: "call_1", name: "grep", arguments: {}, thoughtSignature: STORED },
        { type: "toolCall", id: "call_2", name: "read", arguments: {} },
      ] },
    ] };
    const payload = { messages: [
      { role: "user", content: "hi" },
      { role: "assistant", content: null, tool_calls: [
        { id: "call_1", type: "function", function: { name: "grep", arguments: "{}" } },
        { id: "call_2", type: "function", function: { name: "read", arguments: "{}" } },
      ] },
    ] };
    runtime.streamSimple({ api: "openai-completions", baseUrl: VERTEX } as never, context as never, {});
    const options = stream.mock.calls[0][2] as { onPayload: (p: unknown, m: unknown) => Promise<unknown> };
    await options.onPayload(payload, {});
    expect(payload.messages[1].tool_calls[0].extra_content).toEqual(SIG);
    expect(payload.messages[1].tool_calls[1]).not.toHaveProperty("extra_content");
  });

  it("captures a delta tool call's extra_content onto the finished call and result message", async () => {
    const toolCall = { type: "toolCall", id: "call_1", name: "grep", arguments: {} };
    const message = { role: "assistant", content: [{ type: "text", text: "x" }, toolCall] };
    const { runtime, stream } = fixture(
      [{ type: "toolcall_end", contentIndex: 1, toolCall }, { type: "done", reason: "toolUse", message }],
      message
    );
    const wrapped = runtime.stream({ api: "openai-completions", baseUrl: GEMINI } as never, { messages: [] } as never, {});
    const options = stream.mock.calls[0][2] as { onProviderStreamEvent: (c: unknown, m: unknown) => Promise<void> };
    await options.onProviderStreamEvent(chunkWithSignature, {});
    for await (const _event of wrapped) { /* drain */ }
    expect(toolCall.thoughtSignature).toBe(STORED);
    await wrapped.result();
    expect(message.content[1].thoughtSignature).toBe(STORED);
  });

  it("follows an index-keyed signature once the call id arrives", async () => {
    const toolCall = { type: "toolCall", id: "call_9", name: "find", arguments: {} };
    const { runtime, stream } = fixture([{ type: "toolcall_end", contentIndex: 0, toolCall }], {});
    const wrapped = runtime.stream({ api: "openai-completions", baseUrl: VERTEX } as never, { messages: [] } as never, {});
    const options = stream.mock.calls[0][2] as { onProviderStreamEvent: (c: unknown, m: unknown) => Promise<void> };
    // The signature delta precedes the id-carrying delta for the same tool_call slot.
    await options.onProviderStreamEvent({ choices: [{ delta: { tool_calls: [{ index: 2, extra_content: SIG }] } }] }, {});
    await options.onProviderStreamEvent({ choices: [{ delta: { tool_calls: [{ index: 2, id: "call_9" }] } }] }, {});
    for await (const _event of wrapped) { /* drain */ }
    expect(toolCall.thoughtSignature).toBe(STORED);
  });

  it("chains the caller's own onPayload and onProviderStreamEvent hooks", async () => {
    const { runtime, stream } = fixture();
    const onPayload = vi.fn((payload: unknown) => payload);
    const onProviderStreamEvent = vi.fn(async () => {});
    runtime.streamSimple(
      { api: "openai-completions", baseUrl: VERTEX } as never,
      { messages: [] } as never,
      { onPayload, onProviderStreamEvent, headers: { "X-Keep": "1" } } as never
    );
    const options = stream.mock.calls[0][2] as {
      onPayload: (p: unknown, m: unknown) => Promise<unknown>;
      onProviderStreamEvent: (c: unknown, m: unknown) => Promise<void>;
      headers: Record<string, string>;
    };
    const payload = { messages: [] };
    expect(await options.onPayload(payload, {})).toBe(payload);
    expect(onPayload).toHaveBeenCalledWith(payload, {});
    await options.onProviderStreamEvent(chunkWithSignature, {});
    expect(onProviderStreamEvent).toHaveBeenCalledWith(chunkWithSignature, {});
    expect(options.headers).toEqual({ "X-Keep": "1" });
  });

  it("round-trips end to end: capture on turn one replays through turn two", async () => {
    const { runtime, stream } = fixture();
    const model = { api: "openai-completions", baseUrl: VERTEX } as never;
    runtime.stream(model, { messages: [] } as never, {});
    let options = stream.mock.calls[0][2] as { onProviderStreamEvent: (c: unknown, m: unknown) => Promise<void> };
    await options.onProviderStreamEvent(chunkWithSignature, {});

    // What turn two sends back: the persisted assistant message carries the stamped block.
    const context = { messages: [{ role: "assistant", content: [
      { type: "toolCall", id: "call_1", name: "grep", arguments: {}, thoughtSignature: STORED },
    ] }] };
    runtime.stream(model, context as never, {});
    options = stream.mock.calls[1][2] as never;
    const payload = { messages: [{ role: "assistant", tool_calls: [
      { id: "call_1", type: "function", function: { name: "grep", arguments: "{}" } },
    ] }] };
    await (options as never as { onPayload: (p: unknown, m: unknown) => Promise<unknown> }).onPayload(payload, {});
    expect(payload.messages[0].tool_calls[0].extra_content).toEqual(SIG);
  });

  it.each([
    [{ api: "openai-completions", baseUrl: "https://api.openai.com/v1" }],
    [{ api: "openai-completions", baseUrl: "https://us-central1-aiplatform.googleapis.com/v1/projects/p/endpoints/99" }],
    [{ api: "anthropic-messages", baseUrl: VERTEX }],
  ])("leaves %o untouched", (model) => {
    const { runtime, stream } = fixture();
    const options = { headers: { "X-Keep": "1" } };
    runtime.streamSimple(model as never, { messages: [] } as never, options as never);
    expect(stream.mock.calls[0][2]).toBe(options);
  });
});
