import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createLocalBashOperations, type BashOperations, type ToolDefinition } from "@earendil-works/pi-coding-agent";
import { BASH_CHECKIN_SECONDS, BASH_JOB_TOOL_NAME, createBashJobsExtension } from "./builtin/bash-jobs.js";
import { readOnlyDecision } from "./builtin/subagents/guard.js";

const cleanup: Array<() => Promise<unknown>> = [];
afterEach(async () => {
  for (const close of cleanup.splice(0).reverse()) await close();
  vi.useRealTimers();
});

function harness(operations?: BashOperations, cwd = tmpdir()) {
  const { factory, controller } = createBashJobsExtension(operations);
  cleanup.push(controller.stopAll);
  const tools = new Map<string, ToolDefinition>();
  const handlers = new Map<string, () => Promise<void>>();
  factory({
    registerTool: (tool: ToolDefinition) => tools.set(tool.name, tool),
    on: (name: string, handler: () => Promise<void>) => handlers.set(name, handler),
  } as never);
  const context = { cwd, sessionManager: { getSessionId: () => "fixture", getSessionFile: () => undefined } } as never;
  let calls = 0;
  const call = (tool: ToolDefinition, params: unknown, signal?: AbortSignal, onUpdate?: (result: unknown) => void) =>
    tool.execute(`call-${++calls}`, params, signal, onUpdate, context);
  return {
    controller,
    bash: (params: unknown, signal?: AbortSignal, onUpdate?: (result: unknown) => void) => call(controller.tool(cwd), params, signal, onUpdate),
    manage: (params: unknown, signal?: AbortSignal, onUpdate?: (result: unknown) => void) => call(tools.get(BASH_JOB_TOOL_NAME)!, params, signal, onUpdate),
    fire: (name: string) => handlers.get(name)!(),
  };
}

function backend() {
  const commands: Array<{
    options: Parameters<BashOperations["exec"]>[2];
    finish: (code?: number) => void;
    fail: (message: string) => void;
  }> = [];
  const operations: BashOperations = { exec: vi.fn((_command, _cwd, options) => new Promise((resolve, reject) => {
    const aborted = () => { reject(new Error("aborted")); };
    options.signal?.addEventListener("abort", aborted, { once: true });
    if (options.signal?.aborted) aborted();
    commands.push({
      options,
      finish: (exitCode = 0) => { options.signal?.removeEventListener("abort", aborted); resolve({ exitCode }); },
      fail: (message) => { options.signal?.removeEventListener("abort", aborted); reject(new Error(message)); },
    });
  })) };
  return { operations, commands };
}

function jobId(result: any): string {
  expect(result.details?.shellJob?.status).toBe("running");
  expect(result.structuredContent.status).toBe("running");
  expect(result.structuredContent.exit_code).toBeNull();
  return result.details.shellJob.id;
}
const text = (result: any) => result.content.map((part: { text: string }) => part.text).join("\n");

async function yielded(h: ReturnType<typeof harness>, signal?: AbortSignal, update?: (result: unknown) => void) {
  const call = h.bash({ command: "fixture", yieldTimeout: 0 }, signal, update);
  await vi.advanceTimersByTimeAsync(0);
  return jobId(await call);
}

