import { chmod } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { getSupportedThinkingLevels } from "@earendil-works/pi-ai";
import type { AuthEvent, AuthPrompt } from "@earendil-works/pi-ai";
import type { ModelRuntime } from "@earendil-works/pi-coding-agent";

type LoginRuntime = Pick<ModelRuntime, "getProvider" | "login" | "getAvailable">;

/** The private NDJSON bridge for one Pi OAuth login. No credential object leaves this class. */
export class SubscriptionAuthFlow {
  readonly controller = new AbortController();
  private readonly pending = new Map<string, { resolve(value: string): void; reject(error: Error): void }>();

  constructor(private readonly send: (event: object) => void) {}

  readonly prompt = (prompt: AuthPrompt): Promise<string> => {
    const promptId = randomUUID();
    const { signal: promptSignal, ...safePrompt } = prompt;
    return new Promise((resolve, reject) => {
      const cancel = () => {
        this.pending.delete(promptId);
        reject(new Error("Login cancelled"));
      };
      if (this.controller.signal.aborted || promptSignal?.aborted) return cancel();
      this.pending.set(promptId, {
        resolve(value) { promptSignal?.removeEventListener("abort", cancel); resolve(value); },
        reject(error) { promptSignal?.removeEventListener("abort", cancel); reject(error); }
      });
      promptSignal?.addEventListener("abort", cancel, { once: true });
      this.send({ type: "prompt", promptId, prompt: safePrompt });
    });
  };

  readonly notify = (event: AuthEvent): void => {
    if (event.type === "auth_url" || event.type === "device_code" || event.type === "progress" || event.type === "info") {
      this.send(event);
    }
  };

  respond(promptId: string, value?: string, cancelled = false): void {
    const prompt = this.pending.get(promptId);
    if (!prompt) return;
    this.pending.delete(promptId);
    if (cancelled || value === undefined) prompt.reject(new Error("Login cancelled"));
    else prompt.resolve(value);
  }

  cancel(): void {
    this.controller.abort();
    for (const prompt of this.pending.values()) prompt.reject(new Error("Login cancelled"));
    this.pending.clear();
  }

  async run(runtime: LoginRuntime, providerId: string, authPath: string): Promise<void> {
    try {
      const provider = runtime.getProvider(providerId);
      if (!provider?.auth.oauth?.isSubscription) throw new Error("Unsupported provider");
      await runtime.login(providerId, "oauth", { signal: this.controller.signal, prompt: this.prompt, notify: this.notify });
      await chmod(authPath, 0o600);
      const models = await runtime.getAvailable(providerId, { signal: this.controller.signal });
      this.send({
        type: "complete",
        models: models.map((model) => {
          const thinkingLevels = getSupportedThinkingLevels(model);
          return {
            id: model.id,
            name: model.name,
            contextWindow: model.contextWindow,
            maxTokens: model.maxTokens,
            reasoning: model.reasoning,
            thinkingLevels,
            thinkingLevelMap: Object.fromEntries(thinkingLevels.map((level) => [level, model.thinkingLevelMap?.[level] ?? (level === "off" ? null : level)])),
            vision: model.input.includes("image")
          };
        })
      });
    } catch {
      this.send(this.controller.signal.aborted
        ? { type: "cancelled" }
        : { type: "error", message: "Subscription sign-in failed. Check the browser or device code, then try again." });
    } finally {
      this.cancel();
    }
  }
}
