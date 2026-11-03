import { useLayoutEffect, useState, useRef } from "react";

export type CallbackProps<T> = {
  [K in keyof T as NonNullable<T[K]> extends (...args: never[]) => unknown ? K : never]-?: NonNullable<T[K]>
};

/** Stable event props for memoized surfaces. Each call uses the latest committed handler;
 * async work already started keeps the originating handler's chat/draft closure. Keys must
 * be fixed for the lifetime of the hook. Never use these wrappers during rendering. */
export function useStableCallbacks<T extends Record<string, (...args: never[]) => unknown>>(callbacks: T): T {
  const latest = useRef(callbacks);
  useLayoutEffect(() => { latest.current = callbacks; });
  const [stable] = useState(() => Object.fromEntries(Object.keys(callbacks).map((key) => [
    key, (...args: never[]) => latest.current[key](...args)
  ])) as T);
  return stable;
}
