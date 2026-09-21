import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { api } from "./api";
import { modelIsReady } from "./model-utils";
import { titleFromPrompt } from "./chat-utils";
import { pruneDisabledTools, sameToolCatalog } from "./tool-utils";
import type {
  AppData,
  ExtensionNotice,
  ExtensionUIRequest,
  GitChanges,
  PackageRecord,
  PackageResourceKind,
  ProjectRecord,
  ProviderRecord,
  QuestionAnswer,
  SaveProviderInput,
  TaskMode,
  TaskRecord,
  TaskRuntime,
  ThinkingLevel,
  WorkerEvent
} from "./types";
import { ChangesPanel } from "./components/ChangesPanel";
import { ChatHeader } from "./components/ChatHeader";
import { Composer } from "./components/Composer";
import { Icon } from "./components/Icons";
import { ProjectBar } from "./components/ProjectBar";
import { SettingsPage } from "./components/SettingsPage";
import { Sidebar, type ProjectAction, type TaskAction } from "./components/Sidebar";
import { Transcript } from "./components/Transcript";
import { ExtensionDialog } from "./components/ExtensionDialog";
import { QuestionDialog } from "./components/QuestionDialog";
import type { PlanAction } from "./components/PlanCard";
import { ConfirmDialog } from "./components/ui/ConfirmDialog";

const emptyData: AppData = { version: 1, providers: [], projects: [], tasks: [], toolConfig: { disabled: [] }, toolCatalog: [], packages: [] };

const LAST_MODEL_KEY = "wackcode:lastModel";
const LAST_PROJECT_KEY = "wackcode:lastProject";
const NO_PROJECT_KEY = "none";
const CHANGES_OPEN_KEY = "wackcode:changesOpen";

interface ModelChoice {
  providerId: string;
  modelId: string;
  thinkingLevel: ThinkingLevel;
}

interface Draft {
  projectId: string | null;
  useWorktree: boolean;
  choice?: ModelChoice;
  /** Composer mode chosen before the task exists; sent on the first prompt. */
  mode?: TaskMode;
}

interface ConfirmState {
  title: string;
  body?: string;
  confirmLabel?: string;
  danger?: boolean;
  run: () => Promise<void>;
}

function loadJSON<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

