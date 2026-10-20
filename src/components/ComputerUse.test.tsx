import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ComputerAccessRequest, ExtensionUIRequest, NormalizedBlock } from "../types";
import { summarizeTool } from "../tool-utils";
import { ComputerUseBanner } from "./ComputerUseBanner";
import { InlineDialog } from "./InlineDialog";
import { ToolImageSource, ToolRow } from "./ToolRow";

afterEach(cleanup);

const access: ComputerAccessRequest = {
  taskId: "task-1",
  requestId: "access-1",
  launch: false,
  canAlwaysAllow: true,
  app: { name: "TextEdit", bundleId: "com.apple.TextEdit", path: "/System/Applications/TextEdit.app" },
};

const question: ExtensionUIRequest = {
  taskId: "task-1",
  requestId: "req-1",
  method: "confirm",
  title: "Confirm action",
  message: "Are you sure?",
};

describe("computer-use access card", () => {
  it.each([
    ["Allow for this chat", "allow"],
    ["Always allow", "always"],
    ["Deny", "deny"],
    ["Never allow", "never"],
  ])("comes before other dialogs and answers %s with %s", async (label, decision) => {
    const onAccess = vi.fn();
    const onRespond = vi.fn();
    render(<InlineDialog requests={[question]} accessRequests={[access]} selectedTaskId="task-1" agentName="Nova" onRespond={onRespond} onAccess={onAccess} />);
    expect(screen.getByRole("region", { name: "Allow Nova to use TextEdit?" })).toBeInTheDocument();
    expect(screen.queryByText("Are you sure?")).toBeNull();
    expect(screen.getByText(/all coding chats, even after restarting/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: label }));
    await waitFor(() => expect(onAccess).toHaveBeenCalledExactlyOnceWith(access, decision));
    expect(onRespond).not.toHaveBeenCalled();
  });

  it("never focuses either Allow choice, and Escape denies", async () => {
    const onAccess = vi.fn();
    render(<InlineDialog requests={[]} accessRequests={[access]} selectedTaskId="task-1" onRespond={vi.fn()} onAccess={onAccess} />);
    expect(screen.getByRole("button", { name: "Allow for this chat" })).not.toHaveFocus();
    expect(screen.getByRole("button", { name: "Always allow" })).not.toHaveFocus();
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(onAccess).toHaveBeenCalledWith(access, "deny"));
  });

  it("says when allowing also launches the app, and hides Never allow without a bundle id", () => {
    render(
      <InlineDialog
        requests={[]}
        accessRequests={[{ ...access, launch: true, app: { name: "My App", path: "/tmp/My App.app" } }]}
        selectedTaskId="task-1"
        onRespond={vi.fn()}
        onAccess={vi.fn()}
      />
    );
    expect(screen.getByText(/allowing also launches it/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Never allow" })).toBeNull();
  });

  it("hides Always allow when the host cannot remember a stable app identity", () => {
    render(<InlineDialog requests={[]} accessRequests={[{ ...access, canAlwaysAllow: false, app: { name: "Transient app" } }]}
      selectedTaskId="task-1" onRespond={vi.fn()} onAccess={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Always allow" })).toBeNull();
    expect(screen.getByRole("button", { name: "Allow for this chat" })).toBeEnabled();
  });

  it("keeps failed saves retryable and answers only once while a save is pending", async () => {
    let rejectSave!: (reason: Error) => void;
    const pending = new Promise<void>((_resolve, reject) => { rejectSave = reject; });
    const onAccess = vi.fn().mockReturnValueOnce(pending).mockResolvedValue(undefined);
    render(<InlineDialog requests={[]} accessRequests={[access]} selectedTaskId="task-1" onRespond={vi.fn()} onAccess={onAccess} />);
    const always = screen.getByRole("button", { name: "Always allow" });
    fireEvent.click(always);
    expect(always).toBeDisabled();
    expect(screen.getByRole("button", { name: "Allow for this chat" })).toBeDisabled();
    fireEvent.click(always);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onAccess).toHaveBeenCalledExactlyOnceWith(access, "always");
    rejectSave(new Error("Couldn't save app access."));
    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't save app access.");
    expect(always).toBeEnabled();
    fireEvent.click(always);
    await waitFor(() => expect(onAccess).toHaveBeenCalledTimes(2));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows nothing for another chat's card", () => {
    const { container } = render(<InlineDialog requests={[]} accessRequests={[access]} selectedTaskId="task-2" onRespond={vi.fn()} onAccess={vi.fn()} />);
    expect(container.firstChild).toBeNull();
  });
});

describe("ComputerUseBanner", () => {
  it("shows the app in use and stops the run", () => {
    const onStop = vi.fn();
    const { rerender } = render(<ComputerUseBanner computer={{ active: true, app: "TextEdit", hotkey: true }} onStop={onStop} />);
    expect(screen.getByRole("region", { name: "Computer use" })).toHaveTextContent("Using TextEdit");
    expect(screen.getByText("⌃⌥⌘.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(onStop).toHaveBeenCalled();
    rerender(<ComputerUseBanner computer={{ active: false, hotkey: false }} onStop={onStop} />);
    expect(screen.queryByRole("region", { name: "Computer use" })).toBeNull();
  });
});

describe("computer-use tool rows", () => {
  const call: NormalizedBlock = { type: "tool-call", toolCallId: "call-7", toolName: "computer_screenshot", arguments: { app: "/Users/me/Build/My App.app" } };
  const result: NormalizedBlock = {
    type: "tool-result",
    toolCallId: "call-7",
    toolName: "computer_screenshot",
    text: "My App — window 7",
    images: [{ imageId: "image-1", thumbnail: "data:image/jpeg;base64,cHJldmlldw==" }],
  };

  it("shows the screenshot preview and opens the original full size", async () => {
    const load = vi.fn(async () => "data:image/jpeg;base64,b3JpZ2luYWw=");
    render(<ToolImageSource.Provider value={load}><ToolRow call={call} result={result} /></ToolImageSource.Provider>);
    expect(screen.getByRole("button", { name: /Captured/ })).toHaveTextContent("My App");
    fireEvent.click(screen.getByRole("button", { name: "Open screenshot of My App" }));
    const dialog = screen.getByRole("dialog", { name: "Screenshot of My App" });
    await waitFor(() => expect(dialog.querySelector("img")?.getAttribute("src")).toBe("data:image/jpeg;base64,b3JpZ2luYWw="));
    expect(load).toHaveBeenCalledWith("call-7", 0);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("holds a placeholder until the preview is ready", () => {
    render(<ToolRow call={call} result={{ ...result, images: [{ imageId: "image-1" }] }} />);
    expect(screen.getByLabelText("Preparing screenshot preview")).toBeInTheDocument();
  });

  it("summarizes computer actions by app and action", () => {
    const act = (actions: unknown[]): NormalizedBlock => ({ type: "tool-call", toolName: "computer_act", arguments: { app: "TextEdit", stateId: "s1", actions } });
    expect(summarizeTool(act([{ kind: "typeText", text: "hello\nworld" }])).subject).toBe('TextEdit · typed "hello world"');
    expect(summarizeTool(act([{ kind: "menu", path: ["File", "Save As…"] }])).subject).toBe("TextEdit · chose File › Save As…");
    expect(summarizeTool(act([{ kind: "raise" }, { kind: "wait", ms: 10 }])).subject).toBe("TextEdit · 2 actions");
    expect(summarizeTool({ type: "tool-call", toolName: "computer_apps", arguments: {} }).doneVerb).toBe("Listed apps");
  });
});
