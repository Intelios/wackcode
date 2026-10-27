import { useImperativeHandle, type Ref } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppData, BrowserState, ComputerAccessRequest, ComputerAppGrant, ComputerUseConfig, NormalizedMessage, ProviderRecord, RunInfo, SessionSnapshot, TaskRecord } from "./types";
import { DEFAULT_APPEARANCE } from "./theme";
import App from "./App";
import { api } from "./api";
import { composeFileSection } from "./attachment-utils";

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
  computerUseCursorAppearance: vi.fn().mockResolvedValue(undefined), setVisualTheme: vi.fn().mockResolvedValue(undefined), stageVisual: vi.fn().mockResolvedValue("wcvisual://localhost/artboard"), startCanvas: vi.fn().mockResolvedValue("run"), openTask: vi.fn().mockResolvedValue(undefined),
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
vi.mock("./components/SettingsPage", () => ({ SettingsPage: (props: { initialSection?: string; appearance: typeof DEFAULT_APPEARANCE; toolCatalog: { name: string }[]; computerUse?: ComputerUseConfig; onRevokeComputerApp?: (grant: ComputerAppGrant) => Promise<void>; onSetAppearance: (value: typeof DEFAULT_APPEARANCE) => void; onClose: () => void }) => <>
  <output aria-label="Initial Settings section">{props.initialSection ?? "providers"}</output>
  <output aria-label="Tool catalogue">{props.toolCatalog.map((tool) => tool.name).join(",")}</output>
  <output aria-label="Saved computer app approvals">{props.computerUse?.alwaysAllow.map((grant) => grant.name).join(",")}</output>
  {props.computerUse?.alwaysAllow[0] && <button onClick={() => void props.onRevokeComputerApp?.(props.computerUse!.alwaysAllow[0])}>Revoke first app</button>}
  <button onClick={() => props.onSetAppearance({ ...props.appearance, chatTabs: !props.appearance.chatTabs })}>Toggle tabs</button>
  <button onClick={() => { Promise.resolve(props.onSetAppearance({ ...props.appearance, dynamicStyle: "glyphFlow" })).catch(() => undefined); }}>Use Glyph Flow</button>
  <button onClick={props.onClose}>Close settings</button>
</> }));
const dynamic = vi.hoisted(() => ({ pulse: vi.fn(), ripple: vi.fn() }));
vi.mock("./components/DynamicBackdrop", () => ({ DynamicBackdrop: (props: { ref: Ref<typeof dynamic>; phase: string; style: string }) => {
  useImperativeHandle(props.ref, () => dynamic, []);
  return <output aria-label="Dynamic background">{props.style}:{props.phase}</output>;
} }));
const terminalAttach = vi.hoisted(() => ({ current: undefined as undefined | ((text: string) => void) }));
vi.mock("./components/TerminalPanel", () => ({ TerminalPanel: ({ taskId, onAttach }: { taskId: string; onAttach?: (text: string) => void }) => {
  terminalAttach.current = onAttach;
  return <div aria-label={`Terminal ${taskId}`} />;
} }));
vi.mock("./components/RunPanel", () => ({ RunPanel: ({ kind, run }: { kind: string; run?: RunInfo }) => <output aria-label={`${kind} output`}>{run?.command}:{run?.status}</output> }));

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
const newTab = () => fireEvent.click(screen.getByRole("button", { name: "New chat tab" }));
const chat = (id: string) => fireEvent.click(screen.getByRole("button", { name: `Chat ${id}` }));
const input = () => within(document.querySelector(".composer-input") as HTMLElement).getByRole("textbox");
const chatInput = () => screen.getByRole("textbox", { name: "Message" });
const type = (text: string) => fireEvent.change(input(), { target: { value: text } });
const native = async (command: string) => { await act(async () => events.get("native-tab-action")?.({ payload: command })); };
const worker = async (payload: unknown) => { await act(async () => events.get("worker-event")?.({ payload })); };
const snapshot = (messages: NormalizedMessage[]): SessionSnapshot => ({
  rev: 1, sessionId: "session", messages, modelSwitches: [], runTimings: [],
  stats: { tokens: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 }, cost: 0 },
  thinkingLevel: "off", availableThinkingLevels: ["off"], tools: [], activeTools: []
});
const browser = (taskId: string): BrowserState => ({ taskId, exists: true, url: `http://localhost/${taskId}`, title: taskId,
  loading: false, canGoBack: false, canGoForward: false, agentActive: false, userControl: false, popup: false });
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (reason: Error) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }

