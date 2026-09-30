import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NormalizedBlock } from "../types";
import { OrphanResult, ToolRow } from "./ToolRow";
import { CopyText } from "./ui/CopyButton";

afterEach(cleanup);

const editCall: NormalizedBlock = {
  type: "tool-call",
  toolCallId: "call-1",
  toolName: "edit",
  arguments: { path: "src/app.ts" },
  text: ""
};

const editResult: NormalizedBlock = {
  type: "tool-result",
  toolCallId: "call-1",
  isError: false,
  text: "Edited",
  details: { diff: "@@ -1 +1 @@\n-old text\n+const x = 1;\n" }
};

describe("ToolRow", () => {
  it.each(["bash", "read", "grep", "custom_tool"])("expands and copies the full %s result from its preview", async (toolName) => {
    const text = Array.from({ length: 65 }, (_, i) => `line ${i + 1}`).join("\n");
    const copy = vi.fn().mockResolvedValue(undefined);
    const call = { ...editCall, toolName, arguments: { command: "ls", path: "file.ts" } };
    render(<CopyText.Provider value={copy}><ToolRow call={call} result={{ ...editResult, text, details: undefined }} /></CopyText.Provider>);
    fireEvent.click(screen.getAllByRole("button")[0]);
    expect(screen.getByText("Last 60 of 65 lines")).toBeInTheDocument();
    expect(document.querySelector(".tool-text:last-child pre")?.textContent).toBe(text.split("\n").slice(-60).join("\n"));
    fireEvent.click(screen.getByRole("button", { name: "Copy output" }));
    expect(copy).toHaveBeenCalledWith(text);
    expect(await screen.findByText("Copied to clipboard.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Show all" }));
    expect(document.querySelector(".tool-text:last-child pre")?.textContent).toBe(text);
    expect(screen.getByRole("button", { name: "Show less" })).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(screen.getByRole("button", { name: "Show less" }));
    expect(screen.getByRole("button", { name: "Show all" })).toHaveAttribute("aria-expanded", "false");
  });

  it("expands written content beyond 80 lines and copies it exactly", async () => {
    const content = "written line\n".repeat(90);
    const copy = vi.fn().mockResolvedValue(undefined);
    render(<CopyText.Provider value={copy}><ToolRow call={{ ...editCall, toolName: "write", arguments: { path: "file.ts", content } }} /></CopyText.Provider>);
    fireEvent.click(screen.getByRole("button", { name: /Wrote/ }));
    fireEvent.click(screen.getByRole("button", { name: "Show all" }));
    expect(document.querySelector(".tool-text pre")?.textContent).toBe(content);
    fireEvent.click(screen.getByRole("button", { name: "Copy file content" }));
    expect(copy).toHaveBeenCalledWith(content);
    await screen.findByText("Copied to clipboard.");
  });

  it("keeps expanded live output complete as new lines arrive", async () => {
    const copy = vi.fn().mockResolvedValue(undefined);
    const call = { ...editCall, toolName: "bash", arguments: { command: "ls" } };
    const text = "streamed\n".repeat(65);
    const view = render(<CopyText.Provider value={copy}><ToolRow call={call} running liveText={text} /></CopyText.Provider>);
    fireEvent.click(screen.getByRole("button", { name: /Running/ }));
    fireEvent.click(screen.getByRole("button", { name: "Show all" }));
    view.rerender(<CopyText.Provider value={copy}><ToolRow call={call} running liveText={text + "new line"} /></CopyText.Provider>);
    expect(document.querySelector(".tool-text:last-child pre")?.textContent).toBe(text + "new line");
    fireEvent.click(screen.getByRole("button", { name: "Copy output" }));
    expect(copy).toHaveBeenCalledWith(text + "new line");
    await screen.findByText("Copied to clipboard.");
  });

  it("expands and copies unmatched results too", async () => {
    const text = "orphan line\n".repeat(65);
    const copy = vi.fn().mockResolvedValue(undefined);
    render(<CopyText.Provider value={copy}><OrphanResult block={{ type: "tool-result", text }} /></CopyText.Provider>);
    fireEvent.click(screen.getByRole("button", { name: "Tool result" }));
    fireEvent.click(screen.getByRole("button", { name: "Show all" }));
    expect(document.querySelector("pre")?.textContent).toBe(text);
    fireEvent.click(screen.getByRole("button", { name: "Copy output" }));
    expect(copy).toHaveBeenCalledWith(text);
    await screen.findByText("Copied to clipboard.");
  });

  it("copies the original diff including markers and whitespace", async () => {
    const copy = vi.fn().mockResolvedValue(undefined);
    render(<CopyText.Provider value={copy}><ToolRow call={editCall} result={editResult} /></CopyText.Provider>);
    fireEvent.click(screen.getByRole("button", { name: /Edited/ }));
    fireEvent.click(screen.getByRole("button", { name: "Copy diff" }));
    expect(copy).toHaveBeenCalledWith((editResult.details as { diff: string }).diff);
    await screen.findByText("Copied to clipboard.");
  });

  it("does not offer expansion for short output and copies commands without the prompt", async () => {
    const copy = vi.fn().mockResolvedValue(undefined);
    render(<CopyText.Provider value={copy}><ToolRow call={{ ...editCall, toolName: "bash", arguments: { command: "echo hello" } }} result={{ ...editResult, text: "hello", details: undefined }} /></CopyText.Provider>);
    fireEvent.click(screen.getByRole("button", { name: /Ran/ }));
    expect(screen.queryByRole("button", { name: "Show all" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Copy command" }));
    expect(copy).toHaveBeenCalledWith("echo hello");
    await screen.findByText("Copied to clipboard.");
  });

  it("tokenizes an edit diff with the edited file's language", () => {
    render(<ToolRow call={editCall} result={editResult} />);
    fireEvent.click(screen.getByRole("button", { name: /Edited/ }));
    expect(screen.getByText("const")).toHaveClass("hljs-keyword");
    // Markers and header lines stay plain; the code after the marker is tokenized.
    expect(document.querySelector(".tool-diff .deletion")?.textContent).toBe("-old text\n");
    expect(document.querySelector(".tool-diff .hunk")?.textContent).toBe("@@ -1 +1 @@\n");
    expect(document.querySelector(".tool-diff .deletion .hljs")).toBeNull();
  });

  it("falls back to plain diff text without a language", () => {
    const noLangCall = { ...editCall, arguments: { path: "artifact.unknownext" } };
    render(<ToolRow call={noLangCall} result={editResult} />);
    fireEvent.click(screen.getByRole("button", { name: /Edited/ }));
    expect(screen.getByText("+const x = 1;")).toBeInTheDocument();
    expect(document.querySelector(".hljs")).toBeNull();
  });
});
