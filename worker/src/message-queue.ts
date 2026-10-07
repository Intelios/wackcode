import type { CheckpointRef, CommandPresentation, ImageContent, QueuedMessage } from "./protocol.js";

/**
 * Desktop sends wait outside Pi's run, not in its boundary-only steering queue. Keeping the
 * complete payload here lets Steer promote exactly one id before aborting, without clearing
 * or reconstructing the other messages (and without losing images or literal slash text).
 * Stop pauses delivery, never discards input. Only taking a message or restoring the queue
 * removes it; duplicate text is deliberately allowed and never used as identity.
 */
export interface PendingMessage extends QueuedMessage {
  images?: ImageContent[];
  literal?: boolean;
  presentation?: CommandPresentation;
  runId?: string;
  startedAt?: number;
  checkpoint?: CheckpointRef | null;
}

export class MessageQueue {
  private messages: PendingMessage[] = [];
  private paused = false;

  get hasPending(): boolean { return this.messages.length > 0; }
  get hasRunnable(): boolean { return !this.paused && this.hasPending; }

  view(): QueuedMessage[] {
    return this.messages.map(({ id, text }) => ({ id, text }));
  }

  enqueue(message: PendingMessage): void {
    this.messages.push(message);
  }

  promote(id: string, run: Pick<PendingMessage, "runId" | "startedAt" | "checkpoint">): boolean {
    const index = this.messages.findIndex((message) => message.id === id);
    if (index < 0) return false;
    const [message] = this.messages.splice(index, 1);
    this.messages.unshift({ ...message, ...run });
    return true;
  }

  take(): PendingMessage | undefined {
    return this.hasRunnable ? this.messages.shift() : undefined;
  }

  clear(): string[] {
    const texts = this.messages.map(({ text }) => text);
    this.messages = [];
    return texts;
  }

  pause(): void { this.paused = true; }
  resume(): void { this.paused = false; }
}
