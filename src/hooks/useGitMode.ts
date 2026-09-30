import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import { DIFF_LAYOUT_KEY, GIT_EDITOR_KEY, GIT_PROJECT_KEY, includedFiles, initialGitProject, listDirection, pickEditor, repoWebUrl, selectionState, type GitTab } from "../git-mode";
import type {
  DiffLayout, GitChangeFile, GitChanges, GitCommit, GitCommitFile, GitSyncStatus, ProjectRecord, TaskRecord
} from "../types";

/**
 * Git mode's data, owned by App (this is App logic moved out, so it may call `api`). State is
 * kept per project, so switching repositories and back restores the selection, the commit
 * draft and the history page. Every request carries a counter and lands only while it is the
 * latest of its kind for that project, like `changesRequest` in App.
 *
 * Network: one background fetch when Git mode opens or switches project, and whatever the user
 * clicks. Never on focus and never on a timer (docs/security.md).
 */

export interface GitModeView {
  projectId: string;
  tab: GitTab;
  /** The chat the user entered from; the linked chat defaults to it. */
  cameFrom?: string;
}

export type GitNetworkKind = "fetch" | "pull" | "push" | "publish";

export interface GitLogState {
  commits: GitCommit[];
  hasMore: boolean;
  loading: boolean;
  head: string | null;
  error?: string;
}

export interface ProjectGitState {
  changes?: GitChanges;
  loading: boolean;
  error?: string;
  sync?: GitSyncStatus;
  /** The repository's web page, derived from the fetch remote's URL; null when it has none. */
  repoUrl?: string | null;
  network?: { kind: GitNetworkKind; background: boolean };
  /** The last network failure; a background fetch's is shown quietly on the sync button. */
  networkError?: { message: string; background: boolean };
  /** Set when a pull found commits on both sides. */
  divergence?: { ahead: number; behind: number; upstream: string };
  /** A revert that conflicted and was aborted. */
  revertConflict?: GitCommit;
  log?: GitLogState;
  selectedSha?: string;
  commitFiles?: { sha: string; files: GitCommitFile[]; truncated: boolean };
  commitPath?: string;
  commitDiff?: { sha: string; path: string; file: GitChangeFile };
  commitLoading: boolean;
  /** Unticked paths; everything else is in the next commit, so new files arrive ticked. */
  excluded: string[];
  selectedPath?: string;
  /** Which way the last file or commit swap moved, for the diff's slide. */
  direction: 1 | -1;
  summary: string;
  description: string;
  /** The changes revision a generated message described, to flag it once they move. */
  generatedRevision?: string;
  /** The chat the user chose in the "via" chip. */
  picked?: string;
  committing: boolean;
  /** Bumps on every commit, driving the commit token's flight to History. */
  commitPulse: number;
  /** One-shot confirmation shown in the toolbar ("Committed", "Pushed to origin/main"). */
  flash?: { text: string; nonce: number };
  actionError?: string;
}

const EMPTY: ProjectGitState = {
  loading: false, commitLoading: false, excluded: [], direction: 1, summary: "", description: "", committing: false, commitPulse: 0
};

const LOG_PAGE = 50;
/** Tool calls land in bursts; one refresh after the burst settles is enough. */
const ACTIVITY_DEBOUNCE_MS = 350;

type RequestKind = "changes" | "log" | "files" | "diff";

function loadLayout(): DiffLayout {
  try {
    return localStorage.getItem(DIFF_LAYOUT_KEY) === "split" ? "split" : "unified";
  } catch {
    return "unified";
  }
}

function loadRemembered(): string | null {
  try {
    return localStorage.getItem(GIT_PROJECT_KEY);
  } catch {
    return null;
  }
}

/** The external editor the toolbar last opened a project in. */
function loadEditorChoice(): string | null {
  try {
    return localStorage.getItem(GIT_EDITOR_KEY);
  } catch {
    return null;
  }
}

function remember(projectId: string) {
  try { localStorage.setItem(GIT_PROJECT_KEY, projectId); } catch { /* per-viewer convenience only */ }
}

