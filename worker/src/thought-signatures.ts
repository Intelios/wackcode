/**
 * Round-trips the `extra_content` vendor extension Google's OpenAI-compatible endpoints put on
 * tool calls (`extra_content.google.thought_signature` or `…vertex…` on Vertex's
 * `…/endpoints/openapi` route and the Gemini API's OpenAI route alike). A thinking Gemini
 * requires each function call's thought signature verbatim in the next request and answers
 * `400 INVALID_ARGUMENT "Function call is missing a thought_signature"` without it, which breaks
 * every tool-using second turn. Pi keeps the field neither on stream nor on request, and no
 * `compat` flag or stream option covers it (upstream issue earendil-works/pi#6733), so the
 * runtime's stream functions are wrapped here:
 *
 * - capture: `onProviderStreamEvent` sees every raw chunk, so each delta tool call's
 *   `extra_content` is stashed by call id and stream index;
 * - persist: the captured object is stamped onto the tool call's own `thoughtSignature` field at
 *   `toolcall_end`/`done`/`error`/`result()` — a slot the assistant-message frame serializes
 *   into the session file and transform-messages keeps for the same model, so a signature
 *   survives worker restarts and chat resume without a side channel;
 * - replay: `onPayload` writes the stored object back as `extra_content` on each outgoing
 *   `tool_calls` entry, looked up by call id.
 *
 * The stored field is `{"extra_content": <verbatim>}` — unambiguous next to pi-ai's legacy
 * `reasoning.encrypted` JSON (which carries a `type`). Both hooks chain the caller's own:
 * pi-coding-agent's sdk already sets them, so they are composed, never replaced.
 */
import type { ModelRuntime } from "./model-runtime.js";
import { isGoogleOpenAiEndpoint } from "./catalog-models.js";

type ToolCallBlock = {
  type?: string;
  id?: string;
  name?: string;
  thoughtSignature?: string;
};