describe("bounded shell check-ins", () => {
  it.each([undefined, 999_999])("returns after 60 seconds without killing a command (yieldTimeout=%s)", async (yieldTimeout) => {
    vi.useFakeTimers();
    const { operations, commands } = backend();
    const h = harness(operations);
    let returned = false;
    const running = h.bash({ command: "fixture", timeout: 999_999, yieldTimeout }).then((result) => { returned = true; return result; });
    await vi.advanceTimersByTimeAsync(BASH_CHECKIN_SECONDS * 1000 - 1);
    expect(returned).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    const id = jobId(await running);
    expect(text(await running)).toContain("No output yet (60 seconds)");
    expect(commands[0].options.timeout).toBe(999_999);
    expect(commands[0].options.signal?.aborted).toBe(false);
    const waiting = h.manage({ action: "wait", jobId: id, waitSeconds: 999_999 });
    await vi.advanceTimersByTimeAsync(BASH_CHECKIN_SECONDS * 1000);
    expect(jobId(await waiting)).toBe(id);
    expect(commands).toHaveLength(1);
    expect(commands[0].options.signal?.aborted).toBe(false);
  });

  it("lets a build longer than 30 seconds finish before the first check-in", async () => {
    vi.useFakeTimers();
    const { operations, commands } = backend();
    const h = harness(operations);
    let returned = false;
    const build = h.bash({ command: "build fixture" }).then((result) => { returned = true; return result; });
    await vi.advanceTimersByTimeAsync(45_000);
    expect(returned).toBe(false);
    expect(commands[0].options.signal?.aborted).toBe(false);
    commands[0].finish();
    const result = await build;
    expect(result.details).toBeUndefined();
    expect(result.structuredContent).toMatchObject({ status: "completed", exit_code: 0, wall_time_seconds: 45 });
  });

  it("keeps normal fast successes and non-zero exits authoritative", async () => {
    const h = harness(createLocalBashOperations());
    const good = await h.bash({ command: "printf 'done'" });
    expect(text(good)).toBe("done");
    expect(good.structuredContent).toMatchObject({ exit_code: 0, status: "completed", output: "done" });
    expect(good.details).toBeUndefined();
    const bad = await h.bash({ command: "printf 'failure'; exit 7" });
    expect(text(bad)).toContain("Command exited with code 7");
    expect(bad.isError).toBe(true);
    expect(bad.structuredContent).toMatchObject({ exit_code: 7, status: "failed" });
  });

  it("reports new raw output even when repeated text looks identical, without late tool updates", async () => {
    vi.useFakeTimers();
    const { operations, commands } = backend();
    const h = harness(operations);
    const update = vi.fn();
    const id = await yielded(h, undefined, update);
    const count = update.mock.calls.length;
    commands[0].options.onData(Buffer.from("same\n"));
    await vi.advanceTimersByTimeAsync(100);
    expect(update).toHaveBeenCalledTimes(count);
    const first = await h.manage({ action: "status", jobId: id });
    expect(first.details).toMatchObject({ shellJob: { outputBytes: 5, newOutputBytes: 5 } });
    commands[0].options.onData(Buffer.from("same\n"));
    await vi.advanceTimersByTimeAsync(100);
    const second = await h.manage({ action: "status", jobId: id });
    expect(second.details).toMatchObject({ shellJob: { outputBytes: 10, newOutputBytes: 5 } });
    await vi.advanceTimersByTimeAsync(5000);
    const quiet = await h.manage({ action: "status", jobId: id });
    expect(quiet.details).toMatchObject({ shellJob: { quietSeconds: 5, newOutputBytes: 0 } });
    expect(text(quiet)).toContain("No new output");
    expect(update).toHaveBeenCalledTimes(count);
    const waitingUpdate = vi.fn();
    const waiting = h.manage({ action: "wait", jobId: id }, undefined, waitingUpdate);
    commands[0].options.onData(Buffer.from("last\n"));
    commands[0].finish();
    const final = await waiting;
    expect(text(final)).toContain("same\nsame\nlast\n");
    expect(final.details).toMatchObject({ shellJob: { status: "completed" } });
    expect(final.structuredContent).toMatchObject({ status: "completed", exit_code: 0 });
    expect(waitingUpdate).toHaveBeenCalled();
  });

  it("preserves bounded UTF-8 output and the full log across yields", async () => {
    vi.useFakeTimers();
    const { operations, commands } = backend();
    const h = harness(operations);
    const id = await yielded(h);
    const output = `${"hello 🦆\n".repeat(8000)}end\n`;
    const bytes = Buffer.from(output);
    commands[0].options.onData(bytes.subarray(0, 9)); // Split inside a UTF-8 code point.
    commands[0].options.onData(bytes.subarray(9));
    commands[0].finish();
    const final = await h.manage({ action: "wait", jobId: id });
    expect(text(final)).not.toContain("�");
    expect(text(final).length).toBeLessThan(52_000);
    expect(final.details).toMatchObject({ truncation: { truncated: true } });
    const path = (final.details as { fullOutputPath: string }).fullOutputPath;
    cleanup.push(() => rm(path, { force: true }));
    expect(await readFile(path, "utf8")).toBe(output);
    expect(final.structuredContent).toMatchObject({ full_output_path: path });
  });

  it("keeps explicit hard timeouts separate from a periodic check-in", async () => {
    const h = harness();
    const initial = await h.bash({ command: "printf 'before timeout'; sleep 60", yieldTimeout: 0.03, timeout: 0.15 });
    const id = jobId(initial);
    const final = await h.manage({ action: "wait", jobId: id, waitSeconds: 1 });
    expect(text(final)).toContain("before timeout");
    expect(text(final)).toContain("Command timed out after 0.15 seconds");
    expect(final.details).toMatchObject({ shellJob: { status: "failed" } });
    expect(final.isError).toBe(true);
  });

  it("honors the originating abort signal after the bash call has yielded", async () => {
    vi.useFakeTimers();
    const { operations, commands } = backend();
    const h = harness(operations);
    const origin = new AbortController();
    const id = await yielded(h, origin.signal);
    origin.abort();
    const final = await h.manage({ action: "wait", jobId: id });
    expect(commands[0].options.signal?.aborted).toBe(true);
    expect(final.details).toMatchObject({ shellJob: { status: "stopped" } });
    expect(final.isError).toBe(true);
  });

  it("cancels a waiting job promptly", async () => {
    vi.useFakeTimers();
    const { operations } = backend();
    const h = harness(operations);
    const id = await yielded(h);
    const abort = new AbortController();
    const waiting = h.manage({ action: "wait", jobId: id }, abort.signal);
    abort.abort();
    const result = await waiting;
    expect(result.details).toMatchObject({ shellJob: { status: "stopped" } });
  });

  it("stops the process group, not just the shell, and allows repeated stop/status", async () => {
    const root = await mkdtemp(join(tmpdir(), "wackcode-shell-job-"));
    cleanup.push(() => rm(root, { recursive: true, force: true }));
    const h = harness(undefined, root);
    const initial = await h.bash({ command: "printf 'started'; (sleep 0.3; touch escaped.txt) & wait", yieldTimeout: 0.03 });
    const id = jobId(initial);
    expect(text(initial)).toContain("started");
    const stopped = await h.manage({ action: "stop", jobId: id, waitSeconds: 0 });
    expect(stopped.details).toMatchObject({ shellJob: { status: "stopped" } });
    expect(stopped.isError).toBe(false);
    expect((await h.manage({ action: "stop", jobId: id })).details).toMatchObject({ shellJob: { status: "stopped" } });
    await new Promise((resolve) => setTimeout(resolve, 400));
    await expect(readFile(join(root, "escaped.txt"))).rejects.toMatchObject({ code: "ENOENT" });
  });

  it.each(["agent_settled", "session_shutdown"])("cleans up live jobs at %s, even with no active wait", async (event) => {
    vi.useFakeTimers();
    const { operations, commands } = backend();
    const h = harness(operations);
    const ids = [await yielded(h), await yielded(h)];
    await h.fire(event);
    await h.controller.stopAll(); // Idempotent shutdown.
    expect(commands.every((command) => command.options.signal?.aborted)).toBe(true);
    for (const id of ids) expect((await h.manage({ action: "status", jobId: id })).details).toMatchObject({ shellJob: { status: "stopped" } });
  });

  it("never starts an aborted command, rejects invalid waits and caps live jobs", async () => {
    vi.useFakeTimers();
    const { operations, commands } = backend();
    const h = harness(operations);
    const aborted = new AbortController();
    aborted.abort();
    await expect(h.bash({ command: "fixture" }, aborted.signal)).rejects.toThrow("aborted");
    for (const yieldTimeout of [-1, NaN, Infinity, "30"]) {
      await expect(h.bash({ command: "fixture", yieldTimeout })).rejects.toThrow("finite, non-negative");
    }
    expect(commands).toHaveLength(0);
    const ids = [];
    for (let index = 0; index < 8; index++) ids.push(await yielded(h));
    await expect(h.bash({ command: "fixture" })).rejects.toThrow("Already running 8");
    await h.manage({ action: "stop", jobId: ids[0] });
    await yielded(h);
    expect(commands).toHaveLength(9);
  });

  it("bounds finished-job retention without evicting a live command", async () => {
    vi.useFakeTimers();
    const { operations, commands } = backend();
    const h = harness(operations);
    const live = await yielded(h);
    const finished: string[] = [];
    for (let index = 0; index < 35; index++) {
      const id = await yielded(h);
      commands.at(-1)!.finish();
      await h.manage({ action: "wait", jobId: id });
      finished.push(id);
    }
    expect(jobId(await h.manage({ action: "status", jobId: live }))).toBe(live);
    await expect(h.manage({ action: "status", jobId: finished[0] })).rejects.toThrow("no longer available");
    expect((await h.manage({ action: "status", jobId: finished.at(-1) })).details).toMatchObject({ shellJob: { status: "completed" } });
    expect(commands[0].options.signal?.aborted).toBe(false);
  });

  it("isolates sessions and only gives read-only children management of their admitted jobs", async () => {
    vi.useFakeTimers();
    const first = harness(backend().operations);
    const second = harness(backend().operations);
    const id = await yielded(first);
    await expect(second.manage({ action: "stop", jobId: id })).rejects.toThrow("no longer available");
    await expect(first.manage({ action: "wait", jobId: "unknown" })).rejects.toThrow("do not assume");
    expect(readOnlyDecision("bash_job", { action: "stop", jobId: id }, "/project")).toBeUndefined();
    expect(readOnlyDecision("bash", { command: "rm important.txt" }, "/project")).toMatchObject({ block: true });
  });
});
