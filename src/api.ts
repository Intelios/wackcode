import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  AutoTitleConfig,
  AppearanceConfig,
  BootstrapPayload,
  BuiltinModelSuggestion,
  BrowserState,
  CheckpointChange,
  CheckpointRef,
  ComputerAccessDecision,
  ComputerUseConfig,
  ComputerUseStatus,
  GitChangeFile,
  GitChanges,
  GitBranches,
  GitCheckoutKind,
  GitCheckoutResult,
  GitCommitFiles,
  GitLogPage,
  GitPublishInfo,
  GitPrInfo,
  GitGeneratedMessage,
  GitPullResult,
  GitRevertResult,
  GitSyncStatus,
  GitTarget,
  GitUndoResult,
  DiffComment,
  ImageContent,
  McpServerRecord,
  McpTestResult,
  MemoriesChange,
  MemoriesOverview,
  MemoryConfig,
  MemoryDocument,
  SaveMemoryInput,
  NavigateTaskResult,
  PackageRecord,
  PackageSearchResult,
  ProjectRecord,
  PromptConfig,
  ProviderRecord,
  QuestionAnswer,
  RestoreResult,
  SaveMcpServerInput,
  SaveProviderInput,
  SaveSkillInput,
  SaveSlashCommandInput,
  SkillDocument,
  SkillSearchPage,
  SkillSearchSort,
  SkillsChange,
  SkillsOverview,
  SlashCommand,
  SlashCommandDocument,
  SlashCommandsChange,
  SlashCommandsOverview,
  SubagentConfig,
  SubagentTarget,
  SubscriptionProviderInfo,
  TaskMode,
  TaskRecord,
  TerminalFrame,
  TerminalInfo,
  ThinkingLevel,
  ToolConfig,
  WorkspaceFiles
} from "./types";

