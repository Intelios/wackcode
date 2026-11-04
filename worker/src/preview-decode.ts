/**
 * A small persistent pool of `preview-worker` threads. Saved history holds dozens of
 * screenshots; Pi's `resizeImage` spawns a fresh worker per image, so cold opens spent
 * ~140 ms each on decode plus spawn. The pool runs a few decoders in parallel and reuses
 * them, dropping the spawn entirely. Workers are unref'd: a pool never keeps its process
 * alive, and the reader's stdin-close/parent-watch exits still apply. If worker threads
 * cannot start (sandboxed runtime), everything falls back to serialized in-process decode.
 */
import { existsSync } from "node:fs";
import { availableParallelism } from "node:os";
import { fileURLToPath } from "node:url";
import { Worker } from "node:worker_threads";

type Pi = typeof import("@earendil-works/pi-coding-agent");
export type PreviewDecodeOptions = Parameters<Pi["resizeImage"]>[2];
export type PreviewDecodeResult = Awaited<ReturnType<Pi["resizeImage"]>>;

interface QueuedJob {
  bytes: Uint8Array;
  mimeType: string;
  options?: PreviewDecodeOptions;
  resolve: (result: PreviewDecodeResult) => void;
}

interface Slot {
  worker: Worker;
  seq: number;
}

const WORKER_LIMIT = 4;

export class PreviewPool {
  private readonly size: number;
  private readonly queue: QueuedJob[] = [];
  private readonly slots = new Set<Slot>();
  private readonly pending = new Map<number, { slot: Slot; job: QueuedJob }>();
  private nextSeq = 0;
  private spawning = 0;
  private spawnFailures = 0;
  private degraded = false;
  private inlineQueue = Promise.resolve();

  constructor(private readonly piBase: string, size?: number) {
    const cores = Math.max(1, availableParallelism() - 2);
    this.size = size ?? Math.min(WORKER_LIMIT, cores);
  }

  /** How many decodes may be outstanding before submissions should wait. */
  get concurrency(): number {
    return this.degraded ? 1 : this.size;
  }

  submit(bytes: Uint8Array, mimeType: string, options?: PreviewDecodeOptions): Promise<PreviewDecodeResult> {
    if (this.degraded) return this.submitInline(bytes, mimeType, options);
    return new Promise((resolve) => {
      this.queue.push({ bytes, mimeType, options, resolve });
      this.pump();
    });
  }

  /**
   * Terminate every decoder. Workers are unref'd, but a thread that has touched the WASM
   * decoder still keeps its parent's event loop alive — so callers that own the process's
   * lifetime (the session reader) must close once the last job has resolved.
   */
  close(): void {
    this.degraded = true;
    for (const [seq, inflight] of this.pending) {
      this.pending.delete(seq);
      inflight.job.resolve(null);
    }
    for (const slot of this.slots) void slot.worker.terminate().catch(() => undefined);
    this.slots.clear();
    this.pump();
  }

  private workerUrl(): URL {
    // Tests run this file from src/ while the worker script is only ever tsc'd into dist/.
    const beside = new URL("preview-worker.js", import.meta.url);
    return existsSync(fileURLToPath(beside)) ? beside : new URL("../dist/preview-worker.js", import.meta.url);
  }

  private pump() {
    while (this.queue.length) {
      if (this.degraded) {
        const job = this.queue.shift()!;
        void this.submitInline(job.bytes, job.mimeType, job.options).then(job.resolve);
        continue;
      }
      const free = [...this.slots].find((slot) => slot.seq === 0);
      if (free) this.assign(free);
      else if (this.slots.size + this.spawning < this.size) this.spawn();
      else return;
    }
  }

  private spawn() {
    this.spawning += 1;
    let worker: Worker;
    try {
      worker = new Worker(this.workerUrl(), { workerData: { piBase: this.piBase } });
    } catch {
      this.spawning -= 1;
      this.failSpawn();
      return;
    }
    worker.unref();
    const slot: Slot = { worker, seq: 0 };
    worker.once("online", () => {
      this.spawning -= 1;
      this.slots.add(slot);
      this.pump();
    });
    worker.once("error", () => {
      this.spawning = Math.max(0, this.spawning - 1);
      this.drop(slot);
      this.failSpawn();
    });
    // A decode that kills its thread cannot be told apart from OOM; the job reports
    // failed rather than retrying on siblings and taking the pool down with it.
    worker.once("exit", () => this.drop(slot));
    worker.on("message", (message: { seq: number; result: PreviewDecodeResult }) => {
      const inflight = this.pending.get(message.seq);
      if (!inflight) return;
      this.pending.delete(message.seq);
      inflight.slot.seq = 0;
      inflight.job.resolve(message.result);
      this.pump();
    });
  }

  private assign(slot: Slot) {
    const job = this.queue.shift();
    if (!job) return;
    const seq = ++this.nextSeq;
    slot.seq = seq;
    this.pending.set(seq, { slot, job });
    // Transfer detaches the copy, so the queued bytes are released to GC immediately.
    const bytes = new Uint8Array(job.bytes);
    try {
      slot.worker.postMessage({ seq, bytes, mimeType: job.mimeType, options: job.options }, [bytes.buffer]);
    } catch {
      this.pending.delete(seq);
      slot.seq = 0;
      job.resolve(null);
    }
  }

  private drop(slot: Slot) {
    if (this.slots.delete(slot)) {
      for (const [seq, inflight] of this.pending) {
        if (inflight.slot !== slot) continue;
        this.pending.delete(seq);
        inflight.job.resolve(null);
      }
      void slot.worker.terminate().catch(() => undefined);
      this.pump();
    }
  }

  private failSpawn() {
    // Two failed spawns means the platform cannot run decode threads at all.
    if (++this.spawnFailures >= 2) {
      this.degraded = true;
      for (const slot of this.slots) void slot.worker.terminate().catch(() => undefined);
      this.slots.clear();
      this.pump();
    }
  }

  private submitInline(bytes: Uint8Array, mimeType: string, options?: PreviewDecodeOptions): Promise<PreviewDecodeResult> {
    const run = this.inlineQueue.then(async () => {
      const core = await import(new URL("utils/image-resize-core.js", this.piBase).href) as {
        resizeImageInProcess(input: Uint8Array, mime: string, opts?: PreviewDecodeOptions): Promise<PreviewDecodeResult>;
      };
      return core.resizeImageInProcess(bytes, mimeType, options).catch(() => null);
    });
    this.inlineQueue = run.then(() => undefined, () => undefined);
    return run;
  }
}

/** Shared by the cold reader and the live worker's serial preview queue. */
export function createPreviewPool(piBase: string, size?: number): PreviewPool {
  return new PreviewPool(piBase, size);
}
