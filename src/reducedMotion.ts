// Settings › Developer's "Force reduced motion". A renderer-only flag (this app's own storage,
// so it can't leak into the installed app) applied the same way `applyCachedTheme` is: before
// React mounts. It works through the existing channels rather than a parallel one —
// `useReducedMotion` reads `matchMedia("(prefers-reduced-motion: reduce)")`, so forcing that
// query's answer flips every motion component at once; CSS can't be forced through a media
// query, so a `data-force-reduced-motion` attribute on <html> stands in for it (styles.css
// mirrors the global reduced-motion rule under it).

export const FORCE_REDUCED_MOTION_KEY = "wackcode:forceReducedMotion";

const REDUCE_QUERY = "(prefers-reduced-motion: reduce)";
const changeListeners = new Set<(event: MediaQueryListEvent) => void>();

export function forceReducedMotionOn(): boolean {
  try {
    return localStorage.getItem(FORCE_REDUCED_MOTION_KEY) === "1";
  } catch {
    return false;
  }
}

/** The Developer-section toggle: persists, updates the CSS attribute and wakes the hooks. */
export function setForceReducedMotion(on: boolean) {
  try {
    if (on) {
      localStorage.setItem(FORCE_REDUCED_MOTION_KEY, "1");
    } else {
      localStorage.removeItem(FORCE_REDUCED_MOTION_KEY);
    }
  } catch {}
  if (on) {
    document.documentElement.dataset.forceReducedMotion = "true";
  } else {
    delete document.documentElement.dataset.forceReducedMotion;
  }
  const event = { matches: forceReducedMotionOn(), media: REDUCE_QUERY } as MediaQueryListEvent;
  for (const listener of [...changeListeners]) listener(event);
}

let installed = false;

/** Wraps `matchMedia` so the reduce query answers the flag. Installed once, at module load. */
export function installForceReducedMotion() {
  if (installed) return;
  installed = true;
  const real = window.matchMedia.bind(window);
  const stub = (query: string): MediaQueryList => {
    if (query !== REDUCE_QUERY) return real(query);
    const backing = real(query);
    const list = {
      get matches() {
        return forceReducedMotionOn() || backing.matches;
      },
      media: query,
      onchange: null as ((event: MediaQueryListEvent) => void) | null,
      addListener: (listener: (event: MediaQueryListEvent) => void) => changeListeners.add(listener),
      removeListener: (listener: (event: MediaQueryListEvent) => void) => changeListeners.delete(listener),
      addEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => changeListeners.add(listener),
      removeEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => changeListeners.delete(listener),
      dispatchEvent: () => true,
    };
    return list as MediaQueryList;
  };
  window.matchMedia = stub as typeof window.matchMedia;
  if (forceReducedMotionOn()) {
    document.documentElement.dataset.forceReducedMotion = "true";
  }
}