export default function App() {
  const [data, setData] = useState<AppData>(emptyData);
  const [appDataPath, setAppDataPath] = useState("");
  const [selectedTaskId, setSelectedTaskId] = useState<string>();
  const selectedTaskRef = useRef<string | undefined>(undefined);
  const [runtimes, setRuntimes] = useState<Record<string, TaskRuntime>>({});
  const [changes, setChanges] = useState<GitChanges>();
  const [changesLoading, setChangesLoading] = useState(false);
  const [changesOpen, setChangesOpen] = useState(() => loadJSON(CHANGES_OPEN_KEY, false));
  const [changesWidth, setChangesWidth] = useState(() => loadJSON("wackcode:changesWidth", 430));
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [showArchived, setShowArchived] = useState(false);
  const [confirm, setConfirm] = useState<ConfirmState>();
  const [extensionRequests, setExtensionRequests] = useState<ExtensionUIRequest[]>([]);
  const [booting, setBooting] = useState(true);
  const [globalError, setGlobalError] = useState<string>();
  const [lastModels, setLastModels] = useState<Record<string, ModelChoice>>(() => loadJSON(LAST_MODEL_KEY, {}));
  const [draft, setDraft] = useState<Draft>();

  const selectedTask = data.tasks.find((task) => task.id === selectedTaskId);
  const selectedProject = data.projects.find((project) => project.id === selectedTask?.projectId);
  const runtime = selectedTaskId ? runtimes[selectedTaskId] : undefined;
  const sharedWorkers = selectedTask ? data.tasks.filter((task) => !task.archived && task.id !== selectedTask.id && task.workspacePath === selectedTask.workspacePath && task.status === "running") : [];
  // The worker's latest plan_state is the freshest mode signal; the record (or the draft's
  // choice before a task exists) is the durable fallback.
  const currentMode: TaskMode = runtime?.planState?.mode ?? selectedTask?.mode ?? draft?.mode ?? "build";

  useEffect(() => { selectedTaskRef.current = selectedTaskId; }, [selectedTaskId]);
  useEffect(() => { localStorage.setItem(CHANGES_OPEN_KEY, JSON.stringify(changesOpen)); }, [changesOpen]);
  useEffect(() => { localStorage.setItem("wackcode:changesWidth", JSON.stringify(changesWidth)); }, [changesWidth]);

  const patchTask = useCallback((taskId: string, patch: Partial<TaskRecord>) => {
    setData((current) => ({ ...current, tasks: current.tasks.map((task) => task.id === taskId ? { ...task, ...patch } : task) }));
  }, []);

  const appendNotice = useCallback((taskId: string, entry: ExtensionNotice) => {
    setRuntimes((current) => {
      const existing = current[taskId]?.notices ?? [];
      // Extensions can be chatty; keep the most recent few rather than growing without bound.
      return { ...current, [taskId]: { ...current[taskId], notices: [...existing, entry].slice(-5) } };
    });
  }, []);

  const patchRuntime = useCallback((taskId: string, patch: Partial<TaskRuntime>) => {
    setRuntimes((current) => ({ ...current, [taskId]: { ...current[taskId], ...patch } }));
  }, []);

  const refreshChanges = useCallback(async (taskId = selectedTaskRef.current) => {
    if (!taskId) return;
    setChangesLoading(true);
    try {
      const next = await api.gitChanges(taskId);
      if (selectedTaskRef.current === taskId) setChanges(next);
    } catch (reason) {
      if (selectedTaskRef.current === taskId) setGlobalError(String(reason));
    } finally {
      if (selectedTaskRef.current === taskId) setChangesLoading(false);
    }
  }, []);

  useEffect(() => {
    let active = true;
    api.bootstrap().then((payload) => {
      if (!active) return;
      setData(payload.data);
      setAppDataPath(payload.appDataPath);
      const remembered = loadJSON<string | null>(LAST_PROJECT_KEY, null);
      const projectId = payload.data.projects.some((project) => project.id === remembered)
        ? remembered
        : payload.data.projects[0]?.id ?? null;
      setDraft({ projectId, useWorktree: false });
      if (payload.data.providers.length === 0) setSettingsOpen(true);
    }).catch((reason) => setGlobalError(String(reason))).finally(() => active && setBooting(false));
    return () => { active = false; };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<WorkerEvent>("worker-event", ({ payload }) => {
      const taskId = payload.taskId;
      if (!taskId) return;
      if (payload.type === "ready" || payload.type === "snapshot") {
        patchRuntime(taskId, { snapshot: payload.snapshot, planState: payload.snapshot.planState, partial: undefined, error: undefined });
        if (payload.snapshot.planState) patchTask(taskId, { mode: payload.snapshot.planState.mode });
        if (payload.snapshot.sessionFile) patchTask(taskId, { sessionFile: payload.snapshot.sessionFile });
        const tools = payload.snapshot.tools;
        if (tools) {
          setData((current) => sameToolCatalog(current.toolCatalog, tools) ? current : { ...current, toolCatalog: tools });
        }
      } else if (payload.type === "partial") {
        patchRuntime(taskId, { partial: payload.message });
      } else if (payload.type === "run_state") {
        patchTask(taskId, { status: payload.state, lastError: payload.state === "running" ? null : undefined });
        if (payload.state === "idle") patchRuntime(taskId, { activity: undefined });
      } else if (payload.type === "activity") {
        patchRuntime(taskId, { activity: payload.event });
        if (payload.event === "tool_execution_end") void refreshChanges(taskId);
      } else if (payload.type === "worker_error") {
        patchRuntime(taskId, { error: payload.message });
        patchTask(taskId, { lastError: payload.message });
      } else if (payload.type === "plan_state") {
        // The worker is authoritative; the record mirrors it so the sidebar and a fresh
        // composer can render the mode before a worker reports in.
        patchRuntime(taskId, {
          planState: {
            mode: payload.mode,
            phase: payload.phase,
            ...(payload.plan !== undefined ? { plan: payload.plan } : {})
          }
        });
        patchTask(taskId, { mode: payload.mode });
      } else if (payload.type === "extension_ui_request") {
        setExtensionRequests((current) => [...current, payload]);
      } else if (payload.type === "extension_notice") {
        appendNotice(taskId, { message: payload.message, level: payload.level });
      } else if (payload.type === "extensions_loaded") {
        // Load failures are surfaced but never mark the chat as failed.
        for (const entry of payload.errors) {
          appendNotice(taskId, { message: `Extension failed to load (${entry.path}): ${entry.error}`, level: "warning" });
        }
      } else if (payload.type === "response" && !payload.success && payload.error) {
        patchRuntime(taskId, { error: payload.error });
      }
    }).then((stop) => { unlisten = stop; });
    return () => unlisten?.();
  }, [patchTask, patchRuntime, refreshChanges, appendNotice]);

  useEffect(() => {
    setChanges(undefined);
    if (!selectedTaskId) return;
    void refreshChanges(selectedTaskId);
    api.openTask(selectedTaskId).catch((reason) => {
      patchRuntime(selectedTaskId, { error: String(reason) });
    });
  }, [selectedTaskId, refreshChanges, patchRuntime]);

  useEffect(() => {
    const refresh = () => void refreshChanges();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [refreshChanges]);

  const configuredProviders = useMemo(() => data.providers.filter((item) => item.hasApiKey && item.models.some(modelIsReady)), [data.providers]);

  const rememberModel = useCallback((projectId: string | null, choice: ModelChoice) => {
    setLastModels((current) => {
      const next = { ...current, [projectId ?? NO_PROJECT_KEY]: choice };
      localStorage.setItem(LAST_MODEL_KEY, JSON.stringify(next));
      return next;
    });
  }, []);

  const defaultChoice = useCallback((projectId: string | null): ModelChoice | undefined => {
    const remembered = lastModels[projectId ?? NO_PROJECT_KEY];
    const rememberedProvider = remembered && configuredProviders.find((item) => item.id === remembered.providerId);
    const rememberedModel = rememberedProvider?.models.find((model) => model.id === remembered.modelId && modelIsReady(model));
    if (rememberedProvider && rememberedModel) {
      const levels = rememberedModel.thinkingLevels.length ? rememberedModel.thinkingLevels : (["off"] as ThinkingLevel[]);
      return { providerId: remembered.providerId, modelId: remembered.modelId, thinkingLevel: levels.includes(remembered.thinkingLevel) ? remembered.thinkingLevel : levels.includes("medium") ? "medium" : levels[0] };
    }
    const first = configuredProviders[0];
    const model = first?.models.find(modelIsReady);
    if (!first || !model) return undefined;
    const levels = model.thinkingLevels.length ? model.thinkingLevels : (["off"] as ThinkingLevel[]);
    return { providerId: first.id, modelId: model.id, thinkingLevel: levels.includes("medium") ? "medium" : levels[0] };
  }, [configuredProviders, lastModels]);

  const draftProject = data.projects.find((project) => project.id === draft?.projectId);
  const draftChoice = draft ? draft.choice ?? defaultChoice(draft.projectId) : undefined;

  function lastProjectId(projects = data.projects): string | null {
    const remembered = loadJSON<string | null>(LAST_PROJECT_KEY, null);
    return projects.some((project) => project.id === remembered) ? remembered : projects[0]?.id ?? null;
  }

  function openDraft(projectId?: string | null) {
    const resolved = projectId === undefined ? lastProjectId() : projectId;
    setSelectedTaskId(undefined);
    setDraft({ projectId: resolved, useWorktree: false });
  }

  function setDraftProject(projectId: string | null) {
    localStorage.setItem(LAST_PROJECT_KEY, JSON.stringify(projectId));
    setDraft((current) => ({ projectId, useWorktree: false, choice: current?.choice }));
  }

  function setDraftWorktree(useWorktree: boolean) {
    setDraft((current) => ({ projectId: current?.projectId ?? null, useWorktree, choice: current?.choice }));
  }

  async function addProject() {
    const selected = await open({ directory: true, multiple: false, title: "Add a project folder" });
    if (!selected) return;
    try {
      const project = await api.addProject(selected);
      setData((current) => ({ ...current, projects: current.projects.some((item) => item.id === project.id) ? current.projects : [...current.projects, project] }));
      setDraft((current) => current ? { ...current, projectId: project.id } : current);
      localStorage.setItem(LAST_PROJECT_KEY, JSON.stringify(project.id));
    } catch (reason) { setGlobalError(String(reason)); }
  }

  async function saveProvider(input: SaveProviderInput): Promise<ProviderRecord> {
    const saved = await api.saveProvider(input);
    setData((current) => ({
      ...current,
      providers: current.providers.some((provider) => provider.id === saved.id)
        ? current.providers.map((provider) => provider.id === saved.id ? saved : provider)
        : [...current.providers, saved]
    }));
    return saved;
  }

  async function setDisabledTools(disabled: string[]) {
    const pruned = pruneDisabledTools(disabled, data.toolCatalog);
    const previous = data.toolConfig;
    setData((current) => ({ ...current, toolConfig: { disabled: pruned } }));
    try {
      const saved = await api.setToolConfig(pruned);
      setData((current) => ({ ...current, toolConfig: saved }));
    } catch (reason) {
      setData((current) => ({ ...current, toolConfig: previous }));
      throw reason;
    }
  }

  const refreshPackages = useCallback(async () => {
    const packages = await api.refreshPackages();
    setData((current) => ({ ...current, packages }));
  }, []);

  const runPackageAction = useCallback(async (action: () => Promise<PackageRecord[]>) => {
    const packages = await action();
    setData((current) => ({ ...current, packages }));
  }, []);

  const installPackage = useCallback((source: string) => runPackageAction(() => api.installPackage(source, true)), [runPackageAction]);
  const trustPackage = useCallback((source: string) => runPackageAction(() => api.trustPackage(source)), [runPackageAction]);
  const removePackage = useCallback((source: string) => runPackageAction(() => api.removePackage(source)), [runPackageAction]);
  const updatePackage = useCallback((source: string) => runPackageAction(() => api.updatePackages(source)), [runPackageAction]);
  const setPackageResources = useCallback(
    (source: string, kind: PackageResourceKind, enabled: string[]) =>
      runPackageAction(() => api.setPackageResources({ source, [kind]: enabled })),
    [runPackageAction]
  );
  const searchPackages = useCallback((query: string) => api.searchPackages(query), []);

  async function deleteProvider(providerId: string) {
    await api.deleteProvider(providerId);
    setData((current) => ({ ...current, providers: current.providers.filter((provider) => provider.id !== providerId) }));
  }

  async function configure(patch: Partial<Pick<TaskRecord, "providerId" | "modelId" | "thinkingLevel">>) {
    if (!selectedTask) return;
    const providerId = patch.providerId ?? selectedTask.providerId;
    const nextProvider = data.providers.find((item) => item.id === providerId);
    const modelId = patch.modelId ?? (patch.providerId ? nextProvider?.models.find(modelIsReady)?.id : selectedTask.modelId) ?? "";
    const nextModel = nextProvider?.models.find((item) => item.id === modelId);
    const available = nextModel?.thinkingLevels.length ? nextModel.thinkingLevels : (["off"] as ThinkingLevel[]);
    const thinkingLevel = patch.thinkingLevel && available.includes(patch.thinkingLevel)
      ? patch.thinkingLevel
      : available.includes(selectedTask.thinkingLevel) ? selectedTask.thinkingLevel : available.includes("medium") ? "medium" : available[0];
    try {
      const updated = await api.configureTask({ taskId: selectedTask.id, providerId, modelId, thinkingLevel });
      setData((current) => ({ ...current, tasks: current.tasks.map((task) => task.id === updated.id ? updated : task) }));
      rememberModel(updated.projectId, { providerId, modelId, thinkingLevel });
    } catch (reason) {
      patchRuntime(selectedTask.id, { error: String(reason) });
    }
  }

  function configureDraft(patch: Partial<Pick<TaskRecord, "providerId" | "modelId" | "thinkingLevel">>) {
    setDraft((current) => {
      const base = current?.choice ?? defaultChoice(current?.projectId ?? null);
      const providerId = patch.providerId ?? base?.providerId;
      const provider = configuredProviders.find((item) => item.id === providerId);
      const modelId = patch.modelId ?? (patch.providerId ? provider?.models.find(modelIsReady)?.id : base?.modelId);
      const model = provider?.models.find((item) => item.id === modelId);
      const levels: ThinkingLevel[] = model?.thinkingLevels.length ? model.thinkingLevels : ["off"];
      const thinkingLevel = patch.thinkingLevel && levels.includes(patch.thinkingLevel)
        ? patch.thinkingLevel
        : base?.thinkingLevel && levels.includes(base.thinkingLevel) ? base.thinkingLevel : levels.includes("medium") ? "medium" : levels[0];
      if (!providerId || !modelId || !thinkingLevel) return current;
      return { projectId: current?.projectId ?? null, useWorktree: current?.useWorktree ?? false, choice: { providerId, modelId, thinkingLevel } };
    });
  }

  async function sendPrompt(message: string, modeOverride?: TaskMode): Promise<boolean> {
    if (!selectedTask) {
      const active = draft ?? { projectId: lastProjectId(), useWorktree: false };
      const choice = active.choice ?? defaultChoice(active.projectId);
      if (!choice) {
        setSettingsOpen(true);
        return false;
      }
      let task: TaskRecord;
      try {
        task = await api.createTask({
          projectId: active.projectId,
          useWorktree: active.useWorktree && Boolean(draftProject?.gitHasHead),
          name: titleFromPrompt(message),
          ...choice
        });
      } catch (reason) {
        setGlobalError(String(reason));
        return false;
      }
      rememberModel(active.projectId, choice);
      localStorage.setItem(LAST_PROJECT_KEY, JSON.stringify(active.projectId));
      const mode = modeOverride ?? active.mode ?? "build";
      setData((current) => ({ ...current, tasks: [...current.tasks, task] }));
      patchTask(task.id, { status: "running", lastError: null, mode });
      patchRuntime(task.id, { error: undefined, activity: "starting" });
      try {
        await api.prompt({
          taskId: task.id,
          message,
          providerId: task.providerId,
          modelId: task.modelId,
          thinkingLevel: task.thinkingLevel,
          mode
        });
      } catch (reason) {
        patchTask(task.id, { status: "idle" });
        patchRuntime(task.id, { error: String(reason) });
      }
      // Selection happens after api.prompt so open_task's ensure_worker finds the
      // already-running worker instead of racing it to spawn a second process.
      setSelectedTaskId(task.id);
      setDraft(undefined);
      return true;
    }
    if (selectedTask.status === "running" || selectedTask.status === "stopping") return false;
    patchTask(selectedTask.id, { status: "running", lastError: null });
    patchRuntime(selectedTask.id, { error: undefined, activity: "starting" });
    try {
      await api.prompt({
        taskId: selectedTask.id,
        message,
        providerId: selectedTask.providerId,
        modelId: selectedTask.modelId,
        thinkingLevel: selectedTask.thinkingLevel,
        // modeOverride matters for "Approve & implement": the plan_state → record sync can
        // still be in flight when the follow-up prompt goes out.
        mode: modeOverride ?? currentMode
      });
      if (selectedTask.name === "New chat") {
        const title = titleFromPrompt(message);
        patchTask(selectedTask.id, { name: title });
        void api.renameTask(selectedTask.id, title).catch(() => undefined);
      }
      return true;
    } catch (reason) {
      patchTask(selectedTask.id, { status: "idle" });
      patchRuntime(selectedTask.id, { error: String(reason) });
      return false;
    }
  }

  /**
   * Switch Build ↔ Plan. For a draft the choice is just held locally; for a task it is
   * persisted on the record and pushed to the worker (which refuses while a run is active).
   */
  async function setTaskMode(mode: TaskMode) {
    if (mode === currentMode) return;
    if (!selectedTask) {
      setDraft((current) => ({
        projectId: current?.projectId ?? lastProjectId(),
        useWorktree: current?.useWorktree ?? false,
        choice: current?.choice,
        mode
      }));
      return;
    }
    if (selectedTask.status === "running" || selectedTask.status === "stopping") return;
    const previous = selectedTask.mode;
    patchTask(selectedTask.id, { mode });
    patchRuntime(selectedTask.id, { planState: { mode, phase: "planning" } });
    try {
      const updated = await api.setTaskMode(selectedTask.id, mode);
      patchTask(selectedTask.id, updated);
    } catch (reason) {
      patchTask(selectedTask.id, { mode: previous });
      patchRuntime(selectedTask.id, { error: String(reason) });
    }
  }

  /** The PlanCard action row: approve, copy, save to PLAN.md, or abandon the proposal. */
  async function planAction(action: PlanAction) {
    if (!selectedTask) return;
    const plan = runtime?.planState?.plan;
    if (!plan) return;
    if (action === "implement") {
      try {
        const updated = await api.setTaskMode(selectedTask.id, "build");
        patchTask(selectedTask.id, updated);
        patchRuntime(selectedTask.id, { planState: { mode: "build", phase: "planning" } });
      } catch (reason) {
        patchRuntime(selectedTask.id, { error: String(reason) });
        return;
      }
      await sendPrompt("Implement the plan.", "build");
    } else if (action === "copy") {
      await writeText(plan);
      appendNotice(selectedTask.id, { message: "Plan copied to the clipboard.", level: "info" });
    } else if (action === "save") {
      try {
        const path = await api.exportPlan(selectedTask.id, plan);
        appendNotice(selectedTask.id, { message: `Plan saved to ${path}`, level: "info" });
        void refreshChanges(selectedTask.id);
      } catch (reason) {
        patchRuntime(selectedTask.id, { error: String(reason) });
      }
    } else if (action === "discard") {
      setConfirm({
        title: "Discard this plan?",
        body: "The task switches back to Build mode and the proposed plan is abandoned. The conversation is kept.",
        confirmLabel: "Discard",
        danger: true,
        run: async () => {
          const updated = await api.setTaskMode(selectedTask.id, "build");
          patchTask(selectedTask.id, updated);
          patchRuntime(selectedTask.id, { planState: { mode: "build", phase: "planning" } });
        }
      });
    }
  }

  async function stopTask() {
    if (!selectedTask) return;
    patchTask(selectedTask.id, { status: "stopping" });
    try { await api.stopTask(selectedTask.id); }
    catch (reason) { patchRuntime(selectedTask.id, { error: String(reason) }); }
  }

  async function renameTask(taskId: string, name: string) {
    patchTask(taskId, { name });
    try {
      const updated = await api.renameTask(taskId, name);
      patchTask(taskId, updated);
    } catch (reason) { setGlobalError(String(reason)); }
  }

  function selectAfterRemoval(removedId: string) {
    if (selectedTaskRef.current === removedId) openDraft();
  }

  async function taskAction(task: TaskRecord, action: TaskAction) {
    if (action === "reveal") {
      try { await api.revealTask(task.id); } catch (reason) { setGlobalError(String(reason)); }
    } else if (action === "copy") {
      await writeText(task.workspacePath);
    } else if (action === "archive") {
      try {
        const archived = await api.archiveTask(task.id);
        setData((current) => ({ ...current, tasks: current.tasks.map((item) => item.id === archived.id ? archived : item) }));
        selectAfterRemoval(task.id);
      } catch (reason) { setGlobalError(String(reason)); }
    } else if (action === "unarchive") {
      try {
        const restored = await api.unarchiveTask(task.id);
        setData((current) => ({ ...current, tasks: current.tasks.map((item) => item.id === restored.id ? restored : item) }));
      } catch (reason) { setGlobalError(String(reason)); }
    } else if (action === "worktree") {
      try {
        const updated = await api.convertToWorktree(task.id);
        setData((current) => ({ ...current, tasks: current.tasks.map((item) => item.id === updated.id ? updated : item) }));
      } catch (reason) { setGlobalError(String(reason)); }
    } else if (action === "delete") {
      setConfirm({
        title: `Delete “${task.name}”?`,
        body: task.usesWorktree
          ? "This removes the chat, its saved session, and its git worktree — including any uncommitted changes inside it."
          : "This removes the chat and its saved session. Files in the project are not touched.",
        confirmLabel: "Delete",
        danger: true,
        run: async () => {
          await api.deleteTask(task.id);
          setData((current) => ({ ...current, tasks: current.tasks.filter((item) => item.id !== task.id) }));
          setRuntimes((current) => {
            const next = { ...current };
            delete next[task.id];
            return next;
          });
          selectAfterRemoval(task.id);
        }
      });
    }
  }

  async function projectAction(project: ProjectRecord, action: ProjectAction) {
    if (action === "reveal") {
      try { await api.revealPath(project.path); } catch (reason) { setGlobalError(String(reason)); }
    } else if (action === "remove") {
      setConfirm({
        title: `Remove “${project.name}”?`,
        body: "The project is removed from WackCode along with its archived chats' sessions. Files on disk are not touched.",
        confirmLabel: "Remove",
        danger: true,
        run: async () => {
          await api.removeProject(project.id);
          setData((current) => ({
            ...current,
            projects: current.projects.filter((item) => item.id !== project.id),
            tasks: current.tasks.filter((task) => task.projectId !== project.id)
          }));
          setDraft((current) => current && current.projectId === project.id ? { ...current, projectId: null, useWorktree: false } : current);
          if (selectedTask?.projectId === project.id) openDraft();
        }
      });
    }
  }

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      // ⇧Tab cycles Build ↔ Plan, like Claude Code. A modal or an active run owns the key.
      if (event.key === "Tab" && event.shiftKey) {
        const busy = selectedTask && (selectedTask.status === "running" || selectedTask.status === "stopping");
        if (!settingsOpen && !confirm && extensionRequests.length === 0 && !busy) {
          event.preventDefault();
          void setTaskMode(currentMode === "plan" ? "build" : "plan");
        }
        return;
      }
      if (!(event.metaKey || event.ctrlKey)) return;
      const key = event.key.toLowerCase();
      if (key === "n" && !event.shiftKey) {
        event.preventDefault();
        openDraft(selectedTask ? selectedTask.projectId : draft?.projectId);
      } else if (key === "o" && !event.shiftKey) {
        event.preventDefault();
        void addProject();
      } else if (key === ",") {
        event.preventDefault();
        setSettingsOpen((value) => !value);
      } else if (key === "c" && event.shiftKey) {
        event.preventDefault();
        setChangesOpen((value) => !value);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  if (booting) return <div className="boot-screen"><div className="brand-mark">W</div><span>Starting WackCode</span></div>;

  return (
    <div className="app-shell">
      {settingsOpen ? (
        <SettingsPage
          providers={data.providers}
          packages={data.packages}
          toolCatalog={data.toolCatalog}
          disabledTools={data.toolConfig.disabled}
          appDataPath={appDataPath}
          onClose={() => setSettingsOpen(false)}
          onSave={saveProvider}
          onDelete={deleteProvider}
          onSetDisabledTools={setDisabledTools}
          onRefresh={refreshPackages}
          onInstall={installPackage}
          onTrust={trustPackage}
          onSearch={searchPackages}
          onRemove={removePackage}
          onUpdate={updatePackage}
          onSetResources={setPackageResources}
        />
      ) : (
        <>
      <Sidebar
        projects={data.projects}
        tasks={data.tasks}
        selectedTaskId={selectedTaskId}
        showArchived={showArchived}
        onSelectTask={(id) => { setDraft(undefined); setSelectedTaskId(id); }}
        onNewChat={(project) => openDraft(project?.id ?? null)}
        onNewDraft={() => openDraft()}
        onAddProject={() => void addProject()}
        onToggleArchived={() => setShowArchived((value) => !value)}
        onOpenSettings={() => setSettingsOpen(true)}
        onTaskAction={(task, action) => void taskAction(task, action)}
        onProjectAction={(project, action) => void projectAction(project, action)}
        onRenameTask={(taskId, name) => void renameTask(taskId, name)}
      />

      <main className="workspace">
        {!selectedTask ? (
          configuredProviders.length === 0 ? (
            <div className="workspace-empty">
              <h1>Connect a model provider</h1>
              <p>Add an OpenAI-compatible endpoint and API key to start chatting.</p>
              <button className="primary-button" onClick={() => setSettingsOpen(true)}><Icon name="key" /> Open settings</button>
            </div>
          ) : (
            <div className="draft-hero">
              <h1 className="draft-title">{draftProject ? `What should we build in ${draftProject.name}?` : "What should we build?"}</h1>
              <Composer
                status="idle"
                providerId={draftChoice?.providerId}
                modelId={draftChoice?.modelId}
                thinkingLevel={draftChoice?.thinkingLevel}
                providers={configuredProviders}
                popoverSide="bottom"
                header={
                  <ProjectBar
                    projects={data.projects}
                    projectId={draft?.projectId ?? null}
                    useWorktree={draft?.useWorktree ?? false}
                    onSelectProject={setDraftProject}
                    onToggleWorktree={setDraftWorktree}
                    onAddProject={() => void addProject()}
                  />
                }
                placeholder="Describe a task or ask a question…"
                mode={draft?.mode ?? "build"}
                onModeChange={(mode) => void setTaskMode(mode)}
                onConfigure={configureDraft}
                onSend={sendPrompt}
                onStop={() => undefined}
                onOpenSettings={() => setSettingsOpen(true)}
              />
            </div>
          )
        ) : (
          <>
            <ChatHeader
              task={selectedTask}
              project={selectedProject}
              changesCount={changes?.files.length}
              changesOpen={changesOpen}
              onToggleChanges={() => setChangesOpen((value) => !value)}
              onRename={(name) => void renameTask(selectedTask.id, name)}
              onTaskAction={(task, action) => void taskAction(task, action)}
            />
            {sharedWorkers.length > 0 && <div className="shared-notice"><span>!</span><strong>{sharedWorkers[0].name}</strong> is also running in this folder. File edits are shared.</div>}
            {(runtime?.error || selectedTask.lastError) && <div className="error-banner workspace-error"><span>{runtime?.error || selectedTask.lastError}</span><button onClick={() => patchRuntime(selectedTask.id, { error: undefined })}>Dismiss</button></div>}
            {runtime?.notices?.map((entry, index) => (
              <div className={`extension-notice ${entry.level}`} key={`${index}-${entry.message}`}>
                <span>{entry.message}</span>
                <button onClick={() => patchRuntime(selectedTask.id, { notices: runtime.notices?.filter((_, position) => position !== index) })}>Dismiss</button>
              </div>
            ))}
            <Transcript
              messages={runtime?.snapshot?.messages ?? []}
              partial={runtime?.partial}
              running={selectedTask.status === "running" || selectedTask.status === "stopping"}
              activity={runtime?.activity}
              planState={runtime?.planState}
              onPlanAction={(action) => void planAction(action)}
            />
            <Composer
              status={selectedTask.status}
              providerId={selectedTask.providerId}
              modelId={selectedTask.modelId}
              thinkingLevel={selectedTask.thinkingLevel}
              providers={configuredProviders}
              stats={runtime?.snapshot?.stats}
              mode={currentMode}
              onModeChange={(mode) => void setTaskMode(mode)}
              onConfigure={(patch) => void configure(patch)}
              onSend={sendPrompt}
              onStop={() => void stopTask()}
              onOpenSettings={() => setSettingsOpen(true)}
            />
          </>
        )}
      </main>

      {selectedTask && changesOpen && <ChangesPanel changes={changes} loading={changesLoading} width={changesWidth} onWidthChange={setChangesWidth} onClose={() => setChangesOpen(false)} onRefresh={() => void refreshChanges(selectedTask.id)} />}
        </>
      )}

      {extensionRequests[0] && (() => {
        const request = extensionRequests[0];
        const onRespond = (response: { value?: string; confirmed?: boolean; cancelled?: true; answers?: QuestionAnswer[] }) => {
          setExtensionRequests((current) => current.filter((entry) => entry.requestId !== request.requestId));
          void api.respondExtensionUi({ taskId: request.taskId, requestId: request.requestId, ...response })
            .catch((reason) => setGlobalError(String(reason)));
        };
        return request.method === "questions"
          ? <QuestionDialog key={request.requestId} request={request} onRespond={onRespond} />
          : <ExtensionDialog key={request.requestId} request={request} onRespond={onRespond} />;
      })()}
      {confirm && <ConfirmDialog title={confirm.title} body={confirm.body} confirmLabel={confirm.confirmLabel} danger={confirm.danger} onConfirm={confirm.run} onCancel={() => setConfirm(undefined)} />}
      {globalError && <div className="global-toast"><span>{globalError}</span><button onClick={() => setGlobalError(undefined)}>×</button></div>}
    </div>
  );
}