export const api = {
  bootstrap: () => invoke<BootstrapPayload>("bootstrap"),
  takeMenuNavigation: () => invoke<string | null>("take_menu_navigation"),
  saveProvider: (input: SaveProviderInput) => invoke<ProviderRecord>("save_provider", { input }),
  setProviderEnabled: (providerId: string, enabled: boolean) => invoke<ProviderRecord>("set_provider_enabled", { providerId, enabled }),
  deleteProvider: (providerId: string) => invoke<void>("delete_provider", { providerId }),
  discoverModels: (providerId: string) => invoke<string[]>("discover_models", { input: { providerId } }),
  listBuiltinModels: () => invoke<BuiltinModelSuggestion[]>("list_builtin_models"),
  listSubscriptionProviders: () => invoke<SubscriptionProviderInfo[]>("list_subscription_providers"),
  startSubscriptionLogin: (providerId: string) => invoke<{ loginId: string; provider: ProviderRecord }>("start_subscription_login", { providerId }),
  respondSubscriptionLogin: (loginId: string, promptId: string, value?: string, cancelled = false) =>
    invoke<void>("respond_subscription_login", { loginId, promptId, value, cancelled }),
  cancelSubscriptionLogin: (loginId: string) => invoke<void>("cancel_subscription_login", { loginId }),
  signOutSubscription: (providerId: string) => invoke<ProviderRecord>("sign_out_subscription", { providerId }),
  openSubscriptionAuthUrl: (url: string) => invoke<void>("open_subscription_auth_url", { url }),
  /** Re-lists signed-in subscriptions' models offline; returns only the providers that changed. */
  refreshSubscriptionModels: () => invoke<ProviderRecord[]>("refresh_subscription_models"),
  setToolConfig: (disabled: string[]) => invoke<ToolConfig>("set_tool_config", { input: { disabled } }),
  setAppearanceConfig: (input: AppearanceConfig) => invoke<AppearanceConfig>("set_appearance_config", { input }),
  browserState: (taskId: string) => invoke<BrowserState>("browser_state", { taskId }),
  browserPresent: (input: { taskId: string; visible: boolean; x: number; y: number; width: number; height: number }) =>
    invoke<BrowserState>("browser_present", { input }),
  browserOpen: (taskId: string, url: string) => invoke<BrowserState>("browser_open", { input: { taskId, url } }),
  browserNavigation: (taskId: string, action: "back" | "forward" | "reload" | "stop") =>
    invoke<void>("browser_navigation", { input: { taskId, action } }),
  browserSetControl: (taskId: string, userControl: boolean) => invoke<BrowserState>("browser_set_control", { taskId, userControl }),
  computerUseStatus: () => invoke<ComputerUseStatus>("computer_use_status"),
  /** Shows macOS's own permission prompt (listing WackCode in the pane) and opens the pane. */
  computerUseRequestPermission: (pane: "accessibility" | "screenRecording") => invoke<void>("computer_use_request_permission", { pane }),
  computerUseOpenSettings: (pane: "accessibility" | "screenRecording") => invoke<void>("computer_use_open_settings", { pane }),
  computerUseResetPermissions: () => invoke<void>("computer_use_reset_permissions"),
  computerUseRelaunch: () => invoke<void>("computer_use_relaunch"),
  setComputerUseConfig: (input: ComputerUseConfig) => invoke<ComputerUseConfig>("set_computer_use_config", { input }),
  /** Answers an access card. "never" also adds the app to Settings' list and returns the new settings. */
  computerUseRespondAccess: (taskId: string, requestId: string, decision: ComputerAccessDecision) =>
    invoke<ComputerUseConfig | null>("computer_use_respond_access", { taskId, requestId, decision }),
  computerUseListApps: () => invoke<{ name: string; bundleId: string }[]>("computer_use_list_apps"),
  /** The original image of a screenshot tool result, or null when it's gone. */
  toolImage: (taskId: string, toolCallId: string, index = 0) => invoke<ImageContent | null>("tool_image", { taskId, toolCallId, index }),
  browserReset: (taskId: string) => invoke<BrowserState>("browser_reset", { taskId }),
  browserReturnFromPopup: (taskId: string) => invoke<BrowserState>("browser_return_from_popup", { taskId }),
  /** Opens the native picker in Rust; null when the user cancels. */
  chooseBackgroundImage: () => invoke<AppearanceConfig | null>("choose_background_image"),
  removeBackgroundImage: () => invoke<AppearanceConfig>("remove_background_image"),
  setSubagentConfig: (input: SubagentConfig) => invoke<SubagentConfig>("set_subagent_config", { input }),
  setAutoTitleConfig: (input: AutoTitleConfig) => invoke<AutoTitleConfig>("set_auto_title_config", { input }),
  usageStatus: () => invoke<import("./types").UsageStatus>("usage_status"),
  setUsageRecording: (enabled: boolean) => invoke<boolean>("set_usage_recording", { enabled }),
  setPromptConfig: (input: PromptConfig) => invoke<PromptConfig>("set_prompt_config", { input }),
  saveMcpServer: (input: SaveMcpServerInput) => invoke<McpServerRecord>("save_mcp_server", { input }),
  deleteMcpServer: (serverId: string) => invoke<void>("delete_mcp_server", { serverId }),
  setMcpServerEnabled: (serverId: string, enabled: boolean) =>
    invoke<McpServerRecord>("set_mcp_server_enabled", { serverId, enabled }),
  setMcpServerTools: (serverId: string, disabledTools: string[]) =>
    invoke<McpServerRecord>("set_mcp_server_tools", { serverId, disabledTools }),
  testMcpServer: (serverId: string) => invoke<McpTestResult>("test_mcp_server", { serverId }),
  listPackages: () => invoke<PackageRecord[]>("list_packages"),
  searchPackages: (query: string, from = 0) =>
    invoke<PackageSearchResult[]>("search_packages", { input: { query, from } }),
  packageDetails: (name: string) => invoke<PackageSearchResult>("package_details", { name }),
  refreshPackages: () => invoke<PackageRecord[]>("refresh_packages"),
  installPackage: (source: string, trusted: boolean, skillsOnly = false) =>
    invoke<PackageRecord[]>("install_package", { input: { source, trusted, skillsOnly } }),
  listSkills: () => invoke<SkillsOverview>("list_skills"),
  readSkill: (path: string) => invoke<SkillDocument>("read_skill", { path }),
  saveSkill: (input: SaveSkillInput) => invoke<SkillsChange>("save_skill", { input }),
  deleteSkill: (path: string) => invoke<SkillsChange>("delete_skill", { path }),
  setSkillEnabled: (path: string, enabled: boolean) => invoke<SkillsChange>("set_skill_enabled", { path, enabled }),
  setSkillFolderEnabled: (id: string, enabled: boolean) => invoke<SkillsChange>("set_skill_folder_enabled", { id, enabled }),
  /** Opens a native folder panel; null when cancelled. */
  addSkillFolder: () => invoke<SkillsChange | null>("add_skill_folder"),
  removeSkillFolder: (id: string) => invoke<SkillsChange>("remove_skill_folder", { id }),
  /** Opens a native panel for a folder or a `.md` file; null when cancelled. */
  importSkill: (kind: "folder" | "file" | "zip") => invoke<SkillsChange | null>("import_skill", { kind }),
  copySkillToLibrary: (path: string) => invoke<SkillsChange>("copy_skill_to_library", { path }),
  searchSkillPackages: (query: string, sort: SkillSearchSort, page: number) =>
    invoke<SkillSearchPage>("search_skill_packages", { input: { query, sort, page } }),
  /** Settings › Commands: a fresh scan of every command a chat's `/` would offer. */
  listSlashCommands: () => invoke<SlashCommandsOverview>("list_slash_commands"),
  readSlashCommand: (path: string) => invoke<SlashCommandDocument>("read_slash_command", { path }),
  saveSlashCommand: (input: SaveSlashCommandInput) => invoke<SlashCommandsChange>("save_slash_command", { input }),
  /** Moves the command's file to the Trash. */
  deleteSlashCommand: (path: string) => invoke<SlashCommandsChange>("delete_slash_command", { path }),
  setSlashCommandEnabled: (key: string, enabled: boolean) =>
    invoke<SlashCommandsChange>("set_slash_command_enabled", { key, enabled }),
  /** Settings › Memory: every project's notes and switches, from a fresh scan. */
  listMemories: () => invoke<MemoriesOverview>("list_memories"),
  readMemory: (path: string) => invoke<MemoryDocument>("read_memory", { path }),
  saveMemory: (input: SaveMemoryInput) => invoke<MemoriesChange>("save_memory", { input }),
  /** Moves the note's file to the Trash. */
  deleteMemory: (path: string) => invoke<MemoriesChange>("delete_memory", { path }),
  /** Moves a whole project's memory folder — notes and all — to the Trash. */
  removeMemoryProject: (dir: string) => invoke<MemoriesChange>("remove_memory_project", { dir }),
  setMemoryConfig: (input: MemoryConfig) => invoke<MemoryConfig>("set_memory_config", { input }),
  setProjectMemoryEnabled: (key: string, enabled: boolean) =>
    invoke<MemoriesChange>("set_project_memory_enabled", { key, enabled }),
  /** Shows one note selected in its project's memory folder (`open -R`). */
  findMemoryInFinder: (path: string) => invoke<void>("find_memory_in_finder", { path }),
  trustPackage: (source: string) => invoke<PackageRecord[]>("trust_package", { input: { source, trusted: true } }),
  removePackage: (source: string) => invoke<PackageRecord[]>("remove_package", { source }),
  updatePackages: (source?: string) => invoke<PackageRecord[]>("update_packages", { source }),
  setPackageResources: (input: {
    source: string;
    extensions?: string[];
    skills?: string[];
    prompts?: string[];
    themes?: string[];
  }) => invoke<PackageRecord[]>("set_package_resources", { input }),
  addProject: (path: string) => invoke<ProjectRecord>("add_project", { path }),
  createTask: (input: {
    projectId?: string | null;
    name?: string;
    useWorktree?: boolean;
    providerId: string;
    modelId: string;
    thinkingLevel: ThinkingLevel;
  }) => invoke<TaskRecord>("create_task", { input }),
  renameTask: (taskId: string, name: string) => invoke<TaskRecord>("rename_task", { taskId, name }),
  /** Clears a chat's saved error; Dismiss must persist or the banner returns on relaunch. */
  clearTaskError: (taskId: string) => invoke<void>("clear_task_error", { taskId }),
  deleteTask: (taskId: string) => invoke<void>("delete_task", { taskId }),
  convertToWorktree: (taskId: string) => invoke<TaskRecord>("convert_task_to_worktree", { taskId }),
  removeProject: (projectId: string) => invoke<void>("remove_project", { projectId }),
  configureTask: (input: {
    taskId: string;
    providerId: string;
    modelId: string;
    thinkingLevel: ThinkingLevel;
  }) => invoke<TaskRecord>("configure_task", { input }),
  openTask: (taskId: string) => invoke<void>("open_task", { taskId }),
  /** Stream one sub-agent's transcript as `subagent_stream` events (a reset frame first); null stops. */
  watchSubagent: (taskId: string, target: SubagentTarget | null) => invoke<void>("watch_subagent", { taskId, target }),
  /** Full keyless `/` catalog for the welcome composer; this never creates a chat. */
  listDraftCommands: (projectId?: string | null) => invoke<SlashCommand[]>("list_draft_commands", { projectId }),
  listCommands: (taskId: string) => invoke<SlashCommand[]>("list_commands", { taskId }),
  executeCommand: (input: { taskId: string; commandId: string; args: string; startedAt: number; images?: ImageContent[]; name?: string }) => invoke<string>("execute_command", { input }),
  initAgents: (taskId: string, startedAt: number) => invoke<string>("init_agents", { taskId, startedAt }),
  compactTask: (taskId: string, instructions: string, startedAt: number) => invoke<string>("compact_task", { taskId, instructions, startedAt }),
  goalControl: (taskId: string, action: "set" | "pause" | "resume" | "clear", objective?: string, startedAt?: number) =>
    invoke<string>("goal_control", { taskId, action, objective, startedAt }),
  prompt: (input: {
    taskId: string;
    message: string;
    startedAt: number;
    providerId: string;
    modelId: string;
    thinkingLevel: ThinkingLevel;
    /** The composer's mode; applied before the run. Required for a draft's first message. */
    mode?: TaskMode;
    /** Refused by Rust unless the model has Vision turned on. */
    images?: ImageContent[];
    literal?: boolean;
  }) => invoke<string>("prompt", { input }),
  /**
   * Queue a message on a chat's running prompt: "steer" delivers at the run's next boundary,
   * "follow_up" waits for the run to finish. The worker runs it as a fresh prompt if the run
   * has already settled.
   */
  queueMessage: (input: {
    taskId: string;
    behavior: "steer" | "follow_up";
    message: string;
    images?: ImageContent[];
    /** Sent raw, without command, skill or template expansion. */
    literal?: boolean;
  }) => invoke<void>("queue_message", { input }),
  /** Take the queued messages back out of Pi's pending lists, for the composer. */
  dequeueMessages: (taskId: string) => invoke<{ steering: string[]; followUp: string[] }>("dequeue_messages", { taskId }),
  /** Send a message again as a new version: unchanged (retry) or with new text (edit). */
  resendMessage: (input: {
    taskId: string;
    entryId: string;
    message?: string;
    removeImages?: number[];
    /** Put these files back first; restored again if the resend is refused. */
    restore?: { checkpointId: string; paths?: string[] };
    startedAt: number;
    providerId: string;
    modelId: string;
    thinkingLevel: ThinkingLevel;
  }) => invoke<string>("resend_message", { input }),
  /** Rewind to before a message, switch to another version, or undo a rewind. */
  navigateTask: (input: {
    taskId: string;
    entryId: string;
    target: "before" | "latest";
    kind: "rewind" | "switch" | "undo";
    restore?: { checkpointId: string; paths?: string[] };
  }) => invoke<NavigateTaskResult>("navigate_task", { input }),
  restoreCheckpoint: (input: { taskId: string; checkpointId: string; paths?: string[] }) =>
    invoke<RestoreResult>("restore_checkpoint", { input }),
  checkpointChanges: (taskId: string, checkpointId: string) =>
    invoke<CheckpointChange[]>("checkpoint_changes", { taskId, checkpointId }),
  /** A new chat from a turn of this one (its end when `entryId` is omitted). */
  forkTask: (input: { taskId: string; entryId?: string; checkpoint?: CheckpointRef }) =>
    invoke<TaskRecord>("fork_task", { input }),
  setTaskMode: (taskId: string, mode: TaskMode) => invoke<TaskRecord>("set_task_mode", { input: { taskId, mode } }),
  /** Save the proposed plan as PLAN.md in the task workspace. Returns the path. */
  exportPlan: (taskId: string, content: string) => invoke<string>("export_plan", { input: { taskId, content } }),
  stopTask: (taskId: string) => invoke<void>("stop_task", { taskId }),
  archiveTask: (taskId: string) => invoke<TaskRecord>("archive_task", { taskId }),
  unarchiveTask: (taskId: string) => invoke<TaskRecord>("unarchive_task", { taskId }),
  // Git commands take a `GitTarget`: a chat's workspace, or a project's folder (Git mode, and a
  // draft's branch picker before the project has a chat).
  gitChanges: (target: GitTarget) => invoke<GitChanges>("git_changes", target),
  gitChangeAction: (target: GitTarget, input: { file: string; layer: "staged" | "working"; action: "discard"; hunkId?: number; expected: string }) => invoke<GitChanges>("git_change_action", { ...target, ...input }),
  gitCommit: (target: GitTarget, input: { message: string; files: string[]; expected: string }) => invoke<GitChanges>("git_commit", { ...target, ...input }),
  gitPublishInfo: (target: GitTarget) => invoke<GitPublishInfo>("git_publish_info", target),
  gitPush: (target: GitTarget, remote?: string) => invoke<GitPublishInfo>("git_push", { ...target, remote }),
  gitBranches: (target: GitTarget) => invoke<GitBranches>("git_branches", target),
  gitCheckout: (target: GitTarget, name: string, kind: GitCheckoutKind) => invoke<GitCheckoutResult>("git_checkout", { ...target, name, kind }),
  gitPrPrepare: (target: GitTarget, remote: string) => invoke<GitPrInfo>("git_pr_prepare", { ...target, remote }),
  gitPrCreate: (target: GitTarget, remote: string, base: string, title: string, body: string, draft: boolean) => invoke<string>("git_pr_create", { ...target, remote, base, title, body, draft }),
  /** Written by the chat's own model; `files` limits the diff it sees, `body` adds a description. */
  gitGenerateMessage: (taskId: string, options: { files?: string[]; body?: boolean } = {}) => invoke<GitGeneratedMessage>("git_generate_message", { taskId, ...options }),
  gitSyncStatus: (target: GitTarget) => invoke<GitSyncStatus>("git_sync_status", target),
  /** The fetch remote's configured URL; null when the checkout has no remote. */
  gitRemoteUrl: (target: GitTarget) => invoke<string | null>("git_remote_url", target),
  /** Network: the user's own remote. `background` is Git mode's one fetch on open, which never prompts for credentials. */
  gitFetch: (target: GitTarget, background = false) => invoke<GitSyncStatus>("git_fetch", { ...target, background }),
  gitPull: (target: GitTarget) => invoke<GitPullResult>("git_pull", target),
  gitLog: (target: GitTarget, skip: number, limit: number) => invoke<GitLogPage>("git_log", { ...target, skip, limit }),
  gitCommitFiles: (target: GitTarget, sha: string) => invoke<GitCommitFiles>("git_commit_files", { ...target, sha }),
  gitCommitDiff: (target: GitTarget, sha: string, path: string, oldPath: string | null) => invoke<GitChangeFile>("git_commit_diff", { ...target, sha, path, oldPath }),
  gitUndoCommit: (target: GitTarget, sha: string) => invoke<GitUndoResult>("git_undo_commit", { ...target, sha }),
  gitRevertCommit: (target: GitTarget, sha: string) => invoke<GitRevertResult>("git_revert_commit", { ...target, sha }),
  setDiffComments: (taskId: string, comments: DiffComment[]) => invoke<DiffComment[]>("set_diff_comments", { taskId, comments }),
  listWorkspaceFiles: (taskId?: string, projectId?: string) => invoke<WorkspaceFiles>("list_workspace_files", { taskId, projectId }),
  revealTask: (taskId: string) => invoke<void>("reveal_task", { taskId }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
  /** The GUI editors installed on this Mac (VS Code, Zed, …), for Git mode's picker. */
  listEditors: () => invoke<string[]>("list_editors"),
  openInEditor: (target: GitTarget, editor: string) => invoke<void>("open_in_editor", { ...target, editor }),
  // Terminal output rides a Channel, not React state: PTY bytes stream straight into xterm.
  openTerminal: (taskId: string, cols: number, rows: number, onFrame: (frame: TerminalFrame) => void) => {
    const on_frame = new Channel<TerminalFrame>();
    on_frame.onmessage = onFrame;
    return invoke<TerminalInfo>("open_terminal", { input: { taskId, cols, rows }, onFrame: on_frame });
  },
  writeTerminal: (taskId: string, data: string) => invoke<void>("write_terminal", { input: { taskId, data } }),
  resizeTerminal: (taskId: string, cols: number, rows: number) => invoke<void>("resize_terminal", { input: { taskId, cols, rows } }),
  detachTerminal: (taskId: string) => invoke<void>("detach_terminal", { taskId }),
  restartTerminal: (taskId: string, cols: number, rows: number, onFrame: (frame: TerminalFrame) => void) => {
    const on_frame = new Channel<TerminalFrame>();
    on_frame.onmessage = onFrame;
    return invoke<TerminalInfo>("restart_terminal", { input: { taskId, cols, rows }, onFrame: on_frame });
  },
  closeTerminal: (taskId: string) => invoke<void>("close_terminal", { taskId }),
  respondExtensionUi: (input: {
    taskId: string;
    requestId: string;
    value?: string;
    confirmed?: boolean;
    cancelled?: true;
    answers?: QuestionAnswer[];
    wrapUp?: true;
  }) => invoke<void>("respond_extension_ui", { input })
};
