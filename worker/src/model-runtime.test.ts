import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
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
afterEach(async () => { await Promise.all(dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true }))); });
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
