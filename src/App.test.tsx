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
  bootstrap: vi.fn(), refreshSubscriptionModels: vi.fn().mockResolvedValue([]), listEditors: vi.fn().mockResolvedValue([]),
  takeMenuNavigation: vi.fn().mockResolvedValue(null), setChatTabMenu: vi.fn().mockResolvedValue(undefined),
  computerUseCursorAppearance: vi.fn().mockResolvedValue(undefined), openTask: vi.fn().mockResolvedValue(undefined),
  gitChanges: vi.fn().mockResolvedValue({ isGit: false, files: [] }), listDraftCommands: vi.fn().mockResolvedValue([]),
  listCommands: vi.fn().mockResolvedValue([]), getRun: vi.fn().mockResolvedValue({ cwd: "/tmp", run: null, generation: 0 }),
  createTask: vi.fn(), prompt: vi.fn(), setTaskMode: vi.fn(), executeCommand: vi.fn().mockResolvedValue(undefined),
  stopTask: vi.fn().mockResolvedValue(undefined), setAppearanceConfig: vi.fn(async (config) => config),
  watchSubagent: vi.fn().mockResolvedValue(undefined), archiveTask: vi.fn(), deleteTask: vi.fn().mockResolvedValue(undefined),
  goalControl: vi.fn().mockResolvedValue(undefined), browserState: vi.fn(), browserPresent: vi.fn()
} }));
vi.mock("./components/SettingsPage", () => ({ SettingsPage: (props: { appearance: typeof DEFAULT_APPEARANCE; onSetAppearance: (value: typeof DEFAULT_APPEARANCE) => void; onClose: () => void }) => <>
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
  archived: false, archivedAt: null, lastError: null, createdAt: "2026-01-01", updatedAt: "2026-01-01", autoTitleEligible: false, autoTitleAttemptId: null });
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
    await screen.findByRole("tablist");
    chat("a"); chat("b");
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.click(screen.getByText("Toggle tabs")); fireEvent.click(screen.getByText("Close settings"));
    expect(screen.queryByRole("tablist")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.click(screen.getByText("Toggle tabs")); fireEvent.click(screen.getByText("Close settings"));
    await waitFor(() => expect(screen.getAllByRole("tab")).toHaveLength(3));
    expect(screen.getByRole("tab", { name: "Chat b" })).toHaveAttribute("aria-selected", "true");
  });

  it("preserves independent draft text, attachments, model choice and recovery", async () => {
    bootstrap(); await screen.findByRole("tablist");
    const first = screen.getByRole("tab").id;
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
    bootstrap(); await screen.findByRole("tablist");
    await worker({ type: "snapshot", taskId: "a", snapshot: snapshot(transcript("a")) });
    await worker({ type: "snapshot", taskId: "b", snapshot: snapshot(transcript("b")) });
    chat("a"); fireEvent.click(screen.getByRole("button", { name: /Show work transcript/ }));
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
    bootstrap(); await screen.findByRole("tablist");
    chat("a"); fireEvent.click(screen.getByRole("button", { name: "Terminal" }));
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
    bootstrap(); await screen.findByRole("tablist");
    const original = screen.getByRole("tab").id;
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
    bootstrap(); await screen.findByRole("tablist"); type("Recover me");
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(api.prompt).toHaveBeenCalled());
    await native("close"); type("New draft");
    await act(async () => dispatch.reject(new Error("Fixture failure")));
    expect(input()).toHaveValue("New draft");
    expect(screen.getAllByRole("tab")).toHaveLength(1);
    await native("reopen");
    await waitFor(() => expect(input()).toHaveValue("Recover me"));
    expect(screen.getByText(/Fixture failure/)).toBeInTheDocument();
  });

  it("keeps saved-chat input recoverable after its tab leaves the ten-record shelf", async () => {
    const dispatch = deferred<string>();
    vi.mocked(api.createTask).mockResolvedValue({ ...task("created"), name: "Saved failure" }); vi.mocked(api.prompt).mockReturnValue(dispatch.promise);
    bootstrap(); await screen.findByRole("tablist"); type("Recover after eviction");
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
    bootstrap(); await screen.findByRole("tablist"); const origin = screen.getByRole("tab").id;
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
    expect(screen.getAllByRole("tab")).toHaveLength(1);
    expect(screen.getByRole("tab", { name: /Toggle during send/ })).toHaveAttribute("aria-selected", "true");
  });

  it("dispatches a prepared slash command in its closed origin without taking another draft", async () => {
    const creation = deferred<TaskRecord>(); vi.mocked(api.createTask).mockReturnValue(creation.promise);
    bootstrap(); await screen.findByRole("tablist"); type("/goal Fixture goal");
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(api.createTask).toHaveBeenCalled());
    await native("close"); type("Independent draft");
    await act(async () => creation.resolve({ ...task("goal"), name: "Goal chat" }));
    await waitFor(() => expect(api.goalControl).toHaveBeenCalledWith("goal", "set", "Fixture goal", expect.any(Number)));
    expect(input()).toHaveValue("Independent draft");
    expect(screen.getAllByRole("tab")).toHaveLength(1);
    await native("reopen"); expect(input()).toHaveValue("");
  });

  it("closes archived tabs, restores them read-only and removes deleted recovery entries", async () => {
    vi.mocked(api.archiveTask).mockResolvedValue({ ...task("a"), archived: true });
    bootstrap(); await screen.findByRole("tablist"); chat("a");
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
    bootstrap(); await screen.findByRole("tablist"); chat("a"); chat("b");
    await act(async () => events.get("worker-event")?.({ payload: { type: "run_finished", taskId: "a", runId: "run", outcome: "completed" } }));
    fireEvent.click(screen.getByRole("tab", { name: /Chat a — Finished/ }));
    expect(screen.getByRole("tab", { name: "Chat a" })).toHaveAttribute("aria-selected", "true");
    await native("select-9"); expect(screen.getByRole("tab", { name: "Chat b" })).toHaveAttribute("aria-selected", "true");
    fireEvent.click(screen.getByRole("button", { name: "Settings" })); await native("close");
    fireEvent.click(screen.getByText("Close settings"));
    expect(within(screen.getByRole("tablist")).getAllByRole("tab")).toHaveLength(3);
  });

  it("uses the current controller selection for rapid native shortcut sequences", async () => {
    bootstrap(); await screen.findByRole("tablist");
    chat("a"); chat("b");
    await act(async () => {
      for (const command of ["select-1", "close", "select-2", "close"]) events.get("native-tab-action")?.({ payload: command });
    });
    expect(screen.getAllByRole("tab")).toHaveLength(1);
    expect(screen.getByRole("tab", { name: "Chat a" })).toHaveAttribute("aria-selected", "true");
  });

  it("retires the outgoing native browser before restoring another and scopes late state replies", async () => {
    const pending = deferred<BrowserState>();
    vi.mocked(api.browserState).mockImplementation((id) => id === "a" ? pending.promise : Promise.resolve(browser(id)));
    vi.mocked(api.browserPresent).mockImplementation(async ({ taskId }) => browser(taskId));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      return this.classList.contains("browser-surface") ? new DOMRect(1000, 100, 430, 700) : new DOMRect();
    });
    bootstrap(); await screen.findByRole("tablist"); chat("a");
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
    bootstrap(); await screen.findByRole("tablist");
    await worker({ type: "snapshot", taskId: "a", snapshot: snapshot(transcript("a")) });
    await worker({ type: "snapshot", taskId: "b", snapshot: snapshot(transcript("b")) });
    chat("a"); fireEvent.click(screen.getByRole("button", { name: /^SubAgent Scout/ }));
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
