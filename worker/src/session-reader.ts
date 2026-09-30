/** One-shot, keyless history reader. Its only input is host-owned session metadata on stdin. */
import { readSavedSession, type SavedSessionOptions } from "./saved-session.js";
process.env.PI_TELEMETRY = "0";
process.env.PI_SKIP_VERSION_CHECK = "1";
process.env.PI_OFFLINE = "1";
try {
  let input = "";
  for await (const chunk of process.stdin) input += chunk;
  process.stdout.write(JSON.stringify(await readSavedSession(JSON.parse(input) as SavedSessionOptions)));
} catch (error) {
  // No session contents (including malformed JSON lines) belong in an error frame.
  process.stderr.write(error instanceof Error && !(error instanceof SyntaxError) ? error.message : "Could not read this chat's saved session.");
  process.exitCode = 1;
}
