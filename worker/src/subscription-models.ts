import type { CredentialStore } from "@earendil-works/pi-ai";
import { SUBSCRIPTION_PROVIDER_IDS, subscriptionModels } from "./subscription-auth-flow.js";

process.env.PI_TELEMETRY = "0";
process.env.PI_SKIP_VERSION_CHECK = "1";
process.env.PI_OFFLINE = "1";

/**
 * Re-lists each signed-in subscription's models, so models a Pi update adds reach the picker
 * without a fresh sign-in (`refresh_subscription_models` in `subscriptions.rs`). Offline and
 * read-only: it reads the auth files Rust names, never refreshes a token, and prints only model
 * metadata. A provider that fails is left out, and Rust keeps its last known list.
 */
type Input = { providers: { providerId: string; authPath: string }[] };

type Pi = typeof import("@earendil-works/pi-coding-agent");

/**
 * Pi's file store locks auth.json even to read it, and waits up to 30 s on a lock a killed
 * process left behind. This one reads without a lock and cannot write, so the re-list never
 * stalls behind a chat's token refresh and never leaves a lock of its own. A file it can't read
 * (missing, or caught mid-write) throws rather than listing no models.
 */
function readOnlyCredentials(pi: Pi, providerId: string, authPath: string): CredentialStore {
  const credential = pi.readStoredCredential(providerId, authPath);
  if (credential?.type !== "oauth") throw new Error("Not signed in");
  const readOnly = async (): Promise<never> => { throw new Error("Read-only credentials"); };
  return {
    read: async (id) => id === providerId ? credential : undefined,
    list: async () => [{ providerId, type: credential.type }],
    modify: readOnly,
    delete: readOnly
  };
}

const chunks: Buffer[] = [];
for await (const chunk of process.stdin) chunks.push(chunk as Buffer);
const input = JSON.parse(Buffer.concat(chunks).toString("utf8")) as Input;
const pi = await import("@earendil-works/pi-coding-agent");
const output: Record<string, Awaited<ReturnType<typeof subscriptionModels>>> = {};
for (const { providerId, authPath } of input.providers) {
  if (!SUBSCRIPTION_PROVIDER_IDS.has(providerId)) continue;
  try {
    const credentials = readOnlyCredentials(pi, providerId, authPath);
    const runtime = await pi.ModelRuntime.create({ credentials, modelsPath: null, allowModelNetwork: false, refreshOnCreate: false });
    output[providerId] = await subscriptionModels(runtime, providerId);
  } catch {
    // Keep the last known list for this provider.
  }
}
process.stdout.write(JSON.stringify(output));
