import { mkdtemp, rm } from "node:fs/promises";
import { createServer, type Server } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createModelRuntime, findModel, MODEL_MISSING_MESSAGE, missingModelMessage } from "./model-runtime.js";
import type { WorkerModel, WorkerProvider } from "./protocol.js";

const model = (id: string, name: string): WorkerModel => ({
  id, name, contextWindow: 16_384, maxTokens: 1_024, reasoning: false,
  thinkingLevels: ["off"], thinkingLevelMap: { off: null }, vision: false
});
const provider = (overrides: Partial<WorkerProvider>): WorkerProvider => ({
  id: "custom-0123abcd", name: "OpenCode Go", kind: "custom", baseUrl: "http://127.0.0.1:9/v1",
  api: "openai-completions", models: [model("named", "Named"), model("unnamed", "")], ...overrides
});

const dirs: string[] = [];
const servers: Server[] = [];
afterEach(async () => {
  vi.unstubAllGlobals();
  await Promise.all(servers.splice(0).map((server) => new Promise<void>((resolve, reject) => server.close((error) => error ? reject(error) : resolve()))));
  await Promise.all(dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});
async function load(connection: WorkerProvider) {
  const dir = await mkdtemp(join(tmpdir(), "wackcode-model-runtime-"));
  dirs.push(dir);
  const pi = await import("@earendil-works/pi-coding-agent");
  return createModelRuntime(pi, connection, dir, { apiKey: "test-key" });
}

describe("connection models in Pi", () => {
  it("keeps every model on a connection where one has a blank name", async () => {
    const connection = provider({});
    const runtime = await load(connection);
    expect(runtime.getError()).toBeUndefined();
    expect((await findModel(runtime, connection, "named"))?.name).toBe("Named");
    expect((await findModel(runtime, connection, "unnamed"))?.name).toBe("unnamed");
  });

  it("blames the model only when the connection itself loaded", async () => {
    const connection = provider({});
    expect(missingModelMessage(await load(connection), connection)).toBe(MODEL_MISSING_MESSAGE);
  });

  it("runs a model over its own API, and the rest over the connection's", async () => {
    const connection = provider({ models: [model("glm", "GLM"), { ...model("muse", "Muse"), api: "openai-responses" }, { ...model("minimax", "MiniMax"), api: "anthropic-messages" }] });
    const runtime = await load(connection);
    expect(runtime.getError()).toBeUndefined();
    expect((await findModel(runtime, connection, "glm"))?.api).toBe("openai-completions");
    expect((await findModel(runtime, connection, "muse"))?.api).toBe("openai-responses");
    expect((await findModel(runtime, connection, "minimax"))?.api).toBe("anthropic-messages");
  });

  it("gives a model the base URL its own API needs", async () => {
    // The connection's `…/v1` fits its OpenAI models; the Messages one needs it stripped or its
    // client doubles the path into the gateway's 404 page (`…/v1/v1/messages`).
    const connection = provider({ models: [model("glm", "GLM"), { ...model("haiku", "Haiku"), api: "anthropic-messages" }] });
    const runtime = await load(connection);
    expect(runtime.getError()).toBeUndefined();
    expect((await findModel(runtime, connection, "glm"))?.baseUrl).toBe("http://127.0.0.1:9/v1");
    expect((await findModel(runtime, connection, "haiku"))?.baseUrl).toBe("http://127.0.0.1:9");
  });

  it("gives a Completions model the /v1 its Messages connection's base URL left off", async () => {
    const connection = provider({ api: "anthropic-messages", baseUrl: "http://127.0.0.1:9/zen/go",
      models: [model("haiku", "Haiku"), { ...model("glm", "GLM"), api: "openai-completions" }] });
    const runtime = await load(connection);
    expect(runtime.getError()).toBeUndefined();
    expect((await findModel(runtime, connection, "haiku"))?.baseUrl).toBe("http://127.0.0.1:9/zen/go");
    expect((await findModel(runtime, connection, "glm"))?.baseUrl).toBe("http://127.0.0.1:9/zen/go/v1");
  });

  it("keeps a non-v1 version segment an OpenAI connection's base URL already carries", async () => {
    // z.ai's coding plan speaks Completions under `…/paas/v4`: appending `/v1` on top
    // (`…/v4/v1/chat/completions`) lands on its 404 page, so a model on its connection's own
    // API must use the saved base URL untouched.
    const connection = provider({ baseUrl: "https://api.z.ai/api/coding/paas/v4" });
    const runtime = await load(connection);
    expect(runtime.getError()).toBeUndefined();
    expect((await findModel(runtime, connection, "named"))?.baseUrl).toBe("https://api.z.ai/api/coding/paas/v4");
  });

  it("repairs a Messages connection whose saved base URL kept a pasted endpoint suffix", async () => {
    // Records written before the base URL was normalized — the migration covers them, and the
    // per-model derivation stays correct when it can't have run.
    const connection = provider({ api: "anthropic-messages", baseUrl: "https://opencode.ai/zen/go/v1/messages" });
    const runtime = await load(connection);
    expect(runtime.getError()).toBeUndefined();
    expect((await findModel(runtime, connection, "named"))?.baseUrl).toBe("https://opencode.ai/zen/go");
  });

  it.each(["openai-completions", "anthropic-messages", "google-generative-ai"] as const)("loads a Gemini model on a %s connection with the right versioned base", async (api) => {
    const connection = provider({ api,
      baseUrl: `https://opencode.ai/inference/go${api === "anthropic-messages" ? "" : "/v1"}`,
      models: [{ ...model("gemini-4-argon-contributor", "Gemini 4 Argon Contributor"), api: "google-generative-ai" }] });
    const runtime = await load(connection);
    expect(runtime.getError()).toBeUndefined();
    expect(await findModel(runtime, connection, "gemini-4-argon-contributor")).toMatchObject({
      api: "google-generative-ai", baseUrl: "https://opencode.ai/inference/go/v1"
    });
  });

  it.each(["openai-completions", "anthropic-messages", "google-generative-ai"] as const)("routes native Gemini off the legacy Go prefix on a %s connection", async (api) => {
    const baseUrl = `https://opencode.ai/zen/go${api === "anthropic-messages" ? "" : "/v1"}`;
    const connection = provider({ api, baseUrl, models: [model("default", "Default"),
      { ...model("gemini-4-argon-contributor", "Gemini"), api: "google-generative-ai" }] });
    const runtime = await load(connection);
    expect(runtime.getError()).toBeUndefined();
    expect((await findModel(runtime, connection, "gemini-4-argon-contributor"))?.baseUrl).toBe("https://opencode.ai/inference/go/v1");
    expect((await findModel(runtime, connection, "default"))?.baseUrl).toBe(api === "google-generative-ai" ? "https://opencode.ai/inference/go/v1" : baseUrl);
  });

  it.each(["https://opencode.ai.example/zen/go/v1", "https://mirror.example/zen/go/v1", "https://opencode.ai:8443/zen/go/v1", "https://opencode.ai/zen/v1"])("does not reroute a different origin or non-Go path: %s", async (baseUrl) => {
    const connection = provider({ api: "google-generative-ai", baseUrl });
    const runtime = await load(connection);
    expect((await findModel(runtime, connection, "named"))?.baseUrl).toBe(baseUrl);
  });

  it("sends the SDK request to inference for the user's legacy mixed-API configuration", async () => {
    const fetch = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = input instanceof Request ? input.url : input.toString();
      // Reproduce the real service's distinction: native Google on /zen/go is HTML 404,
      // while /inference/go is the Google API. Never accept the old prefix in this fixture.
      if (new URL(url).pathname !== "/inference/go/v1/models/gemini-4-argon-contributor:streamGenerateContent") {
        return new Response("<!DOCTYPE html><title>Not Found | opencode</title>", { status: 404 });
      }
      return new Response(`data: ${JSON.stringify({ candidates: [{ content: { role: "model", parts: [{ text: "Correct route" }] }, finishReason: "STOP" }] })}\n\n`,
        { headers: { "Content-Type": "text/event-stream" } });
    });
    vi.stubGlobal("fetch", fetch);
    const connection = provider({ baseUrl: "https://opencode.ai/zen/go/v1", models: [model("glm", "GLM"),
      { ...model("gemini-4-argon-contributor", "Gemini"), api: "google-generative-ai" }] });
    const runtime = await load(connection);
    const gemini = (await findModel(runtime, connection, "gemini-4-argon-contributor"))!;
    const result = await runtime.completeSimple(gemini, { messages: [{ role: "user", content: "Check routing", timestamp: Date.now() }] },
      { sessionId: "legacy-go-test", maxRetries: 0 });
    expect(result.errorMessage).toBeUndefined();
    expect(result.content).toContainEqual(expect.objectContaining({ type: "text", text: "Correct route" }));
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(fetch.mock.calls[0][0].toString()).toBe("https://opencode.ai/inference/go/v1/models/gemini-4-argon-contributor:streamGenerateContent?alt=sse");
    const headers = new Headers(fetch.mock.calls[0][1]?.headers);
    expect(headers.get("x-opencode-session")).toBe("legacy-go-test");
    expect(headers.get("user-agent")).toBe("WackCode");
    expect((await findModel(runtime, connection, "glm"))?.baseUrl).toBe(connection.baseUrl);
  });

  it("repairs a pasted Google model endpoint without duplicating the SDK's models path", async () => {
    const connection = provider({ api: "google-generative-ai",
      baseUrl: "https://opencode.ai/inference/go/v1/models/gemini-4-argon-contributor",
      models: [model("gemini-4-argon-contributor", "Gemini"), { ...model("glm", "GLM"), api: "openai-completions" }] });
    const runtime = await load(connection);
    expect(runtime.getError()).toBeUndefined();
    expect((await findModel(runtime, connection, "gemini-4-argon-contributor"))?.baseUrl).toBe("https://opencode.ai/inference/go/v1");
    expect((await findModel(runtime, connection, "glm"))?.baseUrl).toBe("https://opencode.ai/inference/go/v1");
  });

  it.each([
    ["https://example.test/v1beta/models/gemini:streamGenerateContent?alt=sse", "https://example.test/v1beta"],
    ["https://example.test/v1/models/gemini#docs", "https://example.test/v1"],
    ["https://example.test/catalog/models/gemini", "https://example.test/catalog/models/gemini"],
    ["https://example.test/v1/models/gemini:unrelated", "https://example.test/v1/models/gemini:unrelated"]
  ])("canonicalizes only a versioned Google endpoint: %s", async (baseUrl, expected) => {
    const connection = provider({ api: "google-generative-ai", baseUrl });
    const runtime = await load(connection);
    expect(runtime.getError()).toBeUndefined();
    expect((await findModel(runtime, connection, "named"))?.baseUrl).toBe(expected);
  });

  it.each(["google-generative-ai", "openai-completions"] as const)("refuses queries before the Google SDK can corrupt the request path on %s", async (api) => {
    const connection = provider({ api, baseUrl: "https://example.test/v1?key=not-a-real-key",
      models: [{ ...model("gemini", "Gemini"), api: "google-generative-ai" }] });
    await expect(load(connection)).rejects.toThrow("Google base URLs cannot include query parameters");
  });

  it("streams native Gemini text and tool calls through the configured gateway path", async () => {
    const requests: { url?: string; key?: string; body: Record<string, unknown> }[] = [];
    const server = createServer(async (request, response) => {
      const chunks: Buffer[] = [];
      for await (const chunk of request) chunks.push(Buffer.from(chunk));
      requests.push({ url: request.url, key: request.headers["x-goog-api-key"] as string,
        body: JSON.parse(Buffer.concat(chunks).toString()) });
      response.writeHead(200, { "Content-Type": "text/event-stream" });
      response.end(`data: ${JSON.stringify({ candidates: [{ content: { role: "model", parts: [
        { text: "Hello from Gemini" }, { functionCall: { name: "read", args: { path: "README.md" } } }
      ] }, finishReason: "STOP" }], usageMetadata: { promptTokenCount: 10, candidatesTokenCount: 5, totalTokenCount: 15 } })}\n\n`);
    });
    servers.push(server);
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const port = (server.address() as { port: number }).port;
    const connection = provider({ baseUrl: `http://127.0.0.1:${port}/inference/go/v1`,
      models: [{ ...model("gemini-4-argon-contributor", "Gemini"), api: "google-generative-ai", vision: true }] });
    const runtime = await load(connection);
    const gemini = (await findModel(runtime, connection, "gemini-4-argon-contributor"))!;
    const messages = [
      { role: "system" as const, content: "You are a coding assistant.", timestamp: Date.now(), toolsAdded: [
        { name: "read", description: "Read a file", parameters: { type: "object", properties: { path: { type: "string" } }, required: ["path"] } as never }
      ] },
      { role: "user" as const, content: [{ type: "text" as const, text: "Inspect the README" }, { type: "image" as const, mimeType: "image/png", data: "aGVsbG8=" }], timestamp: Date.now() }
    ];
    const result = await runtime.completeSimple(gemini, { messages }, { sessionId: "gemini-test" });
    expect(result.errorMessage).toBeUndefined();
    expect(result.content).toEqual(expect.arrayContaining([
      expect.objectContaining({ type: "text", text: "Hello from Gemini" }),
      expect.objectContaining({ type: "toolCall", name: "read", arguments: { path: "README.md" } })
    ]));
    expect(result.usage).toMatchObject({ input: 10, output: 5 });
    expect(requests).toHaveLength(1);
    expect(requests[0].url).toBe("/inference/go/v1/models/gemini-4-argon-contributor:streamGenerateContent?alt=sse");
    expect(requests[0].key).toBe("test-key");
    expect(requests[0].body).toMatchObject({ contents: [{ parts: [
      { text: "Inspect the README" }, { inlineData: { mimeType: "image/png", data: "aGVsbG8=" } }
    ] }], tools: [{ functionDeclarations: [{ name: "read" }] }] });
    const call = result.content.find((part) => part.type === "toolCall")!;
    const continuation = await runtime.completeSimple(gemini, { messages: [...messages, result,
      { role: "toolResult", toolCallId: call.id, toolName: "read", content: [{ type: "text", text: "# Project" }], isError: false, timestamp: Date.now() }
    ] }, { sessionId: "gemini-test" });
    expect(continuation.errorMessage).toBeUndefined();
    expect(requests[1].body.contents).toEqual(expect.arrayContaining([
      expect.objectContaining({ parts: expect.arrayContaining([
        expect.objectContaining({ functionResponse: expect.objectContaining({ name: "read" }) })
      ]) })
    ]));
  });

  it("names the connection and Pi's reason when Pi rejects it", async () => {
    const connection = provider({ baseUrl: "" });
    const runtime = await load(connection);
    expect(await findModel(runtime, connection, "named")).toBeUndefined();
    const message = missingModelMessage(runtime, connection);
    expect(message).toMatch(/^OpenCode Go couldn't be loaded \(.*baseUrl.*\)\. Fix it in Settings › Providers, or pick another model\.$/);
    expect(message).not.toContain(connection.id);
    expect(message).not.toContain("File:");
  });
});
