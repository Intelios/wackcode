import { useEffect, useState } from "react";

export interface ConfirmingAction {
  taskId: string;
  action: string;
}

/**
 * Two-click confirm for sidebar row actions: the first click arms the row ("Delete?"),
 * the second dispatches it. Clicking elsewhere, pressing Escape, or waiting four seconds
 * disarms. The armed button must carry the `task-confirming` class so the click-away
 * check recognises clicks on it as the confirmation rather than as "elsewhere".
 */
export function useConfirmAction() {
  const [confirming, setConfirming] = useState<ConfirmingAction | null>(null);

  useEffect(() => {
    if (!confirming) return;
    function handleOutside(event: Event) {
      const target = event.target as HTMLElement | null;
      if (!target?.closest(".task-confirming")) {
        setConfirming(null);
      }
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setConfirming(null);
      }
    }
    const timer = setTimeout(() => {
      setConfirming(null);
    }, 4000);

    window.addEventListener("pointerdown", handleOutside);
    window.addEventListener("mousedown", handleOutside);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("pointerdown", handleOutside);
      window.removeEventListener("mousedown", handleOutside);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [confirming]);

  /** Arms the row action, or fires it when this exact action is already armed. */
  function confirm(taskId: string, action: string): boolean {
    if (confirming?.taskId === taskId && confirming.action === action) {
      setConfirming(null);
      return true;
    }
    setConfirming({ taskId, action });
    return false;
  }

  return { confirming, confirm, setConfirming };
}