export function useGitMode(options: {
  projects: ProjectRecord[];
  tasks: TaskRecord[];
  selectedTask?: TaskRecord;
  pinned: ReadonlySet<string>;
}) {
  const [view, setView] = useState<GitModeView | null>(null);
  const [states, setStates] = useState<Record<string, ProjectGitState>>({});
  const [layout, setLayoutState] = useState<DiffLayout>(loadLayout);
  /** The GUI editors installed on this Mac, and the one the toolbar opens. */
  const [editors, setEditors] = useState<string[]>([]);
  const [editorChoice, setEditorChoice] = useState<string | null>(loadEditorChoice);
  const statesRef = useRef(states);
  statesRef.current = states;
  const viewRef = useRef(view);
  viewRef.current = view;
  const optionsRef = useRef(options);
  optionsRef.current = options;
  const requests = useRef<Record<string, Partial<Record<RequestKind, number>>>>({});
  const refreshTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const refreshing = useRef<{ running: boolean; dirty: boolean }>({ running: false, dirty: false });
  const flashNonce = useRef(0);

  const patch = useCallback((projectId: string, next: Partial<ProjectGitState> | ((state: ProjectGitState) => Partial<ProjectGitState>)) => {
    setStates((current) => {
      const before = current[projectId] ?? EMPTY;
      return { ...current, [projectId]: { ...before, ...(typeof next === "function" ? next(before) : next) } };
    });
  }, []);

  const bump = (projectId: string, kind: RequestKind) => {
    const slot = (requests.current[projectId] ??= {});
    slot[kind] = (slot[kind] ?? 0) + 1;
    return slot[kind]!;
  };
  const latest = (projectId: string, kind: RequestKind, request: number) => requests.current[projectId]?.[kind] === request;

  const flash = useCallback((projectId: string, text: string) => {
    patch(projectId, { flash: { text, nonce: ++flashNonce.current } });
  }, [patch]);

  const loadCommitDiff = useCallback(async (projectId: string, sha: string, file: GitCommitFile) => {
    const request = bump(projectId, "diff");
    patch(projectId, { commitPath: file.path, commitLoading: true });
    try {
      const diff = await api.gitCommitDiff({ projectId }, sha, file.path, file.oldPath);
      if (latest(projectId, "diff", request)) patch(projectId, { commitDiff: { sha, path: file.path, file: diff }, commitLoading: false });
    } catch (reason) {
      if (latest(projectId, "diff", request)) patch(projectId, { commitLoading: false, actionError: String(reason) });
    }
  }, [patch]);

  const selectCommit = useCallback(async (projectId: string, sha: string, direction: 1 | -1 = 1) => {
    const request = bump(projectId, "files");
    patch(projectId, { selectedSha: sha, direction, commitFiles: undefined, commitDiff: undefined, commitPath: undefined, commitLoading: true });
    try {
      const files = await api.gitCommitFiles({ projectId }, sha);
      if (!latest(projectId, "files", request)) return;
      patch(projectId, { commitFiles: files });
      if (files.files[0]) await loadCommitDiff(projectId, sha, files.files[0]);
      else patch(projectId, { commitLoading: false });
    } catch (reason) {
      if (latest(projectId, "files", request)) patch(projectId, { commitLoading: false, actionError: String(reason) });
    }
  }, [patch, loadCommitDiff]);

  /** The first page of history; keeps the selected commit when it is still listed. */
  const reloadLog = useCallback(async (projectId: string) => {
    const request = bump(projectId, "log");
    patch(projectId, (state) => ({ log: { commits: state.log?.commits ?? [], hasMore: state.log?.hasMore ?? false, head: state.log?.head ?? null, loading: true } }));
    try {
      const page = await api.gitLog({ projectId }, 0, LOG_PAGE);
      if (!latest(projectId, "log", request)) return;
      const selected = statesRef.current[projectId]?.selectedSha;
      patch(projectId, { log: { commits: page.commits, hasMore: page.hasMore, head: page.head, loading: false } });
      if (page.commits.length && (!selected || !page.commits.some((commit) => commit.sha === selected))) {
        void selectCommit(projectId, page.commits[0].sha);
      }
    } catch (reason) {
      if (latest(projectId, "log", request)) patch(projectId, (state) => ({ log: { ...(state.log ?? { commits: [], hasMore: false, head: null }), loading: false, error: String(reason) } }));
    }
  }, [patch, selectCommit]);

  const loadMoreLog = useCallback(async () => {
    const projectId = viewRef.current?.projectId;
    const log = projectId ? statesRef.current[projectId]?.log : undefined;
    if (!projectId || !log || log.loading || !log.hasMore) return;
    const request = bump(projectId, "log");
    patch(projectId, { log: { ...log, loading: true } });
    try {
      const page = await api.gitLog({ projectId }, log.commits.length, LOG_PAGE);
      if (latest(projectId, "log", request)) {
        patch(projectId, (state) => ({ log: { commits: [...(state.log?.commits ?? []), ...page.commits], hasMore: page.hasMore, head: page.head, loading: false } }));
      }
    } catch (reason) {
      if (latest(projectId, "log", request)) patch(projectId, (state) => ({ log: { ...(state.log ?? log), loading: false, error: String(reason) } }));
    }
  }, [patch]);

  /** Changes plus sync status; history too when HEAD moved under it (an agent may commit). */
  const refreshNow = useCallback(async (projectId: string): Promise<GitSyncStatus | undefined> => {
    const request = bump(projectId, "changes");
    patch(projectId, { loading: true });
    try {
      const [changes, sync] = await Promise.all([
        api.gitChanges({ projectId }),
        api.gitSyncStatus({ projectId }).catch(() => undefined)
      ]);
      if (!latest(projectId, "changes", request)) return sync;
      const log = statesRef.current[projectId]?.log;
      patch(projectId, (state) => ({ changes, sync: sync ?? state.sync, loading: false, error: undefined }));
      const onHistory = viewRef.current?.projectId === projectId && viewRef.current.tab === "history";
      if ((log && sync && log.head !== sync.head) || (onHistory && !log)) void reloadLog(projectId);
      return sync;
    } catch (reason) {
      if (latest(projectId, "changes", request)) patch(projectId, { loading: false, error: String(reason) });
      return undefined;
    }
  }, [patch, reloadLog]);

  /** Single-flight: a refresh asked for while one runs is folded into one more afterwards. */
  const refresh = useCallback(async () => {
    const projectId = viewRef.current?.projectId;
    if (!projectId) return;
    if (refreshing.current.running) { refreshing.current.dirty = true; return; }
    refreshing.current.running = true;
    try {
      do {
        refreshing.current.dirty = false;
        await refreshNow(viewRef.current?.projectId ?? projectId);
      } while (refreshing.current.dirty && viewRef.current);
    } finally {
      refreshing.current.running = false;
    }
  }, [refreshNow]);

  const scheduleRefresh = useCallback(() => {
    clearTimeout(refreshTimer.current);
    refreshTimer.current = setTimeout(() => void refresh(), ACTIVITY_DEBOUNCE_MS);
  }, [refresh]);

  useEffect(() => () => clearTimeout(refreshTimer.current), []);

  const fetchRemote = useCallback(async (projectId: string, background = false) => {
    if (statesRef.current[projectId]?.network) return;
    patch(projectId, { network: { kind: "fetch", background }, networkError: undefined });
    try {
      const sync = await api.gitFetch({ projectId }, background);
      patch(projectId, { sync, network: undefined });
      if (statesRef.current[projectId]?.log) void reloadLog(projectId);
    } catch (reason) {
      patch(projectId, { network: undefined, networkError: { message: String(reason), background } });
    }
  }, [patch, reloadLog]);

  /** Load a project, the installed editors and its repository's web page, and run its one background fetch. */
  const enter = useCallback((projectId: string) => {
    remember(projectId);
    void api.listEditors().then(setEditors).catch(() => setEditors([]));
    void refreshNow(projectId).then((sync) => {
      if (!sync?.fetchRemote || viewRef.current?.projectId !== projectId) return;
      void fetchRemote(projectId, true);
      // The remote's URL becomes the clean state's "Open in GitHub" card. It changes rarely,
      // so one lookup per visit is enough.
      if (statesRef.current[projectId]?.repoUrl === undefined) {
        void api.gitRemoteUrl({ projectId })
          .then((url) => patch(projectId, { repoUrl: repoWebUrl(url) }))
          .catch(() => patch(projectId, { repoUrl: null }));
      }
    });
  }, [refreshNow, fetchRemote, patch]);

  const open = useCallback((request: { projectId?: string; path?: string; cameFrom?: string } = {}) => {
    const { projects, selectedTask, pinned } = optionsRef.current;
    const projectId = request.projectId ?? initialGitProject(projects, selectedTask, loadRemembered(), pinned);
    if (!projectId) return;
    setView({ projectId, tab: viewRef.current?.projectId === projectId ? viewRef.current.tab : "changes", cameFrom: request.cameFrom ?? selectedTask?.id });
    if (request.path) patch(projectId, { selectedPath: request.path });
    enter(projectId);
  }, [enter, patch]);

  const close = useCallback(() => {
    clearTimeout(refreshTimer.current);
    setView(null);
  }, []);

  const switchProject = useCallback((projectId: string) => {
    if (viewRef.current?.projectId === projectId) return;
    setView((current) => current ? { ...current, projectId } : current);
    enter(projectId);
  }, [enter]);

  const setTab = useCallback((tab: GitTab) => {
    setView((current) => current ? { ...current, tab } : current);
    const projectId = viewRef.current?.projectId;
    if (tab === "history" && projectId && !statesRef.current[projectId]?.log) void reloadLog(projectId);
  }, [reloadLog]);

  const setLayout = useCallback((next: DiffLayout) => {
    setLayoutState(next);
    try { localStorage.setItem(DIFF_LAYOUT_KEY, next); } catch { /* per-viewer convenience only */ }
  }, []);

  /** A tool finished or a run settled in some chat: refresh if it works in this project's folder. */
  const onWorkerActivity = useCallback((taskId: string) => {
    const current = viewRef.current;
    if (!current) return;
    const task = optionsRef.current.tasks.find((item) => item.id === taskId);
    if (task && task.projectId === current.projectId && !task.usesWorktree) scheduleRefresh();
  }, [scheduleRefresh]);

  const current = view ? states[view.projectId] ?? EMPTY : undefined;
  const projectId = view?.projectId;

  const actions = useMemo(() => {
    const need = () => viewRef.current?.projectId;
    const state = (id: string) => statesRef.current[id] ?? EMPTY;
    return {
      select(path: string) {
        const id = need(); if (!id) return;
        const paths = state(id).changes?.files.map((file) => file.path) ?? [];
        patch(id, (before) => ({ selectedPath: path, direction: listDirection(paths, before.selectedPath ?? paths[0], path) }));
      },
      toggle(path: string) {
        const id = need(); if (!id) return;
        patch(id, (before) => ({ excluded: before.excluded.includes(path) ? before.excluded.filter((item) => item !== path) : [...before.excluded, path] }));
      },
      toggleAll() {
        const id = need(); if (!id) return;
        patch(id, (before) => {
          const files = before.changes?.files ?? [];
          return { excluded: selectionState(files, new Set(before.excluded)) === "all" ? files.map((file) => file.path) : [] };
        });
      },
      setDraft(draft: Partial<Pick<ProjectGitState, "summary" | "description" | "generatedRevision">>) {
        const id = need(); if (!id) return;
        patch(id, draft);
      },
      /** Show a failed action (a hand-off, a generate) above the diff. */
      reportError(message: string) {
        const id = need(); if (id) patch(id, { actionError: message });
      },
      /** A one-shot confirmation in the toolbar, e.g. after copying a SHA. */
      notify(text: string) {
        const id = need(); if (id) flash(id, text);
      },
      setPicked(taskId: string) {
        const id = need(); if (id) patch(id, { picked: taskId });
      },
      /** Replace the changes with a fresh snapshot an action returned. */
      applyChanges(changes: GitChanges) {
        const id = need(); if (!id) return;
        bump(id, "changes");
        patch(id, { changes });
      },
      dismiss(key: "divergence" | "revertConflict" | "actionError" | "networkError") {
        const id = need(); if (id) patch(id, { [key]: undefined });
      },
      async commit() {
        const id = need(); if (!id) return;
        const before = state(id);
        const changes = before.changes;
        const summary = before.summary.trim();
        if (!changes || !summary || before.committing) return;
        const excluded = new Set(before.excluded);
        const files = includedFiles(changes.files, excluded);
        if (files.length === 0) return;
        const message = before.description.trim() ? `${summary}\n\n${before.description.trim()}` : summary;
        // Everything ticked commits the whole index (renames and all); otherwise just the ticks.
        const scope = selectionState(changes.files, excluded) === "all" && changes.files.every((file) => file.status !== "conflict") ? [] : files;
        patch(id, { committing: true, actionError: undefined });
        try {
          const next = await api.gitCommit({ projectId: id }, { message, files: scope, expected: changes.changesRevision });
          bump(id, "changes");
          // Files left out of this commit stay unticked for the next one, as in GitHub Desktop.
          patch(id, (after) => ({
            changes: next, committing: false, summary: "", description: "", generatedRevision: undefined,
            excluded: after.excluded.filter((path) => next.files.some((file) => file.path === path)),
            commitPulse: after.commitPulse + 1
          }));
          flash(id, "Committed");
          void refreshNow(id);
          void reloadLog(id);
        } catch (reason) {
          patch(id, { committing: false, actionError: String(reason) });
          void refreshNow(id);
        }
      },
      /** Open the project folder in an external editor, remembering which one for next time. */
      openEditor(editor: string) {
        const id = need(); if (!id) return;
        setEditorChoice(editor);
        try { localStorage.setItem(GIT_EDITOR_KEY, editor); } catch { /* per-viewer convenience only */ }
        void api.openInEditor({ projectId: id }, editor).catch((reason) => {
          if (viewRef.current?.projectId === id) patch(id, { actionError: String(reason) });
        });
      },
      fetch() {
        const id = need(); if (id) void fetchRemote(id, false);
      },
      async pull() {
        const id = need(); if (!id || state(id).network) return;
        patch(id, { network: { kind: "pull", background: false }, networkError: undefined, divergence: undefined });
        try {
          const result = await api.gitPull({ projectId: id });
          bump(id, "changes");
          patch(id, {
            network: undefined, sync: result.sync, changes: result.changes,
            divergence: result.outcome === "diverged" && result.sync.upstream
              ? { ahead: result.sync.ahead, behind: result.sync.behind, upstream: result.sync.upstream }
              : undefined
          });
          if (result.outcome === "updated") flash(id, `Pulled ${result.pulled} ${result.pulled === 1 ? "commit" : "commits"}`);
          else if (result.outcome === "up_to_date") flash(id, "Already up to date");
          void reloadLog(id);
        } catch (reason) {
          patch(id, { network: undefined, networkError: { message: String(reason), background: false } });
        }
      },
      async push(remote?: string) {
        const id = need(); if (!id || state(id).network) return;
        const publishing = !state(id).sync?.upstream;
        patch(id, { network: { kind: publishing ? "publish" : "push", background: false }, networkError: undefined });
        try {
          const info = await api.gitPush({ projectId: id }, publishing ? remote ?? state(id).sync?.fetchRemote ?? undefined : undefined);
          patch(id, { network: undefined });
          flash(id, publishing ? `Published to ${info.upstream ?? remote}` : `Pushed to ${info.upstream ?? "remote"}`);
          void refreshNow(id);
          void reloadLog(id);
        } catch (reason) {
          patch(id, { network: undefined, networkError: { message: String(reason), background: false } });
        }
      },
      loadMoreLog,
      selectCommit(sha: string) {
        const id = need(); if (!id) return;
        const commits = state(id).log?.commits.map((commit) => commit.sha) ?? [];
        void selectCommit(id, sha, listDirection(commits, state(id).selectedSha, sha));
      },
      selectCommitFile(path: string) {
        const id = need(); if (!id) return;
        const { selectedSha, commitFiles, commitPath } = state(id);
        const file = commitFiles?.files.find((item) => item.path === path);
        if (!selectedSha || !file) return;
        patch(id, { direction: listDirection(commitFiles!.files.map((item) => item.path), commitPath, path) });
        void loadCommitDiff(id, selectedSha, file);
      },
      async undo(commit: GitCommit) {
        const id = need(); if (!id) return;
        patch(id, { actionError: undefined });
        try {
          const result = await api.gitUndoCommit({ projectId: id }, commit.sha);
          bump(id, "changes");
          patch(id, { changes: result.changes, summary: result.summary, description: result.description, generatedRevision: undefined, excluded: [], selectedSha: undefined });
          setView((currentView) => currentView ? { ...currentView, tab: "changes" } : currentView);
          flash(id, "Commit undone");
          void refreshNow(id);
          void reloadLog(id);
        } catch (reason) {
          patch(id, { actionError: String(reason) });
        }
      },
      async revert(commit: GitCommit) {
        const id = need(); if (!id) return;
        patch(id, { actionError: undefined, revertConflict: undefined });
        const result = await api.gitRevertCommit({ projectId: id }, commit.sha);
        bump(id, "changes");
        patch(id, { changes: result.changes, revertConflict: result.outcome === "conflict" ? commit : undefined });
        if (result.outcome === "reverted") flash(id, "Reverted");
        void refreshNow(id);
        void reloadLog(id);
      }
    };
  }, [patch, flash, refreshNow, reloadLog, fetchRemote, loadMoreLog, selectCommit, loadCommitDiff]);

  return {
    view,
    projectId,
    state: current,
    layout,
    setLayout,
    editors,
    /** The editor the toolbar's button opens: the remembered one, else the first installed. */
    editor: pickEditor(editors, editorChoice),
    open,
    close,
    switchProject,
    setTab,
    refresh,
    scheduleRefresh,
    onWorkerActivity,
    actions
  };
}

export type GitModeController = ReturnType<typeof useGitMode>;
