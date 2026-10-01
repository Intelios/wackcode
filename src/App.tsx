import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { api } from "./api";
import { chatModelGone, modelDisplayName, modelIsReady, pickThinkingLevel } from "./model-utils";
import { titleFromPrompt, samePlanState, sameTodoState, sameGoalState, applySnapshotDelta, applySubagentFrame, pendingSubagentView, validateInitCommand, nextMode, isPlanMode } from "./chat-utils";
import { defaultSelection, latestTurn, messageText, userOfTurn, workspacePrefix } from "./tree-utils";
import { composeFileSection, splitFileSection, type FileAttachment } from "./attachment-utils";
import { displayAgentName, hasSubagentCall, pruneDisabledTools, sameToolCatalog, subagentDetailsFor } from "./tool-utils";
import { CHANGES_VIEW, TERMINAL_VIEW, durableView, rememberedView, toggleView, viewForChat, viewKey, type PanelViewKind, type SidePanelView } from "./side-panel";
import { APP_SLASH_COMMANDS } from "./command-utils";
import { performChatNavigation, withoutResolvedDialog } from "./menu-navigation";
import { PINNED_PROJECTS_KEY, busyChatsInCheckout, includedFiles, linkableChats, resolveLinkedChat } from "./git-mode";
import { useGitMode } from "./hooks/useGitMode";
import { EMPTY_DRAFT, useComposerDrafts } from "./hooks/useComposerDrafts";
import { DEFAULT_APPEARANCE, applyTheme, cacheTheme } from "./theme";
import { AssistantNameContext, agentName } from "./agentName";
import { Backdrop } from "./components/Backdrop";
import type {
  AppData,
  AutoTitleConfig,
  AppearanceConfig,
  BrowserState,
  CommandsConfig,
  MemoryConfig,
  PromptConfig,
  CheckpointChange,
  CheckpointRef,
  ExtensionNotice,
  ExtensionUIRequest,
  ComputerAccessDecision,
  ComputerAccessRequest,
  ComputerUseConfig,
  DiffComment,
  GitChangeFile,
  GitChanges,
  GitCheckoutKind,
  GitCommit,
  GitDiffSection,
  GitTarget,
  ImageContent,
  McpServerRecord,
  NormalizedMessage,
  PackageRecord,
  PackageResourceKind,
  ProjectRecord,
  ProviderRecord,
  RestoreResult,
  SaveProviderInput,
  SlashCommand,
  SubagentConfig,
  SubagentTarget,
  SubscriptionLoginEvent,
  TaskMode,
  TaskRecord,
  TaskRuntime,
  TerminalEvent,
  ThinkingLevel,
  WorkerEvent
} from "./types";
import { ChangesPanel } from "./components/ChangesPanel";
import { BrowserPanel, NativeOverlaysHidden } from "./components/BrowserPanel";
import { GitChangesList, GitHistoryList, GitPanelTop } from "./components/GitSidebar";
import { GitCommitForm } from "./components/GitCommitForm";
import { GitCommitView } from "./components/GitCommitView";
import { GitToolbar } from "./components/GitToolbar";
import { GitActivity, GitNoticeCard, GitWorkspace } from "./components/GitWorkspace";
import { RepoSwitcher } from "./components/RepoSwitcher";
import { TerminalPanel } from "./components/TerminalPanel";
import { SidePanel } from "./components/SidePanel";
import { SubagentPanelLink } from "./components/SubagentChip";
import { ToolImageSource } from "./components/ToolRow";
import { ContextMenuProvider } from "./components/ui/ContextMenu";
import { CopyText } from "./components/ui/CopyButton";
import { SubagentPanel } from "./components/SubagentPanel";
import { ChatHeader } from "./components/ChatHeader";
import { Composer } from "./components/Composer";
import { Icon } from "./components/Icons";
import { DuckMark } from "./components/DuckMark";
import { ProjectBar } from "./components/ProjectBar";
import type { McpActions } from "./components/McpSection";
import { SettingsPage } from "./components/SettingsPage";
import { Sidebar, NO_PROJECT_KEY, type ProjectAction, type TaskAction } from "./components/Sidebar";
import { Transcript, type MessageAction } from "./components/Transcript";
import { RestoreDialog, type RestoreChoice } from "./components/RestoreDialog";
import { TodoPanel } from "./components/TodoPanel";
import { GoalBanner } from "./components/GoalBanner";
import { ComputerUseBanner } from "./components/ComputerUseBanner";
import { InlineDialog, type ExtensionUIResponse } from "./components/InlineDialog";
import type { PlanAction } from "./components/PlanCard";
import { ConfirmDialog } from "./components/ui/ConfirmDialog";
import { SubscriptionLoginDialog } from "./components/SubscriptionLoginDialog";
import { ExploreGroupingEnabled } from "./components/ExploreGroup";
import { ThinkingPreviewEnabled } from "./components/ThinkingRow";

const emptyData: AppData = {
  version: 1,
  providers: [],
  projects: [],
  tasks: [],
  diffComments: {},
  toolConfig: { disabled: [] },
  toolCatalog: [],
  packages: [],
  subagents: { enabled: false, trigger: "on_request", maxConcurrency: 4, agents: [] },
  autoTitle: { enabled: false, providerId: null, modelId: null },
  appearance: DEFAULT_APPEARANCE,
  prompts: {},
  mcp: { servers: [] }
};

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

const REVIEW_PROMPT = "Review all current uncommitted changes, including staged, unstaged, and untracked files. Delegate the review to the Reviewer sub-agent. Report its findings with file and line references. Do not make fixes or edit files.";
const NO_COMMENTS: DiffComment[] = [];

/** Pending diff comments as a prompt: fix them in Build, plan the fixes otherwise. */
function commentsPrompt(comments: DiffComment[], mode: TaskMode): string {
  const list = comments.map((comment) => `${comment.path}:${comment.line} (${comment.layer}, ${comment.side}; ${comment.revision}): ${comment.text}\nSource: ${comment.excerpt}`).join("\n\n");
  const instruction = mode === "build" ? "Address these diff comments in the workspace, then explain what changed." : "Plan how to address these diff comments. Keep the workspace read-only.";
  return `${instruction}\n\n${list}`;
}

const LAST_MODEL_KEY = "wackcode:lastModel";
const LAST_PROJECT_KEY = "wackcode:lastProject";
const NO_PROJECT_MODEL_KEY = "none";
/** Which durable view the side panel shows ("changes" | "terminal" | null = closed). */
const PANEL_VIEW_KEY = "wackcode:sidePanel";
/** The pre-Terminal flag, still read once to migrate it into `PANEL_VIEW_KEY`. */
const CHANGES_OPEN_KEY = "wackcode:changesOpen";
/** The side panel's width, named for its first view so existing widths carry over. */
const PANEL_WIDTH_KEY = "wackcode:changesWidth";
const COLLAPSED_PROJECTS_KEY = "wackcode:collapsedProjects";

/** The durable view the panel should come back to — the stored pick, or the migrated Changes flag. */
function rememberedPanelKind(): PanelViewKind | null {
  const stored = loadJSON<PanelViewKind | null | undefined>(PANEL_VIEW_KEY, undefined);
  if (stored === "changes" || stored === "terminal") return stored;
  if (stored === null) return null;
  return loadJSON<boolean>(CHANGES_OPEN_KEY, false) ? "changes" : null;
}

function rememberedPanelView(): SidePanelView | null {
  const kind = rememberedPanelKind();
  return kind ? durableView(kind) : null;
}

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

interface RestoreDialogState {
  title: string;
  body?: string;
  changes: CheckpointChange[];
  initialSelection: string[];
  sharedWith?: string;
  choices: RestoreChoice[];
  run: (choice: string, paths: string[]) => Promise<void>;
  /** Closed without a choice (or after one): settles whoever is waiting on the dialog. */
  onClose: () => void;
}

