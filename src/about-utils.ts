import type { AppInfo } from "./types";

/** The name of this app in the copy block; the agent's own name is a different thing. */
const APP_NAME = "WackCode";

/** "Development build" / "Installed build", as the page's badge and the copy block both say. */
export function buildLabel(build: AppInfo["build"]): string {
  return build === "development" ? "Development build" : "Installed build";
}

/**
 * The plain-text block Settings › About's "Copy details" puts on the clipboard, shaped to be
 * pasted straight into an issue report: identity, the versions of everything bundled, the
 * machine, and this library's size. Only what the page already shows — no paths, no
 * credentials, nothing personal beyond the counts.
 */
export function aboutSummary(info: AppInfo): string {
  const projects = `${info.projectCount} project${info.projectCount === 1 ? "" : "s"}`;
  const chats = `${info.chatCount} chat${info.chatCount === 1 ? "" : "s"}`;
  const archived = `${info.archivedCount} archived`;
  const workers = `${info.activeWorkers} live worker${info.activeWorkers === 1 ? "" : "s"}`;
  return [
    `${APP_NAME} ${info.appVersion} (${buildLabel(info.build)})`,
    `Pi ${info.piVersion} · Node ${info.nodeVersion} (bundled)`,
    `${info.osVersion} · ${info.chip}`,
    `${projects} · ${chats} (${archived}) · ${workers}`
  ].join("\n");
}