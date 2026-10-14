import type { ModelRuntime } from "./model-runtime.js";

/** Custom connections use UUIDs, not Pi's `opencode-go` provider ID, so they don't get
 * its routing headers. Apply the documented per-conversation header on the configured
 * OpenCode host only, for every API and every runtime (parent, child and auxiliary calls).
 * The session ID comes from Pi per request, never from the connection or an app-global ID.
 */
export function instrumentOpenCodeHeaders(runtime: ModelRuntime): void {
  for (const key of ["stream", "streamSimple", "streamDeferred"] as const) {
    const original = runtime[key].bind(runtime);
    const wrapped = (...args: Parameters<typeof original>) => {
      const [model, , options] = args;
      let openCode = false;
      try { openCode = new URL(model.baseUrl).hostname === "opencode.ai"; } catch { /* Invalid URLs fail in Pi. */ }
      if (openCode) {
        const headers = { ...options?.headers };
        if (!Object.keys(headers).some((name) => name.toLowerCase() === "user-agent")) {
          headers["User-Agent"] = "WackCode";
        }
        const sessionId = options && "sessionId" in options ? options.sessionId : undefined;
        if (typeof sessionId === "string" && sessionId && !Object.keys(headers).some((name) => name.toLowerCase() === "x-opencode-session")) {
          headers["x-opencode-session"] = sessionId;
        }
        args[2] = { ...options, headers };
      }
      return (original as (...values: typeof args) => ReturnType<typeof original>)(...args);
    };
    Object.defineProperty(runtime, key, { value: wrapped, configurable: true });
  }
}
