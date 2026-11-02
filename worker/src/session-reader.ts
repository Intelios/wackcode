/** One-shot, keyless history reader. Its only input is host-owned session metadata on stdin. */
import { streamSavedSession, type SavedSessionOptions } from "./saved-session.js";
import type { SavedHistoryFrame } from "./protocol.js";
process.env.PI_TELEMETRY = "0";
process.env.PI_SKIP_VERSION_CHECK = "1";
process.env.PI_OFFLINE = "1";
// A closed host pipe is terminal. Never try to report its error through that same pipe;
// exiting also terminates an image decoder thread that has not returned yet.
process.stdout.on("error", () => process.exit(0));
process.stderr.on("error", () => process.exit(1));
process.stdin.on("error", () => process.exit(1));
process.on("SIGTERM", () => process.exit(0));
process.on("SIGINT", () => process.exit(0));
process.on("uncaughtException", () => process.exit(1));
process.on("unhandledRejection", () => process.exit(1));
const parentPid = process.ppid;
const parentWatch = setInterval(() => { if (process.ppid !== parentPid) process.exit(0); }, 1000);
parentWatch.unref();
const writeFrame = (frame: SavedHistoryFrame) => new Promise<void>((resolve, reject) => {
  process.stdout.write(`${JSON.stringify(frame)}\n`, (error) => error ? reject(error) : resolve());
});
try {
  let input = "";
  for await (const chunk of process.stdin) input += chunk;
  await streamSavedSession(JSON.parse(input) as SavedSessionOptions, writeFrame);
} catch (error) {
  // No session contents (including malformed JSON lines) belong in an error frame.
  process.stderr.write(error instanceof Error && !(error instanceof SyntaxError) ? error.message : "Could not read this chat's saved session.");
  process.exitCode = 1;
}