beforeEach(() => {
  localStorage.clear(); events.clear(); vi.clearAllMocks(); vi.mocked(api.prompt).mockResolvedValue("run");
  vi.mocked(api.getRun).mockImplementation(async (_taskId, kind = "run") => ({ kind, cwd: "/tmp", run: null, generation: 0 }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
});
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("App project builds", () => {
  it("saves and launches a build beside Run, shares its output across chats, and stops only the build", async () => {
    const project = { id: "project", name: "Project", path: "/tmp", gitRoot: null, gitHasHead: false, runCommand: "pnpm dev", buildCommand: null, branch: null, createdAt: "now" };
    const run: RunInfo = { kind: "run", sessionId: "server", generation: 1, revision: 1, cwd: "/tmp", command: "pnpm dev", status: "running", exit: null };
    const build: RunInfo = { ...run, kind: "build", sessionId: "build", generation: 2, command: "pnpm build" };
    vi.mocked(api.getRun).mockImplementation(async (_taskId, kind = "run") => ({ kind, cwd: "/tmp", run: kind === "run" ? run : null, generation: 1 }));
    vi.mocked(api.saveProjectBuildCommand).mockResolvedValue({ ...project, buildCommand: "pnpm build" });
    vi.mocked(api.startRun).mockResolvedValue(build);
    vi.mocked(api.stopRun).mockResolvedValue({ ...build, revision: 2, status: "stopped" });
    bootstrap(false, false, { projects: [project], tasks: ["a", "b"].map((id) => ({ ...task(id), projectId: "project", workspacePath: "/tmp" })) });
    fireEvent.click(await screen.findByRole("button", { name: "Chat a" }));
    await screen.findByRole("button", { name: "Stop run" });
    fireEvent.click(screen.getByRole("button", { name: "Build" }));
    fireEvent.change(screen.getByLabelText("Command"), { target: { value: "pnpm build" } });
    fireEvent.click(screen.getByRole("button", { name: "Save & Build" }));
    await waitFor(() => expect(api.startRun).toHaveBeenCalledWith("a", "build"));
    expect(api.saveProjectBuildCommand).toHaveBeenCalledWith("project", "pnpm build");
    expect(api.saveProjectRunCommand).not.toHaveBeenCalled();
    expect(await screen.findByLabelText("build output")).toHaveTextContent("pnpm build:running");
    expect(screen.getByRole("button", { name: "Stop run" })).toBeInTheDocument();
    chat("b");
    await waitFor(() => expect(screen.getByRole("heading", { name: "Chat b" })).toBeInTheDocument());
    expect(screen.getByLabelText("build output")).toHaveTextContent("pnpm build:running");
    fireEvent.click(screen.getByRole("button", { name: "Stop build" }));
    await waitFor(() => expect(api.stopRun).toHaveBeenCalledExactlyOnceWith("build"));
    await waitFor(() => expect(screen.getByLabelText("build output")).toHaveTextContent("pnpm build:stopped"));
    expect(screen.getByRole("button", { name: "Stop run" })).toBeInTheDocument();
  });
});

describe("App inline Canvas", () => {
  let scrollTo: PropertyDescriptor | undefined;
  beforeEach(() => {
    scrollTo = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollTo");
    Object.defineProperty(HTMLElement.prototype, "scrollTo", { configurable: true, value: function (this: HTMLElement, options: ScrollToOptions) {
      this.scrollTop = options.top ?? this.scrollTop;
    } });
  });
  afterEach(() => {
    if (scrollTo) Object.defineProperty(HTMLElement.prototype, "scrollTo", scrollTo);
    else Reflect.deleteProperty(HTMLElement.prototype, "scrollTo");
  });
  const canvasMessages = (): NormalizedMessage[] => [
    { id: "canvas-user", entryId: "canvas-user", role: "user", timestamp: 100, blocks: [{ type: "text", text: "Explore a billing screen" }] },
    { id: "canvas-call", role: "assistant", timestamp: 200, blocks: [{ type: "tool-call", toolName: "canvas", toolCallId: "canvas-create", arguments: {
      title: "Billing themes", artboards: [{ id: "base", name: "Base", width: 1280, height: 800, html: "<main>Billing</main>" }]
    } }] },
    { id: "canvas-result", role: "tool", timestamp: 300, blocks: [{ type: "tool-result", toolName: "canvas", toolCallId: "canvas-create", text: "Created Billing themes", details: {
      v: 1, canvas: "themes", title: "Billing themes", applied: [{ index: 0, id: "base", op: "add" }], removed: []
    } }] }
  ];

  const canvasReferenceMessages = (): NormalizedMessage[] => [
    ...canvasMessages(),
    { id: "canvas-user-2", entryId: "canvas-user-2", role: "user", timestamp: 400, blocks: [{ type: "text", text: "Add a phone layout" }] },
    { id: "canvas-revise-call", role: "assistant", timestamp: 500, blocks: [{ type: "tool-call", toolName: "canvas", toolCallId: "canvas-revise", arguments: {
      canvas: "themes", artboards: [{ id: "phone", name: "Phone", width: 390, basedOn: "base" }]
    } }] },
    { id: "canvas-revise-result", role: "tool", timestamp: 600, blocks: [{ type: "tool-result", toolName: "canvas", toolCallId: "canvas-revise", text: "Revised Billing themes", details: {
      v: 1, canvas: "themes", title: "Billing themes", applied: [{ index: 0, id: "phone", op: "add", basedOn: "base", width: 390 }], removed: []
    } }] }
  ];

  async function showCanvas(tabs = false, messages = canvasMessages()) {
    bootstrap(tabs);
    await screen.findByRole("button", { name: "Settings" });
    chat("a");
    await worker({ type: "snapshot", taskId: "a", snapshot: snapshot(messages) });
    return screen.findByRole("button", { name: "Expand canvas" });
  }

  it("shows an accepted board during a run and expands without replacing the composer or Changes", async () => {
    await showCanvas();
    await worker({ type: "run_state", taskId: "a", state: "running", runId: "drawing", startedAt: 100 });
    expect(screen.getByRole("region", { name: "Canvas: Billing themes" })).toBeInTheDocument();
    expect(screen.queryByRole("complementary", { name: "Canvas" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Changes" }));
    const changes = await screen.findByRole("complementary", { name: "Changes" });
    const panelWidth = changes.querySelector<HTMLElement>(".side-panel-inner")!.style.width;
    type("Keep this draft");
    const composer = input();

    fireEvent.click(screen.getByRole("button", { name: "Expand canvas" }));
    expect(document.querySelector(".app-shell")).toHaveClass("canvas-expanded");
    expect(screen.getByRole("region", { name: "Expanded canvas" })).toBeInTheDocument();
    expect(input()).toBe(composer);
    expect(composer).toHaveValue("Keep this draft");
    expect(screen.getAllByRole("region", { name: "Canvas: Billing themes" })).toHaveLength(1);

    await worker({ type: "extension_ui_request", taskId: "a", requestId: "canvas-question", method: "questions", title: "Choose a direction", questions: [
      { id: "direction", header: "Direction", question: "Which billing direction should I refine?", options: [{ label: "Base", description: "Refine the base screen." }] }
    ] });
    expect(screen.getByRole("region", { name: "Questions" })).toBeInTheDocument();
    expect(input()).toBe(composer);
    fireEvent.click(within(screen.getByRole("region", { name: "Expanded canvas" })).getByRole("button", { name: "Restore canvas" }));
    await waitFor(() => expect(document.querySelector(".app-shell")).not.toHaveClass("canvas-expanded"));
    expect(screen.getByRole("complementary", { name: "Changes" })).toBe(changes);
    expect(changes.querySelector<HTMLElement>(".side-panel-inner")!.style.width).toBe(panelWidth);
    expect(composer).toHaveValue("Keep this draft");
    expect(screen.getByRole("region", { name: "Questions" })).toBeInTheDocument();
    expect(api.stopTask).not.toHaveBeenCalled();
  });

  it("hides the native browser while expanded and restores its presentation", async () => {
    vi.mocked(api.browserState).mockImplementation(async (id) => browser(id));
    vi.mocked(api.browserPresent).mockImplementation(async ({ taskId }) => browser(taskId));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      return this.classList.contains("browser-surface") ? new DOMRect(1000, 100, 430, 700) : new DOMRect();
    });
    await showCanvas();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "requestAnimationFrame", "cancelAnimationFrame", "performance"] });
    const settle = async () => {
      await act(async () => { vi.advanceTimersByTime(500); });
      await act(async () => { vi.advanceTimersByTime(500); });
    };
    fireEvent.click(screen.getByRole("button", { name: "Browser" }));
    await settle();
    expect(api.browserPresent).toHaveBeenLastCalledWith(expect.objectContaining({ taskId: "a", visible: true }));
    const address = screen.getByRole("textbox", { name: "Browser address" });
    fireEvent.click(screen.getByRole("button", { name: "Expand canvas" }));
    await settle();
    expect(api.browserPresent).toHaveBeenLastCalledWith(expect.objectContaining({ taskId: "a", visible: false }));
    fireEvent.click(within(screen.getByRole("region", { name: "Expanded canvas" })).getByRole("button", { name: "Restore canvas" }));
    await settle();
    expect(api.browserPresent).toHaveBeenLastCalledWith(expect.objectContaining({ taskId: "a", visible: true }));
    expect(screen.getByRole("textbox", { name: "Browser address" })).toBe(address);
    expect(address).toHaveValue("http://localhost/a");
    expect(api.stopTask).not.toHaveBeenCalled();
  });

  it.each(["chat", "settings"])("does not replay Canvas navigation when returning from %s", async (navigation) => {
    await showCanvas(true, canvasReferenceMessages());
    const navigate = vi.fn(function (this: HTMLElement, options: ScrollToOptions) {
      this.scrollTop = options.top ?? this.scrollTop;
    });
    Object.defineProperty(HTMLElement.prototype, "scrollTo", { configurable: true, value: navigate });
    fireEvent.click(screen.getByRole("button", { name: "View latest" }));
    expect(navigate).toHaveBeenCalledOnce();

    // The user can read elsewhere after the explicit jump. Returning should use the
    // transcript's saved reading state rather than replay that old Canvas request.
    const scroller = document.querySelector<HTMLElement>(".conversation-scroll")!;
    scroller.scrollTop = 180;
    fireEvent.scroll(scroller);
    if (navigation === "chat") {
      chat("b");
      expect(screen.getByRole("heading", { name: "Chat b" })).toBeInTheDocument();
      chat("a");
    } else {
      fireEvent.click(screen.getByRole("button", { name: "Settings" }));
      fireEvent.click(screen.getByRole("button", { name: "Close settings" }));
    }
    expect(await screen.findByRole("region", { name: "Canvas: Billing themes" })).toBeInTheDocument();
    expect(navigate).toHaveBeenCalledOnce();
  });

  it.each(["rewind", "chat", "area", "settings"])("closes expansion on %s without stopping the chat", async (navigation) => {
    await showCanvas();
    fireEvent.click(screen.getByRole("button", { name: "Expand canvas" }));
    expect(document.querySelector(".app-shell")).toHaveClass("canvas-expanded");
    if (navigation === "rewind") await worker({ type: "snapshot", taskId: "a", snapshot: snapshot([]) });
    else if (navigation === "chat") chat("b");
    else if (navigation === "area") fireEvent.click(screen.getByRole("button", { name: "Chat" }));
    else fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Expanded canvas" })).toBeNull());
    expect(document.querySelector(".app-shell")).not.toHaveClass("canvas-expanded");
    expect(api.stopTask).not.toHaveBeenCalled();
  });
});

