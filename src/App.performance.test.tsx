import { memo } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppData, NormalizedMessage, ProviderRecord, SessionSnapshot, TaskRecord } from "./types";
import { DEFAULT_APPEARANCE } from "./theme";
import App from "./App";
import { api } from "./api";

const renders = vi.hoisted(() => ({} as Record<string, number>));
// Count the real component body, retaining its actual memo comparator. Calling through
// the body keeps its hooks on this instrumented fiber; no subtree is stubbed out.
function track(component: any, name: string): any {
  const body = typeof component === "function" ? component : component.type;
  const tracked = (props: any) => { renders[name] = (renders[name] ?? 0) + 1; return body(props); };
  return typeof component === "function" ? tracked : memo(tracked, component.compare);
}
const events = vi.hoisted(() => new Map<string, (event: { payload: unknown }) => void>());
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name, handler) => { events.set(name, handler); return () => events.delete(name); }) }));
vi.mock("@tauri-apps/api/core", () => ({ convertFileSrc: (value: string) => value }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ readText: vi.fn(), writeText: vi.fn() }));
vi.mock("./api", () => ({ api: {
  windowFocused: vi.fn().mockResolvedValue(true), onWindowFocus: vi.fn().mockResolvedValue(() => {}),
  queueMessage: vi.fn().mockResolvedValue(undefined),
  bootstrap: vi.fn(), refreshSubscriptionModels: vi.fn().mockResolvedValue([]),
  takeMenuNavigation: vi.fn().mockResolvedValue(null), setChatTabMenu: vi.fn().mockResolvedValue(undefined),
  computerUseCursorAppearance: vi.fn().mockResolvedValue(undefined), setVisualTheme: vi.fn().mockResolvedValue(undefined), stageVisual: vi.fn().mockResolvedValue("wcvisual://localhost/artboard"), startCanvas: vi.fn().mockResolvedValue("run"), exportArtboard: vi.fn().mockResolvedValue(true), openTask: vi.fn().mockResolvedValue(undefined),
  cancelHistory: vi.fn().mockResolvedValue(undefined), recordHistoryTiming: vi.fn().mockResolvedValue(undefined),
  computerUseRespondAccess: vi.fn(), computerUseRevokeApp: vi.fn(),
  gitPublishInfo: vi.fn().mockResolvedValue({ branch: "main", upstream: null, remotes: [] }),
  gitChanges: vi.fn().mockResolvedValue({ isGit: false, files: [] }), listDraftCommands: vi.fn().mockResolvedValue([]),
  listCommands: vi.fn().mockResolvedValue([]), getRun: vi.fn(async (_taskId, kind = "run") => ({ kind, cwd: "/tmp", run: null, generation: 0 })),
  saveProjectRunCommand: vi.fn(), saveProjectBuildCommand: vi.fn(), startRun: vi.fn(), stopRun: vi.fn(),
  createTask: vi.fn(), prompt: vi.fn(), setTaskMode: vi.fn(), executeCommand: vi.fn().mockResolvedValue(undefined),
  stopTask: vi.fn().mockResolvedValue(undefined), setAppearanceConfig: vi.fn(async (config) => config),
  watchSubagent: vi.fn().mockResolvedValue(undefined), archiveTask: vi.fn(), deleteTask: vi.fn().mockResolvedValue(undefined),
  goalControl: vi.fn().mockResolvedValue(undefined), browserState: vi.fn(), browserPresent: vi.fn(), browserRetirePresentation: vi.fn().mockResolvedValue(undefined),
  setSuggestionDismissed: vi.fn()
} }));
vi.mock("./components/SettingsPage", () => ({ SettingsPage: () => null }));
vi.mock("./components/DynamicBackdrop", () => ({ DynamicBackdrop: () => null }));
vi.mock("./components/TerminalPanel", () => ({ TerminalPanel: () => null }));
vi.mock("./components/RunPanel", () => ({ RunPanel: () => null }));

