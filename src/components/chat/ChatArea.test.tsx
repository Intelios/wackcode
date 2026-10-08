import { useState } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NormalizedMessage, ProviderRecord, TaskRecord, TaskRuntime } from "../../types";
import { ChatArea, type ChatAreaProps } from "./ChatArea";

vi.mock("../../api", () => ({ api: new Proxy({}, { get: () => vi.fn().mockResolvedValue(undefined) }) }));
vi.mock("../MessageEditor", () => ({ MessageEditor: () => null }));

const provider: ProviderRecord = {
  id: "p", name: "Mock", baseUrl: "http://x", apiFormat: "openai-completions", kind: "custom",
  connected: true, hasApiKey: false, createdAt: "2026", updatedAt: "2026",
  models: [{ id: "first", name: "first", contextWindow: 8000, maxTokens: 1000, reasoning: false, vision: true, thinkingLevels: ["off"], thinkingLevelMap: { off: null } }]
};
const task = (patch: Partial<TaskRecord> = {}): TaskRecord => ({ id: "t", projectId: null, name: "My chat", workspacePath: "/tmp/t", worktreePath: null,
  branch: null, usesWorktree: false, providerId: "p", modelId: "first", thinkingLevel: "off", sessionFile: null, status: "idle", mode: "build",
  archived: false, archivedAt: null, lastError: null, kind: "chat", lastActivityAt: null, createdAt: "2026-01-01", updatedAt: "2026-01-01", autoTitleEligible: false, autoTitleAttemptId: null, ...patch });
const msg = (id: string, role: "user" | "assistant", text: string, patch: Partial<NormalizedMessage> = {}): NormalizedMessage =>
  ({ id, role, blocks: [{ type: "text", text }], timestamp: Date.now(), ...patch });

import type { ComposerDraft } from "../../hooks/useComposerDrafts";
let draftValue: ComposerDraft;
function Host(props: ChatAreaProps) {
  const [value, setValue] = useState(() => draftValue);
  const draftState: ChatAreaProps["draftState"] = {
    key: props.draftState.key,
    value,
    update: (next) => setValue((current) => {
      const resolved = typeof next === "function" ? (next as (c: typeof current) => typeof current)(current) : next;
      draftValue = resolved;
      return resolved;
    })
  };
  return <ChatArea {...props} draftState={draftState} />;
}
const base = (): ChatAreaProps => ({
  messages: [], running: false, actionsEnabled: true, vision: true, notes: [], browserOpen: false,
  agentName: "WackCode", providers: [provider], favoriteModels: [], favoriteSaving: false, onSetFavorite: vi.fn(),
  draftState: { key: "draft", get value() { return draftValue; }, update: (next) => { draftValue = typeof next === "function" ? next(draftValue) : next; } } as ChatAreaProps["draftState"],
  composerDisabled: false,
  onMessageAction: vi.fn(), onReveal: vi.fn(), onToggleBrowser: vi.fn(), onRename: vi.fn(), onTaskAction: vi.fn(),
  onConfigure: vi.fn(), onSend: vi.fn().mockResolvedValue(true), onSteer: vi.fn().mockResolvedValue(true), onDequeue: vi.fn(), onStop: vi.fn(), onOpenSettings: vi.fn()
});
const renderArea = (patch: Partial<ChatAreaProps> = {}) => {
  const props = { ...base(), ...patch };
  render(<Host {...props} />);
  return props;
};

beforeEach(() => {
  draftValue = { text: "", images: [], files: [] };
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  Element.prototype.scrollTo = vi.fn();
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("ChatArea", () => {
  it("shows the hero pond and ideas when no chat is selected, and fills the draft from an idea", () => {
    renderArea();
    expect(screen.getByRole("heading", { name: /What's on your mind\?/ })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /./, hidden: false }).length).toBeGreaterThan(0);
    const ideas = screen.getAllByRole("button").filter((b) => b.closest(".chat-idea, .chat-hero-ideas") || b.classList.contains("chat-idea"));
    expect(ideas.length).toBe(3);
    fireEvent.click(ideas[0]);
    expect(draftValue.text).not.toBe("");
    expect(screen.getByRole("textbox", { name: "Message" })).toBeInTheDocument();
  });

  it("shows bubbles, the agent name once per cluster and the docked composer when a chat is open", () => {
    renderArea({ task: task(), runtime: { snapshot: { rev: 1, sessionId: "s", messages: [], modelSwitches: [], runTimings: [], stats: {} as never, thinkingLevel: "off", availableThinkingLevels: [], tools: [], activeTools: [] } } as TaskRuntime,
      messages: [msg("u1", "user", "Hello duck"), msg("a1", "assistant", "Quack hello")] });
    expect(screen.getByText("Hello duck")).toBeInTheDocument();
    expect(screen.getByText("Quack hello")).toBeInTheDocument();
    expect(screen.getByText("WackCode")).toBeInTheDocument(); // the cluster name
    expect(screen.getByRole("button", { name: "Browser" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Chat menu" })).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Message" })).toBeInTheDocument();
  });

  it("shows the typing bubble while running", () => {
    renderArea({ task: task({ status: "running" }), running: true,
      runtime: { snapshot: {} as never } as TaskRuntime, messages: [msg("u1", "user", "Hi")] });
    expect(document.querySelector(".chat-typing")).not.toBeNull();
    expect(document.querySelectorAll(".chat-dots i")).toHaveLength(3);
  });

  it("sends the draft through onSend and restores it when sending fails", async () => {
    const props = renderArea({ task: task(), runtime: { snapshot: {} as never } as TaskRuntime });
    fireEvent.change(screen.getByRole("textbox", { name: "Message" }), { target: { value: "hello" } });
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Message" }), { key: "Enter" });
    expect(props.onSend).toHaveBeenCalledWith("hello", [], [], undefined);
    expect(draftValue.text).toBe("");
    vi.mocked(props.onSend).mockResolvedValueOnce(false);
    fireEvent.change(screen.getByRole("textbox", { name: "Message" }), { target: { value: "again" } });
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Message" }), { key: "Enter" });
    await vi.waitFor(() => expect(draftValue.text).toBe("again"));
  });

  it("swaps the composer for the archived footer", () => {
    renderArea({ task: task({ archived: true }), runtime: { snapshot: {} as never } as TaskRuntime });
    expect(screen.queryByRole("textbox", { name: "Message" })).toBeNull();
    expect(screen.getByRole("button", { name: "Unarchive" })).toBeInTheDocument();
  });

  it("shows errors as system bubbles and dismisses them", () => {
    const onDismiss = vi.fn();
    renderArea({ task: task(), runtime: { snapshot: {} as never } as TaskRuntime,
      notes: [{ key: "e", tone: "error", text: "Something broke", onDismiss }] });
    expect(screen.getByText("Something broke")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(onDismiss).toHaveBeenCalled();
  });

  it("folds three or more finished activities into a single chip", () => {
    const calls = [1, 2, 3].map((n) => ({ type: "tool-call" as const, toolName: "web_fetch", toolCallId: `t${n}`, arguments: { url: `http://${n}.example.com` } }));
    renderArea({ task: task(), runtime: { snapshot: {} as never } as TaskRuntime,
      messages: [msg("u1", "user", "Hi"), msg("a1", "assistant", "Done", { blocks: [{ type: "text", text: "Done" }, ...calls.map((c) => ({ ...c, isError: false }))] })] });
    expect(screen.getByRole("button", { name: /Did 3 things/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Did 3 things/ }));
    expect(screen.getAllByText(/Looked up/)).toHaveLength(3);
  });
});
