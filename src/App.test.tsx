import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppData, BrowserState, NormalizedMessage, ProviderRecord, SessionSnapshot, TaskRecord } from "./types";
import { DEFAULT_APPEARANCE } from "./theme";
import App from "./App";
import { api } from "./api";

const events = vi.hoisted(() => new Map<string, (event: { payload: unknown }) => void>());
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name, handler) => { events.set(name, handler); return () => events.delete(name); }) }));
vi.mock("@tauri-apps/api/core", () => ({ convertFileSrc: (value: string) => value }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ readText: vi.fn(), writeText: vi.fn() }));
vi.mock("./api", () => ({ api: {
  bootstrap: vi.fn(), refreshSubscriptionModels: vi.fn().mockResolvedValue([]),
  takeMenuNavigation: vi.fn().mockResolvedValue(null), setChatTabMenu: vi.fn().mockResolvedValue(undefined),
  computerUseCursorAppearance: vi.fn().mockResolvedValue(undefined), openTask: vi.fn().mockResolvedValue(undefined),
  gitPublishInfo: vi.fn().mockResolvedValue({ branch: "main", upstream: null, remotes: [] }),
  gitChanges: vi.fn().mockResolvedValue({ isGit: false, files: [] }), listDraftCommands: vi.fn().mockResolvedValue([]),
  listCommands: vi.fn().mockResolvedValue([]), getRun: vi.fn().mockResolvedValue({ cwd: "/tmp", run: null, generation: 0 }),
  createTask: vi.fn(), prompt: vi.fn(), setTaskMode: vi.fn(), executeCommand: vi.fn().mockResolvedValue(undefined),
  stopTask: vi.fn().mockResolvedValue(undefined), setAppearanceConfig: vi.fn(async (config) => config),
  watchSubagent: vi.fn().mockResolvedValue(undefined), archiveTask: vi.fn(), deleteTask: vi.fn().mockResolvedValue(undefined),
  goalControl: vi.fn().mockResolvedValue(undefined), browserState: vi.fn(), browserPresent: vi.fn()
} }));
vi.mock("./components/SettingsPage", () => ({ SettingsPage: (props: { appearance: typeof DEFAULT_APPEARANCE; toolCatalog: { name: string }[]; onSetAppearance: (value: typeof DEFAULT_APPEARANCE) => void; onClose: () => void }) => <>
  <output aria-label="Tool catalogue">{props.toolCatalog.map((tool) => tool.name).join(",")}</output>
  <button onClick={() => props.onSetAppearance({ ...props.appearance, chatTabs: !props.appearance.chatTabs })}>Toggle tabs</button>
  <button onClick={props.onClose}>Close settings</button>
</> }));
vi.mock("./components/TerminalPanel", () => ({ TerminalPanel: ({ taskId }: { taskId: string }) => <div aria-label={`Terminal ${taskId}`} /> }));

const provider: ProviderRecord = {
  id: "p", name: "Mock", baseUrl: "http://127.0.0.1:43127/v1", apiFormat: "openai-completions", kind: "custom",
  connected: true, hasApiKey: false, createdAt: "2026-01-01", updatedAt: "2026-01-01",
  models: ["first", "second"].map((id) => ({ id, name: id, contextWindow: 8000, maxTokens: 1000, reasoning: false, vision: true, thinkingLevels: ["off"], thinkingLevelMap: { off: null } }))
};
const task = (id: string): TaskRecord => ({ id, projectId: null, name: `Chat ${id}`, workspacePath: `/tmp/${id}`, worktreePath: null,
  branch: null, usesWorktree: false, providerId: "p", modelId: "first", thinkingLevel: "off", sessionFile: null, status: "idle", mode: "build",
  archived: false, archivedAt: null, lastError: null, kind: "code", lastActivityAt: null, createdAt: "2026-01-01", updatedAt: "2026-01-01", autoTitleEligible: false, autoTitleAttemptId: null });
function bootstrap(enabled = true) {
  const data: AppData = { version: 1, providers: [provider], favoriteModels: [], projects: [], tasks: [task("a"), task("b")], diffComments: {},
    toolConfig: { disabled: [] }, toolCatalog: [], packages: [], subagents: { enabled: false, trigger: "on_request", maxConcurrency: 4, agents: [] },
    autoTitle: { enabled: false, providerId: null, modelId: null }, appearance: { ...DEFAULT_APPEARANCE, chatTabs: enabled }, prompts: {}, mcp: { servers: [] } };
  vi.mocked(api.bootstrap).mockResolvedValue({ data, appDataPath: "/tmp", glassSupported: false, computerUseSupported: false });
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
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
});
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

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
    fireEvent.click(screen.getByRole("button", { name: "Chat menu" }));
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
    vi.mocked(api.bootstrap).mockResolvedValue({ data, appDataPath: "/tmp", glassSupported: false, computerUseSupported: false });
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
    const project = { id: "project", name: "Project", path: "/tmp/a", gitRoot: "/tmp/a", gitHasHead: true, runCommand: null, branch: "main", createdAt: "now" };
    localStorage.setItem("wackcode:gitProject", project.id);
    localStorage.setItem("wackcode:diffLayout", "split");
    localStorage.setItem("wackcode:pinnedProjects", JSON.stringify([project.id]));
    const data: AppData = { version: 1, providers: [provider], favoriteModels: [], projects: [project], tasks: [{ ...task("a"), projectId: project.id }], diffComments: {},
      toolConfig: { disabled: [] }, toolCatalog: [], packages: [], subagents: { enabled: false, trigger: "on_request", maxConcurrency: 4, agents: [] },
      autoTitle: { enabled: false, providerId: null, modelId: null }, appearance: { ...DEFAULT_APPEARANCE, chatTabs: enabled }, prompts: {}, mcp: { servers: [] } };
    vi.mocked(api.bootstrap).mockResolvedValue({ data, appDataPath: "/tmp", glassSupported: false, computerUseSupported: false });
    vi.mocked(api.gitChanges).mockResolvedValueOnce({ isGit: true, root: project.path, branch: "main", files: [], changesRevision: "clean" });
    render(<App />);
    await screen.findByRole("button", { name: "Chat a" });
    chat("a");
    await waitFor(() => expect(api.gitChanges).toHaveBeenCalledWith({ taskId: "a" }));
    fireEvent.keyDown(window, { key: "c", metaKey: true, shiftKey: true });
    expect(await screen.findByRole("heading", { name: "Changes" })).toBeInTheDocument();
    expect(await screen.findByText("Working tree clean")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Git mode" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Open in Git mode" })).toBeNull();
    const editor = input();
    fireEvent.change(editor, { target: { value: "Draft survives" } });
    const shortcut = new KeyboardEvent("keydown", { key: "g", metaKey: true, shiftKey: true, cancelable: true });
    act(() => { window.dispatchEvent(shortcut); });
    expect(shortcut.defaultPrevented).toBe(false);
    expect(screen.getByRole("heading", { name: "Changes" })).toBeInTheDocument();
    expect(editor).toHaveValue("Draft survives");
    expect(screen.getByRole("button", { name: "Close changes panel" })).toBeInTheDocument();
  });
});
