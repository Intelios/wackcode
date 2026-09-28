import type { ExtensionUIRequest } from "./types";

export interface ChatNavigationActions {
  dismissSettings: () => void;
  abandonDraft: () => void;
  selectTask: (taskId: string) => void;
}

/** Shared by sidebar and native-menu navigation so both leave transient screens cleanly. */
export function performChatNavigation(taskId: string, actions: ChatNavigationActions): void {
  actions.dismissSettings();
  actions.abandonDraft();
  actions.selectTask(taskId);
}

export function withoutResolvedDialog(requests: ExtensionUIRequest[], requestId: string): ExtensionUIRequest[] {
  return requests.filter((request) => request.requestId !== requestId);
}
