import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

// Exercise actual OS pipes: a stream mock cannot reproduce the asynchronous EPIPE
// that used to feed worker_error back into uncaughtException forever.
const children: ChildProcessWithoutNullStreams[] = [];
afterEach(() => {
  for (const child of children.splice(0)) {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
  }
});

function worker(preload?: string) {
  const child = spawn(process.env.WACKCODE_TEST_NODE ?? process.execPath, [
    ...(preload ? ["--import", `data:text/javascript,${encodeURIComponent(preload)}`] : []),
    process.env.WACKCODE_TEST_WORKER ?? resolve("dist/index.js"),
  ], { stdio: "pipe", env: { ...process.env, PI_OFFLINE: "1", PI_TELEMETRY: "0", PI_SKIP_VERSION_CHECK: "1" } });
  children.push(child);
  child.stderr.resume();
  return child;
}

async function exits(child: ChildProcessWithoutNullStreams) {
  return new Promise<{ code: number | null; signal: string | null }>((resolveExit, reject) => {
    const timer = setTimeout(() => reject(new Error("Worker stayed alive after losing its host")), 5_000);
    child.once("exit", (code, signal) => { clearTimeout(timer); resolveExit({ code, signal }); });
    child.once("error", (error) => { clearTimeout(timer); reject(error); });
  });
}

describe("worker process lifecycle", () => {
  it("exits when the host closes stdout, without recursively reporting EPIPE", async () => {
    const child = worker();
    const exited = exits(child);
    child.stdout.destroy();
    // An invalid command produces worker_error without initializing a model.
    child.stdin.write("invalid JSON\n");
    expect(await exited).toEqual({ code: 1, signal: null });
  });

  it("exits on stdin EOF even when an extension keeps the event loop alive", async () => {
    const child = worker("setInterval(() => {}, 1000);");
    child.stdout.resume();
    const exited = exits(child);
    child.stdin.end();
    expect(await exited).toEqual({ code: 0, signal: null });
  });

  it("accepts shutdown before initialization", async () => {
    const child = worker("setInterval(() => {}, 1000);");
    let output = "";
    child.stdout.on("data", (chunk) => { output += chunk; });
    const exited = exits(child);
    child.stdin.write(`${JSON.stringify({ type: "shutdown", id: "cold-shutdown" })}\n`);
    expect(await exited).toEqual({ code: 0, signal: null });
    expect(output).toContain('"id":"cold-shutdown","success":true');
  });

  it.each(["throw new Error('fatal fixture')", "Promise.reject(new Error('fatal fixture'))"])(
    "exits after a fatal error instead of just setting exitCode: %s", async (failure) => {
      const child = worker(`setInterval(() => {}, 1000); setTimeout(() => { ${failure}; }, 500);`);
      let output = "";
      child.stdout.on("data", (chunk) => { output += chunk; });
      expect(await exits(child)).toEqual({ code: 1, signal: null });
      expect(output).toContain('"type":"worker_error"');
      expect(output).toContain("fatal fixture");
    },
  );
});