describe("App Settings entry points", () => {
  it("opens Providers from the sidebar button without treating the click event as a section", async () => {
    bootstrap(false);
    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    expect(screen.getByLabelText("Initial Settings section")).toHaveTextContent("providers");
  });

  it.each([false, true])("opens the Team requirement's Settings section (sub-agents enabled=%s)", async (enabled) => {
    bootstrap(false, false, { subagents: { enabled, trigger: "on_request", maxConcurrency: 4, agents: [] } });
    fireEvent.click(await screen.findByRole("button", { name: "first" }));
    fireEvent.click(screen.getByRole("button", { name: "Open Settings" }));
    expect(screen.getByLabelText("Initial Settings section")).toHaveTextContent(enabled ? "subagents" : "packages");
  });
});

describe("App Team drafts", () => {
  const crew = { providerId: "p", modelId: "second", thinkingLevel: "off" as const };
  const subagents: AppData["subagents"] = {
    enabled: true, trigger: "on_request", maxConcurrency: 4, teamDefault: crew,
    agents: [{ id: "builtin:worker", builtin: true, enabled: true, name: "worker", description: "Edits", prompt: "Edit", tools: ["write"], readOnly: false, model: null }],
  };

  it.each(["Implement this", "/goal Implement this", "/canvas Explore this"])("preserves the selected crew when a draft starts with %s", async (message) => {
    vi.mocked(api.createTask).mockImplementation(async (input) => ({ ...task("crew"), team: input.team ?? null }));
    bootstrap(false, false, { subagents });
    fireEvent.click(await screen.findByRole("button", { name: "first" }));
    fireEvent.click(screen.getByRole("switch", { name: "Team" }));
    expect(screen.getByRole("switch", { name: "Team" })).toHaveAttribute("aria-checked", "true");
    fireEvent.keyDown(document, { key: "Escape" });
    type(message);
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(api.createTask).toHaveBeenCalledWith(expect.objectContaining({ team: crew })));
    if (message.startsWith("/goal")) await waitFor(() => expect(api.goalControl).toHaveBeenCalledWith("crew", "set", "Implement this", expect.any(Number)));
    else if (message.startsWith("/canvas")) await waitFor(() => expect(api.startCanvas).toHaveBeenCalled());
    else await waitFor(() => expect(api.prompt).toHaveBeenCalled());
  });
});

