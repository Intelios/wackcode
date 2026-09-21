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
import { SettingsModal } from "./components/SettingsModal";
import { Sidebar, type ProjectAction, type TaskAction } from "./components/Sidebar";
import { Transcript } from "./components/Transcript";
import { ConfirmDialog } from "./components/ui/ConfirmDialog";

const emptyData: AppData = { version: 1, providers: [], projects: [], tasks: [] };

const LAST_MODEL_KEY = "wackcode:lastModel";
const CHANGES_OPEN_KEY = "wackcode:changesOpen";

interface ModelChoice {
  providerId: string;
  modelId: string;
  thinkingLevel: ThinkingLevel;
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
      const first = payload.data.tasks.find((task) => !task.archived) ?? payload.data.tasks[0];
      setSelectedTaskId(first?.id);
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

  const rememberModel = useCallback((projectId: string, choice: ModelChoice) => {
    setLastModels((current) => {
      const next = { ...current, [projectId]: choice };
      localStorage.setItem(LAST_MODEL_KEY, JSON.stringify(next));
      return next;
    });
  }, []);

  const defaultChoice = useCallback((projectId: string): ModelChoice | undefined => {
    const remembered = lastModels[projectId];
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

  async function addProject() {
    const selected = await open({ directory: true, multiple: false, title: "Add a project folder" });
    if (!selected) return;
    try {
      const project = await api.addProject(selected);
      setData((current) => ({ ...current, projects: current.projects.some((item) => item.id === project.id) ? current.projects : [...current.projects, project] }));
    } catch (reason) { setGlobalError(String(reason)); }
  }

  async function newChat(project: ProjectRecord) {
    const choice = defaultChoice(project.id);
    if (!choice) {
      setSettingsOpen(true);
      return;
    }
    try {
      const task = await api.createTask({ projectId: project.id, ...choice });
      rememberModel(project.id, choice);
      setData((current) => ({ ...current, tasks: [...current.tasks, task] }));
      setSelectedTaskId(task.id);
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

  async function sendPrompt(message: string): Promise<boolean> {
    if (!selectedTask || selectedTask.status === "running" || selectedTask.status === "stopping") return false;
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
    setSelectedTaskId((current) => current === removedId
      ? data.tasks.find((task) => !task.archived && task.id !== removedId)?.id
      : current);
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
          if (selectedTask?.projectId === project.id) {
            setSelectedTaskId(data.tasks.find((task) => task.projectId !== project.id && !task.archived)?.id);
          }
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
        const project = selectedProject ?? data.projects[0];
        if (project) void newChat(project);
        else void addProject();
      } else if (key === "o" && !event.shiftKey) {
        event.preventDefault();
        void addProject();
      } else if (key === ",") {
        event.preventDefault();
        setSettingsOpen(true);
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
      <Sidebar
        projects={data.projects}
        tasks={data.tasks}
        selectedTaskId={selectedTaskId}
        showArchived={showArchived}
        onSelectTask={setSelectedTaskId}
        onNewChat={(project) => void newChat(project)}
        onAddProject={() => void addProject()}
        onToggleArchived={() => setShowArchived((value) => !value)}
        onOpenSettings={() => setSettingsOpen(true)}
        onTaskAction={(task, action) => void taskAction(task, action)}
        onProjectAction={(project, action) => void projectAction(project, action)}
        onRenameTask={(taskId, name) => void renameTask(taskId, name)}
      />

      <main className="workspace">
        {!selectedTask ? (
          <div className="workspace-empty">
            {configuredProviders.length === 0 ? (
              <>
                <h1>Connect a model provider</h1>
                <p>Add an OpenAI-compatible endpoint and API key to start chatting.</p>
                <button className="primary-button" onClick={() => setSettingsOpen(true)}><Icon name="key" /> Open settings</button>
              </>
            ) : data.projects.length === 0 ? (
              <>
                <h1>Add a project</h1>
                <p>Chats run inside a project folder. Add one to start.</p>
                <button className="primary-button" onClick={() => void addProject()}><Icon name="folder" /> Add project</button>
              </>
            ) : (
              <>
                <h1>Pick up where you left off</h1>
                <p>Select a chat on the left, or start a new one with the + button on a project.</p>
              </>
            )}
          </div>
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
              task={selectedTask}
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

      {settingsOpen && <SettingsModal providers={data.providers} appDataPath={appDataPath} onClose={() => setSettingsOpen(false)} onSave={saveProvider} onDelete={deleteProvider} />}
      {confirm && <ConfirmDialog title={confirm.title} body={confirm.body} confirmLabel={confirm.confirmLabel} danger={confirm.danger} onConfirm={confirm.run} onCancel={() => setConfirm(undefined)} />}
      {globalError && <div className="global-toast"><span>{globalError}</span><button onClick={() => setGlobalError(undefined)}>×</button></div>}
    </div>
  );
}
