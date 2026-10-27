/**
 * How the window's width is shared between the sidebar, the chat column and the side panel.
 *
 * The chat column is the one thing that must stay usable, so it keeps `CHAT_MIN`. The panel gets
 * the width the user asked for when that fits; when it doesn't, the sidebar steps aside first
 * (only while the user has it open, and it comes back as soon as there is room again), and only
 * then is the panel clamped. The preference itself is never rewritten here: a window made
 * smaller and then larger again gets the same panel back.
 */
export const SIDEBAR_W = 240;
export const CHAT_MIN = 400;
export const PANEL_MIN = 320;
/** A canvas is a board to look around, so its tab asks for most of what the sidebar leaves. */
export const CANVAS_SHARE = 0.62;

/** The chat column's width class, which its header, gutters and composer follow. */
export type ChatSize = "wide" | "compact" | "narrow";

export interface ShellInput {
  windowWidth: number;
  /** The user's choice; the sidebar may still be hidden when a wide panel needs the room. */
  sidebarOpen: boolean;
  panelOpen: boolean;
  /** The width the user last gave the panel. */
  panelPref: number;
}

export interface ShellLayout {
  sidebarShown: boolean;
  /** Open by choice, hidden only because the panel needs its width. */
  sidebarAutoHidden: boolean;
  /** The panel's width on screen (0 while closed). */
  panelWidth: number;
  chatWidth: number;
  chatSize: ChatSize;
}

export function chatSize(width: number): ChatSize {
  return width < 480 ? "narrow" : width < 640 ? "compact" : "wide";
}

/** The widest the panel can be with `sidebar` px taken: what leaves the chat its minimum. */
export function panelRoom(windowWidth: number, sidebar: number): number {
  return Math.max(PANEL_MIN, windowWidth - sidebar - CHAT_MIN);
}

export function solveShell({ windowWidth, sidebarOpen, panelOpen, panelPref }: ShellInput): ShellLayout {
  const pref = Math.max(PANEL_MIN, panelPref);
  let sidebar = sidebarOpen ? SIDEBAR_W : 0;
  let panel = 0;
  if (panelOpen) {
    if (sidebar && pref > panelRoom(windowWidth, sidebar)) sidebar = 0;
    panel = Math.min(pref, panelRoom(windowWidth, sidebar));
  }
  const chat = Math.max(0, windowWidth - sidebar - panel);
  return {
    sidebarShown: sidebar > 0,
    sidebarAutoHidden: sidebarOpen && sidebar === 0,
    panelWidth: panel,
    chatWidth: chat,
    chatSize: chatSize(chat)
  };
}

/** Dragging the resizer: anything up to the room the panel has once the sidebar steps aside. */
export function clampPanelDrag(width: number, windowWidth: number): number {
  return Math.round(Math.max(PANEL_MIN, Math.min(panelRoom(windowWidth, 0), width)));
}

/**
 * The panel's width until the user drags it (and after a double-click on its edge): half of what
 * the sidebar leaves, so it opens wide on any window and grows with it.
 */
export function balancedPanelWidth(windowWidth: number, sidebarOpen: boolean): number {
  const sidebar = sidebarOpen ? SIDEBAR_W : 0;
  return Math.round(Math.max(PANEL_MIN, Math.min(panelRoom(windowWidth, sidebar), (windowWidth - sidebar) / 2)));
}

/**
 * The canvas tab's width until the user drags it: most of the room beside the sidebar, leaving
 * the chat a comfortable column (never less than `CHAT_MIN`). It has its own preference, so a
 * roomy board never widens Changes or the terminal.
 */
export function canvasPanelWidth(windowWidth: number, sidebarOpen: boolean): number {
  const sidebar = sidebarOpen ? SIDEBAR_W : 0;
  return Math.round(Math.max(PANEL_MIN, Math.min(panelRoom(windowWidth, sidebar), (windowWidth - sidebar) * CANVAS_SHARE)));
}
