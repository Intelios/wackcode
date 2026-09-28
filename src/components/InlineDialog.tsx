import { useCallback, useEffect } from "react";
import type { ComputerAccessDecision, ComputerAccessRequest, ExtensionUIRequest, QuestionAnswer } from "../types";
import { ComputerAccessCard } from "./ComputerAccessCard";
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
  /** Pending computer-use access cards (may span multiple tasks). Shown before extension dialogs. */
  accessRequests?: ComputerAccessRequest[];
  /** The task currently selected in the sidebar. */
  selectedTaskId: string;
  agentName?: string;
  /** Called when the user answers or cancels the dialog. */
  onRespond: (request: ExtensionUIRequest, response: ExtensionUIResponse) => void;
  onAccess?: (request: ComputerAccessRequest, decision: ComputerAccessDecision) => void;
}

/**
 * Renders the first pending dialog for the selected task as an inline card above the composer:
 * a computer-use access card first, then extension dialogs. If none matches the selected task,
 * renders nothing.
 *
 * Handles the Escape key to cancel (an access card: deny) — individual cards don't manage
 * global key listeners.
 */
export function InlineDialog({ requests, accessRequests = [], selectedTaskId, agentName = "WackCode", onRespond, onAccess }: InlineDialogProps) {
  const access = onAccess ? accessRequests.find((entry) => entry.taskId === selectedTaskId) : undefined;
  const request = access ? undefined : requests.find((entry) => entry.taskId === selectedTaskId);

  const handleRespond = useCallback(
    (response: ExtensionUIResponse) => {
      if (request) onRespond(request, response);
    },
    [request, onRespond]
  );

  const handleAccess = useCallback(
    (decision: ComputerAccessDecision) => {
      if (access) onAccess?.(access, decision);
    },
    [access, onAccess]
  );

  useEffect(() => {
    if (!request && !access) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      if (access) handleAccess("deny");
      else handleRespond({ cancelled: true });
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [request, access, handleRespond, handleAccess]);

  if (access) {
    return (
      <div className="inline-dialog-wrap">
        <ComputerAccessCard key={access.requestId} request={access} agentName={agentName} onDecide={handleAccess} />
      </div>
    );
  }
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
