import { invoke } from "@tauri-apps/api/core";
import type {
  BootstrapPayload,
  GitChanges,
  ProjectRecord,
  ProviderRecord,
  SaveProviderInput,
  TaskRecord,
  ThinkingLevel
} from "./types";

export const api = {
  bootstrap: () => invoke<BootstrapPayload>("bootstrap"),
  saveProvider: (input: SaveProviderInput) => invoke<ProviderRecord>("save_provider", { input }),
  deleteProvider: (providerId: string) => invoke<void>("delete_provider", { providerId }),
  discoverModels: (providerId: string) => invoke<string[]>("discover_models", { input: { providerId } }),
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
  }) => invoke<string>("prompt", { input }),
  stopTask: (taskId: string) => invoke<void>("stop_task", { taskId }),
  archiveTask: (taskId: string) => invoke<TaskRecord>("archive_task", { taskId }),
  unarchiveTask: (taskId: string) => invoke<TaskRecord>("unarchive_task", { taskId }),
  gitChanges: (taskId: string) => invoke<GitChanges>("git_changes", { taskId }),
  revealTask: (taskId: string) => invoke<void>("reveal_task", { taskId }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path })
};
