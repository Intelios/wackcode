import { useCallback, useEffect } from "react";
import type { ExtensionUIRequest, QuestionAnswer } from "../types";
import { QuestionCard } from "./QuestionDialog";
import { ExtensionCard } from "./ExtensionDialog";

/** Response shape that covers both question answers and simple extension replies. */
export type ExtensionUIResponse = {
  value?: string;
  confirmed?: boolean;
  cancelled?: true;
  answers?: QuestionAnswer[];
  /** "Write the plan now" on an Ultra Plan questionnaire. */
  wrapUp?: true;
};

interface InlineDialogProps {
  /** All pending extension UI requests (FIFO queue, may span multiple tasks). */
  requests: ExtensionUIRequest[];
  /** The task currently selected in the sidebar. */
  selectedTaskId: string;
  /** Called when the user answers or cancels the dialog. */
  onRespond: (request: ExtensionUIRequest, response: ExtensionUIResponse) => void;
}

/**
 * Renders the first pending extension dialog for the selected task as an inline card
 * above the composer. If no request matches the selected task, renders nothing.
 *
 * Handles the Escape key to cancel — individual cards don't manage global key listeners.
 */
export function InlineDialog({ requests, selectedTaskId, onRespond }: InlineDialogProps) {
  const request = requests.find((entry) => entry.taskId === selectedTaskId);

  const handleRespond = useCallback(
    (response: ExtensionUIResponse) => {
      if (request) onRespond(request, response);
    },
    [request, onRespond]
  );

  useEffect(() => {
    if (!request) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") handleRespond({ cancelled: true });
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [request, handleRespond]);

  if (!request) return null;

  return (
    <div className="inline-dialog-wrap">
      {request.method === "questions" ? (
        <QuestionCard key={request.requestId} request={request} onRespond={handleRespond} />
      ) : (
        <ExtensionCard key={request.requestId} request={request} onRespond={handleRespond} />
      )}
    </div>
  );
}
