/**
 * One persistent image decoder for the preview pool. Pi's own `image-resize-worker.js`
 * answers a single message then exits; WackCode keeps this one alive so hundreds of saved
 * screenshots do not pay a worker spawn apiece. Messages are answered one at a time,
 * echoing the job's sequence number so the pool can match replies.
 */
import { parentPort, workerData } from "node:worker_threads";

type Pi = typeof import("@earendil-works/pi-coding-agent");

interface Job {
  seq: number;
  bytes: Uint8Array;
  mimeType: string;
  options?: Parameters<Pi["resizeImage"]>[2];
}

const port = parentPort;
if (!port) throw new Error("preview-worker requires parentPort");

const core = import(new URL("utils/image-resize-core.js", String(workerData?.piBase)).href) as Promise<{
  resizeImageInProcess(input: Uint8Array, mimeType: string, options?: Job["options"]): Promise<unknown>;
}>;

let queue = Promise.resolve();
port.on("message", (job: Job) => {
  queue = queue.then(async () => {
    try {
      const { resizeImageInProcess } = await core;
      port!.postMessage({ seq: job.seq, result: await resizeImageInProcess(job.bytes, job.mimeType, job.options) });
    } catch {
      port!.postMessage({ seq: job.seq, result: null });
    }
  });
});
