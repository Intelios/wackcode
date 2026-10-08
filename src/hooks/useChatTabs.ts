import { useCallback, useRef, useState } from "react";
import { EMPTY_CHAT_TABS, type ChatTabsState } from "../chat-tabs";

/** Synchronous ref backs navigation and async completions between React commits. */
export function useChatTabs() {
  const [state, setState] = useState(EMPTY_CHAT_TABS);
  const ref = useRef(state);
  const update = useCallback((change: (current: ChatTabsState) => ChatTabsState) => {
    const next = change(ref.current);
    ref.current = next;
    setState(next);
    return next;
  }, []);
  return { state, ref, update };
}