function plural(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

interface SubscriptionLoginState {
  loginId: string;
  providerId: string;
  prompt?: Extract<SubscriptionLoginEvent, { type: "prompt" }>;
  authUrl?: string;
  deviceCode?: { userCode: string; verificationUri: string };
  message?: string;
  error?: string;
}

/** What the chat and the side panel's sub-agent transcripts read from Settings › Appearance. */
function ChatContexts({ appearance, children }: { appearance: AppearanceConfig; children: ReactNode }) {
  return (
    <AssistantNameContext.Provider value={agentName(appearance)}>
      <ThinkingPreviewEnabled.Provider value={appearance.thinkingPreview}>
        <ExploreGroupingEnabled.Provider value={appearance.groupExploration}>
          <CopyText.Provider value={writeText}>{children}</CopyText.Provider>
        </ExploreGroupingEnabled.Provider>
      </ThinkingPreviewEnabled.Provider>
    </AssistantNameContext.Provider>
  );
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
  const [glassSupported, setGlassSupported] = useState(false);
  const [computerUseSupported, setComputerUseSupported] = useState(false);
  const savedAppearance = useRef<AppearanceConfig>(DEFAULT_APPEARANCE);
  const [selectedTaskId, setSelectedTaskId] = useState<string>();
  const selectTaskRef = useRef<(id: string) => void>(() => undefined);
  const selectedTaskRef = useRef<string | undefined>(undefined);
  const [runtimes, setRuntimes] = useState<Record<string, TaskRuntime>>({});
  const [changes, setChanges] = useState<GitChanges>();
  const [changesLoading, setChangesLoading] = useState(false);
  const changesRequest = useRef(0);
  const [sidePanel, setSidePanel] = useState<SidePanelView | null>(rememberedPanelView);
  const [panelWidth, setPanelWidth] = useState(() => loadJSON(PANEL_WIDTH_KEY, 430));
  const [browsers, setBrowsers] = useState<Record<string, BrowserState>>({});
  const [browserExpanded, setBrowserExpanded] = useState(false);
  const browserRestoreWidth = useRef(430);
  /** Live terminal sessions by chat id — powers the header's caret hint while the panel is hidden. */
  const [terminals, setTerminals] = useState<Record<string, { sessionId: string; busy: boolean; exit: { code: number; signal: string | null } | null }>>({});
  /** Bumped to watch the shown sub-agent again (a failed watch, a gap, a restarted worker). */
  const [watchNonce, setWatchNonce] = useState(0);
  /** The chat whose worker streams a sub-agent to the panel, so it can be told to stop. */
  const watchedTask = useRef<string | undefined>(undefined);
  /** Watch commands, one at a time: two racing to the worker could leave it on the wrong child. */
  const watchQueue = useRef<Promise<void>>(Promise.resolve());
  const [settingsOpen, setSettingsOpen] = useState(false);
  /** Arms the rebuild choreography on `.app-shell`: whichever screen just mounted assembles
      piece by piece. Cleared by the timer so later remounts don't replay it. */
  const [rebuilding, setRebuilding] = useState(false);
  const rebuildTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  /** The Archived view replacing the sidebar's chat list; closed again by its ✕ or footer tile. */
  const [archivedOpen, setArchivedOpen] = useState(false);
  const [collapsedProjects, setCollapsedProjects] = useState<ReadonlySet<string>>(() => new Set(loadJSON<string[]>(COLLAPSED_PROJECTS_KEY, [])));
  /** Projects pinned to the top of the sidebar and Git mode's repository switcher. */
  const [pinnedProjects, setPinnedProjects] = useState<ReadonlySet<string>>(() => new Set(loadJSON<string[]>(PINNED_PROJECTS_KEY, [])));
  /** The composer ducking out of Git mode's way ("away") and resurfacing ("back"). */
  const [composerPhase, setComposerPhase] = useState<"away" | "back">();
  const composerPhaseTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const [gitGenerating, setGitGenerating] = useState(false);
  /** Bumped to open Git mode's pull request form from the clean state's card. */
  const [gitPrRequest, setGitPrRequest] = useState(0);
  const [confirm, setConfirm] = useState<ConfirmState>();
  const [restoreDialog, setRestoreDialog] = useState<RestoreDialogState>();
  const composerDrafts = useComposerDrafts();
  /** Rewind requests are consumed once by their chat, including across selection changes. */
  const [composerSeed, setComposerSeed] = useState<{ taskId: string; text: string; nonce: number }>();
  const slashDraftPromise = useRef<Promise<TaskRecord | undefined> | undefined>(undefined);
  const draftEpoch = useRef(0);
  /** Taskless `/` catalog for the welcome composer, keyed by its selected project. */
  const [draftSlash, setDraftSlash] = useState<{ projectId: string | null; commands?: SlashCommand[]; loading: boolean; error?: string }>();
  const draftSlashRequest = useRef(0);
  const [extensionRequests, setExtensionRequests] = useState<ExtensionUIRequest[]>([]);
  /** Computer-use access cards the host raised, oldest first (may span chats). */
  const [accessRequests, setAccessRequests] = useState<ComputerAccessRequest[]>([]);
  const [booting, setBooting] = useState(true);
  const [globalError, setGlobalError] = useState<string>();
  const [subscriptionLogin, setSubscriptionLogin] = useState<SubscriptionLoginState>();
  const pendingSubscriptionCancel = useRef(false);
  const [connectedSubscriptionId, setConnectedSubscriptionId] = useState<string>();
  const [lastModels, setLastModels] = useState<Record<string, ModelChoice>>(() => loadJSON(LAST_MODEL_KEY, {}));
  const [draft, setDraft] = useState<Draft>();
  /** The file list behind `@` mentions, for one chat (`task:<id>`) or draft project (`project:<id>`). */
  const [mentions, setMentions] = useState<{ source: string; files?: string[]; truncated?: boolean; loading: boolean; error?: string }>();
  const mentionRequest = useRef(0);
  /** Mid first-send choreography: the hero is exiting while this message rides the composer down. */
  const [transitioning, setTransitioning] = useState<{ message: string; taskId?: string; fromSelectedId?: string }>();

  const selectedTask = data.tasks.find((task) => task.id === selectedTaskId);
  const composerDraftKey = selectedTask ? `task:${selectedTask.id}` : `new:${draftEpoch.current}`;
  const selectedProject = data.projects.find((project) => project.id === selectedTask?.projectId);
  const runtime = selectedTaskId ? runtimes[selectedTaskId] : undefined;
  const sharedWorkers = selectedTask ? data.tasks.filter((task) => !task.archived && task.id !== selectedTask.id && task.workspacePath === selectedTask.workspacePath && task.status === "running") : [];
  // The worker's latest plan_state is the freshest mode signal; the record (or the draft's
  // choice before a task exists) is the durable fallback.
  const currentMode: TaskMode = runtime?.planState?.mode ?? selectedTask?.mode ?? draft?.mode ?? "build";
  const pendingDialogTaskIds = useMemo(
    () => new Set([...extensionRequests.map((r) => r.taskId), ...accessRequests.map((r) => r.taskId)]),
    [extensionRequests, accessRequests]
  );
  const selectedBusy = selectedTask?.status === "running" || selectedTask?.status === "stopping";
  const git = useGitMode({ projects: data.projects, tasks: data.tasks, selectedTask, pinned: pinnedProjects });
  const gitMode = git.view;
  /** Read by long-lived listeners (worker events, window focus) that must not re-subscribe. */
  const gitRef = useRef(git);
  gitRef.current = git;
  const selectedModel = data.providers.find((provider) => provider.id === selectedTask?.providerId)?.models.find((model) => model.id === selectedTask?.modelId);
  /** The chat still opens and reads, but the composer's model pill says "Choose model" until one is picked. */
  const selectedModelGone = chatModelGone(data.providers, selectedTask, runtime?.snapshot);
  const displayModelSwitches = useMemo(() => (runtime?.snapshot?.modelSwitches ?? []).map((entry) => ({
    id: entry.id,
    at: entry.at,
    from: modelDisplayName(data.providers, entry.from),
    to: modelDisplayName(data.providers, entry.to)
  })), [runtime?.snapshot?.modelSwitches, data.providers]);

  // Task-bound views never show in another chat, even for the render before the effect below
  // moves the panel back to the remembered durable view.
  const panelView = (sidePanel?.kind === "subagent" || sidePanel?.kind === "browser") && sidePanel.taskId !== selectedTaskId
    ? rememberedPanelView()
    : sidePanel;
  const shownSubagent = panelView?.kind === "subagent" ? panelView : undefined;
  const snapshotMessages = runtime?.snapshot?.messages;
  const shownSubagentCall = useMemo(
    () => shownSubagent && snapshotMessages
      ? subagentDetailsFor(snapshotMessages, runtime?.partial, runtime?.liveToolDetails, shownSubagent.toolCallId, selectedBusy)
      : undefined,
    [shownSubagent, snapshotMessages, runtime?.partial, runtime?.liveToolDetails, selectedBusy]
  );

  const toggleChanges = useCallback(() => {
    if (browserExpanded) {
      setPanelWidth(browserRestoreWidth.current);
      setBrowserExpanded(false);
    }
    setSidePanel((current) => toggleView(current, CHANGES_VIEW));
  }, [browserExpanded]);
  const toggleBrowser = useCallback(() => {
    const taskId = selectedTaskRef.current;
    if (!taskId) return;
    if (sidePanel?.kind === "browser" && sidePanel.taskId === taskId) {
      if (browserExpanded) setPanelWidth(browserRestoreWidth.current);
      setBrowserExpanded(false);
      setSidePanel(null);
    } else {
      setSidePanel({ kind: "browser", taskId });
    }
  }, [sidePanel, browserExpanded]);
  const toggleTerminal = useCallback(() => {
    if (browserExpanded) {
      setPanelWidth(browserRestoreWidth.current);
      setBrowserExpanded(false);
    }
    setSidePanel((current) => toggleView(current, TERMINAL_VIEW));
  }, [browserExpanded]);
  const closeSidePanel = useCallback(() => setSidePanel(null), []);
  const updateBrowser = useCallback((state: BrowserState) => {
    setBrowsers((current) => current[state.taskId] === state ? current : { ...current, [state.taskId]: state });
  }, []);
  const toggleBrowserExpanded = useCallback(() => {
    setBrowserExpanded((expanded) => {
      if (expanded) {
        setPanelWidth(browserRestoreWidth.current);
      } else {
        browserRestoreWidth.current = panelWidth;
        setPanelWidth(Math.min(1200, Math.max(720, window.innerWidth - 560)));
      }
      return !expanded;
    });
  }, [panelWidth]);
  const openSubagent = useCallback((toolCallId: string, index: number) => {
    const taskId = selectedTaskRef.current;
    if (browserExpanded) {
      setPanelWidth(browserRestoreWidth.current);
      setBrowserExpanded(false);
    }
    if (taskId) setSidePanel((current) => toggleView(current, { kind: "subagent", taskId, toolCallId, index }));
  }, [browserExpanded]);
  const selectSubagentSibling = useCallback((index: number) => {
    setSidePanel((current) => current?.kind === "subagent" ? { ...current, index } : current);
  }, []);
  const subagentLink = useMemo(() => ({
    open: shownSubagent ? { toolCallId: shownSubagent.toolCallId, index: shownSubagent.index } : undefined,
    onOpen: openSubagent
  }), [shownSubagent?.toolCallId, shownSubagent?.index, openSubagent]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleExtensionRespond = useCallback((request: ExtensionUIRequest, response: ExtensionUIResponse) => {
    setExtensionRequests((current) => withoutResolvedDialog(current, request.requestId));
    void api.respondExtensionUi({ taskId: request.taskId, requestId: request.requestId, ...response })
      .catch((reason) => setGlobalError(String(reason)));
  }, []);

  /** Full-size screenshots for the transcript's lightbox, from the open chat's session. */
  const loadToolImage = useCallback((toolCallId: string, index: number) => {
    const taskId = selectedTaskRef.current;
    if (!taskId) return Promise.resolve(undefined);
    return api.toolImage(taskId, toolCallId, index).then((image) => image ? `data:${image.mimeType};base64,${image.data}` : undefined);
  }, []);

  const handleComputerAccess = useCallback((request: ComputerAccessRequest, decision: ComputerAccessDecision) => {
    setAccessRequests((current) => current.filter((entry) => entry.requestId !== request.requestId));
    void api.computerUseRespondAccess(request.taskId, request.requestId, decision)
      .then((config) => { if (config) setData((current) => ({ ...current, computerUse: config })); })
      .catch((reason) => setGlobalError(String(reason)));
  }, []);

  const reduce = useReducedMotion();

  /** Swapping screens always rebuilds: every open and close routes through these so the
      incoming screen plays its assembly once. */
  function armRebuild() {
    setRebuilding(true);
    clearTimeout(rebuildTimer.current);
    rebuildTimer.current = setTimeout(() => setRebuilding(false), 800);
  }
  function openSettings() {
    armRebuild();
    setSettingsOpen(true);
  }
  function closeSettings() {
    armRebuild();
    setSettingsOpen(false);
    // Skills, commands or package resources may have changed while Settings was open.
    setDraftSlash(undefined);
    setRuntimes((current) => Object.fromEntries(Object.entries(current).map(([id, value]) => [id, {
      ...value, slashCommands: undefined, slashCommandsError: undefined
    }])));
  }

  /** Git mode: the sidebar becomes the Git panel and the workspace a diff reader. The chat
      view and the composer stay mounted underneath (App keeps drafts by chat). */
  function openGit(request?: { projectId?: string; path?: string }) {
    if (settingsOpen) closeSettings();
    setArchivedOpen(false);
    clearTimeout(composerPhaseTimer.current);
    setComposerPhase("away");
    // From the new-chat screen, the project the composer is set to is the one in view.
    const draftProjectId = !selectedTask ? draft?.projectId ?? undefined : undefined;
    git.open({ ...request, projectId: request?.projectId ?? draftProjectId });
  }
  function closeGit() {
    if (!gitRef.current.view) return;
    git.close();
    clearTimeout(composerPhaseTimer.current);
    setComposerPhase("back");
    composerPhaseTimer.current = setTimeout(() => setComposerPhase(undefined), 700);
    // The chat's own Changes panel skipped refreshes while Git mode was open.
    void refreshChanges();
  }
  function toggleGit() {
    if (gitRef.current.view) closeGit(); else openGit();
  }
  /** Leave Git mode for one of its chats. */
  function openChatFromGit(taskId: string) {
    selectTask(taskId);
  }

  function selectTask(id: string) {
    performChatNavigation(id, {
      dismissGitMode: () => closeGit(),
      dismissSettings: () => { if (settingsOpen) closeSettings(); },
      abandonDraft: () => {
        if (!selectedTask) composerDrafts.remove(composerDraftKey);
        draftEpoch.current += 1;
        slashDraftPromise.current = undefined;
        setDraft(undefined);
      },
      selectTask: (taskId) => {
        selectedTaskRef.current = taskId;
        setSelectedTaskId(taskId);
      }
    });
  }
  selectTaskRef.current = selectTask;

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const consume = () => {
      void api.takeMenuNavigation()
        .then((taskId) => { if (taskId) selectTaskRef.current(taskId); })
        .catch((reason) => setGlobalError(String(reason)));
    };
    void listen<{ taskId: string }>("native-chat-navigation", consume).then((stop) => {
      unlisten = stop;
      consume();
    });
    return () => unlisten?.();
  }, []);

  useEffect(() => { selectedTaskRef.current = selectedTaskId; }, [selectedTaskId]);
  useEffect(() => {
    const remembered = rememberedView(sidePanel);
    if (remembered !== undefined) localStorage.setItem(PANEL_VIEW_KEY, JSON.stringify(remembered));
  }, [sidePanel]);
  useEffect(() => { localStorage.setItem(PANEL_WIDTH_KEY, JSON.stringify(panelWidth)); }, [panelWidth]);
  // Browser and sub-agent views belong to their chat: another chat opens with the remembered
  // durable view. Browser expansion also belongs to the chat being left.
  useEffect(() => {
    setSidePanel((current) => viewForChat(current, selectedTaskId, rememberedPanelKind()));
    if (browserExpanded) setPanelWidth(browserRestoreWidth.current);
    setBrowserExpanded(false);
  }, [selectedTaskId]);
  useEffect(() => { localStorage.setItem(COLLAPSED_PROJECTS_KEY, JSON.stringify([...collapsedProjects])); }, [collapsedProjects]);
  useEffect(() => { localStorage.setItem(PINNED_PROJECTS_KEY, JSON.stringify([...pinnedProjects])); }, [pinnedProjects]);
  useEffect(() => () => clearTimeout(composerPhaseTimer.current), []);
  // The Archived view closes itself once its last chat leaves (unarchived or deleted):
  // the footer tile that opens it is gone too, so an empty panel would be a dead end.
  useEffect(() => {
    if (archivedOpen && !data.tasks.some((task) => task.archived)) setArchivedOpen(false);
  }, [archivedOpen, data.tasks]);

  const toggleProjectCollapsed = useCallback((key: string) => {
    setCollapsedProjects((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key); else next.add(key);
      return next;
    });
  }, []);

  // Selecting a chat (directly or by creating one) must reveal its group, so a
  // collapsed project can never hide the conversation the user just opened.
  const selectedGroupKey = selectedTask ? selectedTask.projectId ?? NO_PROJECT_KEY : undefined;
  useEffect(() => {
    if (selectedGroupKey) {
      setCollapsedProjects((current) => current.has(selectedGroupKey)
        ? new Set([...current].filter((key) => key !== selectedGroupKey))
        : current);
    }
  }, [selectedGroupKey]);

  const patchTask = useCallback((taskId: string, patch: Partial<TaskRecord>) => {
    // Snapshots and run-state events patch tasks at a high cadence; an unchanged record keeps
    // its object so Sidebar and ChatHeader skip re-rendering.
    setData((current) => {
      const target = current.tasks.find((task) => task.id === taskId);
      if (!target) return current;
      const changed = Object.keys(patch).some((key) => target[key as keyof TaskRecord] !== (patch as Record<string, unknown>)[key]);
      if (!changed) return current;
      return { ...current, tasks: current.tasks.map((task) => task.id === taskId ? { ...task, ...patch } : task) };
    });
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

  // The banner reads the durable `lastError` as well as the live error, so dismissing
  // must clear both and persist, or the banner returns with the next launch.
  const dismissError = useCallback((taskId: string) => {
    patchRuntime(taskId, { error: undefined });
    patchTask(taskId, { lastError: null });
    api.clearTaskError(taskId).catch((reason) => setGlobalError(String(reason)));
  }, [patchRuntime, patchTask]);

  // Follow the shown sub-agent: its chat's worker streams its transcript into
  // `runtime.subagentView` until the panel moves on. Switching children re-targets the same
  // worker; leaving the chat or closing the panel tells it to stop.
  const watchKey = shownSubagent ? viewKey(shownSubagent) : undefined;
  useEffect(() => {
    const target = shownSubagent;
    const watch = (taskId: string, next: SubagentTarget | null) => {
      watchQueue.current = watchQueue.current
        .then(() => api.watchSubagent(taskId, next))
        .catch((reason) => {
          if (!next) return;
          setRuntimes((current) => {
            const view = current[taskId]?.subagentView;
            if (!view || view.toolCallId !== next.toolCallId || view.index !== next.index) return current;
            return { ...current, [taskId]: { ...current[taskId], subagentView: { ...view, loading: false, error: String(reason) } } };
          });
        });
    };
    const previous = watchedTask.current;
    watchedTask.current = target?.taskId;
    if (previous && previous !== target?.taskId) {
      watch(previous, null);
      patchRuntime(previous, { subagentView: undefined });
    }
    if (!target) return;
    const next = { toolCallId: target.toolCallId, index: target.index };
    patchRuntime(target.taskId, { subagentView: pendingSubagentView(next) });
    watch(target.taskId, next);
    // Keyed on the view (and the nonce), not the object, which is rebuilt for every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [watchKey, watchNonce, patchRuntime]);

  const resync = shownSubagent ? runtimes[shownSubagent.taskId]?.subagentView?.resync === true : false;
  useEffect(() => {
    if (resync) setWatchNonce((nonce) => nonce + 1);
  }, [resync]);

  // Terminal sessions live in Rust; these events keep the header's caret hint honest while the
  // panel is hidden or the chat is in the background. Events carry a sessionId: a restart's
  // late busy/exit events for the old shell are ignored.
  useEffect(() => {
    const unlisten = listen<TerminalEvent>("terminal-event", ({ payload }) => {
      if (payload.type === "terminal_closed") {
        setTerminals((current) => {
          const { [payload.taskId]: _removed, ...rest } = current;
          return rest;
        });
      } else if (payload.type === "terminal_started") {
        setTerminals((current) => ({ ...current, [payload.taskId]: { sessionId: payload.sessionId, busy: false, exit: null } }));
      } else {
        setTerminals((current) => {
          const session = current[payload.taskId];
          if (session?.sessionId !== payload.sessionId) return current;
          const next = payload.type === "terminal_busy"
            ? { ...session, busy: payload.busy }
            : { ...session, busy: false, exit: { code: payload.code, signal: payload.signal } };
          return { ...current, [payload.taskId]: next };
        });
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  // Rewinding past a call takes its chips away, and the panel follows.
  useEffect(() => {
    if (!shownSubagent || !snapshotMessages || hasSubagentCall(snapshotMessages, runtime?.partial, shownSubagent.toolCallId)) return;
    setSidePanel(rememberedPanelView());
  }, [shownSubagent, snapshotMessages, runtime?.partial]);

  const refreshChanges = useCallback(async (taskId = selectedTaskRef.current) => {
    if (!taskId) return;
    const request = ++changesRequest.current;
    setChangesLoading(true);
    try {
      const next = await api.gitChanges({ taskId });
      if (selectedTaskRef.current === taskId && changesRequest.current === request) setChanges(next);
    } catch (reason) {
      if (selectedTaskRef.current === taskId && changesRequest.current === request) setGlobalError(String(reason));
    } finally {
      if (selectedTaskRef.current === taskId && changesRequest.current === request) setChangesLoading(false);
    }
  }, []);

  useEffect(() => {
    let active = true;
    api.bootstrap().then((payload) => {
      if (!active) return;
      setData(payload.data);
      setAppDataPath(payload.appDataPath);
      setGlassSupported(payload.glassSupported);
      setComputerUseSupported(payload.computerUseSupported === true);
      savedAppearance.current = payload.data.appearance;
      const remembered = loadJSON<string | null>(LAST_PROJECT_KEY, null);
      const projectId = payload.data.projects.some((project) => project.id === remembered)
        ? remembered
        : payload.data.projects[0]?.id ?? null;
      setDraft({ projectId, useWorktree: false });
      if (payload.data.providers.length === 0) openSettings();
      // A Pi update brings new subscription models; pick them up without asking for a sign-in.
      // Only the list moves, and a failure keeps the last known one.
      void api.refreshSubscriptionModels().then((changed) => {
        if (!active || changed.length === 0) return;
        setData((current) => ({ ...current, providers: current.providers.map((provider) => {
          const fresh = changed.find((item) => item.id === provider.id);
          return fresh ? { ...provider, models: fresh.models } : provider;
        }) }));
      }).catch(() => undefined);
    }).catch((reason) => setGlobalError(String(reason))).finally(() => active && setBooting(false));
    return () => { active = false; };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<WorkerEvent>("worker-event", ({ payload }) => {
      const taskId = payload.taskId;
      if (!taskId) return;
      if (payload.type === "browser_state") {
        updateBrowser(payload.browser);
        if (payload.reveal && selectedTaskRef.current === taskId) {
          setSidePanel({ kind: "browser", taskId });
        }
        return;
      }
      // Usage is persisted by Rust, never accumulated in transcript state.
      if (payload.type === "usage_record") return;
      if (payload.type === "title_changed") {
        patchTask(taskId, { name: payload.name });
        return;
      }
      if (payload.type === "ready" || payload.type === "snapshot") {
        const snapshot = payload.snapshot;
        // Keep the previous plan/todo objects when they are unchanged: the transcript's message
        // memos compare them by identity, and a fresh object per snapshot would re-render the
        // whole transcript on every event.
        setRuntimes((current) => {
          const runtime = current[taskId];
          const planState = samePlanState(runtime?.planState, snapshot.planState) ? runtime?.planState : snapshot.planState;
          const todoState = sameTodoState(runtime?.todoState, snapshot.todoState) ? runtime?.todoState : snapshot.todoState;
          const goalState = sameGoalState(runtime?.goalState, snapshot.goalState) ? runtime?.goalState : snapshot.goalState;
          return {
            ...current,
            [taskId]: { ...runtime, snapshot, activeRun: snapshot.activeRun, planState, todoState, goalState, partial: undefined, error: undefined,
              // A fresh worker never has anything queued; queue_state events are authoritative after this.
              queued: undefined,
              ...(payload.type === "ready" ? {
                slashCommands: undefined,
                slashCommandsError: undefined,
                // Nor is it streaming a sub-agent: the panel watches again.
                ...(runtime?.subagentView ? { subagentView: { ...pendingSubagentView(runtime.subagentView), resync: true } } : {})
              } : {}) }
          };
        });
        if (snapshot.planState) patchTask(taskId, { mode: snapshot.planState.mode });
        if (snapshot.sessionFile) patchTask(taskId, { sessionFile: snapshot.sessionFile });
        const tools = snapshot.tools;
        if (tools) {
          setData((current) => sameToolCatalog(current.toolCatalog, tools) ? current : { ...current, toolCatalog: tools });
        }
      } else if (payload.type === "snapshot_delta") {
        setRuntimes((current) => {
          const runtime = current[taskId];
          const previous = runtime?.snapshot;
          // Deltas chain onto the last full snapshot or delta; a gap means a frame went
          // missing, so this one is dropped and the next full snapshot resynchronizes.
          if (!previous || previous.rev + 1 !== payload.delta.rev) return current;
          const snapshot = applySnapshotDelta(previous, payload.delta);
          return {
            ...current,
            [taskId]: { ...runtime, snapshot, activeRun: snapshot.activeRun, planState: snapshot.planState, todoState: snapshot.todoState, goalState: snapshot.goalState, partial: undefined, error: undefined }
          };
        });
        const delta = payload.delta;
        if (delta.planState) patchTask(taskId, { mode: delta.planState.mode });
        if (delta.sessionFile) patchTask(taskId, { sessionFile: delta.sessionFile });
      } else if (payload.type === "partial") {
        patchRuntime(taskId, { partial: payload.message });
      } else if (payload.type === "queue_state") {
        patchRuntime(taskId, { queued: { steer: [...payload.steering], followUp: [...payload.followUp] } });
      } else if (payload.type === "run_state") {
        // A run starting makes the chat the most recently active, which Git mode's linked
        // chat follows. Only "running" clears the saved error — the host mirrors exactly
        // that in wackcode.json, and clearing more here would desync the two.
        patchTask(taskId, { status: payload.state, ...(payload.state === "running" ? { lastError: null, updatedAt: new Date().toISOString() } : {}) });
        if (payload.state === "idle" || payload.state === "interrupted") gitRef.current.onWorkerActivity(taskId);
        if (payload.state === "running") {
          setRuntimes((current) => ({
            ...current,
            [taskId]: {
              ...current[taskId],
              activeRun: { runId: payload.runId, startedAt: payload.startedAt ?? current[taskId]?.activeRun?.startedAt ?? Date.now() },
              activity: undefined,
              liveToolText: {},
              liveToolDetails: {}
            }
          }));
        } else if (payload.state === "idle" || payload.state === "interrupted") {
          patchRuntime(taskId, { activeRun: undefined, activity: undefined, liveToolText: {}, liveToolDetails: {} });
        }
      } else if (payload.type === "run_finished") {
        // The native menu owns recent-run outcomes; the transcript already reflects the result.
      } else if (payload.type === "activity") {
        patchRuntime(taskId, { activity: payload.event });
        const callId = payload.detail?.toolCallId;
        const liveText = payload.detail?.text;
        const liveDetails = payload.detail?.details;
        if (typeof callId === "string" && callId) {
          if (payload.event === "tool_execution_update" && typeof liveText === "string") {
            setRuntimes((current) => ({
              ...current,
              [taskId]: {
                ...current[taskId],
                liveToolText: { ...current[taskId]?.liveToolText, [callId]: liveText },
                // Kept until the run ends rather than dropped at tool_execution_end: the final
                // result reaches the transcript a snapshot later, and the card must not blink.
                ...(liveDetails !== undefined
                  ? { liveToolDetails: { ...current[taskId]?.liveToolDetails, [callId]: liveDetails } }
                  : {})
              }
            }));
          } else if (payload.event === "tool_execution_end") {
            setRuntimes((current) => {
              const next = { ...current[taskId]?.liveToolText };
              delete next[callId];
              return { ...current, [taskId]: { ...current[taskId], liveToolText: next } };
            });
          }
        }
        if (payload.event === "tool_execution_end") {
          // Git mode reads its own project's changes; the chat's panel catches up on exit.
          if (gitRef.current.view) gitRef.current.onWorkerActivity(taskId);
          // Only the selected chat's panel: a background chat's refresh would bump the request
          // counter and set loading without ever clearing it, and selecting that chat later
          // refreshes changes anyway.
          else if (taskId === selectedTaskRef.current) void refreshChanges(taskId);
        }
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
      } else if (payload.type === "todo_state") {
        patchRuntime(taskId, { todoState: { tasks: payload.tasks } });
      } else if (payload.type === "goal_state") {
        patchRuntime(taskId, { goalState: payload.goal ?? undefined });
      } else if (payload.type === "subagent_stream") {
        // Only the watched child streams. A frame that doesn't apply leaves state untouched, so
        // nothing re-renders; one after a gap asks for a fresh reset.
        setRuntimes((current) => {
          const view = current[taskId]?.subagentView;
          if (!view) return current;
          const next = applySubagentFrame(view, payload);
          if (next === view) return current;
          return { ...current, [taskId]: { ...current[taskId], subagentView: next ?? { ...pendingSubagentView(view), resync: true } } };
        });
      } else if (payload.type === "extension_ui_request") {
        setExtensionRequests((current) => [...current, payload]);
      } else if (payload.type === "extension_ui_resolved") {
        setExtensionRequests((current) => withoutResolvedDialog(current, payload.requestId));
      } else if (payload.type === "computer_access_request") {
        setAccessRequests((current) => [...current.filter((entry) => entry.requestId !== payload.requestId), payload]);
      } else if (payload.type === "computer_access_resolved") {
        setAccessRequests((current) => current.filter((entry) => entry.requestId !== payload.requestId));
      } else if (payload.type === "computer_state") {
        patchRuntime(taskId, { computer: payload.computer.active ? payload.computer : undefined });
      } else if (payload.type === "extension_notice") {
        appendNotice(taskId, { message: payload.message, level: payload.level });
      } else if (payload.type === "extensions_loaded") {
        // Load failures are surfaced but never mark the chat as failed.
        for (const entry of payload.errors) {
          appendNotice(taskId, { message: `Extension failed to load (${entry.path}): ${entry.error}`, level: "warning" });
        }
      } else if (payload.type === "checkpoint_unavailable") {
        appendNotice(taskId, { message: `File checkpoints are off for this chat: ${payload.message}`, level: "warning" });
      } else if (payload.type === "response" && !payload.success && payload.error) {
        patchRuntime(taskId, { error: payload.error });
      }
    }).then((stop) => { unlisten = stop; });
    return () => unlisten?.();
  }, [patchTask, patchRuntime, refreshChanges, appendNotice, updateBrowser]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<SubscriptionLoginEvent>("subscription-login-event", ({ payload }) => {
      if (payload.type === "complete") {
        setData((current) => ({ ...current, providers: current.providers.some((provider) => provider.id === payload.provider.id)
          ? current.providers.map((provider) => provider.id === payload.provider.id ? payload.provider : provider)
          : [...current.providers, payload.provider] }));
        setConnectedSubscriptionId(payload.provider.id);
        setSubscriptionLogin(undefined);
      } else if (payload.type === "cancelled") {
        setSubscriptionLogin(undefined);
      } else if (payload.type === "auth_url" || payload.type === "device_code") {
        const url = payload.type === "auth_url" ? payload.url : payload.verificationUri;
        setSubscriptionLogin((current) => current && current.providerId === payload.providerId ? {
          ...current, loginId: payload.loginId, prompt: undefined,
          authUrl: url,
          deviceCode: payload.type === "device_code" ? { userCode: payload.userCode, verificationUri: payload.verificationUri } : current.deviceCode,
          message: payload.type === "auth_url" ? payload.instructions : current.message
        } : current);
        void api.openSubscriptionAuthUrl(url).catch((reason) => setSubscriptionLogin((current) => current ? { ...current, error: String(reason) } : current));
      } else if (payload.type === "prompt") {
        setSubscriptionLogin((current) => current && current.providerId === payload.providerId
          ? { ...current, loginId: payload.loginId, prompt: payload, error: undefined } : current);
      } else if (payload.type === "info" || payload.type === "progress") {
        setSubscriptionLogin((current) => current && current.providerId === payload.providerId
          ? { ...current, loginId: payload.loginId, message: payload.message } : current);
      } else if (payload.type === "error") {
        setSubscriptionLogin((current) => current && current.providerId === payload.providerId
          ? { ...current, loginId: payload.loginId, error: payload.message, prompt: undefined } : current);
      }
    }).then((stop) => { unlisten = stop; });
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    setChanges(undefined);
    if (!selectedTaskId) return;
    void refreshChanges(selectedTaskId);
    api.openTask(selectedTaskId).catch((reason) => {
      patchRuntime(selectedTaskId, { error: String(reason) });
    });
  }, [selectedTaskId, refreshChanges, patchRuntime]);

  // Clears the first-send handoff: the frozen text leaves the docked composer once the
  // message exists in the transcript, or once the entrance has certainly finished.
  useEffect(() => {
    const taskId = transitioning?.taskId;
    if (!transitioning) return;
    // Selection landed on a different chat, so the handoff is off — but only once it has
    // actually moved. Between attaching the task id and selection switching (after the prompt
    // is out), selectedTaskId still holds the value from before the send; clearing there would
    // unfreeze the hero mid-flight and let a quick second send create a second chat.
    if (taskId && selectedTaskId !== taskId && selectedTaskId !== transitioning.fromSelectedId) {
      setTransitioning(undefined);
      return;
    }
    if (!taskId || selectedTaskId !== taskId) return;
    const arrived = (runtimes[taskId]?.snapshot?.messages ?? []).some((message) => message.role === "user");
    if (arrived) {
      setTransitioning(undefined);
      return;
    }
    const timeout = setTimeout(() => setTransitioning(undefined), 900);
    return () => clearTimeout(timeout);
  }, [transitioning, selectedTaskId, runtimes]);

  useEffect(() => {
    // Focus refreshes what's on screen, never fetches: Git mode's network stays user-driven.
    const refresh = () => {
      if (gitRef.current.view) gitRef.current.scheduleRefresh();
      else void refreshChanges();
    };
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [refreshChanges]);

  const configuredProviders = useMemo(() => data.providers.filter((item) => item.enabled !== false && item.connected && item.models.some(modelIsReady)), [data.providers]);
  // WackCode's own commands minus the ones switched off in Settings › Commands; the worker
  // filters the rest of the catalog itself.
  const enabledAppCommands = useMemo(() => {
    const disabled = new Set(data.commands?.disabled ?? []);
    return APP_SLASH_COMMANDS.filter((command) => !disabled.has(command.id));
  }, [data.commands]);
  // Settings' commands section reports every saved config, keeping `data.commands` (and so the
  // composers) in step. Stable: the section rescans when this changes identity.
  const commandsChanged = useCallback(
    (config: CommandsConfig) => {
      setData((current) => ({ ...current, commands: config }));
      setDraftSlash(undefined);
    },
    []
  );

  async function connectSubscription(providerId: string) {
    pendingSubscriptionCancel.current = false;
    setSubscriptionLogin({ loginId: "", providerId, message: "Starting sign-in…" });
    try {
      const started = await api.startSubscriptionLogin(providerId);
      setData((current) => ({ ...current, providers: current.providers.some((provider) => provider.id === providerId)
        ? current.providers.map((provider) => provider.id === providerId && !provider.connected ? started.provider : provider)
        : [...current.providers, started.provider] }));
      if (pendingSubscriptionCancel.current) {
        await api.cancelSubscriptionLogin(started.loginId);
        return;
      }
      setSubscriptionLogin((current) => current && current.providerId === providerId ? { ...current, loginId: started.loginId } : current);
    } catch (reason) {
      setSubscriptionLogin((current) => current && current.providerId === providerId ? { ...current, error: String(reason) } : current);
    }
  }

  async function signOutSubscription(providerId: string) {
    const provider = await api.signOutSubscription(providerId);
    setData((current) => ({ ...current, providers: current.providers.map((item) => item.id === providerId ? provider : item) }));
  }

  const rememberModel = useCallback((projectId: string | null, choice: ModelChoice) => {
    setLastModels((current) => {
      const next = { ...current, [projectId ?? NO_PROJECT_MODEL_KEY]: choice };
      localStorage.setItem(LAST_MODEL_KEY, JSON.stringify(next));
      return next;
    });
  }, []);

  const defaultChoice = useCallback((projectId: string | null): ModelChoice | undefined => {
    const remembered = lastModels[projectId ?? NO_PROJECT_MODEL_KEY];
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
    if (!selectedTask) composerDrafts.remove(composerDraftKey);
    draftEpoch.current += 1;
    slashDraftPromise.current = undefined;
    const resolved = projectId === undefined ? lastProjectId() : projectId;
    selectedTaskRef.current = undefined;
    setSelectedTaskId(undefined);
    setDraft({ projectId: resolved, useWorktree: false });
  }

  async function loadSlashCommands(taskId: string) {
    patchRuntime(taskId, { slashCommandsLoading: true, slashCommandsError: undefined });
    try {
      const commands = await api.listCommands(taskId);
      patchRuntime(taskId, { slashCommands: commands, slashCommandsLoading: false });
    } catch (reason) {
      patchRuntime(taskId, { slashCommandsLoading: false, slashCommandsError: String(reason) });
    }
  }

  /** Creates the chat a hero send needs, once, at send time — a bare keystroke never does.
   *  Concurrent sends reuse the in-flight promise; the epoch guard drops the task if the user
   *  left the hero meanwhile. Resolves undefined when no chat could be prepared (the reason is
   *  already on screen: settings opened for a missing model, or the error banner). */
  function prepareSlashDraft(): Promise<TaskRecord | undefined> {
    if (slashDraftPromise.current) return slashDraftPromise.current;
    if (selectedTask) return Promise.resolve(selectedTask);
    const active = draft ?? { projectId: lastProjectId(), useWorktree: false };
    const choice = active.choice ?? defaultChoice(active.projectId);
    if (!choice) { openSettings(); return Promise.resolve(undefined); }
    const epoch = draftEpoch.current;
    const pending = (async () => { try {
      let task = await api.createTask({
        projectId: active.projectId,
        useWorktree: active.useWorktree && Boolean(draftProject?.gitHasHead),
        name: "New chat",
        ...choice
      });
      if (active.mode && active.mode !== "build") {
        try { task = await api.setTaskMode(task.id, active.mode); }
        catch (reason) { await api.deleteTask(task.id); throw reason; }
      }
      if (epoch !== draftEpoch.current || selectedTaskRef.current) {
        await api.deleteTask(task.id);
        return undefined;
      }
      setData((current) => ({ ...current, tasks: [...current.tasks, task] }));
      composerDrafts.update(`task:${task.id}`, composerDrafts.get(composerDraftKey));
      setSelectedTaskId(task.id);
      setDraft(undefined);
      void loadSlashCommands(task.id);
      return task;
    } catch (reason) {
      setGlobalError(String(reason));
      slashDraftPromise.current = undefined;
      return undefined;
    } })();
    slashDraftPromise.current = pending;
    return pending;
  }

  function mentionSource(): { source?: string; taskId?: string; projectId?: string } {
    if (selectedTask) return { source: `task:${selectedTask.id}`, taskId: selectedTask.id };
    const projectId = (draft ?? { projectId: lastProjectId() }).projectId;
    return projectId ? { source: `project:${projectId}`, projectId } : {};
  }

  async function requestMentions() {
    const { source, taskId, projectId } = mentionSource();
    const request = ++mentionRequest.current;
    if (!source) {
      setMentions({ source: "", loading: false, error: "Pick a project to mention its files." });
      return;
    }
    // Keep the last list for this source on screen while it refreshes.
    setMentions((current) => current?.source === source && current.files ? { ...current, loading: true, error: undefined } : { source, loading: true });
    try {
      const result = await api.listWorkspaceFiles(taskId, projectId);
      if (request === mentionRequest.current) setMentions({ source, files: result.files, truncated: result.truncated, loading: false });
    } catch (reason) {
      if (request === mentionRequest.current) setMentions({ source, loading: false, error: String(reason) });
    }
  }

  async function loadDraftSlashCommands(projectId: string | null) {
    const request = ++draftSlashRequest.current;
    setDraftSlash({ projectId, loading: true });
    try {
      const commands = await api.listDraftCommands(projectId);
      if (request === draftSlashRequest.current) setDraftSlash({ projectId, commands, loading: false });
    } catch (reason) {
      if (request === draftSlashRequest.current) setDraftSlash({ projectId, loading: false, error: String(reason) });
    }
  }

  function requestSlashCommands() {
    if (selectedTask) {
      void loadSlashCommands(selectedTask.id);
      return;
    }
    // This keyless scan loads the same explicit resources as a worker without creating a chat,
    // touching a provider, or enabling project-local package discovery.
    void loadDraftSlashCommands(draft?.projectId ?? null);
  }

  async function sendSlash(name: string, args: string, images: ImageContent[]): Promise<boolean> {
    // Capture the taskless entry before prepareSlashDraft switches the view to the new chat.
    const draftCommand = !selectedTask && draftSlash?.projectId === (draft?.projectId ?? null)
      ? draftSlash.commands?.find((entry) => entry.name === name)
      : undefined;
    // `/goal pause|resume|clear` drive a live loop, so they are the one app command that is
    // allowed through while the chat is busy — the Composer mirrors this gate.
    const goalControlAction = name === "goal" && /^(pause|resume|clear)$/.test(args.trim()) ? args.trim() as "pause" | "resume" | "clear" : undefined;
    if ((name === "new" || name === "copy") && args.trim()) {
      throw new Error(`/${name} does not accept arguments.`);
    }
    if (name === "goal") {
      const words = args.trim().split(/\s+/).filter(Boolean);
      if (["pause", "resume", "clear"].includes(words[0] ?? "") && words.length > 1) {
        throw new Error(`/goal ${words[0]} takes no arguments.`);
      }
      if (!args.trim()) throw new Error("Describe the goal — /goal <objective>.");
      if (isPlanMode(currentMode)) throw new Error("Goal loops don't run while a planning mode is on. Switch to Build first.");
    }
    if (name === "init") validateInitCommand(args, selectedTask?.projectId ?? draft?.projectId ?? null, currentMode);
    // Commands that act on the current chat have nothing to act on from the draft hero and must
    // not create one; /new just resets the draft. A command that starts work (/init, /goal,
    // a skill, template or extension command) creates the chat here — keystrokes never do.
    if (!selectedTask) {
      if (name === "new") { openDraft(); return true; }
      if (name === "name") throw new Error("There is no chat to rename yet — send a message first.");
      if (name === "copy") throw new Error("There is no reply to copy yet.");
      if (name === "compact") throw new Error("There is no conversation to compact yet.");
      if (goalControlAction) throw new Error(`There is no goal to ${goalControlAction} yet — start one with /goal <objective>.`);
    }
    const task = selectedTask ?? await prepareSlashDraft();
    if (!task) return false; // prepareSlashDraft already said why (settings opened, or the error banner).
    if (selectedBusy && !goalControlAction) throw new Error("Wait for this chat to be ready before running a command.");
    const id = task.id;
    const clearPreparedDraft = () => {
      if (!selectedTask) composerDrafts.update(`task:${id}`, EMPTY_DRAFT);
    };
    if (goalControlAction) {
      await api.goalControl(id, goalControlAction);
      clearPreparedDraft();
      return true;
    }
    if (name === "name" && !args.trim()) throw new Error("Enter a name after /name.");
    try {
      if (name === "new") { openDraft(task.projectId); return true; }
      if (name === "name") {
        const updated = await api.renameTask(id, args.trim());
        patchTask(id, updated);
        appendNotice(id, { message: "Chat renamed.", level: "info" });
        clearPreparedDraft();
        return true;
      }
      if (name === "copy") {
        const assistant = [...(runtime?.snapshot?.messages ?? [])].reverse().find((message) => message.role === "assistant");
        const content = assistant?.blocks.filter((block) => block.type === "text").map((block) => block.text ?? "").join("\n").trim();
        if (!content) throw new Error("There is no assistant message to copy.");
        await writeText(content);
        appendNotice(id, { message: "Assistant message copied.", level: "info" });
        clearPreparedDraft();
        return true;
      }
      const startedAt = Date.now();
      patchTask(id, { status: "running", lastError: null });
      patchRuntime(id, { error: undefined, activity: "starting", activeRun: { startedAt }, slashCommandsError: undefined });
      if (name === "compact") await api.compactTask(id, args, startedAt);
      else if (name === "init") await api.initAgents(id, startedAt);
      else if (name === "goal") await api.goalControl(id, "set", args.trim(), startedAt);
      else {
        const command = runtime?.slashCommands?.find((entry) => entry.name === name) ?? draftCommand;
        if (!command) throw new Error("That command changed. Open the command list and try again.");
        await api.executeCommand({ taskId: id, commandId: command.id, args, startedAt, images });
      }
      clearPreparedDraft();
      return true;
    } catch (reason) {
      patchTask(id, { status: "idle" });
      patchRuntime(id, { activeRun: undefined, slashCommandsError: String(reason) });
      throw reason;
    }
  }

  function setDraftProject(projectId: string | null) {
    localStorage.setItem(LAST_PROJECT_KEY, JSON.stringify(projectId));
    setDraft((current) => ({ projectId, useWorktree: false, choice: current?.choice, mode: current?.mode }));
  }

  function setDraftWorktree(useWorktree: boolean) {
    setDraft((current) => ({ projectId: current?.projectId ?? null, useWorktree, choice: current?.choice, mode: current?.mode }));
  }

  /** Lists a project's branches, relabelling it if its checkout moved since launch. */
  async function listProjectBranches(projectId: string) {
    const result = await api.gitBranches({ projectId });
    setData((current) => ({ ...current, projects: current.projects.map((item) => item.id === projectId && item.branch !== result.current ? { ...item, branch: result.current } : item) }));
    return result;
  }

  /** Switches a checkout's branch; every chat and project in that checkout takes the new label. */
  async function checkoutBranch(target: { taskId: string } | { projectId: string }, name: string, kind: GitCheckoutKind) {
    const result = await api.gitCheckout(target, name, kind);
    const tasks = new Set(result.taskIds);
    const projects = new Set(result.projectIds);
    setData((current) => ({
      ...current,
      tasks: current.tasks.map((task) => tasks.has(task.id) ? { ...task, branch: result.branch } : task),
      projects: current.projects.map((project) => projects.has(project.id) ? { ...project, branch: result.branch } : project)
    }));
    void refreshChanges();
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

  async function setComputerUse(config: ComputerUseConfig) {
    const previous = data.computerUse;
    setData((current) => ({ ...current, computerUse: config }));
    try {
      const saved = await api.setComputerUseConfig(config);
      setData((current) => ({ ...current, computerUse: saved }));
    } catch (reason) {
      setData((current) => ({ ...current, computerUse: previous }));
      throw reason;
    }
  }

  async function setSubagents(config: SubagentConfig) {
    const previous = data.subagents;
    setData((current) => ({ ...current, subagents: config }));
    try {
      const saved = await api.setSubagentConfig(config);
      setData((current) => ({ ...current, subagents: saved }));
    } catch (reason) {
      setData((current) => ({ ...current, subagents: previous }));
      throw reason;
    }
  }

  async function setMemory(config: MemoryConfig) {
    const previous = data.memory;
    setData((current) => ({ ...current, memory: config }));
    try {
      const saved = await api.setMemoryConfig(config);
      setData((current) => ({ ...current, memory: saved }));
    } catch (reason) {
      setData((current) => ({ ...current, memory: previous }));
      throw reason;
    }
  }

  /** Replace (or add) one MCP server as the host saved it. */
  function putMcpServer(server: McpServerRecord) {
    setData((current) => ({
      ...current,
      mcp: {
        servers: current.mcp.servers.some((item) => item.id === server.id)
          ? current.mcp.servers.map((item) => item.id === server.id ? server : item)
          : [...current.mcp.servers, server]
      }
    }));
  }

  /** Apply a switch at once, and put the server back if the host refuses. */
  async function patchMcpServer(serverId: string, patch: Partial<McpServerRecord>, save: () => Promise<McpServerRecord>) {
    const previous = data.mcp.servers.find((server) => server.id === serverId);
    if (previous) putMcpServer({ ...previous, ...patch });
    try {
      putMcpServer(await save());
    } catch (reason) {
      if (previous) putMcpServer(previous);
      throw reason;
    }
  }

  const mcpActions: McpActions = {
    onSaveMcpServer: async (input) => {
      const saved = await api.saveMcpServer(input);
      putMcpServer(saved);
      return saved;
    },
    onDeleteMcpServer: async (serverId) => {
      await api.deleteMcpServer(serverId);
      setData((current) => ({ ...current, mcp: { servers: current.mcp.servers.filter((server) => server.id !== serverId) } }));
    },
    onSetMcpServerEnabled: (serverId, enabled) =>
      patchMcpServer(serverId, { enabled }, () => api.setMcpServerEnabled(serverId, enabled)),
    onSetMcpServerTools: (serverId, disabledTools) =>
      patchMcpServer(serverId, { disabledTools }, () => api.setMcpServerTools(serverId, disabledTools)),
    onTestMcpServer: async (serverId) => {
      const result = await api.testMcpServer(serverId);
      putMcpServer(result.server);
      return result;
    }
  };

  async function setAutoTitle(config: AutoTitleConfig) {
    const saved = await api.setAutoTitleConfig(config);
    setData((current) => ({ ...current, autoTitle: saved }));
  }

  // The theme is applied to <html>, outside React. Not while booting: main.tsx already painted
  // the cached theme, and the placeholder defaults would flash over it.
  useEffect(() => {
    if (booting) return;
    // Image mode without a stored image (it failed to import) looks like Solid, not a hole.
    const appearance = data.appearance.backdrop === "image" && !data.appearance.backgroundImage ? { ...data.appearance, backdrop: "solid" as const } : data.appearance;
    applyTheme(appearance);
    cacheTheme(appearance);
  }, [booting, data.appearance]);

  /** Live preview while a colour picker or slider is being dragged; nothing is saved. */
  function previewAppearance(config: AppearanceConfig) {
    setData((current) => ({ ...current, appearance: config }));
  }

  async function setAppearance(config: AppearanceConfig) {
    setData((current) => ({ ...current, appearance: config }));
    try {
      const saved = await api.setAppearanceConfig(config);
      savedAppearance.current = saved;
      setData((current) => ({ ...current, appearance: saved }));
    } catch (reason) {
      // Back to what is stored, not to an unsaved preview.
      setData((current) => ({ ...current, appearance: savedAppearance.current }));
      throw reason;
    }
  }

  /** Rust opens the picker and stores the copy; a cancelled picker changes nothing. */
  async function chooseBackgroundImage() {
    const saved = await api.chooseBackgroundImage();
    if (!saved) return;
    savedAppearance.current = saved;
    setData((current) => ({ ...current, appearance: saved }));
  }

  async function removeBackgroundImage() {
    const saved = await api.removeBackgroundImage();
    savedAppearance.current = saved;
    setData((current) => ({ ...current, appearance: saved }));
  }

  async function setPrompts(config: PromptConfig) {
    const previous = data.prompts;
    setData((current) => ({ ...current, prompts: config }));
    try {
      const saved = await api.setPromptConfig(config);
      setData((current) => ({ ...current, prompts: saved }));
    } catch (reason) {
      setData((current) => ({ ...current, prompts: previous }));
      throw reason;
    }
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

  const installPackage = useCallback(
    (source: string, options?: { skillsOnly?: boolean }) => runPackageAction(() => api.installPackage(source, true, options?.skillsOnly ?? false)),
    [runPackageAction]
  );
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

  async function setProviderEnabled(providerId: string, enabled: boolean) {
    const provider = await api.setProviderEnabled(providerId, enabled);
    setData((current) => ({ ...current, providers: current.providers.map((item) => item.id === providerId ? provider : item) }));
  }

  async function configure(patch: Partial<Pick<TaskRecord, "providerId" | "modelId" | "thinkingLevel">>) {
    if (!selectedTask) return;
    const providerId = patch.providerId ?? selectedTask.providerId;
    const nextProvider = data.providers.find((item) => item.id === providerId);
    const modelId = patch.modelId ?? (patch.providerId ? nextProvider?.models.find(modelIsReady)?.id : selectedTask.modelId) ?? "";
    const nextModel = nextProvider?.models.find((item) => item.id === modelId);
    const thinkingLevel = pickThinkingLevel(nextModel, patch.thinkingLevel, selectedTask.thinkingLevel);
    const modelChanged = providerId !== selectedTask.providerId || modelId !== selectedTask.modelId;
    try {
      const updated = await api.configureTask({ taskId: selectedTask.id, providerId, modelId, thinkingLevel });
      setData((current) => ({ ...current, tasks: current.tasks.map((task) => task.id === updated.id ? updated : task) }));
      rememberModel(updated.projectId, { providerId, modelId, thinkingLevel });
      // Refresh the saved snapshot's model metadata after a switch. The durable transcript
      // divider is recorded when the next worker starts; viewing history stays read-only.
      if (modelChanged) await api.openTask(updated.id);
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
      const thinkingLevel = pickThinkingLevel(model, patch.thinkingLevel, base?.thinkingLevel);
      if (!providerId || !modelId || !thinkingLevel) return current;
      return { projectId: current?.projectId ?? null, useWorktree: current?.useWorktree ?? false, choice: { providerId, modelId, thinkingLevel }, mode: current?.mode };
    });
  }

  async function sendPrompt(message: string, options: { images?: ImageContent[]; files?: FileAttachment[]; mode?: TaskMode; literal?: boolean; queue?: "steer" | "follow_up" } = {}): Promise<boolean> {
    const { images, files = [], mode: modeOverride, literal, queue } = options;
    // Attached text files travel inside the message text; the composer's own words stay `message`
    // (the frozen hand-off and the chat's stand-in title show those).
    const sent = composeFileSection(message, files);
    if (!selectedTask) {
      const active = draft ?? { projectId: lastProjectId(), useWorktree: false };
      const choice = active.choice ?? defaultChoice(active.projectId);
      if (!choice) {
        openSettings();
        return false;
      }
      const startedAt = Date.now();
      // Freezing the composer with the sent text before the task exists keeps the hero
      // from flashing an empty draft while createTask is in flight.
      setTransitioning({ message });
      let task: TaskRecord;
      const pendingSlash = slashDraftPromise.current;
      try {
        if (pendingSlash) {
          const prepared = await pendingSlash;
          if (!prepared) {
            setTransitioning(undefined);
            return false;
          }
          task = prepared;
        } else {
          task = await api.createTask({
            projectId: active.projectId,
            useWorktree: active.useWorktree && Boolean(draftProject?.gitHasHead),
            name: titleFromPrompt(message),
            ...choice
          });
        }
      } catch (reason) {
        setTransitioning(undefined);
        setGlobalError(String(reason));
        return false;
      }
      // `fromSelectedId` is the selection the handoff began under: the freeze holds until
      // selection switches to the new task (after api.prompt resolves) or truly moves away.
      setTransitioning({ message, taskId: task.id, fromSelectedId: selectedTaskId });
      rememberModel(active.projectId, choice);
      localStorage.setItem(LAST_PROJECT_KEY, JSON.stringify(active.projectId));
      const mode = modeOverride ?? active.mode ?? "build";
      if (!pendingSlash) setData((current) => ({ ...current, tasks: [...current.tasks, task] }));
      patchTask(task.id, { status: "running", lastError: null, mode });
      patchRuntime(task.id, { error: undefined, activity: "starting", activeRun: { startedAt } });
      try {
        await api.prompt({
          taskId: task.id,
          message: sent,
          startedAt,
          providerId: task.providerId,
          modelId: task.modelId,
          thinkingLevel: task.thinkingLevel,
          mode,
          images,
          literal
        });
      } catch (reason) {
        // Stay on the hero; returning false restores the draft in the composer.
        setTransitioning(undefined);
        patchTask(task.id, { status: "idle" });
        patchRuntime(task.id, { error: String(reason), activeRun: undefined });
        return false;
      }
      // Selection happens after api.prompt so open_task's ensure_worker finds the
      // already-running worker instead of racing it to spawn a second process.
      setSelectedTaskId(task.id);
      setDraft(undefined);
      if (pendingSlash) composerDrafts.update(`task:${task.id}`, EMPTY_DRAFT);
      return true;
    }
    return promptTask(selectedTask, sent, { images, mode: modeOverride, literal, queue });
  }

  /**
   * Send an already-composed message to an existing chat, selected or not (Git mode prompts its
   * linked chat without leaving). The mode defaults to the chat's own.
   */
  async function promptTask(task: TaskRecord, sent: string, options: { images?: ImageContent[]; mode?: TaskMode; literal?: boolean; queue?: "steer" | "follow_up" } = {}): Promise<boolean> {
    const { images, mode: modeOverride, literal, queue } = options;
    const status = data.tasks.find((item) => item.id === task.id)?.status ?? task.status;
    if (status === "stopping") return false;
    if (status === "running") {
      // While Pi works, a message queues onto the run: steering delivers it at the run's next
      // boundary, a follow-up waits for the run to finish. Sends that cannot queue — the
      // programmatic ones (plan approval, reviews) and a stopped run — still refuse.
      const behavior = queue ?? (literal ? "steer" : undefined);
      if (!behavior) return false;
      try {
        await api.queueMessage({ taskId: task.id, behavior, message: sent, images, ...(literal ? { literal: true } : {}) });
        return true;
      } catch (reason) {
        patchRuntime(task.id, { error: String(reason) });
        return false;
      }
    }
    const startedAt = Date.now();
    const ownMode = task.id === selectedTask?.id ? currentMode : runtimes[task.id]?.planState?.mode ?? task.mode;
    patchTask(task.id, { status: "running", lastError: null });
    patchRuntime(task.id, { error: undefined, activity: "starting", activeRun: { startedAt } });
    try {
      await api.prompt({
        taskId: task.id,
        message: sent,
        startedAt,
        providerId: task.providerId,
        modelId: task.modelId,
        thinkingLevel: task.thinkingLevel,
        // modeOverride matters for "Approve & implement": the plan_state → record sync can
        // still be in flight when the follow-up prompt goes out.
        mode: modeOverride ?? ownMode,
        images,
        literal
      });
      return true;
    } catch (reason) {
      patchTask(task.id, { status: "idle" });
      patchRuntime(task.id, { error: String(reason), activeRun: undefined });
      return false;
    }
  }

  /** Discard a file's or hunk's changes behind a danger confirmation; `apply` takes the fresh changes. */
  function confirmDiscard(target: GitTarget, file: GitChangeFile, section: GitDiffSection, hunkId: number | undefined, apply: (next: GitChanges) => void, recover: () => void) {
    if (section.layer === "commit") return;
    const layer = section.layer;
    const perform = async () => {
      try {
        apply(await api.gitChangeAction(target, { file: file.path, layer, action: "discard", hunkId, expected: section.revision }));
      } catch (reason) {
        recover();
        throw reason;
      }
    };
    setConfirm({
      title: hunkId === undefined ? "Discard file changes?" : "Discard hunk changes?",
      body: section.layer === "staged"
        ? `This removes the staged changes in ${file.path}.`
        : `This removes the selected working-tree changes in ${file.path}.`,
      confirmLabel: "Discard changes", danger: true, run: perform
    });
  }

  async function changeAction(file: GitChangeFile, section: GitDiffSection, hunkId?: number): Promise<void> {
    if (!selectedTask) return;
    const taskId = selectedTask.id;
    confirmDiscard({ taskId }, file, section, hunkId, (next) => {
      ++changesRequest.current;
      if (selectedTaskRef.current === taskId) setChanges(next);
    }, () => void refreshChanges(taskId));
  }

  async function saveDiffComments(comments: DiffComment[]): Promise<void> {
    if (!selectedTask) return;
    const taskId = selectedTask.id;
    const saved = await api.setDiffComments(taskId, comments);
    setData((current) => ({ ...current, diffComments: { ...current.diffComments, [taskId]: saved } }));
  }

  async function addressDiffComments(comments: DiffComment[]): Promise<boolean> {
    if (!selectedTask) return false;
    const taskId = selectedTask.id;
    const sent = await sendPrompt(commentsPrompt(comments, currentMode), { literal: true });
    if (!sent) throw new Error("Could not send the comments. They remain pending.");
    const saved = await api.setDiffComments(taskId, []);
    setData((current) => ({ ...current, diffComments: { ...current.diffComments, [taskId]: saved } }));
    return true;
  }

  async function reviewChanges(): Promise<boolean> {
    const sent = await sendPrompt(REVIEW_PROMPT, { literal: true });
    if (!sent) throw new Error("Could not start review");
    return true;
  }

  // ── Git mode's hand-offs ── Every AI action goes through the linked chat (created on first
  // use when the project has none) and Git mode stays open, so the diff updates as it works.

  /** A chat in the project's own folder, for Git mode's AI actions. */
  async function createProjectChat(projectId: string, name: string): Promise<TaskRecord | undefined> {
    const choice = defaultChoice(projectId);
    if (!choice) { openSettings(); return undefined; }
    const task = await api.createTask({ projectId, useWorktree: false, name, ...choice });
    setData((current) => ({ ...current, tasks: [...current.tasks, task] }));
    rememberModel(projectId, choice);
    git.actions.setPicked(task.id);
    return task;
  }

  async function gitChat(name: string): Promise<TaskRecord | undefined> {
    if (!gitMode) return undefined;
    return gitLinked ?? await createProjectChat(gitMode.projectId, name);
  }

  /** Sends to the linked chat; failures land above the diff instead of throwing. */
  async function gitSend(message: string, name: string): Promise<boolean> {
    try {
      const task = await gitChat(name);
      if (!task) return false;
      if (await promptTask(task, message, { literal: true })) return true;
      git.actions.reportError(`Couldn't send to ${task.name}. Try again once it finishes.`);
    } catch (reason) {
      git.actions.reportError(String(reason));
    }
    return false;
  }

  async function gitAddressComments(comments: DiffComment[]): Promise<boolean> {
    const task = gitLinked;
    if (!task) return false;
    const mode = runtimes[task.id]?.planState?.mode ?? task.mode;
    if (!await promptTask(task, commentsPrompt(comments, mode), { literal: true })) throw new Error("Could not send the comments. They remain pending.");
    const saved = await api.setDiffComments(task.id, []);
    setData((current) => ({ ...current, diffComments: { ...current.diffComments, [task.id]: saved } }));
    return true;
  }

  /** Comments belong to the linked chat, so its own Changes panel shows them too. */
  async function saveGitComments(comments: DiffComment[]): Promise<void> {
    const task = await gitChat("Review comments");
    if (!task) return;
    const saved = await api.setDiffComments(task.id, comments);
    setData((current) => ({ ...current, diffComments: { ...current.diffComments, [task.id]: saved } }));
  }

  async function gitGenerate() {
    const changes = git.state?.changes;
    if (!gitLinked || !changes) return;
    setGitGenerating(true);
    try {
      const files = includedFiles(changes.files, new Set(git.state?.excluded ?? []));
      const generated = await api.gitGenerateMessage(gitLinked.id, { files, body: true });
      git.actions.setDraft({ summary: generated.summary, description: generated.description, generatedRevision: generated.revision });
    } catch (reason) {
      git.actions.reportError(String(reason));
    } finally {
      setGitGenerating(false);
    }
  }

  function gitAskCommit(commit: GitCommit, kind: "explain" | "review") {
    const target = `commit ${commit.sha} ("${commit.subject}")`;
    void gitSend(kind === "explain"
      ? `Explain what ${target} changed and why. Read it with \`git show ${commit.sha}\`. Don't edit any files.`
      : `Review ${target}. Read it with \`git show ${commit.sha}\`. Delegate the review to the Reviewer sub-agent if it's available, and report findings with file and line references. Don't edit any files.`,
    kind === "explain" ? "Explain a commit" : "Review a commit");
  }

  function gitAskIntegrate() {
    const divergence = git.state?.divergence;
    const branch = git.state?.sync?.branch;
    if (!divergence || !branch) return;
    const plan = gitLinked ? isPlanMode(runtimes[gitLinked.id]?.planState?.mode ?? gitLinked.mode) : false;
    git.actions.dismiss("divergence");
    void gitSend(`${plan ? "Plan how to bring" : "Bring"} ${branch} up to date with ${divergence.upstream}: they have diverged (${divergence.ahead} local and ${divergence.behind} remote commits, already fetched). Rebase or merge as fits this repository, resolve any conflicts, then explain what you did. Don't push.`, "Update branch");
  }

  function gitAskRevert(commit: GitCommit) {
    git.actions.dismiss("revertConflict");
    void gitSend(`Revert commit ${commit.sha} ("${commit.subject}") by hand: \`git revert\` conflicted with later changes. Undo what that commit did while keeping the later work, then explain what changed. Don't commit or push.`, "Revert a commit");
  }

  /** Discarding from Git mode's list drops every layer of the file (staged and working). */
  function gitDiscard(file: GitChangeFile, section?: GitDiffSection, hunkId?: number) {
    if (!gitMode) return;
    const target = section ?? file.sections.find((item) => item.layer === "staged") ?? file.sections[0];
    if (!target) return;
    confirmDiscard({ projectId: gitMode.projectId }, file, target, hunkId, git.actions.applyChanges, () => void git.refresh());
  }

  function gitUndoState(commit?: GitCommit): { can: boolean; reason?: string } {
    const sync = git.state?.sync;
    if (!commit || !sync) return { can: false };
    if (gitBusyReason) return { can: false, reason: gitBusyReason };
    if (commit.sha !== sync.head) return { can: false, reason: "Only the latest commit can be undone" };
    if (commit.parents.length === 0) return { can: false, reason: "The first commit can't be undone" };
    if (commit.parents.length > 1) return { can: false, reason: "Merge commits can't be undone here" };
    if (sync.remotes.length > 0 && !commit.unpushed) return { can: false, reason: "This commit is already pushed" };
    if (!sync.branch) return { can: false, reason: "Check out a branch first" };
    return { can: true };
  }

  function gitRevertState(commit?: GitCommit): { can: boolean; reason?: string } {
    if (!commit) return { can: false };
    if (gitBusyReason) return { can: false, reason: gitBusyReason };
    if (commit.parents.length > 1) return { can: false, reason: "Reverting merge commits isn't supported here" };
    if ((git.state?.changes?.files.length ?? 0) > 0) return { can: false, reason: "Commit or discard your changes first" };
    return { can: true };
  }

  function gitRevert(commit: GitCommit) {
    const branch = git.state?.sync?.branch ?? "this branch";
    setConfirm({
      title: "Revert this commit?",
      body: `This adds a new commit on ${branch} that undoes “${commit.subject}”. Nothing is pushed.`,
      confirmLabel: "Revert commit",
      run: () => git.actions.revert(commit)
    });
  }

  /**
   * Switch Build / Plan / Ultra Plan. For a draft the choice is just held locally; for a task it
   * is persisted on the record and pushed to the worker (which refuses while a run is active).
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
    const previousPlanState = runtime?.planState;
    patchTask(selectedTask.id, { mode });
    // Plan ↔ Ultra Plan keeps a plan awaiting review; the worker's plan_state confirms it.
    const keepsPlan = previousPlanState && previousPlanState.mode !== "build" && mode !== "build";
    patchRuntime(selectedTask.id, { planState: keepsPlan ? { ...previousPlanState, mode } : { mode, phase: "planning" } });
    try {
      const updated = await api.setTaskMode(selectedTask.id, mode);
      patchTask(selectedTask.id, updated);
    } catch (reason) {
      // `currentMode` reads the runtime first, so the optimistic plan state must go too.
      patchTask(selectedTask.id, { mode: previous });
      patchRuntime(selectedTask.id, { planState: previousPlanState, error: String(reason) });
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
      await sendPrompt("Implement the plan.", { mode: "build" });
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

  /** Open the restore dialog; resolves true once a choice has run, false when closed without one. */
  function chooseFiles(state: Omit<RestoreDialogState, "onClose">): Promise<boolean> {
    return new Promise((resolve) => {
      let settled = false;
      setRestoreDialog({
        ...state,
        run: async (choice, paths) => {
          await state.run(choice, paths);
          settled = true;
          resolve(true);
        },
        onClose: () => { if (!settled) resolve(false); }
      });
    });
  }

  /**
   * What a restore in this chat covers: its checkpoints span the worktree or the whole
   * repository, but only files inside the chat's own folder are selected by default.
   */
  function restoreScope(task: TaskRecord): { prefix: string; sharedWith?: string } {
    const project = data.projects.find((item) => item.id === task.projectId);
    const root = task.worktreePath ?? project?.gitRoot ?? task.workspacePath;
    const others = data.tasks.filter((other) => other.id !== task.id && !other.archived
      && (other.worktreePath ?? data.projects.find((item) => item.id === other.projectId)?.gitRoot ?? other.workspacePath) === root);
    return {
      prefix: workspacePrefix(root, task.workspacePath),
      sharedWith: others.length === 0 ? undefined : others.length === 1 ? `“${others[0].name}”` : `${plural(others.length, "other chat")}`
    };
  }

  async function changesSince(task: TaskRecord, checkpoint?: CheckpointRef): Promise<CheckpointChange[]> {
    if (!checkpoint) return [];
    try {
      return await api.checkpointChanges(task.id, checkpoint.id);
    } catch {
      // A checkpoint that can no longer be read only means there is nothing to offer.
      return [];
    }
  }

  function afterRestore(taskId: string, result?: RestoreResult | null, error?: string | null) {
    if (error) patchRuntime(taskId, { error });
    if (!result) return;
    void refreshChanges(taskId);
    patchRuntime(taskId, { lastRestore: result.restored.length > 0 ? { count: result.restored.length, undo: result.undo } : undefined });
    if (result.skipped.length > 0) {
      appendNotice(taskId, {
        message: `${plural(result.skipped.length, "file")} left as they are, because something the checkpoint doesn't hold is in the way (ignored or very large files): ${result.skipped.slice(0, 3).join(", ")}${result.skipped.length > 3 ? "…" : ""}`,
        level: "info"
      });
    }
  }

  /** Retry (unchanged) or edit: send a user message again as a new version of itself. */
  async function resend(user: NormalizedMessage, edit?: { text: string; files: FileAttachment[]; removeImages: number[] }): Promise<boolean> {
    const task = selectedTask;
    const entryId = user.entryId;
    if (!task || !entryId) return false;
    const send = async (restore?: { checkpointId: string; paths: string[] }) => {
      const startedAt = Date.now();
      patchTask(task.id, { status: "running", lastError: null });
      patchRuntime(task.id, { error: undefined, activity: "starting", activeRun: { startedAt }, lastRestore: undefined });
      try {
        await api.resendMessage({
          taskId: task.id,
          entryId,
          message: edit ? composeFileSection(edit.text, edit.files) : undefined,
          removeImages: edit?.removeImages,
          restore,
          startedAt,
          providerId: task.providerId,
          modelId: task.modelId,
          thinkingLevel: task.thinkingLevel
        });
        if (restore) void refreshChanges(task.id);
      } catch (reason) {
        patchTask(task.id, { status: "idle" });
        patchRuntime(task.id, { activeRun: undefined, activity: undefined });
        throw reason;
      }
    };
    const changes = await changesSince(task, user.checkpoint);
    if (changes.length === 0 || !user.checkpoint) {
      try {
        await send();
        return true;
      } catch (reason) {
        patchRuntime(task.id, { error: String(reason) });
        return false;
      }
    }
    const checkpointId = user.checkpoint.id;
    const scope = restoreScope(task);
    return chooseFiles({
      title: edit ? "Send the edited message" : "Retry this message",
      body: `${plural(changes.length, "file")} changed since this message was sent. Put them back as they were, or keep them as they are now?`,
      changes,
      initialSelection: defaultSelection(changes, scope.prefix),
      sharedWith: scope.sharedWith,
      choices: [
        { id: "keep", label: "Keep files" },
        { id: "restore", label: "Restore & send", files: true, danger: true }
      ],
      run: (choice, paths) => send(choice === "restore" ? { checkpointId, paths } : undefined)
    });
  }

  async function rewind(user: NormalizedMessage) {
    const task = selectedTask;
    const entryId = user.entryId;
    if (!task || !entryId) return;
    const changes = await changesSince(task, user.checkpoint);
    const checkpointId = user.checkpoint?.id;
    const scope = restoreScope(task);
    const moveConversation = async (restore?: { checkpointId: string; paths: string[] }) => {
      const result = await api.navigateTask({ taskId: task.id, entryId, target: "before", kind: "rewind", restore });
      if (result.navigate.editorText) setComposerSeed({ taskId: task.id, text: result.navigate.editorText, nonce: Date.now() });
      patchRuntime(task.id, { lastRestore: undefined });
      afterRestore(task.id, result.restore, result.restoreError);
    };
    const withFiles = changes.length > 0 && checkpointId !== undefined;
    void chooseFiles({
      title: "Rewind to before this message?",
      body: `The conversation goes back to just before this message, and its text returns to the composer. Later messages are kept as another version.${withFiles ? ` ${plural(changes.length, "file")} changed since it was sent.` : ""}`,
      changes,
      initialSelection: defaultSelection(changes, scope.prefix),
      sharedWith: withFiles ? scope.sharedWith : undefined,
      choices: withFiles
        ? [
            { id: "files", label: "Files only", files: true },
            { id: "conversation", label: "Conversation only" },
            { id: "both", label: "Conversation & files", files: true, danger: true }
          ]
        : [{ id: "conversation", label: "Rewind" }],
      run: async (choice, paths) => {
        if (choice === "files" && checkpointId) {
          afterRestore(task.id, await api.restoreCheckpoint({ taskId: task.id, checkpointId, paths }));
        } else {
          await moveConversation(choice === "both" && checkpointId ? { checkpointId, paths } : undefined);
        }
      }
    });
  }

  /** After moving to another branch: offer the files that branch was left with. */
  async function offerBranchFiles(task: TaskRecord, files: CheckpointRef | undefined, title: string) {
    const changes = await changesSince(task, files);
    if (!files || changes.length === 0) return;
    const scope = restoreScope(task);
    void chooseFiles({
      title,
      body: `${plural(changes.length, "file")} differ from how this version of the conversation left them.`,
      changes,
      initialSelection: defaultSelection(changes, scope.prefix),
      sharedWith: scope.sharedWith,
      choices: [
        { id: "keep", label: "Keep current files" },
        { id: "restore", label: "Restore files", files: true, danger: true }
      ],
      run: async (choice, paths) => {
        if (choice === "restore") afterRestore(task.id, await api.restoreCheckpoint({ taskId: task.id, checkpointId: files.id, paths }));
      }
    });
  }

  async function moveInTree(entryId: string, kind: "switch" | "undo") {
    const task = selectedTask;
    if (!task) return;
    try {
      const result = await api.navigateTask({ taskId: task.id, entryId, target: "latest", kind });
      patchRuntime(task.id, { lastRestore: undefined });
      await offerBranchFiles(task, result.navigate.files, kind === "undo" ? "Restore the files from before the rewind?" : "Use this version's files?");
    } catch (reason) {
      patchRuntime(task.id, { error: String(reason) });
    }
  }

  function forkChat(task: TaskRecord, answer?: NormalizedMessage) {
    const messages = runtimes[task.id]?.snapshot?.messages ?? [];
    const latestAnswer = latestTurn(messages)?.answer;
    const atEnd = !answer || answer.id === latestAnswer?.id;
    const files = task.usesWorktree
      ? "in its own worktree with a copy of the files. Ignored files such as .env or node_modules are not copied."
      : !task.projectId
        ? "in its own scratch folder with a copy of the files."
        : "in the same folder. Both chats will share its files.";
    setConfirm({
      title: answer ? "Fork from here?" : `Fork “${task.name}”?`,
      body: `A new chat continues from ${atEnd ? "the end of this conversation" : "this turn"}, ${files}`,
      confirmLabel: "Fork",
      run: async () => {
        const forked = await api.forkTask({
          taskId: task.id,
          entryId: answer?.turn?.endEntryId,
          checkpoint: atEnd ? undefined : answer?.turn?.after
        });
        setData((current) => ({ ...current, tasks: [...current.tasks.filter((item) => item.id !== forked.id), forked] }));
        setDraft(undefined);
        setSelectedTaskId(forked.id);
      }
    });
  }

  async function undoRestore() {
    const task = selectedTask;
    const last = task ? runtimes[task.id]?.lastRestore : undefined;
    if (!task || !last) return;
    try {
      afterRestore(task.id, await api.restoreCheckpoint({ taskId: task.id, checkpointId: last.undo.id }));
    } catch (reason) {
      patchRuntime(task.id, { error: String(reason) });
    }
  }

  async function messageAction(action: MessageAction): Promise<boolean> {
    const messages = runtime?.snapshot?.messages ?? [];
    if (action.type === "copy") {
      // The generated attached-files section is transport, not the message's words.
      await writeText(splitFileSection(messageText(action.message)).text);
    } else if (action.type === "copy-prompt") {
      await writeText(action.text);
    } else if (action.type === "edit") {
      return resend(action.message, { text: action.text, files: action.files, removeImages: action.removeImages });
    } else if (action.type === "retry") {
      const user = action.message.role === "user" ? action.message : userOfTurn(messages, action.message);
      return user ? resend(user) : false;
    } else if (action.type === "rewind") {
      await rewind(action.message);
    } else if (action.type === "switch") {
      await moveInTree(action.entryId, "switch");
    } else if (action.type === "fork" && selectedTask) {
      forkChat(selectedTask, action.message);
    }
    return true;
  }

  // Stable identities for the memoized transcript: the latest handlers are read through refs.
  const handlers = useRef({ messageAction, planAction, undoRewind: () => undefined as void });
  handlers.current = {
    messageAction,
    planAction,
    undoRewind: () => {
      const undo = runtime?.snapshot?.tree?.undo;
      if (undo) void moveInTree(undo, "undo");
    }
  };
  const onMessageAction = useCallback((action: MessageAction) => handlers.current.messageAction(action), []);
  const onPlanAction = useCallback((action: PlanAction) => void handlers.current.planAction(action), []);
  const onUndoRewind = useCallback(() => handlers.current.undoRewind(), []);

  async function stopTask() {
    if (!selectedTask) return;
    patchTask(selectedTask.id, { status: "stopping" });
    try { await api.stopTask(selectedTask.id); }
    catch (reason) { patchRuntime(selectedTask.id, { error: String(reason) }); }
  }

  /** Take the queued messages back out of Pi and hand their texts to the composer. */
  async function dequeueMessages(): Promise<string[] | undefined> {
    if (!selectedTask) return undefined;
    try {
      const cleared = await api.dequeueMessages(selectedTask.id);
      const texts = [...cleared.steering, ...cleared.followUp];
      return texts.length > 0 ? texts : undefined;
    } catch (reason) {
      patchRuntime(selectedTask.id, { error: String(reason) });
      return undefined;
    }
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

  function removeTaskLocally(taskId: string) {
    composerDrafts.remove(`task:${taskId}`);
    setData((current) => ({ ...current, tasks: current.tasks.filter((item) => item.id !== taskId) }));
    setRuntimes((current) => {
      const next = { ...current };
      delete next[taskId];
      return next;
    });
    selectAfterRemoval(taskId);
  }

  async function performDeleteTask(task: TaskRecord) {
    try {
      await api.deleteTask(task.id);
      removeTaskLocally(task.id);
    } catch (reason) {
      setGlobalError(String(reason));
    }
  }

  /**
   * Run a per-chat call over many chats one at a time, so each row leaves the list as it
   * finishes (and parallel git worktree removals never race). One failure doesn't stop the rest.
   */
  async function forEachChat(tasks: TaskRecord[], verb: string, run: (task: TaskRecord) => Promise<void>) {
    const failed: string[] = [];
    for (const task of tasks) {
      try { await run(task); } catch { failed.push(task.name); }
    }
    if (failed.length === 1) throw `Could not ${verb} “${failed[0]}”.`;
    if (failed.length > 1) throw `Could not ${verb} ${failed.length} chats.`;
  }

  function archiveAll(projectId: string | null) {
    const targets = data.tasks.filter((task) => task.projectId === projectId && !task.archived);
    if (targets.length === 0) return;
    const groupName = projectId === null ? "No project" : data.projects.find((project) => project.id === projectId)?.name ?? "this project";
    const running = targets.filter((task) => task.status === "running" || task.status === "stopping").length;
    setConfirm({
      title: targets.length === 1 ? `Archive 1 chat in “${groupName}”?` : `Archive ${targets.length} chats in “${groupName}”?`,
      body: `They move to Archived, where you can unarchive or delete them.${running > 0 ? ` ${running === 1 ? "1 is" : `${running} are`} still working and will be stopped.` : ""}`,
      confirmLabel: "Archive all",
      run: () => forEachChat(targets, "archive", async (task) => {
        const archived = await api.archiveTask(task.id);
        setData((current) => ({ ...current, tasks: current.tasks.map((item) => item.id === archived.id ? archived : item) }));
        selectAfterRemoval(task.id);
      })
    });
  }

  function deleteAllArchived() {
    const targets = data.tasks.filter((task) => task.archived);
    if (targets.length === 0) return;
    const worktrees = targets.some((task) => task.usesWorktree);
    setConfirm({
      title: targets.length === 1 ? "Delete 1 archived chat?" : `Delete all ${targets.length} archived chats?`,
      body: worktrees
        ? "This removes them, their saved sessions, and their git worktrees — including any uncommitted changes inside them. It can't be undone."
        : "This removes them and their saved sessions. Files in your projects are not touched. It can't be undone.",
      confirmLabel: "Delete all",
      danger: true,
      run: () => forEachChat(targets, "delete", async (task) => {
        await api.deleteTask(task.id);
        removeTaskLocally(task.id);
      })
    });
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
    } else if (action === "fork") {
      forkChat(task);
    } else if (action === "worktree") {
      try {
        const updated = await api.convertToWorktree(task.id);
        setData((current) => ({ ...current, tasks: current.tasks.map((item) => item.id === updated.id ? updated : item) }));
      } catch (reason) { setGlobalError(String(reason)); }
    } else if (action === "delete-direct") {
      await performDeleteTask(task);
    } else if (action === "delete") {
      setConfirm({
        title: `Delete “${task.name}”?`,
        body: task.usesWorktree
          ? "This removes the chat, its saved session, and its git worktree — including any uncommitted changes inside it."
          : "This removes the chat and its saved session. Files in the project are not touched.",
        confirmLabel: "Delete",
        danger: true,
        run: () => performDeleteTask(task)
      });
    }
  }

  function setProjectPinned(projectId: string, pinned: boolean) {
    setPinnedProjects((current) => {
      const next = new Set(current);
      if (pinned) next.add(projectId); else next.delete(projectId);
      return next;
    });
  }

  async function projectAction(project: ProjectRecord, action: ProjectAction) {
    if (action === "pin" || action === "unpin") {
      setProjectPinned(project.id, action === "pin");
    } else if (action === "reveal") {
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
          setProjectPinned(project.id, false);
          if (gitRef.current.view?.projectId === project.id) closeGit();
          if (selectedTask?.projectId === project.id) openDraft();
        }
      });
    }
  }

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      // ⇧Tab cycles Build → Plan → Ultra Plan, like Claude Code. A modal or an active run owns the key.
      if (event.key === "Tab" && event.shiftKey) {
        // Inside the terminal the keystroke belongs to the shell, not the mode switcher.
        if (event.target instanceof HTMLElement && event.target.closest(".xterm")) return;
        // Git mode hides the composer: ⇧Tab keeps its usual job of moving focus back.
        if (gitMode) return;
        const busy = selectedTask && (selectedTask.status === "running" || selectedTask.status === "stopping");
        if (!settingsOpen && !confirm && !restoreDialog && extensionRequests.length === 0 && accessRequests.length === 0 && !busy) {
          event.preventDefault();
          void setTaskMode(nextMode(currentMode));
        }
        return;
      }
      if (!(event.metaKey || event.ctrlKey)) return;
      const key = event.key.toLowerCase();
      if (key === "n" && !event.shiftKey) {
        event.preventDefault();
        closeGit();
        openDraft(selectedTask ? selectedTask.projectId : draft?.projectId);
      } else if (key === "o" && !event.shiftKey) {
        event.preventDefault();
        void addProject();
      } else if (key === ",") {
        event.preventDefault();
        if (settingsOpen) closeSettings(); else openSettings();
      } else if (key === "c" && event.shiftKey) {
        event.preventDefault();
        closeGit();
        toggleChanges();
      } else if (key === "t" && event.shiftKey) {
        event.preventDefault();
        closeGit();
        toggleTerminal();
      } else if (key === "g" && event.shiftKey) {
        event.preventDefault();
        if (data.projects.length > 0 || gitMode) toggleGit();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  // ── Git mode's derived state. Memoized: App re-renders on every streaming delta, and the Git
  // components are memo'd so a running agent doesn't re-render the whole file list.
  const gitState = git.state;
  const gitProjectId = gitMode?.projectId;
  const gitProject = data.projects.find((project) => project.id === gitProjectId);
  const gitLinkable = useMemo(() => gitProjectId ? linkableChats(data.tasks, gitProjectId) : [], [data.tasks, gitProjectId]);
  const gitLinked = useMemo(
    () => gitProjectId ? resolveLinkedChat(data.tasks, gitProjectId, gitState?.picked, gitMode?.cameFrom) : undefined,
    [data.tasks, gitProjectId, gitState?.picked, gitMode?.cameFrom]
  );
  const gitBusy = useMemo(() => gitProjectId
    ? busyChatsInCheckout(data.tasks, gitProjectId).sort((a, b) => Number(b.id === gitLinked?.id) - Number(a.id === gitLinked?.id))
    : [], [data.tasks, gitProjectId, gitLinked?.id]);
  const gitBusyReason = gitBusy[0] ? `Wait for ${gitBusy[0].name} to finish before changing Git files` : undefined;
  const gitComments = gitLinked ? data.diffComments[gitLinked.id] ?? NO_COMMENTS : NO_COMMENTS;
  const gitExcluded = useMemo(() => new Set(gitState?.excluded ?? []), [gitState?.excluded]);
  const gitCommentCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const comment of gitComments) counts.set(comment.path, (counts.get(comment.path) ?? 0) + 1);
    return counts;
  }, [gitComments]);
  const canReview = data.subagents.enabled && data.subagents.agents.some((agent) => agent.id === "builtin:reviewer" && agent.enabled);
  const reviewReason = !data.subagents.enabled ? "Enable sub-agents in Settings" : !canReview ? "Enable Reviewer in Settings" : undefined;
  const gitReviewState = useMemo(() => ({ can: canReview, reason: reviewReason }), [canReview, reviewReason]);
  const gitChangeFiles = gitState?.changes?.files;
  const gitSelectedFile = gitChangeFiles?.find((file) => file.path === gitState?.selectedPath) ?? gitChangeFiles?.[0];
  const gitCommit = gitState?.log?.commits.find((commit) => commit.sha === gitState.selectedSha);

  /** Git mode's callbacks, stable across renders (the `handlers` pattern): each forwards to
      this render's closure, so memo'd Git components only re-render when their data moves. */
  const gitImpl = {
    exit: () => closeGit(),
    switchProject: (projectId: string) => git.switchProject(projectId),
    setPinned: (projectId: string, pinned: boolean) => setProjectPinned(projectId, pinned),
    addProject: () => void addProject(),
    discard: (file: GitChangeFile, section?: GitDiffSection, hunkId?: number) => gitDiscard(file, section, hunkId),
    copyPath: (path: string) => void writeText(path).then(() => git.actions.notify("Path copied")),
    copy: (text: string, what: string) => void writeText(text).then(() => git.actions.notify(`${what} copied`)),
    revealFolder: (path: string) => {
      const root = git.state?.changes?.root ?? gitProject?.path;
      if (!root) return;
      const slash = path.lastIndexOf("/");
      void api.revealPath(slash > 0 ? `${root}/${path.slice(0, slash)}` : root).catch((reason) => setGlobalError(String(reason)));
    },
    revealProject: () => { if (gitProject) void api.revealPath(gitProject.path).catch((reason) => setGlobalError(String(reason))); },
    generate: () => void gitGenerate(),
    summary: (summary: string) => git.actions.setDraft({ summary }),
    description: (description: string) => git.actions.setDraft({ description }),
    commit: () => void git.actions.commit(),
    listBranches: () => gitProjectId ? listProjectBranches(gitProjectId) : Promise.reject(new Error("No project")),
    checkout: async (name: string, kind: GitCheckoutKind) => {
      if (!gitProjectId) return;
      await checkoutBranch({ projectId: gitProjectId }, name, kind);
      void git.refresh();
    },
    publish: (remote: string) => void git.actions.push(remote),
    openEditor: (editor: string) => void git.actions.openEditor(editor),
    openRepo: (url: string) => void api.revealPath(url).catch((reason) => setGlobalError(String(reason))),
    preparePr: (remote: string) => gitProjectId ? api.gitPrPrepare({ projectId: gitProjectId }, remote) : Promise.reject(new Error("No project")),
    createPr: async (remote: string, base: string, title: string, body: string, draft: boolean) => {
      if (!gitProjectId) throw new Error("No project");
      const url = await api.gitPrCreate({ projectId: gitProjectId }, remote, base, title, body, draft);
      void api.revealPath(url).catch((reason) => setGlobalError(String(reason)));
      return url;
    },
    openPr: (url: string) => void api.revealPath(url).catch((reason) => setGlobalError(String(reason))),
    requestPr: () => setGitPrRequest((value) => value + 1),
    link: (taskId: string) => git.actions.setPicked(taskId),
    openChat: (taskId: string) => openChatFromGit(taskId),
    review: async () => { await gitSend(REVIEW_PROMPT, "Review changes"); },
    settings: () => openSettings(),
    saveComments: (comments: DiffComment[]) => saveGitComments(comments),
    addressComments: (comments: DiffComment[]) => gitAddressComments(comments),
    askIntegrate: () => gitAskIntegrate(),
    askCommit: (kind: "explain" | "review") => { if (gitCommit) gitAskCommit(gitCommit, kind); },
    askRevert: () => { const commit = git.state?.revertConflict; if (commit) gitAskRevert(commit); },
    undo: () => { if (gitCommit) void git.actions.undo(gitCommit); },
    revert: () => { if (gitCommit) gitRevert(gitCommit); }
  };
  const gitImplRef = useRef(gitImpl);
  gitImplRef.current = gitImpl;
  const gitHandlers = useMemo(() => {
    const keys = Object.keys(gitImplRef.current) as (keyof typeof gitImpl)[];
    return Object.fromEntries(keys.map((key) => [key, (...args: unknown[]) => (gitImplRef.current[key] as (...values: unknown[]) => unknown)(...args)])) as typeof gitImpl;
  }, []);

  // Served by the asset protocol, which may read only <app data>/backgrounds/ (tauri.conf.json).
  const backgroundImageUrl = data.appearance.backgroundImage && appDataPath
    ? convertFileSrc(`${appDataPath}/backgrounds/${data.appearance.backgroundImage}`)
    : undefined;

  if (booting) return <div className="boot-screen"><div className="brand-mark"><DuckMark /></div><span>Starting WackCode</span></div>;

  // A list fetched for another chat or project never shows here.
  const composerMentions = mentions?.source === (mentionSource().source ?? "") ? mentions : undefined;
  const draftCatalog = !selectedTask && draftSlash?.projectId === (draft?.projectId ?? null) ? draftSlash : undefined;

  // Git mode's sidebar: the repository switcher and tabs on top, the file list with the commit
  // form (Changes) or the commit list (History) below.
  const gitSidebar = gitMode && gitProject ? {
    top: (
      <GitPanelTop
        switcher={(
          <RepoSwitcher
            projects={data.projects}
            currentId={gitProject.id}
            branch={gitState?.sync?.branch ?? gitState?.changes?.branch ?? gitProject.branch}
            pinned={pinnedProjects}
            onSelect={gitHandlers.switchProject}
            onSetPinned={gitHandlers.setPinned}
            onAddProject={gitHandlers.addProject}
          />
        )}
        tab={gitMode.tab}
        changesCount={gitChangeFiles?.length ?? 0}
        commitPulse={gitState?.commitPulse ?? 0}
        onTab={git.setTab}
      />
    ),
    page: (
      <AnimatePresence mode="wait" initial={false}>
        <motion.div
          key={gitMode.tab}
          className="git-page-body"
          initial={reduce ? false : { opacity: 0, x: gitMode.tab === "history" ? 14 : -14 }}
          animate={{ opacity: 1, x: 0 }}
          exit={reduce ? { opacity: 0 } : { opacity: 0, x: gitMode.tab === "history" ? -14 : 14 }}
          transition={{ duration: reduce ? 0 : 0.15, ease: EASE }}
        >
          {gitMode.tab === "history" ? (
            <GitHistoryList
              commits={gitState?.log?.commits ?? []}
              hasMore={gitState?.log?.hasMore ?? false}
              loading={gitState?.log?.loading ?? true}
              error={gitState?.log?.error}
              showUnpushed={(gitState?.sync?.remotes.length ?? 0) > 0}
              selectedSha={gitState?.selectedSha}
              onSelect={git.actions.selectCommit}
              onLoadMore={git.actions.loadMoreLog}
            />
          ) : gitState?.changes && !gitState.changes.isGit ? (
            <div className="git-sidebar-empty">{gitProject.name} isn't a Git repository.</div>
          ) : (
            <>
              <GitChangesList
                files={gitChangeFiles ?? []}
                excluded={gitExcluded}
                selectedPath={gitSelectedFile?.path}
                commentCounts={gitCommentCounts}
                disabled={Boolean(gitBusyReason)}
                onSelect={git.actions.select}
                onToggle={git.actions.toggle}
                onToggleAll={git.actions.toggleAll}
                onDiscard={gitHandlers.discard}
                onCopyPath={gitHandlers.copyPath}
                onReveal={gitHandlers.revealFolder}
              />
              <GitCommitForm
                branch={gitState?.sync?.branch ?? gitState?.changes?.branch ?? null}
                summary={gitState?.summary ?? ""}
                description={gitState?.description ?? ""}
                stale={Boolean(gitState?.generatedRevision && gitState.changes && gitState.generatedRevision !== gitState.changes.changesRevision)}
                checkedCount={includedFiles(gitChangeFiles ?? [], gitExcluded).length}
                blockedReason={gitBusyReason ?? (gitChangeFiles?.some((file) => file.status === "conflict") && includedFiles(gitChangeFiles, gitExcluded).length === 0 ? "Resolve conflicts before committing" : undefined)}
                generate={{
                  available: Boolean(gitLinked) && !gitBusyReason,
                  reason: !gitLinked ? "Start a chat in this project to generate messages" : gitBusyReason,
                  running: gitGenerating
                }}
                agentName={agentName(data.appearance)}
                committing={gitState?.committing ?? false}
                commitPulse={gitState?.commitPulse ?? 0}
                onSummary={gitHandlers.summary}
                onDescription={gitHandlers.description}
                onGenerate={gitHandlers.generate}
                onCommit={gitHandlers.commit}
              />
            </>
          )}
        </motion.div>
      </AnimatePresence>
    )
  } : undefined;

  return (
    <ContextMenuProvider
      scope={`${composerDraftKey}-${settingsOpen}-${gitMode?.projectId ?? ""}-${gitMode?.tab ?? ""}-${panelView?.kind ?? ""}-${selectedBusy}-${gitBusyReason ?? ""}-${archivedOpen}-${Boolean(confirm || restoreDialog || subscriptionLogin)}`}
      copyText={writeText}
      readText={readText}
      openLink={api.revealPath}
      onError={setGlobalError}
      items={[
        { label: "New chat", icon: <Icon name="plus" />, hint: "⌘N", onSelect: () => { if (settingsOpen) closeSettings(); closeGit(); openDraft(selectedTask ? selectedTask.projectId : draft?.projectId); } },
        { label: "Add project…", icon: <Icon name="folder" />, hint: "⌘O", onSelect: () => { void addProject(); } },
        "separator",
        { label: "Settings…", icon: <Icon name="settings" />, hint: "⌘,", onSelect: openSettings }
      ]}
    >
    <div className={`app-shell${rebuilding ? " rebuild" : ""}${gitMode ? " git-mode" : ""}`}>
      {data.appearance.backdrop === "image" && (
        <Backdrop
          imageUrl={backgroundImageUrl}
          scene={settingsOpen || selectedTask || gitMode ? "chat" : "hero"}
          dim={data.appearance.imageDim}
          blur={data.appearance.imageBlur}
          crop={{ zoom: data.appearance.imageZoom, x: data.appearance.imageX, y: data.appearance.imageY }}
        />
      )}
      {settingsOpen ? (
        <SettingsPage
          providers={data.providers}
          packages={data.packages}
          toolCatalog={data.toolCatalog}
          disabledTools={data.toolConfig.disabled}
          appDataPath={appDataPath}
          onClose={closeSettings}
          onSave={saveProvider}
          onDelete={deleteProvider}
          onSetProviderEnabled={setProviderEnabled}
          onConnectSubscription={connectSubscription}
          onSignOutSubscription={signOutSubscription}
          connectedSubscriptionId={connectedSubscriptionId}
          onSetDisabledTools={setDisabledTools}
          subagents={data.subagents}
          onSetSubagents={setSubagents}
          computerUse={data.computerUse ?? { enabled: false, neverAllow: [] }}
          computerUseSupported={computerUseSupported}
          onSetComputerUse={setComputerUse}
          autoTitle={data.autoTitle}
          onSetAutoTitle={setAutoTitle}
          appearance={data.appearance}
          glassSupported={glassSupported}
          onSetAppearance={setAppearance}
          onPreviewAppearance={previewAppearance}
          backgroundImageUrl={backgroundImageUrl}
          onChooseBackgroundImage={chooseBackgroundImage}
          onRemoveBackgroundImage={removeBackgroundImage}
          prompts={data.prompts}
          onSetPrompts={setPrompts}
          onCommandsChanged={commandsChanged}
          memory={data.memory ?? { enabled: true, disabledProjects: [] }}
          onSetMemory={setMemory}
          mcp={data.mcp}
          mcpActions={mcpActions}
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
        pinnedProjectIds={pinnedProjects}
        tasks={data.tasks}
        selectedTaskId={selectedTaskId}
        archivedOpen={archivedOpen}
        pendingDialogTaskIds={pendingDialogTaskIds}
        collapsedProjectIds={collapsedProjects}
        onSelectTask={selectTask}
        onNewChat={(project) => { closeGit(); openDraft(project?.id ?? null); }}
        onNewDraft={() => { closeGit(); openDraft(); }}
        onAddProject={() => void addProject()}
        onToggleArchived={() => {
          if (gitMode) { closeGit(); setArchivedOpen(true); } else setArchivedOpen((value) => !value);
        }}
        git={gitSidebar}
        onToggleGit={data.projects.length > 0 || gitMode ? toggleGit : null}
        onToggleProjectCollapsed={toggleProjectCollapsed}
        onOpenSettings={openSettings}
        onTaskAction={(task, action) => void taskAction(task, action)}
        onProjectAction={(project, action) => void projectAction(project, action)}
        onRenameTask={(taskId, name) => void renameTask(taskId, name)}
        onArchiveAll={archiveAll}
        onDeleteAllArchived={deleteAllArchived}
      />

      <main className="workspace">
        <AnimatePresence initial={false}>
        {selectedTask ? (
          <motion.div key="chat" className={`chat-view${gitMode ? " git-behind" : ""}`} inert={gitMode ? true : undefined} initial={false} exit={{ opacity: 0, transition: { duration: reduce ? 0 : 0.18, ease: EASE } }}>
            <motion.div initial={reduce ? false : { opacity: 0, y: -36 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: reduce ? 0 : 0.45, delay: reduce ? 0 : 0.05, ease: EASE }}>
              <ChatHeader
              task={selectedTask}
              project={selectedProject}
              git={changes ? (changes.isGit ? { branch: changes.branch } : undefined) : selectedTask.branch ? { branch: selectedTask.branch } : undefined}
              onListBranches={() => api.gitBranches({ taskId: selectedTask.id })}
              onCheckoutBranch={(name, kind) => checkoutBranch({ taskId: selectedTask.id }, name, kind)}
              changesCount={changes?.files.length}
              changesOpen={panelView?.kind === "changes"}
              browserOpen={panelView?.kind === "browser"}
              onToggleChanges={toggleChanges}
              onToggleBrowser={toggleBrowser}
              terminal={terminals[selectedTask.id]}
              terminalOpen={panelView?.kind === "terminal"}
              onToggleTerminal={toggleTerminal}
              onRename={(name) => void renameTask(selectedTask.id, name)}
              onTaskAction={(task, action) => void taskAction(task, action)}
              />
            </motion.div>
            {sharedWorkers.length > 0 && <div className="shared-notice"><span>!</span><strong>{sharedWorkers[0].name}</strong> is also running in this folder. File edits are shared.</div>}
            {(runtime?.error || selectedTask.lastError) && <div className="error-banner workspace-error"><span>{runtime?.error || selectedTask.lastError}</span><button onClick={() => dismissError(selectedTask.id)}>Dismiss</button></div>}
            {selectedModelGone && (
              <div className="model-missing-notice" role="status">
                <span>!</span>
                {runtime?.snapshot?.modelIssue ?? "This chat's model is no longer configured. Pick another to continue."}
              </div>
            )}
            {runtime?.notices?.map((entry, index) => (
              <div className={`extension-notice ${entry.level}`} key={`${index}-${entry.message}`}>
                <span>{entry.message}</span>
                <button onClick={() => patchRuntime(selectedTask.id, { notices: runtime.notices?.filter((_, position) => position !== index) })}>Dismiss</button>
              </div>
            ))}
            {runtime?.lastRestore && (
              <div className="extension-notice info restore-notice" role="status">
                <span>Restored {plural(runtime.lastRestore.count, "file")}.</span>
                <button onClick={() => void undoRestore()} disabled={selectedBusy}>Undo</button>
                <button onClick={() => patchRuntime(selectedTask.id, { lastRestore: undefined })}>Dismiss</button>
              </div>
            )}
            <motion.div className="chat-transcript" initial={reduce ? false : { opacity: 0, y: 16 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: reduce ? 0 : 0.4, delay: reduce ? 0 : 0.14, ease: EASE }}>
            <ChatContexts appearance={data.appearance}>
            <SubagentPanelLink.Provider value={subagentLink}>
            <ToolImageSource.Provider value={loadToolImage}>
            <Transcript
              messages={runtime?.snapshot?.messages ?? []}
              modelSwitches={displayModelSwitches}
              partial={runtime?.partial}
              running={selectedTask.status === "running" || selectedTask.status === "stopping"}
              activeRun={runtime?.activeRun}
              runTimings={runtime?.snapshot?.runTimings}
              activity={runtime?.activity}
              liveToolText={runtime?.liveToolText}
              liveToolDetails={runtime?.liveToolDetails}
              planState={runtime?.planState}
              onPlanAction={onPlanAction}
              actionsEnabled={!selectedBusy && !pendingDialogTaskIds.has(selectedTask.id)}
              vision={selectedModel?.vision === true}
              modelName={selectedModel?.name || selectedModel?.id}
              onMessageAction={onMessageAction}
              onUndoRewind={runtime?.snapshot?.tree?.undo ? onUndoRewind : undefined}
            />
            </ToolImageSource.Provider>
            </SubagentPanelLink.Provider>
            </ChatContexts>
            </motion.div>
            <InlineDialog
              requests={extensionRequests}
              accessRequests={accessRequests}
              selectedTaskId={selectedTask.id}
              agentName={agentName(data.appearance)}
              onRespond={handleExtensionRespond}
              onAccess={handleComputerAccess}
            />
            <ComputerUseBanner computer={runtime?.computer} onStop={() => void stopTask()} />
            <TodoPanel
              key={selectedTask.id}
              tasks={runtime?.todoState?.tasks}
              busy={selectedTask.status === "running" || selectedTask.status === "stopping"}
            />
            <GoalBanner
              goal={runtime?.goalState}
              onAction={(action) => void api.goalControl(selectedTask.id, action)
                .catch((reason) => appendNotice(selectedTask.id, { message: String(reason), level: "warning" }))}
            />
          </motion.div>
        ) : configuredProviders.length === 0 ? (
          <div key="empty" className="workspace-empty">
            <h1>Connect a model provider</h1>
            <p>{data.providers.length > 0
              ? "Every connection is turned off. Switch one back on in Settings to start chatting."
              : "Sign in with a subscription or add an OpenAI-compatible endpoint and API key to start chatting."}</p>
            <button className="primary-button" onClick={openSettings}><Icon name="key" /> Open settings</button>
          </div>
        ) : null}
        </AnimatePresence>

        {/* Git mode covers the chat view, which stays mounted underneath (scroll position, and
            no transcript remount on every toggle). */}
        <AnimatePresence initial={false}>
          {gitMode && gitProject && (
            <motion.div
              key="git"
              className="git-view"
              initial={reduce ? false : { opacity: 0 }}
              animate={{ opacity: 1, transition: { duration: reduce ? 0 : 0.2, ease: EASE } }}
              exit={{ opacity: 0, transition: { duration: reduce ? 0 : 0.18, ease: EASE } }}
            >
              <motion.div className="git-toolbar-wrap" initial={reduce ? false : { opacity: 0, y: -36 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: reduce ? 0 : 0.45, delay: reduce ? 0 : 0.05, ease: EASE }}>
                <GitToolbar
                  branch={gitState?.sync?.branch ?? gitState?.changes?.branch ?? null}
                  busyReason={gitBusyReason}
                  onListBranches={gitHandlers.listBranches}
                  onCheckout={gitHandlers.checkout}
                  sync={gitState?.sync}
                  network={gitState?.network}
                  networkError={gitState?.networkError}
                  onFetch={git.actions.fetch}
                  onPull={git.actions.pull}
                  onPush={git.actions.push}
                  onPublish={gitHandlers.publish}
                  editors={git.editors}
                  editor={git.editor}
                  onOpenEditor={gitHandlers.openEditor}
                  onDismissNetworkError={() => git.actions.dismiss("networkError")}
                  onPreparePr={gitHandlers.preparePr}
                  onCreatePr={gitHandlers.createPr}
                  onOpenPr={gitHandlers.openPr}
                  linked={gitLinked}
                  linkable={gitLinkable}
                  onLink={gitHandlers.link}
                  onOpenChat={gitHandlers.openChat}
                  review={gitReviewState}
                  onReview={gitHandlers.review}
                  onSettings={gitHandlers.settings}
                  comments={gitComments}
                  files={gitChangeFiles ?? []}
                  mode={gitLinked ? runtimes[gitLinked.id]?.planState?.mode ?? gitLinked.mode : "build"}
                  onComments={gitHandlers.saveComments}
                  onAddressComments={gitHandlers.addressComments}
                  layout={git.layout}
                  onLayout={git.setLayout}
                  showLayout={gitMode.tab === "history" ? Boolean(gitState?.commitDiff) : Boolean(gitSelectedFile)}
                  flash={gitState?.flash}
                  prRequest={gitPrRequest}
                  agentName={agentName(data.appearance)}
                  onExit={gitHandlers.exit}
                />
              </motion.div>
              <motion.div className="git-main" initial={reduce ? false : { opacity: 0, y: 16 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: reduce ? 0 : 0.4, delay: reduce ? 0 : 0.14, ease: EASE }}>
                <GitActivity busy={gitBusy} waiting={pendingDialogTaskIds} agentName={agentName(data.appearance)} onOpenChat={gitHandlers.openChat} />
                {gitMode.tab === "history" ? (
                  <div className="git-history-main">
                    {gitState?.revertConflict && (
                      <GitNoticeCard
                        icon="rewind"
                        title="Reverting that commit conflicted with later changes."
                        body="Nothing changed: the revert was aborted. The agent can undo it by hand while keeping the later work."
                        action={`Ask ${agentName(data.appearance)} to revert it`}
                        onAction={gitHandlers.askRevert}
                        onDismiss={() => git.actions.dismiss("revertConflict")}
                      />
                    )}
                    {gitState?.actionError && (
                      <div className="error-banner git-error" role="alert">
                        <span>{gitState.actionError}</span>
                        <button type="button" className="icon-button" aria-label="Dismiss error" onClick={() => git.actions.dismiss("actionError")}><Icon name="close" /></button>
                      </div>
                    )}
                    <GitCommitView
                      commit={gitCommit}
                      files={gitState?.commitFiles?.sha === gitCommit?.sha ? gitState?.commitFiles?.files : undefined}
                      truncated={gitState?.commitFiles?.truncated ?? false}
                      selectedPath={gitState?.commitPath}
                      diff={gitState?.commitDiff?.sha === gitCommit?.sha ? gitState?.commitDiff?.file : undefined}
                      loading={gitState?.commitLoading ?? false}
                      layout={git.layout}
                      direction={gitState?.direction ?? 1}
                      showUnpushed={(gitState?.sync?.remotes.length ?? 0) > 0}
                      undo={gitUndoState(gitCommit)}
                      revert={gitRevertState(gitCommit)}
                      agentName={agentName(data.appearance)}
                      onSelectFile={git.actions.selectCommitFile}
                      onUndo={gitHandlers.undo}
                      onRevert={gitHandlers.revert}
                      onAsk={gitHandlers.askCommit}
                      onCopy={gitHandlers.copy}
                    />
                  </div>
                ) : (
                  <GitWorkspace
                    isGit={gitState?.changes?.isGit ?? true}
                    loading={gitState?.loading ?? true}
                    error={gitState?.error}
                    file={gitSelectedFile}
                    layout={git.layout}
                    direction={gitState?.direction ?? 1}
                    comments={gitComments}
                    disabled={Boolean(gitBusyReason)}
                    onComments={gitHandlers.saveComments}
                    onDiscard={gitHandlers.discard}
                    sync={gitState?.sync}
                    agentName={agentName(data.appearance)}
                    linked={gitLinked}
                    onPush={git.actions.push}
                    onPull={git.actions.pull}
                    onPublish={gitHandlers.publish}
                    onCreatePr={gitHandlers.requestPr}
                    repoUrl={gitState?.repoUrl}
                    onOpenRepo={gitHandlers.openRepo}
                    onOpenChat={gitHandlers.openChat}
                    onReveal={gitHandlers.revealProject}
                    divergence={gitState?.divergence}
                    onAskIntegrate={gitHandlers.askIntegrate}
                    onDismissDivergence={() => git.actions.dismiss("divergence")}
                    actionError={gitState?.actionError}
                    onDismissError={() => git.actions.dismiss("actionError")}
                  />
                )}
              </motion.div>
            </motion.div>
          )}
        </AnimatePresence>

        {configuredProviders.length > 0 && (
          <div
            className={`composer-layer ${selectedTask ? "dock" : "hero"}`}
            data-git={composerPhase}
            inert={gitMode ? true : undefined}
          >
            <AnimatePresence initial={false} mode="popLayout">
              {!selectedTask && (
                <motion.h1
                  key="draft-title"
                  className="draft-title"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1, transition: { duration: 0.25, ease: EASE } }}
                  exit={{ opacity: 0, y: -56, transition: { duration: 0.22, ease: EASE } }}
                >
                  {draftProject ? `What should we build in ${draftProject.name}?` : "What should we build?"}
                </motion.h1>
              )}
            </AnimatePresence>
            {/* One persistent composer: it layout-animates between the centered hero slot and
                the docked chat slot, so it can never vanish mid-transition. */}
            <Composer
              comet={!selectedTask || transitioning !== undefined}
              frozen={transitioning?.message}
              status={selectedTask?.status ?? "idle"}
              providerId={selectedTask?.providerId ?? draftChoice?.providerId}
              modelId={selectedTask?.modelId ?? draftChoice?.modelId}
              thinkingLevel={selectedTask?.thinkingLevel ?? draftChoice?.thinkingLevel}
              providers={configuredProviders}
              stats={selectedTask ? runtime?.snapshot?.stats : undefined}
              popoverSide={selectedTask ? "top" : "bottom"}
              header={!selectedTask ? (
                <ProjectBar
                  projects={data.projects}
                  projectId={draft?.projectId ?? null}
                  useWorktree={draft?.useWorktree ?? false}
                  onSelectProject={setDraftProject}
                  onToggleWorktree={setDraftWorktree}
                  onAddProject={() => void addProject()}
                  onListBranches={listProjectBranches}
                  onCheckoutBranch={(projectId, name, kind) => checkoutBranch({ projectId }, name, kind)}
                />
              ) : undefined}
              placeholder={!selectedTask ? "Describe a task or ask a question…" : undefined}
              agentName={agentName(data.appearance)}
              mode={currentMode}
              disabled={selectedTask ? pendingDialogTaskIds.has(selectedTask.id) : false}
              onModeChange={(mode) => void setTaskMode(mode)}
              onConfigure={selectedTask ? (patch) => void configure(patch) : configureDraft}
              onSend={(message, images, files, queue) => sendPrompt(message, { images, files, queue })}
              onLiteral={(message, images, files) => sendPrompt(message, { images, files, literal: true })}
              commands={[...enabledAppCommands, ...(selectedTask ? runtime?.slashCommands ?? [] : draftCatalog?.commands ?? [])]}
              commandsReady={selectedTask ? runtime?.slashCommands !== undefined : draftCatalog?.commands !== undefined}
              commandsLoading={selectedTask ? runtime?.slashCommandsLoading : draftCatalog?.loading}
              commandsError={selectedTask ? runtime?.slashCommandsError : draftCatalog?.error}
              onRequestCommands={requestSlashCommands}
              mentionFiles={composerMentions?.files}
              mentionsLoading={composerMentions?.loading}
              mentionsError={composerMentions?.error}
              mentionsTruncated={composerMentions?.truncated}
              onRequestMentions={() => void requestMentions()}
              onCommand={sendSlash}
              queuedMessages={runtime?.queued}
              onDequeue={dequeueMessages}
              draftState={composerDrafts.forChat(composerDraftKey)}
              seed={selectedTask && composerSeed?.taskId === selectedTask.id ? composerSeed : undefined}
              onStop={() => void stopTask()}
              onOpenSettings={openSettings}
            />
          </div>
        )}
      </main>

      {selectedTask && <NativeOverlaysHidden.Provider value={gitMode !== null}><SidePanel
        view={gitMode ? null : panelView}
        width={panelWidth}
        onWidthChange={setPanelWidth}
        label={panelView?.kind === "browser" ? "Browser"
          : panelView?.kind === "terminal" ? "Terminal"
          : shownSubagent
            ? `SubAgent ${displayAgentName(shownSubagentCall?.details.results[shownSubagent.index]?.agent ?? "")}`.trim()
            : "Changes"}
      >{(view) => view.kind === "browser" ? (
        <BrowserPanel
          taskId={view.taskId}
          state={browsers[view.taskId]}
          visible={!confirm && !restoreDialog && !subscriptionLogin && !extensionRequests.some((request) => request.taskId === view.taskId) && !accessRequests.some((request) => request.taskId === view.taskId)}
          expanded={browserExpanded}
          onState={updateBrowser}
          onExpand={toggleBrowserExpanded}
          onReset={() => setConfirm({
            title: "Reset browser session?",
            body: "This closes the page and clears this chat's cookies and site data.",
            confirmLabel: "Reset browser",
            danger: true,
            run: async () => updateBrowser(await api.browserReset(view.taskId))
          })}
          onClose={() => {
            if (browserExpanded) setPanelWidth(browserRestoreWidth.current);
            setBrowserExpanded(false);
            closeSidePanel();
          }}
        />
      ) : view.kind === "terminal" ? (
        <TerminalPanel key={selectedTask.id} taskId={selectedTask.id} appearance={data.appearance} onClose={closeSidePanel} />
      ) : view.kind === "subagent" ? (
        <ChatContexts appearance={data.appearance}>
          <SubagentPanel
            toolCallId={view.toolCallId}
            index={view.index}
            details={shownSubagentCall?.details}
            live={shownSubagentCall !== undefined && !shownSubagentCall.finished}
            stream={runtime?.subagentView}
            onSelect={selectSubagentSibling}
            onClose={closeSidePanel}
            onRetry={() => setWatchNonce((nonce) => nonce + 1)}
          />
        </ChatContexts>
      ) : <ChangesPanel
        key={selectedTask.id}
        changes={changes}
        loading={changesLoading}
        busy={selectedTask.status === "running" || selectedTask.status === "stopping"}
        mode={currentMode}
        canReview={canReview}
        reviewReason={reviewReason}
        comments={data.diffComments[selectedTask.id] ?? []}
        onClose={closeSidePanel}
        onRefresh={() => void refreshChanges(selectedTask.id)}
        onSettings={openSettings}
        onReview={reviewChanges}
        onAction={changeAction}
        onCommit={async (message, files, revision) => {
          try {
            const next = await api.gitCommit({ taskId: selectedTask.id }, { message, files, expected: revision });
            ++changesRequest.current;
            if (selectedTaskRef.current === selectedTask.id) setChanges(next);
          } catch (reason) { void refreshChanges(selectedTask.id); throw reason; }
        }}
        onGenerate={() => api.gitGenerateMessage(selectedTask.id)}
        onPublishInfo={() => api.gitPublishInfo({ taskId: selectedTask.id })}
        onPush={async (remote) => { await api.gitPush({ taskId: selectedTask.id }, remote); void refreshChanges(selectedTask.id); }}
        onPreparePr={(remote) => api.gitPrPrepare({ taskId: selectedTask.id }, remote)}
        onCreatePr={async (remote, base, title, body, draft) => {
          const url = await api.gitPrCreate({ taskId: selectedTask.id }, remote, base, title, body, draft);
          void api.revealPath(url).catch((reason) => setGlobalError(String(reason)));
          return url;
        }}
        onOpenPr={(url) => { void api.revealPath(url).catch((reason) => setGlobalError(String(reason))); }}
        onComments={saveDiffComments}
        onAddressComments={addressDiffComments}
        onOpenGitMode={selectedTask.projectId && !selectedTask.usesWorktree ? (path) => openGit({ projectId: selectedTask.projectId!, path }) : undefined}
      />}</SidePanel></NativeOverlaysHidden.Provider>}
        </>
      )}

      {restoreDialog && (
        <RestoreDialog
          title={restoreDialog.title}
          body={restoreDialog.body}
          changes={restoreDialog.changes}
          initialSelection={restoreDialog.initialSelection}
          sharedWith={restoreDialog.sharedWith}
          choices={restoreDialog.choices}
          onChoose={restoreDialog.run}
          onCancel={() => { restoreDialog.onClose(); setRestoreDialog(undefined); }}
        />
      )}
      {confirm && <ConfirmDialog title={confirm.title} body={confirm.body} confirmLabel={confirm.confirmLabel} danger={confirm.danger} onConfirm={confirm.run} onCancel={() => setConfirm(undefined)} />}
      {subscriptionLogin && <SubscriptionLoginDialog
        login={subscriptionLogin}
        onOpenUrl={(url) => api.openSubscriptionAuthUrl(url)}
        onCopyCode={(code) => writeText(code)}
        onRespond={async (promptId, value) => {
          await api.respondSubscriptionLogin(subscriptionLogin.loginId, promptId, value);
          setSubscriptionLogin((current) => current ? { ...current, prompt: undefined, message: "Waiting for sign-in…" } : current);
        }}
        onCancel={() => {
          if (subscriptionLogin.loginId) void api.cancelSubscriptionLogin(subscriptionLogin.loginId).catch((reason) => setGlobalError(String(reason)));
          else pendingSubscriptionCancel.current = true;
          setSubscriptionLogin(undefined);
        }}
      />}
      {globalError && <div className="global-toast"><span>{globalError}</span><button onClick={() => setGlobalError(undefined)}>×</button></div>}
    </div>
    </ContextMenuProvider>
  );
}
