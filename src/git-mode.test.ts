import { describe, expect, it } from "vitest";
import {
  busyChatsInCheckout, commitLabel, fetchedLabel, includedFiles, initialGitProject, linkableChats, listDirection,
  orderProjects, pickEditor, repoWebCopy, repoWebUrl, resolveLinkedChat, selectionState, sidebarPageDirection, syncAction
} from "./git-mode";
import type { GitChangeFile, GitSyncStatus, ProjectRecord, TaskRecord } from "./types";

function project(id: string, gitRoot: string | null = `/repo/${id}`): ProjectRecord {
  return { id, name: id, path: `/repo/${id}`, gitRoot, gitHasHead: true, runCommand: null, branch: "main", createdAt: "2026-01-01T00:00:00Z" };
}

function task(id: string, patch: Partial<TaskRecord> = {}): TaskRecord {
  return {
    id, projectId: "p", name: id, autoTitleEligible: false, autoTitleAttemptId: null, workspacePath: "/repo/p",
    worktreePath: null, branch: "main", usesWorktree: false, providerId: "x", modelId: "m", thinkingLevel: "off",
    sessionFile: null, status: "idle", mode: "build", archived: false, archivedAt: null, lastError: null, kind: "code", lastActivityAt: null,
    createdAt: "2026-01-01T00:00:00Z", updatedAt: "2026-01-01T00:00:00Z", ...patch
  };
}

function file(path: string, status = "modified"): GitChangeFile {
  return { path, oldPath: null, status, staged: false, unstaged: true, untracked: false, binary: false, hunkable: true, truncated: false, sections: [] };
}

function sync(patch: Partial<GitSyncStatus> = {}): GitSyncStatus {
  return { branch: "main", upstream: "origin/main", remotes: ["origin"], fetchRemote: "origin", ahead: 0, behind: 0, fetchedAt: null, head: "abc", hasHead: true, ...patch };
}

