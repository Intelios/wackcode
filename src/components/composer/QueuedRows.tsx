/**
 * Shared by the Code composer and Chat mode's composer: the queued-message list and the
 * single-flight queue actions behind it. Queue/run events are authoritative (docs/frontend.md
 * › Composer drafts): nothing here removes a row optimistically, and actions lock per draft key.
 */
import { useRef, useState, type ReactNode } from "react";
import { imageDataUrl, splitFileSection, type FileAttachment } from "../../attachment-utils";
import type { BrowserContextAttachment, BrowserContextRecord, ImageContent, QueuedMessage, RestoredPrompt, TaskStatus } from "../../types";
import { contextLabel, restoreContexts } from "../../browser-context";
import { Icon } from "../Icons";
import { Tooltip } from "../ui/Tooltip";
import { BrowserContextChip } from "../browser/BrowserContextChip";

interface QueueActionOptions {
  draftKey: string;
  /** The key on screen right now; late results only touch notices/focus when it still matches. */
  activeDraftKey: { current: string };
  status: TaskStatus;
  /** Disabled, frozen or otherwise unable to act, before the lock is considered. */
  blocked: boolean;
  onSteer?: (messageId: string) => Promise<boolean>;
  onDequeue?: () => Promise<(RestoredPrompt | string)[] | undefined>;
  onError: (message: string) => void;
  /** Restored texts (file sections split back out) for the originating draft. */
  onRestore: (text: string, files: FileAttachment[], images: ImageContent[], contexts: BrowserContextAttachment[]) => void;
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
      const parts = texts.map((item) => {
        const prompt = typeof item === "string" ? { text: item, images: [] } : item;
        const split = splitFileSection(prompt.text);
        return { ...split, ...restoreContexts(split.browserContexts ?? [], prompt.images ?? []) };
      });
      onRestore(parts.map((part) => part.text).filter(Boolean).join("\n\n"), parts.flatMap((part) => part.files),
        parts.flatMap((part) => part.images), parts.flatMap((part) => part.browserContexts));
    });
  }

  return { pending, steer, restore };
}

/** Show the words, not generated file sections. A message made only of files shows their names
 *  instead, so its row is never blank. Never reconstruct a steered payload from them. */
type QueuedDisplayMessage = QueuedMessage & { browserContexts?: BrowserContextRecord[] };
export function queuedDisplay(queued: QueuedMessage[] | undefined): QueuedDisplayMessage[] {
  return (queued ?? []).map((entry) => {
    const { text, files, browserContexts = [] } = splitFileSection(entry.text);
    return { ...entry, text: text || [...files.map((file) => file.name), ...browserContexts.map(contextLabel)].join(", "), ...(browserContexts.length ? { browserContexts } : {}) };
  });
}

interface QueuedRowsProps {
  entries: QueuedDisplayMessage[];
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
        <div className={`queued-message${entry.browserContexts?.length ? " queued-browser-message" : ""}`} role="listitem" key={entry.id}>
          {tag ?? <span className="queued-tag">Queued</span>}
          <div className="queued-content"><span className="queued-text">{entry.text}</span>
            {entry.browserContexts?.length ? <div className="message-browser-contexts">{entry.browserContexts.map((context) => <BrowserContextChip key={context.id} context={context}
              preview={entry.images?.[context.imageIndex] ? imageDataUrl(entry.images[context.imageIndex]) : undefined} />)}</div> : null}
          </div>
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
