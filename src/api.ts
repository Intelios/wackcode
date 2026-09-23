import { invoke } from "@tauri-apps/api/core";
import type {
  AutoTitleConfig,
  AppearanceConfig,
  BootstrapPayload,
  BuiltinModelSuggestion,
  CheckpointChange,
  CheckpointRef,
  GitChanges,
  ImageContent,
  NavigateTaskResult,
  PackageRecord,
  PackageSearchResult,
  ProjectRecord,
  PromptConfig,
  ProviderRecord,
  QuestionAnswer,
  RestoreResult,
  SaveProviderInput,
  SlashCommand,
  SubagentConfig,
  SubscriptionProviderInfo,
  TaskMode,
  TaskRecord,
  ThinkingLevel,
  ToolConfig,
  WorkspaceFiles
} from "./types";

export const api = {
  bootstrap: () => invoke<BootstrapPayload>("bootstrap"),
  saveProvider: (input: SaveProviderInput) => invoke<ProviderRecord>("save_provider", { input }),
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
  setToolConfig: (disabled: string[]) => invoke<ToolConfig>("set_tool_config", { input: { disabled } }),
  setAppearanceConfig: (input: AppearanceConfig) => invoke<AppearanceConfig>("set_appearance_config", { input }),
  /** Opens the native picker in Rust; null when the user cancels. */
  chooseBackgroundImage: () => invoke<AppearanceConfig | null>("choose_background_image"),
  removeBackgroundImage: () => invoke<AppearanceConfig>("remove_background_image"),
  setSubagentConfig: (input: SubagentConfig) => invoke<SubagentConfig>("set_subagent_config", { input }),
  setAutoTitleConfig: (input: AutoTitleConfig) => invoke<AutoTitleConfig>("set_auto_title_config", { input }),
  setPromptConfig: (input: PromptConfig) => invoke<PromptConfig>("set_prompt_config", { input }),
  listPackages: () => invoke<PackageRecord[]>("list_packages"),
  searchPackages: (query: string, from = 0) =>
    invoke<PackageSearchResult[]>("search_packages", { input: { query, from } }),
  packageDetails: (name: string) => invoke<PackageSearchResult>("package_details", { name }),
  refreshPackages: () => invoke<PackageRecord[]>("refresh_packages"),
  installPackage: (source: string, trusted: boolean) =>
    invoke<PackageRecord[]>("install_package", { input: { source, trusted } }),
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
  listCommands: (taskId: string) => invoke<SlashCommand[]>("list_commands", { taskId }),
  executeCommand: (input: { taskId: string; commandId: string; args: string; startedAt: number; images?: ImageContent[] }) => invoke<string>("execute_command", { input }),
  initAgents: (taskId: string, startedAt: number) => invoke<string>("init_agents", { taskId, startedAt }),
  compactTask: (taskId: string, instructions: string, startedAt: number) => invoke<string>("compact_task", { taskId, instructions, startedAt }),
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
  gitChanges: (taskId: string) => invoke<GitChanges>("git_changes", { taskId }),
  listWorkspaceFiles: (taskId?: string, projectId?: string) => invoke<WorkspaceFiles>("list_workspace_files", { taskId, projectId }),
  revealTask: (taskId: string) => invoke<void>("reveal_task", { taskId }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
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
