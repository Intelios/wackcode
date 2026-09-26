/**
 * Runs one call's tasks with at most `limit` in flight. All tasks, including editors,
 * take slots in input order. The caller assigns independent work in the shared workspace;
 * the scheduler does not lock files. Results keep the input order.
 *
 * `run` is expected to settle every task itself (a failed child is a result, not an error);
 * a rejection is still collected and rethrown once everything else has finished, so no child
 * is left running unobserved.
 */
export async function runScheduled<T, R>(
  items: readonly T[],
  limit: number,
  run: (item: T, index: number) => Promise<R>,
): Promise<R[]> {
  const slots = Math.max(1, Math.floor(limit) || 1);
  const results = new Array<R>(items.length);
  if (items.length === 0) return results;
  return new Promise<R[]>((resolve, reject) => {
    const pending = items.map((_, index) => index);
    let running = 0;
    let finished = 0;
    let failure: { error: unknown } | undefined;

    const pump = () => {
      if (finished === items.length) {
        if (failure) reject(failure.error);
        else resolve(results);
        return;
      }
      while (pending.length > 0 && running < slots) {
        const index = pending.shift()!;
        running += 1;
        run(items[index], index)
          .then(
            (value) => { results[index] = value; },
            (error: unknown) => { failure ??= { error }; },
          )
          .finally(() => {
            running -= 1;
            finished += 1;
            pump();
          });
      }
    };
    pump();
  });
}
