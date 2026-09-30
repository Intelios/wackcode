import { formatRelativeTime } from "./chat-utils";
import type { GitChangeFile, GitSyncStatus, ProjectRecord, TaskRecord } from "./types";

/**
 * Git mode's pure rules: which project it opens on, which chat its AI actions go through,
 * what the sync button offers, and how the file checkboxes add up. `useGitMode` holds the
 * state; this file only decides.
 */

export type GitTab = "changes" | "history";

/** The project Git mode last showed, so reopening lands there. */
export const GIT_PROJECT_KEY = "wackcode:gitProject";
/** Unified or split diffs, remembered per user. */
export const DIFF_LAYOUT_KEY = "wackcode:diffLayout";
/** Project ids pinned to the top of the sidebar and the repository switcher. */
export const PINNED_PROJECTS_KEY = "wackcode:pinnedProjects";

/** Pinned projects first, each group in its existing order. */
export function orderProjects(projects: ProjectRecord[], pinned: ReadonlySet<string>): ProjectRecord[] {
  return [...projects.filter((project) => pinned.has(project.id)), ...projects.filter((project) => !pinned.has(project.id))];
}

/**
 * The project Git mode opens on: the selected chat's, then the one it showed last, then the
 * first pinned Git project, then the first Git project, then any project.
 */
export function initialGitProject(
  projects: ProjectRecord[],
  selected: TaskRecord | undefined,
  remembered: string | null,
  pinned: ReadonlySet<string>
): string | undefined {
  const exists = (id: string | null | undefined): id is string => Boolean(id) && projects.some((project) => project.id === id);
  if (exists(selected?.projectId)) return selected.projectId;
  if (exists(remembered)) return remembered;
  const ordered = orderProjects(projects, pinned);
  return (ordered.find((project) => project.gitRoot) ?? ordered[0])?.id;
}

/** Chats that work in the project's own folder (not a worktree), most recently active first. */
export function linkableChats(tasks: TaskRecord[], projectId: string): TaskRecord[] {
  return tasks
    .filter((task) => task.projectId === projectId && !task.archived && !task.usesWorktree)
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
}

/**
 * The chat Git mode's AI actions go through: the one the user picked, else the one they came
 * from, else the project's most recent. Undefined when the project has no such chat yet.
 */
export function resolveLinkedChat(tasks: TaskRecord[], projectId: string, picked?: string, cameFrom?: string): TaskRecord | undefined {
  const chats = linkableChats(tasks, projectId);
  return chats.find((task) => task.id === picked) ?? chats.find((task) => task.id === cameFrom) ?? chats[0];
}

/** Chats running in the project's own folder: Rust refuses Git writes until they finish. */
export function busyChatsInCheckout(tasks: TaskRecord[], projectId: string): TaskRecord[] {
  return tasks.filter((task) => task.projectId === projectId && !task.usesWorktree && (task.status === "running" || task.status === "stopping"));
}

/** What the toolbar's sync button does next, the way GitHub Desktop's does. */
export type SyncAction =
  | { kind: "none"; label: string; reason: string }
  | { kind: "publish"; remote: string }
  | { kind: "pull"; remote: string; behind: number; ahead: number }
  | { kind: "push"; remote: string; ahead: number }
  | { kind: "fetch"; remote: string; fetchedAt: string | null };

export function syncAction(sync: GitSyncStatus | undefined): SyncAction {
  if (!sync) return { kind: "none", label: "Checking remote…", reason: "Reading the repository" };
  const remote = sync.fetchRemote;
  if (!remote) return { kind: "none", label: "No remote", reason: "Add a remote with `git remote add` to push and pull" };
  if (!sync.branch) return { kind: "fetch", remote, fetchedAt: sync.fetchedAt };
  if (!sync.upstream) {
    return sync.hasHead
      ? { kind: "publish", remote }
      : { kind: "none", label: "Publish branch", reason: "Make a first commit, then publish it" };
  }
  // With an upstream, Rust's `fetchRemote` is the upstream's own remote.
  if (sync.behind > 0) return { kind: "pull", remote, behind: sync.behind, ahead: sync.ahead };
  if (sync.ahead > 0) return { kind: "push", remote, ahead: sync.ahead };
  return { kind: "fetch", remote, fetchedAt: sync.fetchedAt };
}

/** "Fetched just now" / "Fetched 4m ago" / "Fetched 3 Sep" / "Never fetched". */
export function fetchedLabel(fetchedAt: string | null, now: Date = new Date()): string {
  if (!fetchedAt) return "Never fetched";
  const relative = formatRelativeTime(fetchedAt, now);
  if (!relative) return "Never fetched";
  if (relative === "now") return "Fetched just now";
  return /^\d+[mhd]$/.test(relative) ? `Fetched ${relative} ago` : `Fetched ${relative}`;
}

/** Files that can't be committed until resolved; their checkbox is locked off. */
export function isCommittable(file: GitChangeFile): boolean {
  return file.status !== "conflict";
}

/** The paths a commit will include: every committable file the user hasn't unticked. */
export function includedFiles(files: GitChangeFile[], excluded: ReadonlySet<string>): string[] {
  return files.filter((file) => isCommittable(file) && !excluded.has(file.path)).map((file) => file.path);
}

/** The tri-state "all files" checkbox. */
export function selectionState(files: GitChangeFile[], excluded: ReadonlySet<string>): "all" | "none" | "some" {
  const committable = files.filter(isCommittable);
  const included = includedFiles(files, excluded).length;
  if (included === 0) return "none";
  return included === committable.length ? "all" : "some";
}

export function commitLabel(count: number, branch: string | null): string {
  const target = branch ?? "detached HEAD";
  if (count === 0) return `Commit to ${target}`;
  return `Commit ${count} ${count === 1 ? "file" : "files"} to ${target}`;
}

export type SidebarPage = "chats" | "archived" | "git";
const PAGE_ORDER: SidebarPage[] = ["chats", "archived", "git"];

/** 1 when `to` sits after `from` (it enters from the right), -1 when before. */
export function sidebarPageDirection(from: SidebarPage, to: SidebarPage): 1 | -1 {
  return PAGE_ORDER.indexOf(to) >= PAGE_ORDER.indexOf(from) ? 1 : -1;
}

/** Diff swap direction when moving through a list: down the list slides up, and back. */
export function listDirection(paths: string[], from: string | undefined, to: string): 1 | -1 {
  return paths.indexOf(to) >= paths.indexOf(from ?? "") ? 1 : -1;
}

/** The first line of a multi-line error: Rust errors can carry Git's own trailing detail. */
export function firstLine(message: string): string {
  return message.split("\n").find((line) => line.trim())?.trim() ?? message;
}
