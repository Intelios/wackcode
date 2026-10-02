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
  it("shows changes instead of full-file arguments when an edit has no saved diff", () => {
    const oldText = Array.from({ length: 120 }, (_, i) => `const line${i + 1} = 1;`).join("\n");
    const newText = oldText.replace("line110 = 1", "line110 = 2");
    const call = { ...editCall, arguments: { path: "src/app.ts", edits: [{ oldText, newText }] } };
    render(<ToolRow call={call} result={{ ...editResult, details: undefined }} />);
    fireEvent.click(screen.getByRole("button", { name: /Edited/ }));
    const preview = document.querySelector(".tool-diff")!;
    expect(preview.textContent).toContain("-const line110 = 1;");
    expect(preview.textContent).toContain("+const line110 = 2;");
    expect(preview.textContent).not.toContain("const line1 = 1;");
    expect(preview.textContent).not.toContain("oldText");
    expect(preview.querySelector(".addition .hljs-keyword")).not.toBeNull();
  });

  it("focuses long saved diffs while copying the complete original", async () => {
    const diff = `@@ -1,100 +1,100 @@\n${" unchanged\n".repeat(100)}-old\n+new`;
    const copy = vi.fn().mockResolvedValue(undefined);
    render(<CopyText.Provider value={copy}><ToolRow call={editCall} result={{ ...editResult, details: { diff } }} /></CopyText.Provider>);
    fireEvent.click(screen.getByRole("button", { name: /Edited/ }));
    expect(document.querySelector(".tool-diff")?.textContent?.match(/unchanged/g)).toHaveLength(3);
    fireEvent.click(screen.getByRole("button", { name: "Copy diff" }));
    expect(copy).toHaveBeenCalledWith(diff);
    await screen.findByText("Copied to clipboard.");
  });

  it("keeps failed edits as errors instead of showing unapplied changes", () => {
    render(<ToolRow call={{ ...editCall, arguments: { oldText: "old", newText: "new" } }} result={{ ...editResult, isError: true, text: "Text was not found", details: undefined }} />);
    fireEvent.click(screen.getByRole("button", { name: /Edited/ }));
    expect(screen.getByText("Text was not found")).toBeInTheDocument();
    expect(document.querySelector(".tool-diff")).toBeNull();
  });

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

  it("dims the directory and anchors the file name on path subjects", () => {
    render(<ToolRow call={{ ...editCall, toolName: "read", arguments: { path: "src-tauri/src/models.rs" } }} />);
    const subject = document.querySelector(".tool-row-subject")!;
    expect(subject).toHaveClass("has-dir");
    expect(subject.querySelector(".tool-row-subject-dir")?.textContent).toBe("src-tauri/src/");
    expect(subject.querySelector(".tool-row-subject-name")?.textContent).toBe("models.rs");
  });

  it("leaves non-path subjects whole", () => {
    render(<ToolRow call={{ ...editCall, toolName: "bash", arguments: { command: "cd src && pnpm test" } }} />);
    const subject = document.querySelector(".tool-row-subject")!;
    expect(subject).not.toHaveClass("has-dir");
    expect(subject.querySelector(".tool-row-subject-dir")).toBeNull();
    expect(subject.querySelector(".tool-row-subject-name")?.textContent).toBe("cd src && pnpm test");
  });

  it("falls back to plain diff text without a language", () => {
    const noLangCall = { ...editCall, arguments: { path: "artifact.unknownext" } };
    render(<ToolRow call={noLangCall} result={editResult} />);
    fireEvent.click(screen.getByRole("button", { name: /Edited/ }));
    expect(screen.getByText("+const x = 1;")).toBeInTheDocument();
    expect(document.querySelector(".hljs")).toBeNull();
  });
});