describe("App computer app approvals", () => {
  const access: ComputerAccessRequest = { taskId: "a", requestId: "access-a", launch: false, canAlwaysAllow: true,
    app: { name: "TextEdit", bundleId: "com.apple.TextEdit", path: "/System/Applications/TextEdit.app" } };
  const grant: ComputerAppGrant = { name: "TextEdit", bundleId: "com.apple.TextEdit", path: "/System/Applications/TextEdit.app" };
  const saved: ComputerUseConfig = { enabled: true, showAgentCursor: true, neverAllow: [], alwaysAllow: [grant] };

  it("keeps an access card retryable when saving fails and updates Settings only after success", async () => {
    vi.mocked(api.computerUseRespondAccess).mockRejectedValueOnce(new Error("App approval couldn't be saved.")).mockResolvedValueOnce(saved);
    bootstrap(false);
    await screen.findByRole("button", { name: "Settings" });
    chat("a");
    await worker({ type: "computer_access_request", ...access });
    fireEvent.click(screen.getByRole("button", { name: "Always allow" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("App approval couldn't be saved.");
    expect(screen.getByRole("button", { name: "Always allow" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Always allow" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Always allow" })).toBeNull());
    expect(api.computerUseRespondAccess).toHaveBeenNthCalledWith(2, "a", "access-a", "always");
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByLabelText("Saved computer app approvals")).toHaveTextContent("TextEdit");

    vi.mocked(api.computerUseRevokeApp).mockResolvedValueOnce({ ...saved, alwaysAllow: [] });
    fireEvent.click(screen.getByRole("button", { name: "Revoke first app" }));
    await waitFor(() => expect(screen.getByLabelText("Saved computer app approvals")).toBeEmptyDOMElement());
    expect(api.computerUseRevokeApp).toHaveBeenCalledExactlyOnceWith(grant);
  });

  it("retires an expired access card so it cannot hide the next request", async () => {
    bootstrap(false);
    await screen.findByRole("button", { name: "Settings" });
    chat("a");
    await worker({ type: "computer_access_request", ...access });
    await worker({ type: "computer_access_request", ...access, requestId: "next-card", app: { name: "My App", path: "/tmp/My App.app" } });
    expect(screen.getByRole("region", { name: "Allow WackCode to use TextEdit?" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Allow WackCode to use My App?" })).toBeNull();
    await worker({ type: "computer_access_resolved", taskId: "a", requestId: access.requestId, decision: "cancelled" });
    expect(screen.queryByRole("region", { name: "Allow WackCode to use TextEdit?" })).toBeNull();
    expect(screen.getByRole("region", { name: "Allow WackCode to use My App?" })).toBeInTheDocument();
  });
});

describe("App plan review", () => {
  const plan = "# The plan\n\n- Ship it";
  async function openPlan() {
    bootstrap(false);
    await screen.findByRole("button", { name: "Settings" });
    chat("a");
    await worker({ type: "snapshot", taskId: "a", snapshot: {
      ...snapshot([
        { id: "call", role: "assistant", blocks: [{ type: "tool-call", toolName: "plan_mode_complete", toolCallId: "plan", arguments: { plan } }] },
        { id: "result", role: "tool", blocks: [{ type: "tool-result", toolName: "plan_mode_complete", toolCallId: "plan", details: { version: 1, source: "plan_mode_complete", plan } }] }
      ]), planState: { mode: "ultraplan", phase: "ready", plan }
    } });
    vi.mocked(api.setTaskMode).mockImplementation(async (_id, mode) => ({ ...task("a"), mode }));
    expect(screen.getByRole("button", { name: "Approve & implement" })).toBeInTheDocument();
  }

  it("keeps the whole action row while switching from Ultra Plan to Build and back", async () => {
    await openPlan();
    for (const [name, mode] of [["Build", "build"], ["Plan", "plan"], ["Plan", "ultraplan"]] as const) {
      const pending = deferred<TaskRecord>();
      vi.mocked(api.setTaskMode).mockReturnValueOnce(pending.promise);
      fireEvent.click(screen.getByRole("radio", { name }));
      // The optimistic mode change must preserve the proposal too.
      for (const label of ["Approve & implement", "Save PLAN.md", "Copy", "Discard"]) {
        expect(screen.getByRole("button", { name: label })).toBeEnabled();
      }
      await act(async () => pending.resolve({ ...task("a"), mode }));
      await worker({ type: "plan_state", taskId: "a", mode, phase: "ready", plan });
      expect(screen.getByRole("button", { name: "Approve & implement" })).toBeEnabled();
    }
  });

  it("explicitly discards the proposal even when already in Build", async () => {
    await openPlan();
    fireEvent.click(screen.getByRole("radio", { name: "Build" }));
    await waitFor(() => expect(api.setTaskMode).toHaveBeenCalledWith("a", "build"));
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    const dialog = screen.getByRole("alertdialog", { name: "Discard this plan?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(api.setTaskMode).toHaveBeenLastCalledWith("a", "build", { discardPlan: true }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Approve & implement" })).not.toBeInTheDocument());
    fireEvent.click(screen.getByRole("radio", { name: "Plan" }));
    expect(screen.queryByRole("button", { name: "Approve & implement" })).not.toBeInTheDocument();
  });

  it("keeps approval available if starting implementation fails", async () => {
    await openPlan();
    vi.mocked(api.prompt).mockRejectedValueOnce(new Error("Could not start the worker."));
    fireEvent.click(screen.getByRole("button", { name: "Approve & implement" }));
    await waitFor(() => expect(api.prompt).toHaveBeenCalledWith(expect.objectContaining({ taskId: "a", mode: "build", message: "Implement the plan." })));
    await screen.findByText(/Could not start the worker\./);
    expect(screen.getByRole("button", { name: "Approve & implement" })).toBeEnabled();
  });
});

describe("App tab workspace", () => {
  it("keeps tabs optional and restores the dormant workspace on toggling", async () => {
    bootstrap(false);
    await screen.findByRole("button", { name: "Settings" });
    expect(screen.queryByRole("tablist")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.click(screen.getByText("Toggle tabs"));
    fireEvent.click(screen.getByText("Close settings"));
    // One tab still looks like the tabs-off layout; the strip arrives with the second chat.
    expect(screen.queryByRole("tablist")).toBeNull();
    chat("a");
    await screen.findByRole("tablist");
    chat("b");
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.click(screen.getByText("Toggle tabs")); fireEvent.click(screen.getByText("Close settings"));
    expect(screen.queryByRole("tablist")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.click(screen.getByText("Toggle tabs")); fireEvent.click(screen.getByText("Close settings"));
    await waitFor(() => expect(screen.getAllByRole("tab")).toHaveLength(3));
    expect(screen.getByRole("tab", { name: "Chat b" })).toHaveAttribute("aria-selected", "true");
  });

  it("preserves independent draft text, attachments, model choice and recovery", async () => {
    bootstrap(); await screen.findByRole("button", { name: "Settings" });
    await native("new");
    const first = screen.getAllByRole("tab")[0].id;
    fireEvent.click(document.getElementById(first)!);
    type("First draft");
    fireEvent.change(screen.getByTestId("attach-input"), { target: { files: [new File(["notes"], "notes.txt", { type: "text/plain" })] } });
    await screen.findByText("notes.txt");
    newTab(); type("Second draft");
    fireEvent.click(screen.getByRole("button", { name: "first" }));
    fireEvent.click(within(screen.getByRole("dialog", { name: "Choose model" })).getByRole("button", { name: "second" }));
    fireEvent.click(screen.getByRole("radio", { name: "Plan" }));
    expect(screen.queryByText("notes.txt")).toBeNull();
    fireEvent.click(document.getElementById(first)!);
    expect(input()).toHaveValue("First draft");
    expect(screen.getByText("notes.txt")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "first" })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Build" })).toHaveAttribute("aria-checked", "true");
    await native("close"); await native("reopen");
    expect(input()).toHaveValue("First draft");
    expect(screen.getByText("notes.txt")).toBeInTheDocument();
    newTab();
    expect(screen.getByRole("button", { name: "second" })).toBeInTheDocument();
  });

  it("retains each transcript's disclosures while navigating with the workspace disabled", async () => {
    const transcript = (id: string): NormalizedMessage[] => [
      { id: `u-${id}`, entryId: `u-${id}`, role: "user", blocks: [{ type: "text", text: `Question ${id}` }] },
      { id: `p-${id}`, role: "assistant", blocks: [{ type: "text", text: `Checking ${id}` }] },
      { id: `a-${id}`, entryId: `a-${id}`, role: "assistant", stopReason: "stop", blocks: [{ type: "text", text: `Answer ${id}` }], turn: { userEntryId: `u-${id}`, endEntryId: `a-${id}` } }
    ];
    bootstrap(); await screen.findByRole("button", { name: "Settings" });
    await worker({ type: "snapshot", taskId: "a", snapshot: snapshot(transcript("a")) });
    await worker({ type: "snapshot", taskId: "b", snapshot: snapshot(transcript("b")) });
    chat("a"); await screen.findByRole("tablist");
    fireEvent.click(screen.getByRole("button", { name: /Show work transcript/ }));
    chat("b");
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.click(screen.getByText("Toggle tabs")); fireEvent.click(screen.getByText("Close settings"));
    chat("a"); expect(screen.getByRole("button", { name: /Hide work transcript/ })).toHaveAttribute("aria-expanded", "true");
    chat("b"); expect(screen.getByRole("button", { name: /Show work transcript/ })).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.click(screen.getByText("Toggle tabs")); fireEvent.click(screen.getByText("Close settings"));
    chat("a"); expect(screen.getByRole("button", { name: /Hide work transcript/ })).toHaveAttribute("aria-expanded", "true");
  });

  it("takes a terminal selection into the draft as a marked chip", async () => {
    bootstrap(); await screen.findByRole("button", { name: "Settings" });
    chat("a"); await screen.findByRole("tablist");
    fireEvent.click(screen.getByRole("button", { name: "Terminal" }));
    await screen.findByLabelText("Terminal a");

    act(() => terminalAttach.current!("ls --long\ntotal 12"));
    const chip = await screen.findByText("Terminal output.txt");
    expect(chip.closest(".attachment-file")).toHaveClass("terminal");
    expect(screen.getByText("2 lines")).toBeInTheDocument();
  });

  it("deduplicates chats, remembers panels, and leaves workers running on close", async () => {
    bootstrap(); await screen.findByRole("button", { name: "Settings" });
    chat("a"); await screen.findByRole("tablist");
    fireEvent.click(screen.getByRole("button", { name: "Terminal" }));
    await screen.findByLabelText("Terminal a");
    chat("b"); expect(screen.queryByLabelText("Terminal a")).toBeNull();
    chat("a"); await screen.findByLabelText("Terminal a");
    expect(screen.getAllByRole("tab")).toHaveLength(3);
    await native("close");
    expect(api.stopTask).not.toHaveBeenCalled();
    expect(api.deleteTask).not.toHaveBeenCalled();
    expect(screen.getByRole("tab", { name: "Chat b" })).toHaveAttribute("aria-selected", "true");
  });

  it("finishes an originating send after switching without stealing selection", async () => {
    const creation = deferred<TaskRecord>(); const dispatch = deferred<string>();
    vi.mocked(api.createTask).mockReturnValue(creation.promise); vi.mocked(api.prompt).mockReturnValue(dispatch.promise);
    bootstrap(); await screen.findByRole("button", { name: "Settings" });
    chat("a"); await screen.findByRole("tablist");
    const original = screen.getByRole("tab", { name: "New chat" }).id;
    fireEvent.click(document.getElementById(original)!);
    type("First send"); fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    newTab(); type("Keep my draft");
    await act(async () => creation.resolve({ ...task("created"), name: "First send" }));
    await waitFor(() => expect(api.prompt).toHaveBeenCalled());
    await act(async () => dispatch.resolve("run"));
    expect(input()).toHaveValue("Keep my draft");
    expect(document.getElementById(original)).toHaveAttribute("aria-selected", "false");
    fireEvent.click(document.getElementById(original)!);
    await waitFor(() => expect(api.openTask).toHaveBeenCalledWith("created"));
    expect(screen.getByRole("tab", { name: /First send/ })).toHaveAttribute("aria-selected", "true");
  });

  it("does not reopen a closed sending tab and recovers its failed input", async () => {
    const dispatch = deferred<string>();
    vi.mocked(api.createTask).mockResolvedValue({ ...task("created"), name: "Failed send" }); vi.mocked(api.prompt).mockReturnValue(dispatch.promise);
    bootstrap(); await screen.findByRole("button", { name: "Settings" }); type("Recover me");
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(api.prompt).toHaveBeenCalled());
    await native("close"); type("New draft");
    await act(async () => dispatch.reject(new Error("Fixture failure")));
    expect(input()).toHaveValue("New draft");
    expect(screen.queryByRole("tablist")).toBeNull();
    await native("reopen");
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    await waitFor(() => expect(input()).toHaveValue("Recover me"));
    expect(screen.getByText(/Fixture failure/)).toBeInTheDocument();
  });

  it("keeps saved-chat input recoverable after its tab leaves the ten-record shelf", async () => {
    const dispatch = deferred<string>();
    vi.mocked(api.createTask).mockResolvedValue({ ...task("created"), name: "Saved failure" }); vi.mocked(api.prompt).mockReturnValue(dispatch.promise);
    bootstrap(); await screen.findByRole("button", { name: "Settings" }); type("Recover after eviction");
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(api.prompt).toHaveBeenCalled());
    await native("close");
    for (let index = 0; index < 10; index++) await native("close");
    await act(async () => dispatch.reject(new Error("Fixture failure")));
    fireEvent.click(screen.getByRole("button", { name: "Saved failure" }));
    await waitFor(() => expect(input()).toHaveValue("Recover after eviction"));
    type("Editable retry"); expect(input()).toHaveValue("Editable retry");
  });

  it("binds a closed draft before dispatch so a sidebar opening recovers its identity", async () => {
    const creation = deferred<TaskRecord>(); const dispatch = deferred<string>();
    vi.mocked(api.createTask).mockReturnValue(creation.promise); vi.mocked(api.prompt).mockReturnValue(dispatch.promise);
    bootstrap(); await screen.findByRole("button", { name: "Settings" });
    await native("new");
    const origin = screen.getAllByRole("tab")[0].id;
    fireEvent.click(document.getElementById(origin)!);
    type("Sidebar race"); fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await native("close"); type("Other draft");
    await act(async () => creation.resolve({ ...task("created"), name: "Sidebar race" }));
    await waitFor(() => expect(api.prompt).toHaveBeenCalled());
    fireEvent.click(screen.getByRole("button", { name: "Sidebar race" }));
    expect(document.getElementById(origin)).toHaveAttribute("aria-selected", "true");
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    await act(async () => dispatch.reject(new Error("Fixture failure")));
    await waitFor(() => expect(input()).toHaveValue("Sidebar race"));
    await native("select-1"); expect(input()).toHaveValue("Other draft");
  });

  it.each([true, false])("keeps a pending send owned by its composer through preference changes (initial tabs: %s)", async (enabled) => {
    const creation = deferred<TaskRecord>(); vi.mocked(api.createTask).mockReturnValue(creation.promise);
    bootstrap(enabled); await screen.findByRole("button", { name: "Settings" });
    type("Toggle during send"); fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.click(screen.getByText("Toggle tabs")); fireEvent.click(screen.getByText("Close settings"));
    await act(async () => creation.resolve({ ...task("created"), name: "Toggle during send" }));
    await waitFor(() => expect(api.prompt).toHaveBeenCalledOnce());
    expect(screen.getByRole("heading", { name: "Toggle during send" })).toBeInTheDocument();
    if (enabled) {
      fireEvent.click(screen.getByRole("button", { name: "Settings" }));
      fireEvent.click(screen.getByText("Toggle tabs")); fireEvent.click(screen.getByText("Close settings"));
    }
    // The single bound tab keeps the strip hidden; a second chat reveals it.
    expect(screen.queryByRole("tablist")).toBeNull();
    chat("a"); await screen.findByRole("tablist");
    expect(screen.getByRole("tab", { name: /Toggle during send/ })).toBeInTheDocument();
  });

  it("dispatches a prepared slash command in its closed origin without taking another draft", async () => {
    const creation = deferred<TaskRecord>(); vi.mocked(api.createTask).mockReturnValue(creation.promise);
    bootstrap(); await screen.findByRole("button", { name: "Settings" }); type("/goal Fixture goal");
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(api.createTask).toHaveBeenCalled());
    await native("close"); type("Independent draft");
    await act(async () => creation.resolve({ ...task("goal"), name: "Goal chat" }));
    await waitFor(() => expect(api.goalControl).toHaveBeenCalledWith("goal", "set", "Fixture goal", expect.any(Number)));
    expect(input()).toHaveValue("Independent draft");
    expect(screen.queryByRole("tablist")).toBeNull();
    await native("reopen");
    expect(screen.getAllByRole("tab")).toHaveLength(2); expect(input()).toHaveValue("");
  });

  it("closes archived tabs, restores them read-only and removes deleted recovery entries", async () => {
    vi.mocked(api.archiveTask).mockResolvedValue({ ...task("a"), archived: true });
    bootstrap(); await screen.findByRole("button", { name: "Settings" });
    chat("a"); await screen.findByRole("tablist");
    fireEvent.click(screen.getByRole("button", { name: "Terminal" }));
    fireEvent.click(screen.getByRole("button", { name: "Chat menu" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Archive" }));
    await waitFor(() => expect(screen.queryByRole("tab", { name: "Chat a" })).toBeNull());
    await native("reopen");
    expect(screen.queryByLabelText("Terminal a")).toBeNull();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    // Deleting happens only from the Archived view; the chat menu no longer offers it.
    fireEvent.click(screen.getByRole("button", { name: "Chat menu" }));
    expect(screen.queryByRole("menuitem", { name: "Delete" })).toBeNull();
    fireEvent.keyDown(document.body, { key: "Escape" });
    fireEvent.click(screen.getByRole("button", { name: "Show archived" }));
    fireEvent.contextMenu(await within(await screen.findByRole("navigation", { name: "Archived chats" })).findByText("Chat a"));
    fireEvent.click(screen.getByRole("menuitem", { name: "Delete" }));
    fireEvent.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(screen.queryByRole("tab", { name: "Chat a" })).toBeNull());
    await native("reopen"); expect(screen.queryByRole("tab", { name: "Chat a" })).toBeNull();
  });

  it("shows completion while away and clears it on selection; native actions pause in Settings", async () => {
    bootstrap(); await screen.findByRole("button", { name: "Settings" });
    chat("a"); await screen.findByRole("tablist"); chat("b");
    await act(async () => events.get("worker-event")?.({ payload: { type: "run_finished", taskId: "a", runId: "run", outcome: "completed" } }));
    fireEvent.click(screen.getByRole("tab", { name: /Chat a — Finished/ }));
    expect(screen.getByRole("tab", { name: "Chat a" })).toHaveAttribute("aria-selected", "true");
    await native("select-9"); expect(screen.getByRole("tab", { name: "Chat b" })).toHaveAttribute("aria-selected", "true");
    fireEvent.click(screen.getByRole("button", { name: "Settings" })); await native("close");
    fireEvent.click(screen.getByText("Close settings"));
    expect(within(screen.getByRole("tablist")).getAllByRole("tab")).toHaveLength(3);
  });

  it("uses the current controller selection for rapid native shortcut sequences", async () => {
    bootstrap(); await screen.findByRole("button", { name: "Settings" });
    chat("a"); await screen.findByRole("tablist"); chat("b");
    await act(async () => {
      for (const command of ["select-1", "close", "select-2", "close"]) events.get("native-tab-action")?.({ payload: command });
    });
    // Down to one tab the strip hides; Chat a is the chat that survived.
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.getByRole("heading", { name: "Chat a" })).toBeInTheDocument();
  });

  it("retires the outgoing native browser before restoring another and scopes late state replies", async () => {
    const pending = deferred<BrowserState>();
    vi.mocked(api.browserState).mockImplementation((id) => id === "a" ? pending.promise : Promise.resolve(browser(id)));
    vi.mocked(api.browserPresent).mockImplementation(async ({ taskId }) => browser(taskId));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      return this.classList.contains("browser-surface") ? new DOMRect(1000, 100, 430, 700) : new DOMRect();
    });
    bootstrap(); await screen.findByRole("button", { name: "Settings" }); chat("a");
    await screen.findByRole("tablist");
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "requestAnimationFrame", "cancelAnimationFrame", "performance"] });
    const settle = async () => {
      await act(async () => { vi.advanceTimersByTime(500); });
      await act(async () => { vi.advanceTimersByTime(500); });
    };
    fireEvent.click(screen.getByRole("button", { name: "Browser" }));
    await settle();
    expect(api.browserPresent).toHaveBeenCalledWith(expect.objectContaining({ taskId: "a", visible: true }));
    const atSwitch = vi.mocked(api.browserPresent).mock.calls.length;
    chat("b"); fireEvent.click(screen.getByRole("button", { name: "Browser" }));
    await settle();
    expect(api.browserPresent).toHaveBeenLastCalledWith(expect.objectContaining({ taskId: "b", visible: true }));
    expect(vi.mocked(api.browserPresent).mock.calls[atSwitch][0]).toMatchObject({ taskId: "a", visible: false });
    await act(async () => pending.resolve(browser("a")));
    expect(screen.getByRole("textbox", { name: "Browser address" })).toHaveValue("http://localhost/b");
    chat("a");
    await settle();
    expect(api.browserPresent).toHaveBeenLastCalledWith(expect.objectContaining({ taskId: "a", visible: true }));
    expect(api.stopTask).not.toHaveBeenCalled();
  });

  it("hands off the single sub-agent watch and closes a target removed while away", async () => {
    const transcript = (id: string): NormalizedMessage[] => [
      { id: `u-${id}`, role: "user", blocks: [{ type: "text", text: "Delegate" }] },
      { id: `call-${id}`, role: "assistant", blocks: [{ type: "tool-call", toolName: "subagent", toolCallId: `job-${id}`, arguments: { agent: "scout", task: "Inspect" } }] },
      { id: `result-${id}`, role: "tool", blocks: [{ type: "tool-result", toolName: "subagent", toolCallId: `job-${id}`, details: {
        v: 1, mode: "single", results: [{ agent: "scout", task: "Inspect", readOnly: true, status: "done", activity: [], output: "Done",
          usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 1 } }]
      } }] }
    ];
    bootstrap(); await screen.findByRole("button", { name: "Settings" });
    await worker({ type: "snapshot", taskId: "a", snapshot: snapshot(transcript("a")) });
    await worker({ type: "snapshot", taskId: "b", snapshot: snapshot(transcript("b")) });
    chat("a"); await screen.findByRole("tablist");
    fireEvent.click(screen.getByRole("button", { name: /^SubAgent Scout/ }));
    await waitFor(() => expect(api.watchSubagent).toHaveBeenLastCalledWith("a", { toolCallId: "job-a", index: 0 }));
    chat("b"); fireEvent.click(screen.getByRole("button", { name: /^SubAgent Scout/ }));
    await waitFor(() => expect(api.watchSubagent).toHaveBeenLastCalledWith("b", { toolCallId: "job-b", index: 0 }));
    expect(api.watchSubagent).toHaveBeenCalledWith("a", null);
    chat("a");
    await waitFor(() => expect(api.watchSubagent).toHaveBeenLastCalledWith("a", { toolCallId: "job-a", index: 0 }));
    expect(api.watchSubagent).toHaveBeenCalledWith("b", null);
    chat("b");
    await worker({ type: "snapshot", taskId: "a", snapshot: snapshot([]) });
    const watches = vi.mocked(api.watchSubagent).mock.calls.filter(([id, target]) => id === "a" && target).length;
    chat("a");
    await waitFor(() => expect(screen.queryByRole("complementary", { name: "SubAgent Scout" })).toBeNull());
    expect(vi.mocked(api.watchSubagent).mock.calls.filter(([id, target]) => id === "a" && target)).toHaveLength(watches);
  });
});

