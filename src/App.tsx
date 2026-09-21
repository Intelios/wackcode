import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { api } from "./api";
import { modelIsReady } from "./model-utils";
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
import { Icon } from "./components/Icons";
import { SettingsModal } from "./components/SettingsModal";
import { TaskDialog } from "./components/TaskDialog";
import { Transcript } from "./components/Transcript";

const emptyData: AppData = { version: 1, providers: [], projects: [], tasks: [] };

function shortPath(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts.length > 3 ? `…/${parts.slice(-3).join("/")}` : path;
}

function formatTokens(value?: number): string {
  if (value === undefined) return "unknown";
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}m`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}

export default function App() {
  const [data, setData] = useState<AppData>(emptyData);
  const [appDataPath, setAppDataPath] = useState("");
  const [selectedTaskId, setSelectedTaskId] = useState<string>();
  const selectedTaskRef = useRef<string | undefined>(undefined);
  const [runtimes, setRuntimes] = useState<Record<string, TaskRuntime>>({});
  const [changes, setChanges] = useState<GitChanges>();
  const [changesLoading, setChangesLoading] = useState(false);
  const [changesOpen, setChangesOpen] = useState(true);
  const [changesWidth, setChangesWidth] = useState(430);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [newTaskProject, setNewTaskProject] = useState<ProjectRecord>();
  const [showArchived, setShowArchived] = useState(false);
  const [draft, setDraft] = useState("");
  const [booting, setBooting] = useState(true);
  const [globalError, setGlobalError] = useState<string>();

  const selectedTask = data.tasks.find((task) => task.id === selectedTaskId);
  const selectedProject = data.projects.find((project) => project.id === selectedTask?.projectId);
  const runtime = selectedTaskId ? runtimes[selectedTaskId] : undefined;
  const provider = data.providers.find((item) => item.id === selectedTask?.providerId);
  const model = provider?.models.find((item) => item.id === selectedTask?.modelId);
  const sharedWorkers = selectedTask ? data.tasks.filter((task) => !task.archived && task.id !== selectedTask.id && task.workspacePath === selectedTask.workspacePath && task.status === "running") : [];

  useEffect(() => { selectedTaskRef.current = selectedTaskId; }, [selectedTaskId]);

  const patchTask = useCallback((taskId: string, patch: Partial<TaskRecord>) => {
    setData((current) => ({ ...current, tasks: current.tasks.map((task) => task.id === taskId ? { ...task, ...patch } : task) }));
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
        setRuntimes((current) => ({ ...current, [taskId]: { ...current[taskId], snapshot: payload.snapshot, error: undefined } }));
        if (payload.snapshot.sessionFile) patchTask(taskId, { sessionFile: payload.snapshot.sessionFile });
      } else if (payload.type === "run_state") {
        patchTask(taskId, { status: payload.state, lastError: payload.state === "running" ? null : undefined });
      } else if (payload.type === "activity") {
        setRuntimes((current) => ({ ...current, [taskId]: { ...current[taskId], activity: payload.event } }));
        if (payload.event === "tool_execution_end") void refreshChanges(taskId);
      } else if (payload.type === "worker_error") {
        setRuntimes((current) => ({ ...current, [taskId]: { ...current[taskId], error: payload.message } }));
        patchTask(taskId, { lastError: payload.message });
      } else if (payload.type === "response" && !payload.success && payload.error) {
        setRuntimes((current) => ({ ...current, [taskId]: { ...current[taskId], error: payload.error } }));
      }
    }).then((stop) => { unlisten = stop; });
    return () => unlisten?.();
  }, [patchTask, refreshChanges]);

  useEffect(() => {
    setChanges(undefined);
    if (!selectedTaskId) return;
    void refreshChanges(selectedTaskId);
    api.openTask(selectedTaskId).catch((reason) => {
      setRuntimes((current) => ({ ...current, [selectedTaskId]: { ...current[selectedTaskId], error: String(reason) } }));
    });
  }, [selectedTaskId, refreshChanges]);

  useEffect(() => {
    const refresh = () => void refreshChanges();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [refreshChanges]);

  async function addProject() {
    const selected = await open({ directory: true, multiple: false, title: "Open a project in WackCode" });
    if (!selected) return;
    try {
      const project = await api.addProject(selected);
      setData((current) => ({ ...current, projects: current.projects.some((item) => item.id === project.id) ? current.projects : [...current.projects, project] }));
      setNewTaskProject(project);
    } catch (reason) { setGlobalError(String(reason)); }
  }

  async function createTask(input: Parameters<typeof api.createTask>[0]) {
    const task = await api.createTask(input);
    setData((current) => ({ ...current, tasks: [...current.tasks, task] }));
    setSelectedTaskId(task.id);
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
    } catch (reason) {
      setRuntimes((current) => ({ ...current, [selectedTask.id]: { ...current[selectedTask.id], error: String(reason) } }));
    }
  }

  async function sendPrompt() {
    if (!selectedTask || !draft.trim() || selectedTask.status === "running" || selectedTask.status === "stopping") return;
    const message = draft.trim();
    setDraft("");
    patchTask(selectedTask.id, { status: "running", lastError: null });
    setRuntimes((current) => ({ ...current, [selectedTask.id]: { ...current[selectedTask.id], error: undefined, activity: "starting" } }));
    try {
      await api.prompt({
        taskId: selectedTask.id,
        message,
        providerId: selectedTask.providerId,
        modelId: selectedTask.modelId,
        thinkingLevel: selectedTask.thinkingLevel
      });
    } catch (reason) {
      patchTask(selectedTask.id, { status: "idle" });
      setDraft(message);
      setRuntimes((current) => ({ ...current, [selectedTask.id]: { ...current[selectedTask.id], error: String(reason) } }));
    }
  }

  async function stopTask() {
    if (!selectedTask) return;
    patchTask(selectedTask.id, { status: "stopping" });
    try { await api.stopTask(selectedTask.id); }
    catch (reason) { setRuntimes((current) => ({ ...current, [selectedTask.id]: { ...current[selectedTask.id], error: String(reason) } })); }
  }

  async function archiveSelected() {
    if (!selectedTask) return;
    try {
      const archived = await api.archiveTask(selectedTask.id);
      setData((current) => ({ ...current, tasks: current.tasks.map((task) => task.id === archived.id ? archived : task) }));
      setSelectedTaskId(data.tasks.find((task) => !task.archived && task.id !== selectedTask.id)?.id);
    } catch (reason) { setGlobalError(String(reason)); }
  }

  const configuredProviders = useMemo(() => data.providers.filter((item) => item.hasApiKey && item.models.some(modelIsReady)), [data.providers]);
  const thinkingLevels = model?.thinkingLevels.length ? model.thinkingLevels : (["off"] as ThinkingLevel[]);
  const context = runtime?.snapshot?.stats.contextUsage;

  if (booting) return <div className="boot-screen"><div className="brand-mark">W</div><span>Starting WackCode</span></div>;

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="titlebar-drag" data-tauri-drag-region />
        <div className="brand-row"><div className="brand-mark small">W</div><strong>WackCode</strong></div>
        <button className="new-project-button" onClick={addProject}><Icon name="folder" /><span>Open project</span><kbd>⌘O</kbd></button>
        <nav className="project-list" aria-label="Projects and tasks">
          {data.projects.length === 0 && <div className="sidebar-empty">Open a folder to start your first coding task.</div>}
          {data.projects.map((project) => {
            const tasks = data.tasks.filter((task) => task.projectId === project.id && (!task.archived || showArchived));
            return <section className="project-group" key={project.id}>
              <div className="project-heading"><span title={project.path}>{project.name}</span><button onClick={() => setNewTaskProject(project)} aria-label={`New task in ${project.name}`}><Icon name="plus" /></button></div>
              {tasks.map((task) => <button key={task.id} className={`task-item ${selectedTaskId === task.id ? "active" : ""} ${task.archived ? "archived" : ""}`} onClick={() => setSelectedTaskId(task.id)}>
                <span className={`task-status ${task.lastError ? "error" : task.status}`} />
                <span className="task-name">{task.name}</span>
                {task.usesWorktree && <Icon name="branch" />}
              </button>)}
            </section>;
          })}
        </nav>
        <div className="sidebar-footer">
          {data.tasks.some((task) => task.archived) && <button className="sidebar-action" onClick={() => setShowArchived((value) => !value)}><Icon name="archive" /> {showArchived ? "Hide archived" : "Show archived"}</button>}
          <button className="sidebar-action" onClick={() => setSettingsOpen(true)}><Icon name="settings" /> Settings</button>
          <div className="privacy-status"><span /> Local workspace</div>
        </div>
      </aside>

      <main className="workspace">
        {!selectedTask ? (
          <div className="workspace-empty">
            <div className="empty-art"><span>W</span><i /><i /><i /></div>
            <h1>Your code, your machine.</h1>
            <p>Open a project and start a task. WackCode uses Pi for the agent loop while keeping sessions and settings local.</p>
            <button className="primary-button" onClick={addProject}><Icon name="folder" /> Open a project</button>
          </div>
        ) : (
          <>
            <header className="workspace-header">
              <div className="task-title">
                <h1>{selectedTask.name}</h1>
                <div className="workspace-meta">
                  <button title={selectedTask.workspacePath} onClick={() => api.revealTask(selectedTask.id)}>{shortPath(selectedTask.workspacePath)}</button>
                  {selectedTask.branch && <span><Icon name="branch" /> {selectedTask.branch}</span>}
                  {selectedTask.usesWorktree && <em>worktree</em>}
                </div>
              </div>
              <div className="header-actions">
                <button className="icon-button" title="Copy workspace path" onClick={() => writeText(selectedTask.workspacePath)}><Icon name="copy" /></button>
                <button className="icon-button" title="Archive task" onClick={archiveSelected}><Icon name="archive" /></button>
                {!changesOpen && <button className="panel-button" onClick={() => setChangesOpen(true)}><Icon name="panel" /> Changes</button>}
              </div>
            </header>
            {sharedWorkers.length > 0 && <div className="shared-notice"><span>!</span><strong>{sharedWorkers[0].name}</strong> is also running in this folder. File edits are shared.</div>}
            {(runtime?.error || selectedTask.lastError) && <div className="error-banner workspace-error"><span>{runtime?.error || selectedTask.lastError}</span><button onClick={() => setRuntimes((current) => ({ ...current, [selectedTask.id]: { ...current[selectedTask.id], error: undefined } }))}>Dismiss</button></div>}
            <div className="conversation-scroll">
              <Transcript messages={runtime?.snapshot?.messages ?? []} running={selectedTask.status === "running" || selectedTask.status === "stopping"} activity={runtime?.activity} />
            </div>
            <div className="composer-wrap">
              <div className="composer">
                <textarea value={draft} onChange={(event) => setDraft(event.target.value)} onKeyDown={(event) => {
                  if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) { event.preventDefault(); void sendPrompt(); }
                }} placeholder={selectedTask.status === "running" ? "Draft your next message while Pi works…" : "Ask Pi to inspect, change, or run something…"} rows={3} />
                <div className="composer-toolbar">
                  <div className="model-controls">
                    <select value={selectedTask.providerId} disabled={selectedTask.status === "running" || selectedTask.status === "stopping"} onChange={(event) => void configure({ providerId: event.target.value })} aria-label="Connection">
                      {configuredProviders.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
                    </select>
                    <select value={selectedTask.modelId} disabled={selectedTask.status === "running" || selectedTask.status === "stopping"} onChange={(event) => void configure({ modelId: event.target.value })} aria-label="Model">
                      {(provider?.models.filter(modelIsReady) ?? []).map((item) => <option key={item.id} value={item.id}>{item.name || item.id}</option>)}
                    </select>
                    <select value={selectedTask.thinkingLevel} disabled={selectedTask.status === "running" || selectedTask.status === "stopping"} onChange={(event) => void configure({ thinkingLevel: event.target.value as ThinkingLevel })} aria-label="Reasoning effort">
                      {thinkingLevels.map((level) => <option key={level} value={level}>{level === "off" ? "Reasoning off" : `${level} reasoning`}</option>)}
                    </select>
                  </div>
                  {selectedTask.status === "running" || selectedTask.status === "stopping" ? (
                    <button className="stop-button" onClick={stopTask} disabled={selectedTask.status === "stopping"}><Icon name="stop" /> {selectedTask.status === "stopping" ? "Stopping" : "Stop"}</button>
                  ) : (
                    <button className="send-button" onClick={sendPrompt} disabled={!draft.trim()} aria-label="Send message"><Icon name="send" /></button>
                  )}
                </div>
              </div>
              <div className="session-stats">
                <span>{formatTokens(runtime?.snapshot?.stats.tokens.total)} tokens</span>
                <span>{context?.percent == null ? "context unknown" : `${Math.round(context.percent)}% context`}</span>
                <span>{runtime?.snapshot?.stats.cost ? `$${runtime.snapshot.stats.cost.toFixed(3)}` : "cost unknown"}</span>
                <em>⌘↵ to send</em>
              </div>
            </div>
          </>
        )}
      </main>

      {selectedTask && changesOpen && <ChangesPanel changes={changes} loading={changesLoading} width={changesWidth} onWidthChange={setChangesWidth} onClose={() => setChangesOpen(false)} onRefresh={() => void refreshChanges(selectedTask.id)} />}

      {settingsOpen && <SettingsModal providers={data.providers} appDataPath={appDataPath} onClose={() => setSettingsOpen(false)} onSave={saveProvider} onDelete={deleteProvider} />}
      {newTaskProject && <TaskDialog project={newTaskProject} providers={data.providers} onClose={() => setNewTaskProject(undefined)} onCreate={createTask} />}
      {globalError && <div className="global-toast"><span>{globalError}</span><button onClick={() => setGlobalError(undefined)}>×</button></div>}
    </div>
  );
}
