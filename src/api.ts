import { invoke } from "@tauri-apps/api/core";
import type {
  BootstrapPayload,
  GitChanges,
  ImageContent,
  PackageRecord,
  PackageSearchResult,
  ProjectRecord,
  ProviderRecord,
  QuestionAnswer,
  SaveProviderInput,
  TaskMode,
  TaskRecord,
  ThinkingLevel,
  ToolConfig
} from "./types";

export const api = {
  bootstrap: () => invoke<BootstrapPayload>("bootstrap"),
  saveProvider: (input: SaveProviderInput) => invoke<ProviderRecord>("save_provider", { input }),
  deleteProvider: (providerId: string) => invoke<void>("delete_provider", { providerId }),
  discoverModels: (providerId: string) => invoke<string[]>("discover_models", { input: { providerId } }),
  setToolConfig: (disabled: string[]) => invoke<ToolConfig>("set_tool_config", { input: { disabled } }),
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
  prompt: (input: {
    taskId: string;
    message: string;
    providerId: string;
    modelId: string;
    thinkingLevel: ThinkingLevel;
    /** The composer's mode; applied before the run. Required for a draft's first message. */
    mode?: TaskMode;
    /** Refused by Rust unless the model has Vision turned on. */
    images?: ImageContent[];
  }) => invoke<string>("prompt", { input }),
  setTaskMode: (taskId: string, mode: TaskMode) => invoke<TaskRecord>("set_task_mode", { input: { taskId, mode } }),
  /** Save the proposed plan as PLAN.md in the task workspace. Returns the path. */
  exportPlan: (taskId: string, content: string) => invoke<string>("export_plan", { input: { taskId, content } }),
  stopTask: (taskId: string) => invoke<void>("stop_task", { taskId }),
  archiveTask: (taskId: string) => invoke<TaskRecord>("archive_task", { taskId }),
  unarchiveTask: (taskId: string) => invoke<TaskRecord>("unarchive_task", { taskId }),
  gitChanges: (taskId: string) => invoke<GitChanges>("git_changes", { taskId }),
  revealTask: (taskId: string) => invoke<void>("reveal_task", { taskId }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
  respondExtensionUi: (input: {
    taskId: string;
    requestId: string;
    value?: string;
    confirmed?: boolean;
    cancelled?: true;
    answers?: QuestionAnswer[];
  }) => invoke<void>("respond_extension_ui", { input })
};