describe("App areas", () => {
  const chatTask = (id: string, name: string): TaskRecord => ({ ...task(id), kind: "chat", name });
  const tool = (name: string) => ({ name, description: "", source: { kind: "builtin" as const }, available: true });
  function boot(tasks: TaskRecord[], enabled = false) {
    const data: AppData = { version: 1, providers: [provider], favoriteModels: [], projects: [], tasks, diffComments: {},
      toolConfig: { disabled: [] }, toolCatalog: [], packages: [], subagents: { enabled: false, trigger: "on_request", maxConcurrency: 4, agents: [] },
      autoTitle: { enabled: false, providerId: null, modelId: null }, appearance: { ...DEFAULT_APPEARANCE, chatTabs: enabled }, prompts: {}, mcp: { servers: [] } };
    vi.mocked(api.bootstrap).mockResolvedValue({ data, appDataPath: "/tmp", glassSupported: false, computerUseSupported: false, devBuild: false });
    return render(<App />);
  }
  const area = (name: "Code" | "Chat") => screen.getByRole("button", { name });
  const enter = (name: "Code" | "Chat") => fireEvent.click(area(name));

  it("keeps each area's chats apart and brings back the draft that was left behind", async () => {
    boot([task("a"), chatTask("c", "Trip plan")]);
    await screen.findByRole("button", { name: "Settings" });
    expect(area("Code")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Chat a" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Trip plan" })).toBeNull();
    type("Half a thought");

    enter("Chat");
    expect(area("Chat")).toHaveAttribute("aria-pressed", "true");
    expect(await screen.findByRole("heading", { name: /What's on your mind\?/ })).toBeInTheDocument();
    // The sidebar's page slides out before the other area's slides in.
    expect(await screen.findByRole("button", { name: "Trip plan" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Chat a" })).toBeNull();
    // No project, no planning modes, no Git.
    for (const name of ["Add project", "Git mode"]) expect(screen.queryByRole("button", { name })).toBeNull();
    expect(screen.queryByRole("radio", { name: "Plan" })).toBeNull();
    // Chat mode has its own composer, and the Code one isn't mounted.
    expect(document.querySelector(".composer-input")).toBeNull();
    expect(chatInput()).toHaveValue("");
    fireEvent.change(chatInput(), { target: { value: "Chat draft" } });

    enter("Code");
    expect(input()).toHaveValue("Half a thought");
    expect(screen.getByRole("radio", { name: "Plan" })).toBeInTheDocument();
    enter("Chat");
    expect(chatInput()).toHaveValue("Chat draft");
  });

  it("hops the duck across on a real switch, in the direction of travel", async () => {
    boot([task("a")]);
    await screen.findByRole("button", { name: "Settings" });
    // Boot never plays the cameo; it starts with the first real Code ↔ Chat change.
    expect(document.querySelector(".area-hop")).toBeNull();
    await native("area-chat");
    expect(document.querySelector(".area-hop")).toHaveAttribute("data-direction", "right");
    enter("Code");
    expect(document.querySelector(".area-hop")).toHaveAttribute("data-direction", "left");
  });

  it("returns to the chat each area had open", async () => {
    boot([task("a"), chatTask("c", "Trip plan")]);
    await screen.findByRole("button", { name: "Settings" });
    chat("a");
    await waitFor(() => expect(api.openTask).toHaveBeenCalledWith("a"));
    enter("Chat");
    fireEvent.click(await screen.findByRole("button", { name: "Trip plan" }));
    await waitFor(() => expect(api.openTask).toHaveBeenCalledWith("c"));
    expect(screen.getByRole("heading", { name: "Trip plan" })).toBeInTheDocument();
    enter("Code");
    expect(screen.getByRole("heading", { name: "Chat a" })).toBeInTheDocument();
    enter("Chat");
    expect(screen.getByRole("heading", { name: "Trip plan" })).toBeInTheDocument();
  });

  it("creates a Chat mode chat with no project, in Build, and never asks Git about it", async () => {
    vi.mocked(api.createTask).mockResolvedValue(chatTask("made", "Hello there"));
    boot([task("a")]);
    await screen.findByRole("button", { name: "Settings" });
    enter("Chat");
    fireEvent.change(chatInput(), { target: { value: "Hello there" } });
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(api.prompt).toHaveBeenCalled());
    expect(api.createTask).toHaveBeenCalledWith({ kind: "chat", name: "Hello there", providerId: "p", modelId: "first", thinkingLevel: "off" });
    expect(vi.mocked(api.prompt).mock.calls[0][0]).toMatchObject({ taskId: "made", mode: "build" });
    await waitFor(() => expect(api.openTask).toHaveBeenCalledWith("made"));
    expect(api.gitChanges).not.toHaveBeenCalledWith({ taskId: "made" });
    expect(api.setTaskMode).not.toHaveBeenCalled();
    // The chat's header offers the browser and its menu only; the scratchpad is in the menu.
    expect(await screen.findByRole("button", { name: "Browser" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Chat menu" })).toBeInTheDocument();
    for (const name of ["Changes", "Terminal", "Games"]) expect(screen.queryByRole("button", { name })).toBeNull();
    enter("Code");
    expect(screen.queryByRole("button", { name: "Hello there" })).toBeNull();
  });

  it("names a chat after its only attachment when a paste has no words", async () => {
    vi.mocked(api.createTask).mockResolvedValue(chatTask("made", "Pasted text 1.txt"));
    boot([task("a")]);
    await screen.findByRole("button", { name: "Settings" });
    enter("Chat");
    const text = "x".repeat(40 * 1024);
    fireEvent.paste(chatInput(), { clipboardData: { types: ["text/plain"], getData: () => text } });
    expect(await screen.findByRole("button", { name: "Remove file 1" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(api.prompt).toHaveBeenCalled());
    expect(api.createTask).toHaveBeenCalledWith({ kind: "chat", name: "Pasted text 1.txt", providerId: "p", modelId: "first", thinkingLevel: "off" });
    expect(vi.mocked(api.prompt).mock.calls[0][0].message).toBe(composeFileSection("", [{ name: "Pasted text 1.txt", text }]));
  });

  it("switches with ⌥⌘1 and ⌥⌘2, and leaves the Code shortcuts alone in Chat", async () => {
    boot([task("a")]);
    await screen.findByRole("button", { name: "Settings" });
    // ⌥ changes the character the key reports; the physical digit is what counts.
    fireEvent.keyDown(window, { code: "Digit2", key: "™", metaKey: true, altKey: true });
    expect(area("Chat")).toHaveAttribute("aria-pressed", "true");
    fireEvent.keyDown(window, { key: "G", metaKey: true, shiftKey: true });
    expect(area("Chat")).toHaveAttribute("aria-pressed", "true");
    fireEvent.keyDown(window, { code: "Digit1", key: "¡", metaKey: true, altKey: true });
    expect(area("Code")).toHaveAttribute("aria-pressed", "true");
    await native("area-chat");
    expect(area("Chat")).toHaveAttribute("aria-pressed", "true");
  });

  it("opens a chat picked from the tray menu in its own area", async () => {
    boot([task("a"), chatTask("c", "Trip plan")]);
    await screen.findByRole("button", { name: "Settings" });
    vi.mocked(api.takeMenuNavigation).mockResolvedValueOnce("c");
    await act(async () => events.get("native-chat-navigation")?.({ payload: { taskId: "c" } }));
    await waitFor(() => expect(area("Chat")).toHaveAttribute("aria-pressed", "true"));
    expect(screen.getByRole("heading", { name: "Trip plan" })).toBeInTheDocument();
  });

  it("gives each area its own tab bar", async () => {
    boot([task("a"), chatTask("c", "Trip plan")], true);
    await screen.findByRole("button", { name: "Settings" });
    // A lone draft tab stays below the strip's two-tab threshold.
    expect(screen.queryByRole("tablist")).toBeNull();
    chat("a");
    await screen.findByRole("tablist");
    await waitFor(() => expect(screen.getAllByRole("tab")).toHaveLength(2));
    enter("Chat");
    // A fresh draft tab; Code's two are not in this bar, which hides again.
    expect(screen.queryByRole("tablist")).toBeNull();
    fireEvent.click(await screen.findByRole("button", { name: "Trip plan" }));
    await screen.findByRole("tablist");
    await waitFor(() => expect(screen.getAllByRole("tab")).toHaveLength(2));
    expect(screen.getByRole("tab", { name: /Trip plan/ })).toHaveAttribute("aria-selected", "true");
    enter("Code");
    await waitFor(() => expect(screen.getByRole("tab", { name: /Chat a/ })).toHaveAttribute("aria-selected", "true"));
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    expect(screen.queryByRole("tab", { name: /Trip plan/ })).toBeNull();
    enter("Chat");
    await waitFor(() => expect(screen.getByRole("tab", { name: /Trip plan/ })).toHaveAttribute("aria-selected", "true"));
  });

  it("never lets a Chat mode chat's reduced tool list replace the catalogue Settings prunes against", async () => {
    boot([task("a"), chatTask("c", "Trip plan")]);
    await screen.findByRole("button", { name: "Settings" });
    await worker({ type: "snapshot", taskId: "a", snapshot: { ...snapshot([]), tools: [tool("read"), tool("bash")] } });
    await worker({ type: "snapshot", taskId: "c", snapshot: { ...snapshot([]), tools: [tool("read")] } });
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByRole("status", { name: "Tool catalogue" })).toHaveTextContent("read,bash");
  });
});


describe("Changes after Git Mode removal", () => {
  it.each([false, true])("keeps the chat and Changes panel accessible with tabs=%s and old Git preferences", async (enabled) => {
    const project = { id: "project", name: "Project", path: "/tmp/a", gitRoot: "/tmp/a", gitHasHead: true, runCommand: null, buildCommand: null, branch: "main", createdAt: "now" };
    localStorage.setItem("wackcode:gitProject", project.id);
    localStorage.setItem("wackcode:diffLayout", "split");
    localStorage.setItem("wackcode:pinnedProjects", JSON.stringify([project.id]));
    const data: AppData = { version: 1, providers: [provider], favoriteModels: [], projects: [project], tasks: [{ ...task("a"), projectId: project.id }], diffComments: {},
      toolConfig: { disabled: [] }, toolCatalog: [], packages: [], subagents: { enabled: false, trigger: "on_request", maxConcurrency: 4, agents: [] },
      autoTitle: { enabled: false, providerId: null, modelId: null }, appearance: { ...DEFAULT_APPEARANCE, chatTabs: enabled }, prompts: {}, mcp: { servers: [] } };
    vi.mocked(api.bootstrap).mockResolvedValue({ data, appDataPath: "/tmp", glassSupported: false, computerUseSupported: false, devBuild: false });
    vi.mocked(api.gitChanges).mockResolvedValueOnce({ isGit: true, root: project.path, branch: "main", files: [], changesRevision: "clean" });
    render(<App />);
    await screen.findByRole("button", { name: "Chat a" });
    chat("a");
    await waitFor(() => expect(api.gitChanges).toHaveBeenCalledWith({ taskId: "a" }));
    fireEvent.keyDown(window, { key: "c", metaKey: true, shiftKey: true });
    expect(await screen.findByRole("complementary", { name: "Changes" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "main clean" })).toBeInTheDocument();
    expect(await screen.findByText("Working tree clean")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Git mode" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Open in Git mode" })).toBeNull();
    const editor = input();
    fireEvent.change(editor, { target: { value: "Draft survives" } });
    const shortcut = new KeyboardEvent("keydown", { key: "g", metaKey: true, shiftKey: true, cancelable: true });
    act(() => { window.dispatchEvent(shortcut); });
    expect(shortcut.defaultPrevented).toBe(false);
    expect(screen.getByRole("complementary", { name: "Changes" })).toBeInTheDocument();
    expect(editor).toHaveValue("Draft survives");
    expect(screen.getByRole("button", { name: "Close changes panel" })).toBeInTheDocument();
  });
});


describe("App dynamic background wiring", () => {
  it("pulses on dispatch, not queued messages or chat navigation", async () => {
    bootstrap(true, true);
    await screen.findByRole("button", { name: "Settings" });
    chat("a");
    type("hello");
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(api.prompt).toHaveBeenCalled());
    expect(dynamic.pulse).toHaveBeenCalledOnce();
    type("queued hello");
    fireEvent.keyDown(input(), { key: "Enter" });
    await waitFor(() => expect(api.queueMessage).toHaveBeenCalled());
    expect(dynamic.pulse).toHaveBeenCalledOnce();
    chat("b");
    expect(screen.getByLabelText("Dynamic background")).toHaveTextContent("idle");
    chat("a");
    expect(dynamic.pulse).toHaveBeenCalledOnce();
  });
  it("restores the persisted appearance when a save fails", async () => {
    bootstrap(true, true);
    await screen.findByRole("button", { name: "Settings" });
    const saving = deferred<typeof DEFAULT_APPEARANCE>();
    vi.mocked(api.setAppearanceConfig).mockReturnValueOnce(saving.promise);
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.click(screen.getByText("Use Glyph Flow"));
    expect(screen.getByLabelText("Dynamic background")).toHaveTextContent("glyphFlow");
    await act(async () => saving.reject(new Error("Could not save")));
    expect(screen.getByLabelText("Dynamic background")).toHaveTextContent("colourDrift");
  });
});

describe("App suggested tasks", () => {
  const brief = "**Where:** `src/components/Composer.tsx:88`\n\nThe stop button stays disabled after a failed save.";
  const project = { id: "proj", name: "Project", path: "/tmp/proj", gitRoot: null, gitHasHead: false, runCommand: null, buildCommand: null, branch: null, createdAt: "2026-01-01" };
  const suggestedTask = (id: string, patch: Partial<TaskRecord> = {}): TaskRecord => ({ ...task(id), projectId: "proj", ...patch });
  const startedChat = (id: string, originId: string): TaskRecord => suggestedTask(id, {
    name: "Dismiss stays disabled",
    originSuggestion: { taskId: originId, suggestionId: "s1" }
  });
  function boot(tasks: TaskRecord[]) {
    const data: AppData = { version: 1, providers: [provider], favoriteModels: [], projects: [project], tasks, diffComments: {},
      toolConfig: { disabled: [] }, toolCatalog: [], packages: [], subagents: { enabled: false, trigger: "on_request", maxConcurrency: 4, agents: [] },
      autoTitle: { enabled: false, providerId: null, modelId: null }, appearance: { ...DEFAULT_APPEARANCE, chatTabs: true }, prompts: {}, mcp: { servers: [] } };
    vi.mocked(api.bootstrap).mockResolvedValue({ data, appDataPath: "/tmp", glassSupported: false, computerUseSupported: false, devBuild: false });
    return render(<App />);
  }
  /** A settled turn that filed one suggested task. */
  const turn = (): NormalizedMessage[] => [
    { id: "u", entryId: "u", role: "user", timestamp: 100, blocks: [{ type: "text", text: "Fix the layout" }] },
    { id: "a", entryId: "a", role: "assistant", timestamp: 200, stopReason: "stop", turn: { userEntryId: "u", endEntryId: "a" }, blocks: [
      { type: "tool-call", toolCallId: "s1", toolName: "suggest_task", arguments: { title: "Dismiss stays disabled", kind: "bug", description: brief } },
      { type: "tool-result", toolCallId: "s1", toolName: "suggest_task", text: "Suggested", details: { v: 1, kind: "bug", title: "Dismiss stays disabled" } },
      { type: "text", text: "The layout is fixed." }
    ] }
  ];
  async function showTray(tasks = [suggestedTask("a"), suggestedTask("b")]) {
    boot(tasks);
    await screen.findByRole("button", { name: "Settings" });
    await worker({ type: "snapshot", taskId: "a", snapshot: snapshot(turn()) });
    chat("a");
    return screen.findByRole("heading", { name: "Spotted along the way" });
  }

  it("starts a suggestion here as a literal prompt in the chosen mode", async () => {
    await showTray();
    fireEvent.click(screen.getByRole("button", { name: /Start here/ }));
    await waitFor(() => expect(api.prompt).toHaveBeenCalledWith(expect.objectContaining({ taskId: "a", mode: "build", literal: true })));
    const sent = vi.mocked(api.prompt).mock.calls.at(-1)?.[0].message ?? "";
    expect(sent.startsWith("Dismiss stays disabled\n\n**Where:**")).toBe(true);
    expect(sent).toContain("<suggested-task-v1>");
    expect(sent).toContain('"id":"s1"');
  });

  it("starts a suggestion in a new chat in the background, keeping the same project and model", async () => {
    await showTray();
    vi.mocked(api.createTask).mockResolvedValue(startedChat("new", "a"));
    fireEvent.click(screen.getByRole("button", { name: /Start in new chat/ }));
    await waitFor(() => expect(api.createTask).toHaveBeenCalledWith({
      projectId: "proj", useWorktree: false, name: "Dismiss stays disabled", providerId: "p", modelId: "first", thinkingLevel: "off",
      origin: { taskId: "a", suggestionId: "s1" }
    }));
    await waitFor(() => expect(api.prompt).toHaveBeenCalledWith(expect.objectContaining({ taskId: "new", mode: "build", literal: true })));
    const sent = vi.mocked(api.prompt).mock.calls.at(-1)?.[0].message ?? "";
    expect(sent).toContain('"origin":{"chatId":"a","chatName":"Chat a"}');
    // The user stays where they were, and the card now points at the chat it opened.
    expect(screen.getByRole("heading", { name: "Spotted along the way" })).toBeInTheDocument();
    await screen.findByRole("button", { name: "“Dismiss stays disabled”" });
    expect(screen.queryByRole("button", { name: /Start here/ })).toBeNull();
  });

  it("dismisses a suggestion for good, and restores it", async () => {
    await showTray();
    vi.mocked(api.setSuggestionDismissed).mockResolvedValue(suggestedTask("a", { dismissedSuggestions: ["s1"] }));
    fireEvent.click(screen.getByRole("button", { name: "Dismiss “Dismiss stays disabled”" }));
    await waitFor(() => expect(api.setSuggestionDismissed).toHaveBeenCalledWith({ taskId: "a", suggestionId: "s1", dismissed: true }));
    await screen.findByRole("button", { name: "Restore" });
    vi.mocked(api.setSuggestionDismissed).mockResolvedValue(suggestedTask("a"));
    fireEvent.click(screen.getByRole("button", { name: "Restore" }));
    await waitFor(() => expect(api.setSuggestionDismissed).toHaveBeenLastCalledWith({ taskId: "a", suggestionId: "s1", dismissed: false }));
    await screen.findByRole("button", { name: /Start here/ });
  });

  it("shows a suggestion's chat in this chat's transcript as a caption, not as transport", async () => {
    boot([suggestedTask("a")]);
    await screen.findByRole("button", { name: "Settings" });
    const message = `Dismiss stays disabled\n\n${brief}\n\n<suggested-task-v1>\nStarted from a suggested task "Dismiss stays disabled" in the chat "Settings cleanup".\n{"version":1,"id":"s1","kind":"bug","title":"Dismiss stays disabled","origin":{"chatId":"b","chatName":"Settings cleanup"}}\n</suggested-task-v1>`;
    await worker({ type: "snapshot", taskId: "a", snapshot: snapshot([
      { id: "u", entryId: "u", role: "user", blocks: [{ type: "text", text: message }] },
      { id: "a", entryId: "a", role: "assistant", blocks: [{ type: "text", text: "On it" }] }
    ]) });
    chat("a");
    expect(await screen.findByText("Suggested task")).toBeInTheDocument();
    expect(screen.getByText("from “Settings cleanup”")).toBeInTheDocument();
    expect(screen.queryByText(/suggested-task-v1/)).toBeNull();
  });
});
