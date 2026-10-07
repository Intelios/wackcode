/**
 * One scheduler per chat, shared by foreground and background launches. Admission is atomic:
 * no batch partially starts before its eight-unfinished-child budget is checked. Every task
 * owns its signal; queued cancellation also settles without ever creating a child session.
 */
export class SubagentJobs {
  private limit = 4;
  private running = 0;
  private readonly pending: Array<{ controller: AbortController; run: () => Promise<void>; finish: () => void }> = [];
  private readonly live = new Set<{ controller: AbortController; done: Promise<void> }>();

  configure(limit: number): void {
    this.limit = Math.min(8, Math.max(1, Math.floor(limit) || 1));
    this.pump();
  }

  get size(): number { return this.live.size; }

  admit(count: number): void {
    if (this.size + count > 8) throw new Error("At most 8 sub-agents can be unfinished in this chat. Wait for or stop existing jobs before starting more.");
  }

  enqueue(controller: AbortController, run: () => Promise<void>): Promise<void> {
    let finish!: () => void;
    const done = new Promise<void>((resolve) => { finish = resolve; });
    const job = { controller, done };
    const cancel = () => this.pump();
    controller.signal.addEventListener("abort", cancel, { once: true });
    this.live.add(job);
    this.pending.push({ controller, run, finish: () => {
      controller.signal.removeEventListener("abort", cancel);
      this.live.delete(job);
      finish();
    } });
    this.pump();
    return done;
  }

  async stopAll(): Promise<void> {
    const jobs = [...this.live];
    for (const job of jobs) job.controller.abort();
    this.pump();
    await Promise.all(jobs.map((job) => job.done));
  }

  private pump(): void {
    // Cancelled queued jobs consume no slot, including when the running limit is full.
    for (let i = this.pending.length - 1; i >= 0; i--) {
      if (!this.pending[i].controller.signal.aborted) continue;
      const [job] = this.pending.splice(i, 1);
      void Promise.resolve().then(job.run).finally(job.finish);
    }
    while (this.running < this.limit && this.pending.length) {
      const job = this.pending.shift()!;
      this.running += 1;
      void Promise.resolve().then(job.run).finally(() => {
        this.running -= 1;
        job.finish();
        this.pump();
      });
    }
  }
}
