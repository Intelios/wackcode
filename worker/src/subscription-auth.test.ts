import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { ModelRuntime } from "@earendil-works/pi-coding-agent";
import type { Model, Provider } from "@earendil-works/pi-ai";

const directories: string[] = [];
afterEach(async () => { await Promise.all(directories.splice(0).map((path) => rm(path, { recursive: true, force: true }))); });

async function runtime(providerId: string, credential: Record<string, unknown>) {
  const directory = await mkdtemp(join(tmpdir(), "wackcode-subscription-"));
  directories.push(directory);
  const authPath = join(directory, "auth.json");
  await writeFile(authPath, JSON.stringify({ [providerId]: credential }), { mode: 0o600 });
  return ModelRuntime.create({ authPath, modelsPath: null, allowModelNetwork: false, refreshOnCreate: false });
}

describe("subscription model runtime", () => {
  it("Pi marks all six exposed OAuth providers as subscription backed", async () => {
    const directory = await mkdtemp(join(tmpdir(), "wackcode-subscription-providers-"));
    directories.push(directory);
    const current = await ModelRuntime.create({ authPath: join(directory, "auth.json"), modelsPath: null, allowModelNetwork: false, refreshOnCreate: false });
    for (const id of ["openai-codex", "github-copilot", "anthropic", "xai", "meta", "kimi-coding"]) {
      expect(current.getProvider(id)?.auth.oauth?.isSubscription, id).toBe(true);
    }
  });

  it("uses Pi's native Codex model and a stored OAuth credential without network discovery", async () => {
    const first = await runtime("openai-codex", { type: "oauth", access: "first-token", refresh: "refresh-token", expires: Date.now() + 3_600_000 });
    const model = (await first.getAvailable("openai-codex"))[0];
    expect(model?.provider).toBe("openai-codex");
    expect(model?.api).toBe("openai-codex-responses");
    expect((await first.getAuth(model!))?.auth.apiKey).toBe("first-token");

    const second = await runtime("openai-codex", { type: "oauth", access: "second-token", refresh: "refresh-token", expires: Date.now() + 3_600_000 });
    expect((await second.getAuth(second.getModel("openai-codex", model!.id)!))?.auth.apiKey).toBe("second-token");
  });

  it("honours Copilot's account model allowlist", async () => {
    const baseline = await runtime("github-copilot", { type: "oauth", access: "token", refresh: "refresh", expires: Date.now() + 3_600_000 });
    const model = baseline.getModels("github-copilot")[0];
    expect(model).toBeDefined();
    const filtered = await runtime("github-copilot", {
      type: "oauth", access: "token", refresh: "refresh", expires: Date.now() + 3_600_000,
      availableModelIds: [model.id]
    });
    expect((await filtered.getAvailable("github-copilot")).map((item) => item.id)).toEqual([model.id]);
  });

  it("refreshes an expired credential and loads the refreshed token in a new runtime", async () => {
    const directory = await mkdtemp(join(tmpdir(), "wackcode-subscription-refresh-"));
    directories.push(directory);
    const authPath = join(directory, "auth.json");
    const providerId = "mock-subscription";
    await writeFile(authPath, JSON.stringify({ [providerId]: {
      type: "oauth", access: "expired-token", refresh: "refresh-token", expires: Date.now() - 1_000
    } }), { mode: 0o600 });
    let refreshes = 0;
    const model: Model<"openai-completions"> = {
      id: "mock-model", name: "Mock model", provider: providerId, api: "openai-completions",
      baseUrl: "https://example.test", reasoning: false, input: ["text"],
      contextWindow: 8_000, maxTokens: 2_000, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }
    };
    const provider: Provider<"openai-completions"> = {
      id: providerId, name: "Mock subscription", getModels: () => [model],
      auth: { oauth: {
        name: "Mock subscription", isSubscription: true,
        login: async () => { throw new Error("Live login is not used in this test"); },
        refresh: async () => {
          refreshes++;
          return { type: "oauth", access: "refreshed-token", refresh: "new-refresh-token", expires: Date.now() + 3_600_000 };
        },
        toAuth: async (credential) => ({ apiKey: credential.access })
      } },
      stream: () => { throw new Error("No inference in this test"); },
      streamSimple: () => { throw new Error("No inference in this test"); }
    };
    async function createRuntime() {
      const runtime = await ModelRuntime.create({ authPath, modelsPath: null, allowModelNetwork: false, refreshOnCreate: false });
      runtime.registerNativeProvider(provider);
      return runtime;
    }
    const first = await createRuntime();
    expect((await first.getAuth(model))?.auth.apiKey).toBe("refreshed-token");
    expect(refreshes).toBe(1);
    expect(JSON.parse(await readFile(authPath, "utf8"))[providerId].access).toBe("refreshed-token");
    const second = await createRuntime();
    expect((await second.getAuth(model))?.auth.apiKey).toBe("refreshed-token");
    expect(refreshes).toBe(1);
  });
});