vi.mock("./components/Sidebar", async (original) => {
  const actual = await original<typeof import("./components/Sidebar")>();
  return { ...actual, Sidebar: track(actual.Sidebar, "Sidebar") };
});
vi.mock("./components/ChatHeader", async (original) => {
  const actual = await original<typeof import("./components/ChatHeader")>();
  return { ...actual, ChatHeader: track(actual.ChatHeader, "ChatHeader") };
});
vi.mock("./components/ModelPicker", async (original) => {
  const actual = await original<typeof import("./components/ModelPicker")>();
  return { ...actual, ModelPicker: track(actual.ModelPicker, "ModelPicker") };
});
vi.mock("./components/Transcript", async (original) => {
  const actual = await original<typeof import("./components/Transcript")>();
  return { ...actual, Transcript: track(actual.Transcript, "Transcript") };
});
vi.mock("./components/chat/ChatAreaHeader", async (original) => {
  const actual = await original<typeof import("./components/chat/ChatAreaHeader")>();
  return { ...actual, ChatAreaHeader: track(actual.ChatAreaHeader, "ChatAreaHeader") };
});
vi.mock("./components/chat/ChatConversation", async (original) => {
  const actual = await original<typeof import("./components/chat/ChatConversation")>();
  return { ...actual, ChatConversation: track(actual.ChatConversation, "ChatConversation") };
});
vi.mock("./components/composer/AttachmentTray", async (original) => {
  const actual = await original<typeof import("./components/composer/AttachmentTray")>();
  return { ...actual, AttachmentTray: track(actual.AttachmentTray, "AttachmentTray") };
});
vi.mock("./components/ProjectBar", async (original) => {
  const actual = await original<typeof import("./components/ProjectBar")>();
  return { ...actual, ProjectBar: track(actual.ProjectBar, "ProjectBar") };
});
vi.mock("./components/ChatTabBar", async (original) => {
  const actual = await original<typeof import("./components/ChatTabBar")>();
  return { ...actual, ChatTabBar: track(actual.ChatTabBar, "ChatTabBar") };
});
const provider: ProviderRecord = {
  id: "p", name: "Mock", baseUrl: "http://127.0.0.1:43127/v1", apiFormat: "openai-completions", kind: "custom",
  connected: true, hasApiKey: false, createdAt: "2026-01-01", updatedAt: "2026-01-01",
  models: ["first", "second"].map((id) => ({ id, name: id, contextWindow: 8000, maxTokens: 1000, reasoning: false, vision: true, thinkingLevels: ["off"], thinkingLevelMap: { off: null } }))
};
const task = (id: string): TaskRecord => ({ id, projectId: null, name: `Chat ${id}`, workspacePath: `/tmp/${id}`, worktreePath: null,
  branch: null, usesWorktree: false, providerId: "p", modelId: "first", thinkingLevel: "off", sessionFile: null, status: "idle", mode: "build",
  archived: false, archivedAt: null, lastError: null, kind: "code", lastActivityAt: null, originSuggestion: null, dismissedSuggestions: [], createdAt: "2026-01-01", updatedAt: "2026-01-01", autoTitleEligible: false, autoTitleAttemptId: null });
