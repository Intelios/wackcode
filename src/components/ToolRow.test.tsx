import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NormalizedBlock } from "../types";
import { OrphanResult, ToolImageSource, ToolRow } from "./ToolRow";
import { CopyText } from "./ui/CopyButton";
import { LinkOpenerContext } from "../link-opener";

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

  it.each(["bash", "read", "grep", "custom_tool"])("expands the full %s result from its preview, without a copy button on plain output", (toolName) => {
    const text = Array.from({ length: 65 }, (_, i) => `line ${i + 1}`).join("\n");
    const copy = vi.fn().mockResolvedValue(undefined);
    const call = { ...editCall, toolName, arguments: { command: "ls", path: "file.ts" } };
    render(<CopyText.Provider value={copy}><ToolRow call={call} result={{ ...editResult, text, details: undefined }} /></CopyText.Provider>);
    fireEvent.click(screen.getAllByRole("button")[0]);
    expect(screen.getByText("Last 60 of 65 lines")).toBeInTheDocument();
    expect(document.querySelector(".tool-text:last-child pre")?.textContent).toBe(text.split("\n").slice(-60).join("\n"));
    // Output and arguments are not content the agent authored for reuse: no copy button —
    // selecting the text still reaches the clipboard through the context menu.
    expect(screen.queryByRole("button", { name: "Copy output" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Copy arguments" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show all" }));
    expect(document.querySelector(".tool-text:last-child pre")?.textContent).toBe(text);
    expect(screen.getByRole("button", { name: "Show less" })).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(screen.getByRole("button", { name: "Show less" }));
    expect(screen.getByRole("button", { name: "Show all" })).toHaveAttribute("aria-expanded", "false");
    expect(copy).not.toHaveBeenCalled();
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

  it("keeps expanded live output complete as new lines arrive", () => {
    const call = { ...editCall, toolName: "bash", arguments: { command: "ls" } };
    const text = "streamed\n".repeat(65);
    const view = render(<ToolRow call={call} running liveText={text} />);
    fireEvent.click(screen.getByRole("button", { name: /Running/ }));
    fireEvent.click(screen.getByRole("button", { name: "Show all" }));
    view.rerender(<ToolRow call={call} running liveText={text + "new line"} />);
    expect(document.querySelector(".tool-text:last-child pre")?.textContent).toBe(text + "new line");
  });

  it("expands unmatched results without offering to copy them", () => {
    const text = "orphan line\n".repeat(65);
    render(<OrphanResult block={{ type: "tool-result", text }} />);
    fireEvent.click(screen.getByRole("button", { name: "Tool result" }));
    fireEvent.click(screen.getByRole("button", { name: "Show all" }));
    expect(document.querySelector("pre")?.textContent).toBe(text);
    expect(screen.queryByRole("button", { name: "Copy output" })).toBeNull();
  });

  it("shows the quill mark for edits and writes, live only while the tool runs", () => {
    const { rerender } = render(<ToolRow call={editCall} running />);
    const quill = document.querySelector(".quill-mark")!;
    expect(quill).toHaveClass("live");
    rerender(<ToolRow call={editCall} result={editResult} />);
    expect(document.querySelector(".quill-mark")).not.toHaveClass("live");
    render(<ToolRow call={{ ...editCall, toolName: "write", arguments: { path: "file.ts", content: "line" } }} />);
    expect(document.querySelectorAll(".quill-mark")).toHaveLength(2);
  });

  it("keeps static icons for other tools", () => {
    render(<ToolRow call={{ ...editCall, toolName: "bash", arguments: { command: "ls" } }} result={{ ...editResult, details: undefined }} />);
    expect(document.querySelector(".quill-mark")).toBeNull();
    expect(document.querySelector(".tool-row-icon")).not.toBeNull();
  });

  it("rolls the line-count stats with signed accessible labels", () => {
    const { rerender } = render(<ToolRow call={editCall} result={editResult} />);
    expect(document.querySelector(".tool-row-stats .add .rolling-number")).toHaveAttribute("aria-label", "+1");
    expect(document.querySelector(".tool-row-stats .del .rolling-number")).toHaveAttribute("aria-label", "\u22121");
    const bigger = { ...editResult, details: { diff: "@@\n-a\n-b\n+c\n+d\n+e\n" } };
    rerender(<ToolRow call={editCall} result={bigger} />);
    expect(document.querySelector(".tool-row-stats .add .rolling-number")).toHaveAttribute("aria-label", "+3");
    expect(document.querySelector(".tool-row-stats .del .rolling-number")).toHaveAttribute("aria-label", "\u22122");
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

describe("ToolRow images", () => {
  const path = "/Users/me/project/captures/manual_774.png";
  const call: NormalizedBlock = { type: "tool-call", toolCallId: "call-9", toolName: "read", arguments: { path } };
  const result: NormalizedBlock = {
    type: "tool-result", toolCallId: "call-9", toolName: "read", text: "Read image file [image/png]",
    images: [{ imageId: "image-1", thumbnail: "data:image/png;base64,cHJldmlldw==" }]
  };

  it("shows the picture a read returned under its row and opens the original full size", async () => {
    const load = vi.fn(async () => "data:image/png;base64,b3JpZ2luYWw=");
    const view = render(<ToolImageSource.Provider value={load}><ToolRow call={call} result={result} /></ToolImageSource.Provider>);
    expect(screen.getByRole("img", { name: "manual_774.png" })).toHaveAttribute("src", "data:image/png;base64,cHJldmlldw==");
    // A read is a chip: its image previews sit under the strip, outside the chip itself.
    expect(view.container.querySelector(".tool-chip .tool-row-icon")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open image manual_774.png" }));
    const dialog = screen.getByRole("dialog", { name: "manual_774.png" });
    await waitFor(() => expect(dialog.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,b3JpZ2luYWw="));
    expect(load).toHaveBeenCalledWith("call-9", 0);
  });

  it("expands to the full path and Pi's notes, never the raw arguments", () => {
    const noted = { ...result, text: "Read image file [image/png]\n[Image: displayed at 2000x1500.]" };
    const view = render(<ToolRow call={call} result={noted} />);
    fireEvent.click(screen.getByRole("button", { name: /^Read/ }));
    const detail = view.container.querySelector(".tool-detail");
    expect([...detail!.querySelectorAll("pre")].map((pre) => pre.textContent)).toEqual([path, "[Image: displayed at 2000x1500.]"]);
    expect(detail).not.toHaveTextContent("Read image file");
    expect(detail).not.toHaveTextContent("{");
  });

  it("names an image from any other tool after the call, and skips one with no preview to come", () => {
    const mcpCall: NormalizedBlock = { type: "tool-call", toolCallId: "call-10", toolName: "mcp__design__export", arguments: {} };
    const mcpResult: NormalizedBlock = {
      type: "tool-result", toolCallId: "call-10", toolName: "mcp__design__export", text: "Exported",
      images: [{ imageId: "image-1", failed: true }, { imageId: "image-2", thumbnail: "data:image/png;base64,dHdv" }, { imageId: "image-3" }]
    };
    const load = vi.fn(async () => undefined);
    render(<ToolImageSource.Provider value={load}><ToolRow call={mcpCall} result={mcpResult} /></ToolImageSource.Provider>);
    expect(screen.getAllByRole("img")).toHaveLength(1);
    expect(screen.getByLabelText("Preparing image preview")).toBeInTheDocument();
    // The second image keeps its place in the result, which is what the original is fetched by.
    fireEvent.click(screen.getByRole("button", { name: "Open image from export (design)" }));
    expect(load).toHaveBeenCalledWith("call-10", 1);
  });

  it("shows no image strip when no preview could be made at all", () => {
    const view = render(<ToolRow call={call} result={{ ...result, images: [{ imageId: "image-1", failed: true }] }} />);
    expect(view.container.querySelector(".tool-images")).not.toBeInTheDocument();
  });
});

describe("ToolRow sheets", () => {
  const open = (name: RegExp) => fireEvent.click(screen.getByRole("button", { name }));

  it("numbers a read's lines from its offset and keeps the note out of the code", () => {
    const call: NormalizedBlock = { type: "tool-call", toolCallId: "r1", toolName: "read", arguments: { path: "/repo/src/app.ts", offset: 40, limit: 2 } };
    const text = "const a = 1;\nconst b = 2;\n\n[10 more lines in file. Use offset=42 to continue.]";
    render(<ToolRow call={call} result={{ type: "tool-result", toolCallId: "r1", text }} />);
    open(/^Read/);
    const code = document.querySelector<HTMLElement>(".tool-code")!;
    expect(code.textContent).toBe("const a = 1;\nconst b = 2;");
    expect(code.style.counterReset).toBe("line 39");
    expect(code.querySelector(".hljs-keyword")).not.toBeNull();
    expect(screen.getByText("lines 40–41 of 51")).toBeInTheDocument();
    expect(document.querySelector(".tool-head-path")?.textContent).toBe("/repo/src/app.ts");
  });

  it("groups a grep's hits by file and lights the pattern", () => {
    const call: NormalizedBlock = { type: "tool-call", toolCallId: "g1", toolName: "grep", arguments: { pattern: "hit" } };
    render(<ToolRow call={call} result={{ type: "tool-result", toolCallId: "g1", text: "src/a.ts:4: const hit = 1;\nsrc/b.ts:10: hit()" }} />);
    open(/^Searched/);
    expect([...document.querySelectorAll(".match-file")].map((file) => file.textContent)).toEqual(["src/a.ts\n", "src/b.ts\n"]);
    expect(document.querySelector(".match-row")).toHaveAttribute("data-line", "4");
    expect(document.querySelectorAll(".tool-matches mark")).toHaveLength(2);
    expect(screen.getByText("2 matches in 2 files")).toBeInTheDocument();
  });

  it("shows how a failed command ended as a pill, apart from its output", () => {
    const call: NormalizedBlock = { type: "tool-call", toolCallId: "b1", toolName: "bash", arguments: { command: "pnpm test" } };
    render(<ToolRow call={call} result={{ type: "tool-result", toolCallId: "b1", isError: true, text: "1 failed\n\nCommand exited with code 1" }} />);
    open(/^Ran/);
    expect(screen.getByText("exit 1")).toHaveClass("tool-pill");
    expect(document.querySelector(".tool-text pre")?.textContent).toBe("1 failed");
  });

  it("marks the option the user picked for a question", () => {
    const questions = [{ id: "q", header: "Next", question: "Which way?", options: [{ label: "Left", description: "West." }, { label: "Right", description: "East." }] }];
    const call: NormalizedBlock = { type: "tool-call", toolCallId: "q1", toolName: "ask_user_question", arguments: { questions } };
    const result: NormalizedBlock = { type: "tool-result", toolCallId: "q1", text: "The user answered your questions:", details: { cancelled: false, answers: [{ questionId: "q", selected: ["Right"] }] } };
    render(<ToolRow call={call} result={result} />);
    open(/^Asked/);
    const options = screen.getAllByRole("listitem");
    expect(options.map((option) => option.getAttribute("aria-current"))).toEqual([null, "true"]);
    expect(screen.getByText("Answered")).toBeInTheDocument();
    expect(document.querySelector(".tool-detail")).not.toHaveTextContent("{");
  });

  it("opens a web search into its result cards, and a result into the opener", () => {
    const call: NormalizedBlock = { type: "tool-call", toolCallId: "w1", toolName: "web_search", arguments: { query: "tauri 2 release" } };
    const result: NormalizedBlock = {
      type: "tool-result", toolCallId: "w1", text: "Web search via DeepSeek…",
      details: {
        v: 1, query: "tauri 2 release", provider: { name: "DeepSeek", kind: "deepseek" },
        results: [
          { title: "Tauri 2.0 is out", url: "https://v2.tauri.app/blog/", host: "v2.tauri.app", snippet: "Stable at last", date: "2026-06-01T00:00:00+00:00" },
          { title: "Bad", url: "javascript:alert(1)", host: "x" },
        ],
      },
    };
    const openLink = vi.fn();
    render(<LinkOpenerContext.Provider value={openLink}><ToolRow call={call} result={result} /></LinkOpenerContext.Provider>);
    open(/^Searched the web/);
    expect(screen.getByText("DeepSeek · 1 result")).toBeInTheDocument();
    const link = screen.getByRole("link", { name: "Tauri 2.0 is out" });
    expect(link).toHaveAttribute("href", "https://v2.tauri.app/blog/");
    expect(screen.queryByRole("link", { name: "Bad" })).not.toBeInTheDocument();
    fireEvent.click(link);
    expect(openLink).toHaveBeenCalledWith("https://v2.tauri.app/blog/", { external: false });
    fireEvent.click(link, { metaKey: true });
    expect(openLink).toHaveBeenLastCalledWith("https://v2.tauri.app/blog/", { external: true });
  });

  it("falls back to the plain sheet for a failed or unrecognised web search", () => {
    const call: NormalizedBlock = { type: "tool-call", toolCallId: "w2", toolName: "web_search", arguments: { query: "x" } };
    render(<ToolRow call={call} result={{ type: "tool-result", toolCallId: "w2", isError: true, text: "Your DeepSeek balance is empty." }} />);
    open(/Searched the web/);
    expect(screen.getByText("Your DeepSeek balance is empty.")).toBeInTheDocument();
    expect(document.querySelector(".web-results")).toBeNull();
  });

  it("says which provider read a routed page", () => {
    const call: NormalizedBlock = { type: "tool-call", toolCallId: "f2", toolName: "web_fetch", arguments: { url: "https://example.com" } };
    render(<ToolRow call={call} result={{ type: "tool-result", toolCallId: "f2", text: "page", details: { via: "DeepSeek" } }} />);
    open(/^Fetched/);
    expect(screen.getByText("via DeepSeek")).toBeInTheDocument();
  });

  it("lays any other call's arguments out as labelled rows, not JSON", () => {
    const call: NormalizedBlock = { type: "tool-call", toolCallId: "f1", toolName: "web_fetch", arguments: { url: "https://example.com", maxLength: 4000 } };
    render(<ToolRow call={call} result={{ type: "tool-result", toolCallId: "f1", text: "page text" }} />);
    open(/^Fetched/);
    expect([...document.querySelectorAll(".tool-args dt")].map((term) => term.textContent)).toEqual(["url", "maxLength"]);
    expect(document.querySelector(".tool-args")).not.toHaveTextContent("{");
    expect(screen.getByText("Result")).toBeInTheDocument();
  });
});

describe("ToolRow work rows", () => {
  it("opens an edit straight into its diff: the row's bar is the sheet's head", () => {
    render(<ToolRow call={editCall} result={editResult} />);
    fireEvent.click(screen.getByRole("button", { name: /Edited/ }));
    const row = document.querySelector(".tool-row.open")!;
    expect(row.querySelector(".tool-detail .tool-diff")).not.toBeNull();
    expect(row.querySelector(".tool-head")).toBeNull();
  });

  it("gives a failed edit no second head, only what was sent and what went wrong", () => {
    render(<ToolRow call={{ ...editCall, arguments: { path: "src/app.ts", oldText: "old", newText: "new" } }} result={{ ...editResult, isError: true, text: "Text was not found", details: undefined }} />);
    fireEvent.click(screen.getByRole("button", { name: /Edited/ }));
    expect(document.querySelector(".tool-row .tool-head")).toBeNull();
    expect([...document.querySelectorAll(".tool-args dt")].map((term) => term.textContent)).toEqual(["path", "oldText", "newText"]);
    expect(screen.getByText("Error")).toBeInTheDocument();
  });

  it("marks a command row, which keeps the command in full as its first pane and in the button's name", () => {
    render(<ToolRow call={{ ...editCall, toolName: "bash", arguments: { command: "pnpm install" } }} result={{ ...editResult, text: "done", details: undefined }} />);
    const bar = screen.getByRole("button", { name: /Ran/ });
    fireEvent.click(bar);
    expect(bar.closest(".tool-row")).toHaveClass("command", "open");
    expect(bar).toHaveAccessibleName(/pnpm install/);
    expect(document.querySelector(".tool-row .tool-command")?.textContent).toBe("pnpm install");
  });
});
