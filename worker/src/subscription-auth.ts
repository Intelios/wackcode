import { mkdir } from "node:fs/promises";
import { dirname } from "node:path";
import { JsonLineDecoder } from "./framing.js";
import { SubscriptionAuthFlow } from "./subscription-auth-flow.js";

process.env.PI_TELEMETRY = "0";
process.env.PI_SKIP_VERSION_CHECK = "1";
process.env.PI_OFFLINE = "1";

type Input =
  | { type: "start"; providerId: string; authPath: string }
  | { type: "response"; promptId: string; value?: string; cancelled?: boolean }
  | { type: "cancel" };

const allowed = new Set(["openai-codex", "github-copilot", "anthropic", "xai", "meta", "kimi-coding"]);
const flow = new SubscriptionAuthFlow((event) => process.stdout.write(`${JSON.stringify(event)}\n`));
let started = false;

async function start(providerId: string, authPath: string): Promise<void> {
  if (started || !allowed.has(providerId)) {
    process.stdout.write(`${JSON.stringify({ type: "error", message: "This subscription provider is unavailable." })}\n`);
    return;
  }
  started = true;
  try {
    await mkdir(dirname(authPath), { recursive: true, mode: 0o700 });
    const pi = await import("@earendil-works/pi-coding-agent");
    const runtime = await pi.ModelRuntime.create({ authPath, modelsPath: null, allowModelNetwork: false, refreshOnCreate: false });
    await flow.run(runtime, providerId, authPath);
  } catch {
    process.stdout.write(`${JSON.stringify({ type: "error", message: "Subscription sign-in failed. Check the browser or device code, then try again." })}\n`);
  } finally {
    process.stdin.destroy();
  }
}

const decoder = new JsonLineDecoder();
process.stdin.on("data", (chunk: Buffer) => {
  for (const line of decoder.push(chunk)) {
    let input: Input;
    try { input = JSON.parse(line) as Input; } catch { continue; }
    if (input.type === "cancel") flow.cancel();
    else if (input.type === "response") flow.respond(input.promptId, input.value, input.cancelled);
    else if (input.type === "start") void start(input.providerId, input.authPath);
  }
});
process.stdin.on("end", () => flow.cancel());
process.on("SIGTERM", () => flow.cancel());
process.stdin.resume();