interface CaptureMap {
  /** `extra_content` by the tool call's id. */
  byId: Map<string, unknown>;
  /** `extra_content` by the delta's stream index, for deltas that arrive without an id. */
  byIndex: Map<number, unknown>;
  /** Stream index → call id, so an index-keyed capture can still reach a later-named call. */
  indexToId: Map<number, string>;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function encodeSignature(extraContent: unknown): string {
  return JSON.stringify({ extra_content: extraContent });
}

/** The `extra_content` previously stored on a tool call's `thoughtSignature`, else undefined. */
export function decodeSignature(thoughtSignature: unknown): unknown {
  if (typeof thoughtSignature !== "string" || !thoughtSignature) return undefined;
  try {
    const parsed: unknown = JSON.parse(thoughtSignature);
    if (isRecord(parsed) && Object.keys(parsed).length === 1 && parsed.extra_content !== undefined) {
      return parsed.extra_content;
    }
    return undefined;
  } catch {
    return undefined;
  }
}

/** Seed the map from the outgoing context so signatures captured in earlier turns replay. */
function seedFromContext(context: unknown, capture: CaptureMap): void {
  if (!isRecord(context) || !Array.isArray(context.messages)) return;
  for (const message of context.messages) {
    if (!isRecord(message) || message.role !== "assistant" || !Array.isArray(message.content)) continue;
    for (const block of message.content as ToolCallBlock[]) {
      if (block?.type !== "toolCall" || !block.id) continue;
      const extraContent = decodeSignature(block.thoughtSignature);
      if (extraContent !== undefined) capture.byId.set(block.id, extraContent);
    }
  }
}

/** Read each delta tool call's `extra_content` out of a raw streamed chunk. */
function captureFromChunk(chunk: unknown, capture: CaptureMap): void {
  if (!isRecord(chunk) || !Array.isArray(chunk.choices)) return;
  for (const choice of chunk.choices) {
    const toolCalls = isRecord(choice) && isRecord(choice.delta) ? choice.delta.tool_calls : undefined;
    if (!Array.isArray(toolCalls)) continue;
    for (const toolCall of toolCalls) {
      if (!isRecord(toolCall)) continue;
      const index = typeof toolCall.index === "number" ? toolCall.index : undefined;
      const id = typeof toolCall.id === "string" && toolCall.id ? toolCall.id : undefined;
      if (index !== undefined && id !== undefined) capture.indexToId.set(index, id);
      if (toolCall.extra_content === undefined) continue;
      if (id !== undefined) capture.byId.set(id, toolCall.extra_content);
      if (index !== undefined) capture.byIndex.set(index, toolCall.extra_content);
    }
  }
}

function signatureFor(id: string | undefined, capture: CaptureMap): unknown {
  if (id === undefined) return undefined;
  const direct = capture.byId.get(id);
  if (direct !== undefined) return direct;
  for (const [index, callId] of capture.indexToId) {
    if (callId === id) return capture.byIndex.get(index);
  }
  return undefined;
}

/** Write captured `extra_content` back onto each outgoing `tool_calls` entry by call id. */
function injectIntoPayload(payload: unknown, capture: CaptureMap): void {
  if (!isRecord(payload) || !Array.isArray(payload.messages)) return;
  for (const message of payload.messages) {
    if (!isRecord(message) || message.role !== "assistant" || !Array.isArray(message.tool_calls)) continue;
    for (const toolCall of message.tool_calls) {
      if (!isRecord(toolCall) || typeof toolCall.id !== "string" || toolCall.extra_content !== undefined) continue;
      const extraContent = signatureFor(toolCall.id, capture);
      if (extraContent !== undefined) toolCall.extra_content = extraContent;
    }
  }
}

/** Stamp captured signatures onto the tool call blocks of a finished assistant message. */
function annotateMessage(message: unknown, capture: CaptureMap): void {
  if (!isRecord(message) || !Array.isArray(message.content)) return;
  for (const block of message.content as ToolCallBlock[]) {
    if (block?.type !== "toolCall" || block.thoughtSignature !== undefined) continue;
    const extraContent = signatureFor(block.id, capture);
    if (extraContent !== undefined) block.thoughtSignature = encodeSignature(extraContent);
  }
}

/** Annotate stream events as they pass so the persisted message carries each signature. */
function wrapEventStream<S extends AsyncIterable<unknown> & { result(): Promise<unknown> }>(stream: S, capture: CaptureMap): S {
  const annotateEvent = (event: unknown) => {
    if (!isRecord(event)) return;
    if (event.type === "toolcall_end" && isRecord(event.toolCall)) {
      const toolCall = event.toolCall as ToolCallBlock;
      if (toolCall.thoughtSignature === undefined) {
        const extraContent = signatureFor(toolCall.id, capture);
        if (extraContent !== undefined) toolCall.thoughtSignature = encodeSignature(extraContent);
      }
    } else if (event.type === "done" || event.type === "error") {
      annotateMessage(event.type === "done" ? event.message : event.error, capture);
    }
  };
  const wrapped = Object.create(stream) as S;
  wrapped[Symbol.asyncIterator] = async function* () {
    const inner = stream[Symbol.asyncIterator]();
    while (true) {
      const step = await inner.next();
      if (step.done) return step.value;
      annotateEvent(step.value);
      yield step.value;
    }
  };
  const result = stream.result.bind(stream);
  wrapped.result = () => result().then((message: unknown) => {
    annotateMessage(message, capture);
    return message;
  });
  return wrapped;
}

function usesThoughtSignatures(model: unknown): boolean {
  return isRecord(model) && model.api === "openai-completions" && isGoogleOpenAiEndpoint(model.baseUrl as string | undefined);
}

/**
 * Wraps the runtime's stream entry points for Google OpenAI-compatible models only; every other
 * model takes the original path untouched. Seeding, injection and event annotation share one
 * per-call capture map, so a signature learned on this turn's chunks is also replayed for calls
 * an earlier turn stored on `thoughtSignature` (including ones restored from a saved session).
 */
export function instrumentGoogleThoughtSignatures(runtime: ModelRuntime): void {
  for (const key of ["stream", "streamSimple", "streamDeferred"] as const) {
    const original = runtime[key].bind(runtime);
    const wrapped = (...args: Parameters<typeof original>) => {
      const [model, context, options] = args;
      if (!usesThoughtSignatures(model)) {
        return (original as (...values: typeof args) => ReturnType<typeof original>)(...args);
      }
      const capture: CaptureMap = { byId: new Map(), byIndex: new Map(), indexToId: new Map() };
      seedFromContext(context, capture);
      type OptionHooks = {
        onPayload?: (payload: unknown, model: unknown) => unknown;
        onProviderStreamEvent?: (data: unknown, model: unknown) => unknown;
      };
      const opts = options as OptionHooks | undefined;
      const previousOnPayload = opts?.onPayload;
      const previousOnStreamEvent = opts?.onProviderStreamEvent;
      args[2] = {
        ...options,
        onPayload: async (payload: unknown, requestModel: unknown) => {
          // The extension hook runs first so its rewritten payload still carries the replays.
          const target = previousOnPayload !== undefined ? await previousOnPayload(payload, requestModel) : payload;
          injectIntoPayload(target ?? payload, capture);
          return target ?? payload;
        },
        onProviderStreamEvent: async (data: unknown, requestModel: unknown) => {
          captureFromChunk(data, capture);
          await previousOnStreamEvent?.(data, requestModel);
        },
      };
      const stream = (original as (...values: typeof args) => ReturnType<typeof original>)(...args);
      return wrapEventStream(stream, capture);
    };
    Object.defineProperty(runtime, key, { value: wrapped, configurable: true });
  }
}
