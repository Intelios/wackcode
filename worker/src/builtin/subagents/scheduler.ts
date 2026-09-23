/**
 * Runs one call's tasks with at most `limit` in flight. Tasks that can edit the workspace share
 * a single writer slot, so two children never write at once; read-only tasks queued behind a
 * waiting writer still start as soon as a slot is free. Results keep the input order.
 *
 * `run` is expected to settle every task itself (a failed child is a result, not an error);
 * a rejection is still collected and rethrown once everything else has finished, so no child
 * is left running unobserved.
 */
export async function runScheduled<T, R>(
  items: readonly T[],
  limit: number,
  isWriter: (item: T) => boolean,
  run: (item: T, index: number) => Promise<R>,
): Promise<R[]> {
  const slots = Math.max(1, Math.floor(limit) || 1);
  const results = new Array<R>(items.length);
  if (items.length === 0) return results;
  return new Promise<R[]>((resolve, reject) => {
    const pending = items.map((_, index) => index);
    let running = 0;
    let writerBusy = false;
    let finished = 0;
    let failure: { error: unknown } | undefined;

    const pump = () => {
      if (finished === items.length) {
        if (failure) reject(failure.error);
        else resolve(results);
        return;
      }
      for (let position = 0; position < pending.length && running < slots; ) {
        const index = pending[position];
        const writer = isWriter(items[index]);
        if (writer && writerBusy) {
          position += 1;
          continue;
        }
        pending.splice(position, 1);
        running += 1;
        if (writer) writerBusy = true;
        run(items[index], index)
          .then(
            (value) => { results[index] = value; },
            (error: unknown) => { failure ??= { error }; },
          )
          .finally(() => {
            running -= 1;
            if (writer) writerBusy = false;
            finished += 1;
            pump();
          });
      }
    };
    pump();
  });
}
