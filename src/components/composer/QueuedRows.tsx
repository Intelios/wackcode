/**
 * Shared by the Code composer and Chat mode's composer: the queued-message list and the
 * single-flight queue actions behind it. Queue/run events are authoritative (docs/frontend.md
 * › Composer drafts): nothing here removes a row optimistically, and actions lock per draft key.
 */
import { useRef, useState, type ReactNode } from "react";
import { splitFileSection, type FileAttachment } from "../../attachment-utils";
import type { QueuedMessage, TaskStatus } from "../../types";
import { Icon } from "../Icons";
import { Tooltip } from "../ui/Tooltip";

interface QueueActionOptions {
  draftKey: string;
  /** The key on screen right now; late results only touch notices/focus when it still matches. */
  activeDraftKey: { current: string };
  status: TaskStatus;
  /** Disabled, frozen or otherwise unable to act, before the lock is considered. */
  blocked: boolean;
  onSteer?: (messageId: string) => Promise<boolean>;
  onDequeue?: () => Promise<string[] | undefined>;
  onError: (message: string) => void;
  /** Restored texts (file sections split back out) for the originating draft. */
  onRestore: (text: string, files: FileAttachment[]) => void;
}

export function useQueueActions({ draftKey, activeDraftKey, status, blocked, onSteer, onDequeue, onError, onRestore }: QueueActionOptions) {
  // The ref guards same-tick repeats; state disables controls without changing worker-owned rows.
  const locks = useRef(new Set<string>());
  const [pendingKeys, setPendingKeys] = useState<ReadonlySet<string>>(() => new Set());
  const pending = pendingKeys.has(draftKey);

  async function change(action: () => Promise<void>) {
    if (blocked || status === "stopping" || locks.current.has(draftKey)) return;
    locks.current.add(draftKey);
    setPendingKeys(new Set(locks.current));
    try {
      await action();
    } catch (reason) {
      // App normally reports failures through its runtime and resolves false/undefined.
      // A rejected callback still leaves the queue and draft intact and releases the controls.
      if (activeDraftKey.current === draftKey) onError(String(reason));
    } finally {
      locks.current.delete(draftKey);
      setPendingKeys(new Set(locks.current));
    }
  }

  async function steer(messageId: string) {
    if (!onSteer || status !== "running") return;
    await change(async () => { await onSteer(messageId); });
  }

  /** Restore all queued texts to their originating draft, keeping newer typing and files.
   * Queue-state events remove the rows; the generated file sections return to the tray. */
  async function restore() {
    if (!onDequeue) return;
    await change(async () => {
      const texts = await onDequeue();
      if (!texts || texts.length === 0) return;
      const parts = texts.map((text) => splitFileSection(text));
      onRestore(parts.map((part) => part.text).filter(Boolean).join("\n\n"), parts.flatMap((part) => part.files));
    });
  }

  return { pending, steer, restore };
}

/** Show the words, not generated file sections. Never reconstruct a steered payload from them. */
export function queuedDisplay(queued: QueuedMessage[] | undefined): QueuedMessage[] {
  return (queued ?? []).map((entry) => ({ ...entry, text: splitFileSection(entry.text).text }));
}

interface QueuedRowsProps {
  entries: QueuedMessage[];
  agentName: string;
  disabled: boolean;
  canSteer: boolean;
  canRestore: boolean;
  onSteer: (id: string) => void;
  onRestore: () => void;
  className?: string;
  /** Replaces the default "Queued" tag, e.g. a little avatar in Chat mode. */
  tag?: ReactNode;
}

export function QueuedRows({ entries, agentName, disabled, canSteer, canRestore, onSteer, onRestore, className = "composer-queue", tag }: QueuedRowsProps) {
  if (entries.length === 0) return null;
  return (
    <div className={className} role="list" aria-label="Queued messages">
      {entries.map((entry) => (
        <div className="queued-message" role="listitem" key={entry.id}>
          {tag ?? <span className="queued-tag">Queued</span>}
          <span className="queued-text">{entry.text}</span>
          <Tooltip label={`Interrupt ${agentName} and send this next. Keep other messages queued.`}>
            <button type="button" className="secondary-button compact queued-steer" disabled={disabled || !canSteer} onClick={() => onSteer(entry.id)}>
              Steer
            </button>
          </Tooltip>
          <Tooltip label="Restore all queued messages to the composer">
            <button type="button" className="queued-remove" aria-label="Restore queued messages to the composer" disabled={disabled || !canRestore} onClick={onRestore}>
              <Icon name="close" />
            </button>
          </Tooltip>
        </div>
      ))}
    </div>
  );
}
