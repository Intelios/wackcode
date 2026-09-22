import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AuthInteraction } from "@earendil-works/pi-ai";
import type { ModelRuntime } from "@earendil-works/pi-coding-agent";
import { SubscriptionAuthFlow } from "./subscription-auth-flow.js";

const directories: string[] = [];
afterEach(async () => { await Promise.all(directories.splice(0).map((path) => rm(path, { recursive: true, force: true }))); });

async function authPath() {
  const directory = await mkdtemp(join(tmpdir(), "wackcode-login-flow-"));
  directories.push(directory);
  return join(directory, "auth.json");
}

const model = {
  id: "fake-model", name: "Fake model", provider: "openai-codex", api: "openai-codex-responses",
  baseUrl: "https://example.test", reasoning: true, input: ["text"], contextWindow: 8_000,
  maxTokens: 2_000, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }
};

describe("subscription login bridge", () => {
  it("relays a device code and manual prompt, then saves only private credentials", async () => {
    const path = await authPath();
    const events: Record<string, unknown>[] = [];
    const flow = new SubscriptionAuthFlow((event) => events.push(event as Record<string, unknown>));
    const runtime = {
      getProvider: () => ({ auth: { oauth: { isSubscription: true } } }),
      login: async (_id: string, _type: string, interaction: AuthInteraction) => {
        interaction.notify({ type: "device_code", userCode: "ABCD-EFGH", verificationUri: "https://example.test/device" });
        const code = await interaction.prompt({ type: "manual_code", message: "Paste the code" });
        await writeFile(path, JSON.stringify({ "openai-codex": { type: "oauth", access: `access-${code}`, refresh: "refresh-secret", expires: Date.now() + 60_000 } }), { mode: 0o644 });
      },
      getAvailable: async () => [model]
    } as unknown as Pick<ModelRuntime, "getProvider" | "login" | "getAvailable">;

    const running = flow.run(runtime, "openai-codex", path);
    await vi.waitFor(() => expect(events.some((event) => event.type === "prompt")).toBe(true));
    const prompt = events.find((event) => event.type === "prompt")!;
    flow.respond(prompt.promptId as string, "private-code");
    await running;

    expect(events.map((event) => event.type)).toEqual(["device_code", "prompt", "complete"]);
    expect(JSON.stringify(events)).not.toContain("private-code");
    expect(JSON.stringify(events)).not.toContain("refresh-secret");
    expect(JSON.parse(await readFile(path, "utf8"))["openai-codex"].access).toBe("access-private-code");
    expect((await stat(path)).mode & 0o777).toBe(0o600);
  });

  it("cancels a pending prompt without sending an authorization code", async () => {
    const path = await authPath();
    const events: Record<string, unknown>[] = [];
    const flow = new SubscriptionAuthFlow((event) => events.push(event as Record<string, unknown>));
    const runtime = {
      getProvider: () => ({ auth: { oauth: { isSubscription: true } } }),
      login: async (_id: string, _type: string, interaction: AuthInteraction) => {
        await interaction.prompt({ type: "manual_code", message: "Paste the code" });
      },
      getAvailable: async () => [model]
    } as unknown as Pick<ModelRuntime, "getProvider" | "login" | "getAvailable">;

    const running = flow.run(runtime, "openai-codex", path);
    await vi.waitFor(() => expect(events.some((event) => event.type === "prompt")).toBe(true));
    flow.cancel();
    await running;
    expect(events.map((event) => event.type)).toEqual(["prompt", "cancelled"]);
  });
});