function bootstrap(enabled = true, dynamic = false, overrides: Partial<AppData> = {}) {
  const data: AppData = { version: 1, providers: [provider], favoriteModels: [], projects: [], tasks: [task("a"), task("b")], diffComments: {},
    toolConfig: { disabled: [] }, toolCatalog: [], packages: [], subagents: { enabled: false, trigger: "on_request", maxConcurrency: 4, agents: [] },
    autoTitle: { enabled: false, providerId: null, modelId: null }, appearance: { ...DEFAULT_APPEARANCE, chatTabs: enabled, backdrop: dynamic ? "dynamic" : "solid" }, prompts: {}, mcp: { servers: [] }, ...overrides };
  vi.mocked(api.bootstrap).mockResolvedValue({ data, appDataPath: "/tmp", glassSupported: false, computerUseSupported: false, devBuild: false });
  return render(<App />);
}
const chat = (id: string) => fireEvent.click(screen.getByRole("button", { name: `Chat ${id}` }));
const input = () => within(document.querySelector(".composer-input") as HTMLElement).getByRole("textbox");
const chatInput = () => screen.getByRole("textbox", { name: "Message" });
const worker = async (payload: unknown) => { await act(async () => events.get("worker-event")?.({ payload })); };
const snapshot = (messages: NormalizedMessage[]): SessionSnapshot => ({
  rev: 1, sessionId: "session", messages, modelSwitches: [], runTimings: [],
  stats: { tokens: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 }, cost: 0 },
  thinkingLevel: "off", availableThinkingLevels: ["off"], tools: [], activeTools: []
});
beforeEach(() => {
  localStorage.clear(); events.clear(); vi.clearAllMocks(); vi.mocked(api.prompt).mockResolvedValue("run");
  vi.mocked(api.getRun).mockImplementation(async (_taskId, kind = "run") => ({ kind, cwd: "/tmp", run: null, generation: 0 }));
  vi.stubGlobal("requestAnimationFrame", vi.fn(() => 1));
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
});
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

// Opt-in probe: WACKCODE_PERF=1 pnpm test:web src/App.performance.test.tsx --maxWorkers=1 --disableConsoleIntercept
// Measures a flushed event + React commit, excluding query/setup, layout/paint and rAF
// reveal animations (§1.1 is a separate cost). For comparable timings, select one
// scenario with -t so every scenario gets a fresh process; use 30 warmups + 70 samples.
const benchmark = process.env.WACKCODE_PERF ? describe : describe.skip;
benchmark("App hot-path benchmark", () => {
  const scenarios = [
    [40, 240, 5120], [40, 240, 600], [2, 240, 5120], [40, 0, 5120], [2, 0, 600],
  ];
  it.each(scenarios)("partial: %i chats, %i messages, %i chars", async (chats, count, chars) => {
    bootstrap(false, false, { tasks: Array.from({ length: chats }, (_, i) => task(i ? `chat-${i}` : "a")) });
    fireEvent.click(await screen.findByRole("button", { name: "Chat a" }));
    await worker({ type: "snapshot", taskId: "a", snapshot: snapshot(Array.from({ length: count }, (_, i) => ({
      id: `m${i}`, entryId: `m${i}`, role: i % 2 ? "assistant" : "user", blocks: [{ type: "text", text: `Message ${i}: A short saved paragraph.` }]
    }))) });
    await worker({ type: "run_state", taskId: "a", state: "running", runId: "r", startedAt: Date.now() });
    const base = "A paragraph with **bold** and `code` words.\n\n".repeat(150).slice(0, chars);
    const before = { ...renders };
    const samples: number[] = [];
    for (let i = 0; i < 100; i++) {
      const start = performance.now();
      await worker({ type: "partial", taskId: "a", message: { id: "live", role: "assistant", blocks: [{ type: "text", text: base + i }] } });
      if (i >= 30) samples.push(performance.now() - start);
    }
    samples.sort((a, b) => a - b);
    console.log(JSON.stringify({ scenario: "partial", chats, count, chars, median: samples[Math.floor(samples.length / 2)], mean: samples.reduce((a, b) => a + b, 0) / samples.length, renders: Object.fromEntries(Object.keys(renders).map((key) => [key, renders[key] - (before[key] ?? 0)])) }));
  });
  it.each([[1, 0], [40, 240]])("typing: %i chats, %i messages", async (chats, count) => {
    bootstrap(false, false, { tasks: Array.from({ length: chats }, (_, i) => task(i ? `chat-${i}` : "a")) });
    fireEvent.click(await screen.findByRole("button", { name: "Chat a" }));
    await worker({ type: "snapshot", taskId: "a", snapshot: snapshot(Array.from({ length: count }, (_, i) => ({
      id: `m${i}`, entryId: `m${i}`, role: i % 2 ? "assistant" : "user", blocks: [{ type: "text", text: `Message ${i}: A short saved paragraph.` }]
    }))) });
    const field = input();
    const before = { ...renders };
    const samples: number[] = [];
    for (let i = 0; i < 100; i++) {
      const start = performance.now();
      await act(async () => { fireEvent.change(field, { target: { value: "x".repeat(i + 1) } }); });
      if (i >= 30) samples.push(performance.now() - start);
    }
    samples.sort((a, b) => a - b);
    console.log(JSON.stringify({ scenario: "typing", chats, count, median: samples[Math.floor(samples.length / 2)], mean: samples.reduce((a, b) => a + b, 0) / samples.length, renders: Object.fromEntries(Object.keys(renders).map((key) => [key, renders[key] - (before[key] ?? 0)])) }));
  });
});

