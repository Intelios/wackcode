import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { api } from "./api";
import { modelIsReady } from "./model-utils";
import { titleFromPrompt } from "./chat-utils";
import type {
  AppData,
  GitChanges,
  ProjectRecord,
  ProviderRecord,
  SaveProviderInput,
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
import { ConfirmDialog } from "./components/ui/ConfirmDialog";

const emptyData: AppData = { version: 1, providers: [], projects: [], tasks: [] };

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
  const [booting, setBooting] = useState(true);
  const [globalError, setGlobalError] = useState<string>();
  const [lastModels, setLastModels] = useState<Record<string, ModelChoice>>(() => loadJSON(LAST_MODEL_KEY, {}));
  const [draft, setDraft] = useState<Draft>();

  const selectedTask = data.tasks.find((task) => task.id === selectedTaskId);
  const selectedProject = data.projects.find((project) => project.id === selectedTask?.projectId);
  const runtime = selectedTaskId ? runtimes[selectedTaskId] : undefined;
  const sharedWorkers = selectedTask ? data.tasks.filter((task) => !task.archived && task.id !== selectedTask.id && task.workspacePath === selectedTask.workspacePath && task.status === "running") : [];

  useEffect(() => { selectedTaskRef.current = selectedTaskId; }, [selectedTaskId]);
  useEffect(() => { localStorage.setItem(CHANGES_OPEN_KEY, JSON.stringify(changesOpen)); }, [changesOpen]);
  useEffect(() => { localStorage.setItem("wackcode:changesWidth", JSON.stringify(changesWidth)); }, [changesWidth]);

  const patchTask = useCallback((taskId: string, patch: Partial<TaskRecord>) => {
    setData((current) => ({ ...current, tasks: current.tasks.map((task) => task.id === taskId ? { ...task, ...patch } : task) }));
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
        patchRuntime(taskId, { snapshot: payload.snapshot, partial: undefined, error: undefined });
        if (payload.snapshot.sessionFile) patchTask(taskId, { sessionFile: payload.snapshot.sessionFile });
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
      } else if (payload.type === "response" && !payload.success && payload.error) {
        patchRuntime(taskId, { error: payload.error });
      }
    }).then((stop) => { unlisten = stop; });
    return () => unlisten?.();
  }, [patchTask, patchRuntime, refreshChanges]);

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

  async function sendPrompt(message: string): Promise<boolean> {
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
      setData((current) => ({ ...current, tasks: [...current.tasks, task] }));
      patchTask(task.id, { status: "running", lastError: null });
      patchRuntime(task.id, { error: undefined, activity: "starting" });
      try {
        await api.prompt({
          taskId: task.id,
          message,
          providerId: task.providerId,
          modelId: task.modelId,
          thinkingLevel: task.thinkingLevel
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
        thinkingLevel: selectedTask.thinkingLevel
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
          appDataPath={appDataPath}
          onClose={() => setSettingsOpen(false)}
          onSave={saveProvider}
          onDelete={deleteProvider}
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
            <Transcript
              messages={runtime?.snapshot?.messages ?? []}
              partial={runtime?.partial}
              running={selectedTask.status === "running" || selectedTask.status === "stopping"}
              activity={runtime?.activity}
            />
            <Composer
              status={selectedTask.status}
              providerId={selectedTask.providerId}
              modelId={selectedTask.modelId}
              thinkingLevel={selectedTask.thinkingLevel}
              providers={configuredProviders}
              stats={runtime?.snapshot?.stats}
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

      {confirm && <ConfirmDialog title={confirm.title} body={confirm.body} confirmLabel={confirm.confirmLabel} danger={confirm.danger} onConfirm={confirm.run} onCancel={() => setConfirm(undefined)} />}
      {globalError && <div className="global-toast"><span>{globalError}</span><button onClick={() => setGlobalError(undefined)}>×</button></div>}
    </div>
  );
}
