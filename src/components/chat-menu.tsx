import type { ProjectRecord, TaskRecord } from "../types";
import { canFork } from "../tree-utils";
import type { TaskAction } from "./Sidebar";
import { Icon } from "./Icons";
import type { MenuEntry } from "./ui/Menu";

/** Shared by the header dropdown and sidebar right-click; the same availability rules apply. */
export function chatMenu(task: TaskRecord, project: ProjectRecord | undefined, rename: () => void, action: (task: TaskRecord, action: TaskAction) => void): MenuEntry[] {
  return [
    { label: "Rename", icon: <Icon name="pencil" />, onSelect: rename },
    { label: "Move to worktree", icon: <Icon name="branch" />, disabled: Boolean(task.usesWorktree || task.sessionFile || !project?.gitHasHead), onSelect: () => action(task, "worktree") },
    { label: "Fork chat", icon: <Icon name="branch" />, disabled: !canFork(task), onSelect: () => action(task, "fork") },
    "separator",
    { label: "Reveal in Finder", icon: <Icon name="folder" />, onSelect: () => action(task, "reveal") },
    { label: "Copy path", icon: <Icon name="copy" />, onSelect: () => action(task, "copy") },
    "separator",
    { label: task.archived ? "Unarchive" : "Archive", icon: <Icon name={task.archived ? "unarchive" : "archive"} />, onSelect: () => action(task, task.archived ? "unarchive" : "archive") },
    { label: "Delete", icon: <Icon name="trash" />, danger: true, onSelect: () => action(task, "delete") }
  ];
}