describe("App render isolation", () => {
  it("keeps the welcome project picker and shell out of typing and background partials", async () => {
    bootstrap(false);
    await screen.findByRole("button", { name: "Settings" });
    const field = input();
    for (const name of Object.keys(renders)) renders[name] = 0;
    for (let i = 0; i < 3; i++) await act(async () => { fireEvent.change(field, { target: { value: `Welcome draft ${i}` } }); });
    await worker({ type: "partial", taskId: "a", message: { id: "background", role: "assistant", blocks: [{ type: "text", text: "Background answer" }] } });
    for (const name of ["Sidebar", "ProjectBar", "ModelPicker", "Transcript", "ChatTabBar"])
      expect(renders[name] ?? 0, `${name} on welcome`).toBe(0);
    expect(field).toHaveValue("Welcome draft 2");
  });
  it.each(["code", "chat"] as const)("keeps the %s shell and editor out of partial frames, and history out of typing", async (kind) => {
    bootstrap(true, false, { tasks: [{ ...task("a"), kind }, { ...task("b"), kind }] });
    await screen.findByRole("button", { name: "Settings" });
    if (kind === "chat") fireEvent.click(within(screen.getByRole("group", { name: "Area" })).getByRole("button", { name: "Chat" }));
    fireEvent.click(await screen.findByRole("button", { name: "Chat a" }));
    await worker({ type: "snapshot", taskId: "a", snapshot: snapshot([
      { id: "u", role: "user", blocks: [{ type: "text", text: "Saved question" }] },
      { id: "a", role: "assistant", blocks: [{ type: "text", text: "Saved answer" }] },
    ]) });
    await worker({ type: "run_state", taskId: "a", state: "running", runId: "r", startedAt: Date.now() });
    for (const name of Object.keys(renders)) renders[name] = 0;
    for (let i = 0; i < 3; i++) await worker({ type: "partial", taskId: "a", message: {
      id: "live", role: "assistant", blocks: [{ type: "text", text: `Live words ${i}` }]
    } });
    for (const name of ["Sidebar", "ChatHeader", "ChatAreaHeader", "ChatTabBar", "AttachmentTray", "ModelPicker"])
      expect(renders[name] ?? 0, `${name} during partials`).toBe(0);
    expect(renders[kind === "code" ? "Transcript" : "ChatConversation"]).toBe(3);
    const field = kind === "code" ? input() : chatInput();
    for (const name of Object.keys(renders)) renders[name] = 0;
    for (let i = 0; i < 3; i++) await act(async () => { fireEvent.change(field, { target: { value: `Draft ${i}` } }); });
    for (const name of ["Sidebar", "ChatHeader", "ChatAreaHeader", "ChatTabBar", "Transcript", "ChatConversation", "ModelPicker"])
      expect(renders[name] ?? 0, `${name} during typing`).toBe(0);
    expect(field).toHaveValue("Draft 2");
    // Selection changes must rebind the handlers even though their identities stay stable.
    chat("b");
    const other = kind === "code" ? input() : chatInput();
    fireEvent.change(other, { target: { value: "Send from b" } });
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(api.prompt).toHaveBeenCalledWith(expect.objectContaining({ taskId: "b", message: "Send from b" })));
    chat("a");
    expect(kind === "code" ? input() : chatInput()).toHaveValue("Draft 2");
  });
});
