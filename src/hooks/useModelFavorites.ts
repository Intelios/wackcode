import { useCallback, useRef, useState } from "react";
import { api } from "../api";
import type { ModelRef } from "../types";

/** App owns this queue, shared by every picker. A failed save must not block the next one. */
export function useModelFavorites(onSaved: (models: ModelRef[]) => void, onError: (message: string) => void) {
  const queue = useRef<Promise<void>>(Promise.resolve());
  const pending = useRef(0);
  const [favoriteSaving, setFavoriteSaving] = useState(false);

  const setModelFavorite = useCallback((reference: ModelRef, favorite: boolean): Promise<void> => {
    pending.current += 1;
    setFavoriteSaving(true);
    const request = queue.current.then(async () => {
      const saved = await api.setModelFavorite(reference, favorite);
      onSaved(saved);
    }).catch((reason: unknown) => {
      onError(String(reason));
      throw reason;
    }).finally(() => {
      pending.current -= 1;
      if (pending.current === 0) setFavoriteSaving(false);
    });
    queue.current = request.catch(() => undefined);
    return request;
  }, [onSaved, onError]);

  return { favoriteSaving, setModelFavorite };
}
