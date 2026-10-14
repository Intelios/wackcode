import { describe, expect, it, vi } from "vitest";
import type { ModelRuntime } from "./model-runtime.js";
import { instrumentOpenCodeHeaders } from "./opencode-headers.js";

function fixture() {
  const stream = vi.fn();
  const runtime = { stream, streamSimple: stream, streamDeferred: stream } as unknown as ModelRuntime;
  instrumentOpenCodeHeaders(runtime);
  return { runtime, stream };
}

describe("OpenCode routing for custom connections", () => {
  it.each(["stream", "streamSimple", "streamDeferred"] as const)("adds per-session headers through %s without mutating caller options", (method) => {
    const { runtime, stream } = fixture();
    const model = { provider: "custom-uuid", baseUrl: "https://opencode.ai/inference/go/v1" } as never;
    const options = { sessionId: "parent-session", headers: { "X-Other": "kept" } };
    runtime[method](model, {} as never, options);
    runtime[method](model, {} as never, { sessionId: "child-session" });
    expect(stream.mock.calls[0][2]).toEqual({ sessionId: "parent-session", headers: {
      "X-Other": "kept", "User-Agent": "WackCode", "x-opencode-session": "parent-session"
    } });
    expect(stream.mock.calls[1][2].headers["x-opencode-session"]).toBe("child-session");
    expect(options.headers).toEqual({ "X-Other": "kept" });
  });

  it("honors explicit headers case-insensitively and doesn't invent a session", () => {
    const { runtime, stream } = fixture();
    const model = { baseUrl: "https://opencode.ai/inference/go/v1" } as never;
    const headers = { "X-OpenCode-Session": "explicit", "user-agent": "custom-agent" };
    runtime.streamSimple(model, { messages: [] }, { sessionId: "pi-session", headers });
    expect(stream.mock.calls[0][2].headers).toEqual(headers);
    runtime.streamSimple(model, { messages: [] });
    expect(stream.mock.calls[1][2].headers).toEqual({ "User-Agent": "WackCode" });
  });

  it.each(["https://opencode.ai.example/v1", "https://example.com/opencode.ai/v1", "https://generativelanguage.googleapis.com/v1beta"])("leaves %s untouched", (baseUrl) => {
    const { runtime, stream } = fixture();
    const options = { sessionId: "private-session" };
    runtime.streamSimple({ baseUrl } as never, { messages: [] }, options);
    expect(stream.mock.calls[0][2]).toBe(options);
  });
});
