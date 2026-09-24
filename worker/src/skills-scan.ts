/**
 * Settings › Skills: a short-lived process the host starts to list skill folders. It reads one
 * `{ request, agentDir }` line on stdin, runs Pi's own skill loader over the folders and package
 * resources named there (so Settings shows exactly what a chat loads), prints one
 * `SkillScanResult` line and exits.
 *
 * It only reads files: it never starts a Pi session, holds a provider key, or touches the network.
 */
import { JsonLineDecoder } from "./framing.js";
import type { SkillScanRequest, SkillScanResult } from "./protocol.js";

process.env.PI_TELEMETRY = "0";
process.env.PI_SKIP_VERSION_CHECK = "1";
process.env.PI_OFFLINE = "1";

function finish(result: SkillScanResult): never {
  process.stdout.write(`${JSON.stringify(result)}\n`);
  process.exit(0);
}

async function scan(request: SkillScanRequest, agentDir: string): Promise<never> {
  try {
    const pi = await import("@earendil-works/pi-coding-agent");
    const { scanSkills } = await import("./user-skills.js");
    finish({ ok: true, ...scanSkills(pi, request, agentDir) });
  } catch (error) {
    finish({ ok: false, error: error instanceof Error ? error.message : String(error) });
  }
}

const decoder = new JsonLineDecoder();
let started = false;
process.stdin.on("data", (chunk: Buffer) => {
  const [line] = decoder.push(chunk);
  if (line === undefined || started) return;
  started = true;
  process.stdin.pause();
  let parsed: { request: SkillScanRequest; agentDir: string };
  try {
    parsed = JSON.parse(line) as { request: SkillScanRequest; agentDir: string };
  } catch {
    finish({ ok: false, error: "The desktop app sent an invalid request." });
  }
  void scan(parsed.request, parsed.agentDir);
});
process.stdin.on("end", () => {
  if (!started) finish({ ok: false, error: "The desktop app sent no folders to scan." });
});
