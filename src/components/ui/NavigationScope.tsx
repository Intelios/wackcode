import { createContext, useContext, useLayoutEffect, useRef } from "react";

/** Navigation retires transient UI even when Git keeps the chat mounted underneath.
 * Disclosure choices and drafts live outside this scope and remain intact. */
export const NavigationScope = createContext("");

export function useNavigationDismiss(dismiss: () => void) {
  const scope = useContext(NavigationScope);
  const previous = useRef(scope);
  useLayoutEffect(() => {
    if (previous.current === scope) return;
    previous.current = scope;
    dismiss();
  }, [scope, dismiss]);
}
