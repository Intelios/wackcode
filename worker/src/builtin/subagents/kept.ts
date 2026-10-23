/**
 * Retained finished children for Team chats: the sessions a `job` continuation re-prompts
 * instead of starting over. A record answers to every jobId it has been launched under —
 * continuing re-keys it to the new call's jobId, with the older id kept as an alias so a stale
 * reference still finds the worker. At most `max` stay alive; the least recently used session
 * that is idle is evicted (and disposed) first. Active rounds are pinned until they end.
 * Everything is in-memory: nothing survives the worker.
 */
export interface KeptChild<Session> {
  session: Session;
  /** Extra data the runner needs for a later round (spec, model, connection, cleanup). */
  data: unknown;
  /** jobIds that resolve to this record; the latest launch's id is `jobId`. */
  aliases: Set<string>;
  jobId: string;
  rounds: number;
  busy: boolean;
  touched: number;
}

export class KeptSessions<Session> {
  private readonly kept = new Map<string, KeptChild<Session>>();
  private tick = 0;

  constructor(private readonly max: number, private readonly dispose: (session: Session) => void) {}

  get(jobId: string): KeptChild<Session> | undefined {
    return this.kept.get(jobId);
  }

  keys(): string[] {
    return [...this.kept.keys()];
  }

  records(): KeptChild<Session>[] {
    return [...new Set(this.kept.values())];
  }

  /** Keep a finished child under `jobId`, evicting the stalest first. */
  put(jobId: string, child: Omit<KeptChild<Session>, "aliases" | "jobId" | "rounds" | "busy" | "touched">): KeptChild<Session> {
    const record: KeptChild<Session> = { ...child, aliases: new Set([jobId]), jobId, rounds: 1, busy: false, touched: ++this.tick };
    this.kept.set(jobId, record);
    this.evict();
    return record;
  }

  /** A new round begins: marks the record busy and re-keys it to the new call's jobId. */
  begin(record: KeptChild<Session>, jobId: string | undefined): void {
    record.busy = true;
    record.rounds += 1;
    record.touched = ++this.tick;
    if (jobId && jobId !== record.jobId) {
      this.kept.set(jobId, record);
      record.aliases.add(jobId);
      record.jobId = jobId;
    }
  }

  end(record: KeptChild<Session>): void {
    record.busy = false;
    record.touched = ++this.tick;
    this.evict();
  }

  /** Dispose and forget one record (all its aliases). */
  drop(record: KeptChild<Session>): void {
    for (const alias of record.aliases) this.kept.delete(alias);
    this.kept.delete(record.jobId);
    this.dispose(record.session);
  }

  /** Dispose and forget everything. */
  clear(): void {
    for (const record of this.records()) this.drop(record);
  }

  private evict(): void {
    const records = this.records();
    while (records.length > this.max) {
      const idle = records.filter((record) => !record.busy);
      if (!idle.length) break;
      const oldest = idle.reduce((a, b) => (a.touched <= b.touched ? a : b));
      this.drop(oldest);
      records.splice(records.indexOf(oldest), 1);
    }
  }
}
