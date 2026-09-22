import type { CheckpointChange, NormalizedMessage, TaskRecord } from "./types";

/**
 * Pure helpers for the transcript's retry / edit / rewind / fork actions. The session tree
 * itself lives in the worker; these only read what a snapshot says about it.
 */

/** Rewinding the first message leaves only Pi's system entry, which the transcript never shows. */
export function hasVisibleMessages(messages: NormalizedMessage[]): boolean {
  return messages.some((message) => message.role === "user" || message.role === "assistant");
}

/** The text of a message, for copying. */
export function messageText(message: NormalizedMessage): string {
  return message.blocks.filter((block) => block.type === "text").map((block) => block.text ?? "").join("\n").trim();
}

/**
 * The latest turn: its user message, and the assistant message that ends it when it has one.
 * Retry is offered here only — retrying an earlier turn is what editing it is for.
 */
export function latestTurn(messages: NormalizedMessage[]): { user: NormalizedMessage; answer?: NormalizedMessage } | undefined {
  const userIndex = messages.map((message) => message.role).lastIndexOf("user");
  if (userIndex < 0) return undefined;
  const user = messages[userIndex];
  if (!user.entryId) return undefined;
  const answer = messages.slice(userIndex + 1).reverse().find((message) => message.turn?.userEntryId === user.entryId);
  return { user, answer };
}

/** The user message a turn-ending assistant message answers. */
export function userOfTurn(messages: NormalizedMessage[], message: NormalizedMessage): NormalizedMessage | undefined {
  const userEntryId = message.turn?.userEntryId;
  return userEntryId ? messages.find((candidate) => candidate.entryId === userEntryId) : undefined;
}

/** What restoring does to a file, in words. */
export function changeLabel(status: CheckpointChange["status"]): string {
  if (status === "delete") return "Deleted";
  if (status === "recreate") return "Restored";
  return "Reverted";
}

/**
 * The folder a chat works in, relative to the root its checkpoints cover — "" when they are the
 * same. A chat in a subfolder of a repository snapshots the whole repository, but only its own
 * files are selected by default when restoring.
 */
export function workspacePrefix(root: string | undefined, workspace: string): string {
  // A worktree's workspace is recorded as `<root>/<relative>`, which ends in "/" when the
  // project is the repository root.
  const base = root?.replace(/\/+$/, "");
  const folder = workspace.replace(/\/+$/, "");
  if (!base || base === folder || !folder.startsWith(`${base}/`)) return "";
  return `${folder.slice(base.length + 1)}/`;
}

export function defaultSelection(changes: CheckpointChange[], prefix: string): string[] {
  const inside = prefix ? changes.filter((change) => change.path.startsWith(prefix)) : changes;
  return inside.map((change) => change.path);
}

/** A chat can be forked once Pi has saved its session, and not while it is working. */
export function canFork(task: TaskRecord): boolean {
  return Boolean(task.sessionFile) && !task.archived && task.status !== "running" && task.status !== "stopping";
}