describe("git mode rules", () => {
  it("floats pinned projects to the top in their existing order", () => {
    const ordered = orderProjects([project("a"), project("b"), project("c"), project("d")], new Set(["d", "b"]));
    expect(ordered.map((item) => item.id)).toEqual(["b", "d", "a", "c"]);
  });

  it("opens on the selected chat's project, then the remembered one, then a pinned Git project", () => {
    const projects = [project("plain", null), project("a"), project("b")];
    expect(initialGitProject(projects, task("t", { projectId: "a" }), "b", new Set())).toBe("a");
    expect(initialGitProject(projects, task("t", { projectId: null }), "b", new Set())).toBe("b");
    expect(initialGitProject(projects, undefined, "gone", new Set(["b"]))).toBe("b");
    expect(initialGitProject(projects, undefined, null, new Set())).toBe("a");
    expect(initialGitProject([], undefined, null, new Set())).toBeUndefined();
  });

  it("links the picked chat, then the one you came from, then the most recent; never worktrees or archived", () => {
    const tasks = [
      task("old", { updatedAt: "2026-01-01T00:00:00Z" }),
      task("recent", { updatedAt: "2026-03-01T00:00:00Z" }),
      task("worktree", { usesWorktree: true, updatedAt: "2026-04-01T00:00:00Z" }),
      task("archived", { archived: true, updatedAt: "2026-05-01T00:00:00Z" }),
      task("elsewhere", { projectId: "q", updatedAt: "2026-06-01T00:00:00Z" })
    ];
    expect(linkableChats(tasks, "p").map((item) => item.id)).toEqual(["recent", "old"]);
    expect(resolveLinkedChat(tasks, "p", "old", "recent")?.id).toBe("old");
    expect(resolveLinkedChat(tasks, "p", undefined, "old")?.id).toBe("old");
    expect(resolveLinkedChat(tasks, "p", undefined, "worktree")?.id).toBe("recent");
    expect(resolveLinkedChat(tasks, "q")?.id).toBe("elsewhere");
    expect(resolveLinkedChat(tasks, "empty")).toBeUndefined();
  });

  it("counts running and stopping chats in the project's own folder as busy", () => {
    const tasks = [task("a", { status: "running" }), task("b", { status: "stopping" }), task("c"), task("d", { status: "running", usesWorktree: true })];
    expect(busyChatsInCheckout(tasks, "p").map((item) => item.id)).toEqual(["a", "b"]);
  });

  it("picks the sync button's action like GitHub Desktop", () => {
    expect(syncAction(undefined).kind).toBe("none");
    expect(syncAction(sync({ remotes: [], fetchRemote: null }))).toMatchObject({ kind: "none", label: "No remote" });
    expect(syncAction(sync({ branch: null }))).toMatchObject({ kind: "fetch", remote: "origin" });
    expect(syncAction(sync({ upstream: null }))).toEqual({ kind: "publish", remote: "origin" });
    expect(syncAction(sync({ upstream: null, hasHead: false })).kind).toBe("none");
    expect(syncAction(sync({ behind: 3, ahead: 1 }))).toEqual({ kind: "pull", remote: "origin", behind: 3, ahead: 1 });
    expect(syncAction(sync({ ahead: 2 }))).toEqual({ kind: "push", remote: "origin", ahead: 2 });
    expect(syncAction(sync({ fetchedAt: "x" }))).toEqual({ kind: "fetch", remote: "origin", fetchedAt: "x" });
  });

  it("says when the last fetch was", () => {
    const now = new Date("2026-09-29T12:00:00Z");
    expect(fetchedLabel(null, now)).toBe("Never fetched");
    expect(fetchedLabel("2026-09-29T11:59:40Z", now)).toBe("Fetched just now");
    expect(fetchedLabel("2026-09-29T11:56:00Z", now)).toBe("Fetched 4m ago");
    expect(fetchedLabel("2026-09-01T11:56:00Z", now)).toBe("Fetched 1 Sep");
  });

  it("adds up the file checkboxes, leaving conflicts out", () => {
    const files = [file("a"), file("b"), file("c", "conflict")];
    expect(selectionState(files, new Set())).toBe("all");
    expect(selectionState(files, new Set(["a"]))).toBe("some");
    expect(selectionState(files, new Set(["a", "b"]))).toBe("none");
    expect(selectionState([], new Set())).toBe("none");
    expect(includedFiles(files, new Set(["b"]))).toEqual(["a"]);
  });

  it("labels the commit button and page directions", () => {
    expect(commitLabel(3, "main")).toBe("Commit 3 files to main");
    expect(commitLabel(1, "main")).toBe("Commit 1 file to main");
    expect(commitLabel(0, null)).toBe("Commit to detached HEAD");
    expect(sidebarPageDirection("chats", "git")).toBe(1);
    expect(sidebarPageDirection("git", "chats")).toBe(-1);
    expect(listDirection(["a", "b", "c"], "c", "a")).toBe(-1);
    expect(listDirection(["a", "b", "c"], undefined, "b")).toBe(1);
  });

  it("keeps the remembered editor while it is installed, else the first one", () => {
    expect(pickEditor(["Visual Studio Code", "Zed"], "Zed")).toBe("Zed");
    expect(pickEditor(["Visual Studio Code", "Zed"], "Gone")).toBe("Visual Studio Code");
    expect(pickEditor(["Zed"], null)).toBe("Zed");
    expect(pickEditor([], "Zed")).toBeUndefined();
    expect(pickEditor([], null)).toBeUndefined();
  });

  it("turns a remote URL into the repository's web page", () => {
    expect(repoWebUrl("https://github.com/owner/repo.git")).toBe("https://github.com/owner/repo");
    expect(repoWebUrl("https://github.com/owner/repo.git/")).toBe("https://github.com/owner/repo");
    expect(repoWebUrl("http://intranet.example/git/repo")).toBe("http://intranet.example/git/repo");
    expect(repoWebUrl("git@github.com:owner/repo.git")).toBe("https://github.com/owner/repo");
    expect(repoWebUrl("git@gitlab.com:group/subgroup/repo.git")).toBe("https://gitlab.com/group/subgroup/repo");
    expect(repoWebUrl("ssh://git@github.com/owner/repo.git")).toBe("https://github.com/owner/repo");
    expect(repoWebUrl("ssh://git@ssh.github.com:443/owner/repo.git")).toBe("https://ssh.github.com/owner/repo");
    expect(repoWebUrl("git://github.com/owner/repo.git")).toBe("https://github.com/owner/repo");
    expect(repoWebUrl("https://token@github.com/owner/repo.git")).toBe("https://github.com/owner/repo");
  });

  it("has no web page for local paths and unusable remote URLs", () => {
    expect(repoWebUrl("/Users/me/other/repo")).toBeNull();
    expect(repoWebUrl("C:\\Users\\me\\repo")).toBeNull();
    expect(repoWebUrl("file:///tmp/repo")).toBeNull();
    expect(repoWebUrl("https://github.com/")).toBeNull();
    expect(repoWebUrl("not a url")).toBeNull();
    expect(repoWebUrl("")).toBeNull();
    expect(repoWebUrl(null)).toBeNull();
    expect(repoWebUrl(undefined)).toBeNull();
  });

  it("names GitHub in the clean-state card copy, other hosts by name", () => {
    expect(repoWebCopy("https://github.com/owner/repo")).toMatchObject({ title: "Open in GitHub", label: "Open in GitHub" });
    expect(repoWebCopy("https://github.example.com/owner/repo")).toMatchObject({ label: "Open in GitHub" });
    expect(repoWebCopy("https://gitlab.com/owner/repo")).toMatchObject({ title: "Open on the web", label: "Open on gitlab.com" });
  });
});
