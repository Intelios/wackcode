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
  archived: false, archivedAt: null, lastError: null, kind: "chat", lastActivityAt: null, originSuggestion: null, dismissedSuggestions: [], createdAt: "2026-01-01", updatedAt: "2026-01-01", autoTitleEligible: false, autoTitleAttemptId: null, ...patch });
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
  it("shows selected attachments and tool images inline in the agent reply", () => {
    const thumbnail = "data:image/png;base64,dGh1bWI=";
    renderArea({ task: task(), messages: [
      msg("u1", "user", "Compare the screens", { entryId: "upload", blocks: [{ type: "text", text: "Compare the screens" }, { type: "image", thumbnail }] }),
      { id: "result", entryId: "result-entry", role: "tool", blocks: [{ type: "tool-result", toolCallId: "screenshot", toolName: "browser_screenshot", text: "Screenshot", images: [{ imageId: "one", thumbnail }] }] },
      msg("a1", "assistant", "Before.\n\n![Original](wackcode-image://entry/upload/0)\n\nAfter.\n\n![Updated](wackcode-image://entry/result-entry/0 \"New layout\")")
    ] });
    expect(screen.getByRole("button", { name: "Open image: Original" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open image: Updated" })).toBeInTheDocument();
    expect(screen.getByAltText("Updated")).toHaveAttribute("src", thumbnail);
    expect(screen.getByText("New layout")).toBeInTheDocument();
  });

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

  it("draws a settled visual reply as a card where the call landed", () => {
    const call: NormalizedMessage = { id: "a1", role: "assistant", timestamp: Date.now(), stopReason: "stop",
      blocks: [
        { type: "text", text: "Here is the shape of it." },
        { type: "tool-call", toolCallId: "v1", toolName: "show_visual", arguments: { title: "Latency by region", html: "<p>42</p>" } },
        { type: "tool-result", toolCallId: "v1", toolName: "show_visual", text: "Shown." }
      ] };
    renderArea({ task: task(), messages: [msg("u1", "user", "Show me"), call] });
    expect(screen.getByRole("figure", { name: "Visual: Latency by region" })).toBeInTheDocument();
    expect(screen.getByText("Here is the shape of it.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Expand visual" })).toBeInTheDocument();
    // The visual is not folded into the reply's activity chips.
    expect(document.querySelector(".chat-visual")).not.toBeNull();
  });

  it("keeps a visual that is still being drawn, or one that failed, as a chip", () => {
    const running: NormalizedMessage = { id: "a1", role: "assistant", timestamp: Date.now(),
      blocks: [{ type: "tool-call", toolCallId: "v1", toolName: "show_visual", arguments: { title: "Latency", html: "<p>42</p>" } }] };
    const { unmount } = render(<Host {...{ ...base(), task: task(), messages: [msg("u1", "user", "Show me"), running] }} />);
    expect(screen.queryByRole("figure")).toBeNull();
    unmount();
    cleanup();
    const failed: NormalizedMessage = { id: "a1", role: "assistant", timestamp: Date.now(), stopReason: "stop",
      blocks: [
        { type: "tool-call", toolCallId: "v1", toolName: "show_visual", arguments: { title: "Latency", html: "<p>42</p>" } },
        { type: "tool-result", toolCallId: "v1", toolName: "show_visual", text: "html is too large", isError: true }
      ] };
    renderArea({ task: task(), messages: [msg("u1", "user", "Show me"), failed] });
    expect(screen.queryByRole("figure")).toBeNull();
    expect(screen.getByText(/Drawing|Drew/)).toBeInTheDocument();
  });

  it("says when a message came from a visual the user clicked", () => {
    const sent = msg("u1", "user", 'Pick option B\n\n<visual-action-v1>\nSent from the visual "Latency by region".\n</visual-action-v1>');
    renderArea({ task: task(), messages: [sent, msg("a1", "assistant", "Done.")] });
    expect(screen.getByText("Pick option B")).toBeInTheDocument();
    expect(screen.getByText("Sent from “Latency by region”")).toBeInTheDocument();
    expect(screen.queryByText(/visual-action-v1/)).toBeNull();
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

  it("turns a large paste into an attachment and sends it with no words", () => {
    const props = renderArea({ task: task(), runtime: { snapshot: {} as never } as TaskRuntime });
    const area = screen.getByRole("textbox", { name: "Message" });
    const text = "x".repeat(40 * 1024);
    fireEvent.paste(area, { clipboardData: { types: ["text/plain"], getData: () => text } });
    expect(area).toHaveValue("");
    expect(screen.getByRole("button", { name: "Remove file 1" })).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Pasted text 1.txt");
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    expect(props.onSend).toHaveBeenCalledWith("", [], [{ name: "Pasted text 1.txt", text }], undefined);
    expect(draftValue.files).toEqual([]);
  });

  it("puts a pasted attachment back into the chat draft", () => {
    renderArea({ task: task(), runtime: { snapshot: {} as never } as TaskRuntime });
    const area = screen.getByRole("textbox", { name: "Message" });
    const text = "a pasted line\n".repeat(3000);
    fireEvent.paste(area, { clipboardData: { types: ["text/plain"], getData: () => text } });
    expect(screen.getByRole("button", { name: "Insert as text" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Insert as text" }));
    expect(draftValue).toEqual({ text, images: [], files: [] });
  });

  it("offers only chats in Chat mode's @ picker, and sends the reference as an id section", async () => {
    const props = renderArea({ task: task(), runtime: { snapshot: {} as never } as TaskRuntime,
      mentionChats: [{ kind: "chat", id: "other", name: "Auth refactor", label: "No project", archived: false, at: 1 }] });
    const area = screen.getByRole("textbox", { name: "Message" });
    fireEvent.change(area, { target: { value: "keep going in @auth", selectionStart: 21 } });
    const list = screen.getByRole("listbox", { name: "Chats" });
    expect(list).toHaveTextContent("Auth refactor");
    fireEvent.keyDown(area, { key: "Enter" });
    expect(area).toHaveValue('keep going in @"Auth refactor" ');
    fireEvent.keyDown(area, { key: "Enter" });
    await vi.waitFor(() => expect(props.onSend).toHaveBeenCalledTimes(1));
    const sent = (props.onSend as ReturnType<typeof vi.fn>).mock.calls[0][0] as string;
    expect(sent).toContain("<referenced-chats-v1>");
    expect(sent).toContain('{"id":"other","name":"Auth refactor"}');
  });

  it("marks a referenced chat's token in the transcript and opens it on click", () => {
    const open = vi.fn();
    renderArea({ task: task(), runtime: { snapshot: {} as never } as TaskRuntime, chatLinks: { known: new Set(["other"]), open },
      messages: [msg("u1", "user", 'continue @"Auth refactor"\n\n<referenced-chats-v1>\nChats the user referenced with @.\n{"version":1,"chats":[{"id":"other","name":"Auth refactor"}]}\n</referenced-chats-v1>')] });
    expect(screen.queryByText(/referenced-chats-v1/)).toBeNull();
    const token = screen.getByRole("button", { name: /@"Auth refactor"/ });
    fireEvent.click(token);
    expect(open).toHaveBeenCalledWith("other");
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
