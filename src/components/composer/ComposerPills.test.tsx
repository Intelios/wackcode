import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GitChangeFile, GitChanges, NormalizedBlock, TodoTask } from "../../types";
import { ComposerPills, runDetail } from "./ComposerPills";

afterEach(cleanup);

const bashCall: NormalizedBlock = { type: "tool-call", toolName: "bash", toolCallId: "b", arguments: { command: "pnpm test providers" } };
const readCall: NormalizedBlock = { type: "tool-call", toolName: "read", toolCallId: "r", arguments: { path: "src/App.tsx" } };

/** A changed file with one diff section; pass two layers to make a partially staged file. */
function changeFile(path: string, sections: { layer: "staged" | "working"; additions: number; deletions: number }[]): GitChangeFile {
  return {
    path, oldPath: null, status: "modified",
    staged: sections.some((section) => section.layer === "staged"),
    unstaged: sections.some((section) => section.layer === "working"),
    untracked: false, binary: false, hunkable: true, truncated: false,
    sections: sections.map((section) => ({ ...section, revision: "rev", diff: "", hunks: [], truncated: false }))
  };
}

function gitChanges(files: GitChangeFile[]): GitChanges {
  return { isGit: true, root: "/repo", branch: "main", files, changesRevision: "rev" };
}

describe("runDetail", () => {
  it("names a canvas while the model is still generating tool arguments", () => {
    expect(runDetail({ partial: { id: "p", role: "assistant", blocks: [
      { type: "thinking", text: "Thinking", durationMs: 1000 },
      { type: "tool-call", toolName: "canvas", toolCallId: "c", arguments: { title: "Billing", artboards: [{ name: "Phone", html: "<" }] } }
    ] } })).toEqual({ verb: "Drawing on canvas", subject: "Billing — Phone", subjectMono: false });
  });
  it("names the latest in-flight tool, a command by itself and a file by verb and path", () => {
    expect(runDetail({ liveToolCalls: { b: bashCall } })).toEqual({ subject: "pnpm test providers", subjectMono: true });
    expect(runDetail({ liveToolCalls: { b: bashCall, r: readCall } })).toEqual({ verb: "Reading", subject: "src/App.tsx", subjectMono: true });
  });

  it("falls back through thinking, compaction and activity, and is empty while prose streams", () => {
    const thinking = { id: "p", role: "assistant" as const, blocks: [{ type: "thinking" as const, text: "…" }] };
    expect(runDetail({ partial: thinking })).toEqual({ subject: "Thinking…" });
    expect(runDetail({ compaction: { reason: "overflow", startedAt: 1 } })).toEqual({ subject: "Compacting context…" });
    expect(runDetail({ activity: "mcp_connect_start" })).toEqual({ subject: "Starting MCP servers…" });
    expect(runDetail({ activity: "auto_retry_1" })).toEqual({ subject: "Retrying…" });
    expect(runDetail({ partial: { id: "p", role: "assistant", blocks: [{ type: "text", text: "Half an answer" }] } })).toBeUndefined();
    expect(runDetail({})).toBeUndefined();
  });
});

describe("ComposerPills", () => {
  const todos: TodoTask[] = [
    { id: 1, subject: "Set up renderer", status: "completed" },
    { id: 2, subject: "Run provider tests", status: "in_progress" },
    { id: 3, subject: "Write tests", status: "pending" }
  ];

  it("renders the working pill first with the clock and the live tool's command", () => {
    const { container } = render(
      <ComposerPills run={{ startedAt: Date.now() - 30_000 }} liveToolCalls={{ b: bashCall }} todos={todos} />
    );
    const pills = container.querySelectorAll(".composer-pill");
    expect(pills).toHaveLength(2);
    expect(pills[0]).toHaveTextContent(/^Working for 30s/);
    expect(pills[0].querySelector(".composer-pill-detail .mono")).toHaveTextContent("pnpm test providers");
    expect(pills[1]).toHaveTextContent("1/3Run provider tests");
  });

  it("shows the run's state detail and omits it while nothing specific is happening", () => {
    const thinking = { id: "p", role: "assistant" as const, blocks: [{ type: "thinking" as const, text: "…" }] };
    const thinkingView = render(<ComposerPills run={{ startedAt: Date.now() - 5_000 }} partial={thinking} />);
    expect(thinkingView.container.querySelector(".composer-pill")).toHaveTextContent("Thinking…");
    thinkingView.unmount();
    const streaming = render(<ComposerPills run={{ startedAt: Date.now() - 5_000 }}
      partial={{ id: "p", role: "assistant", blocks: [{ type: "text", text: "Writing" }] }} />);
    const pill = streaming.container.querySelector(".composer-pill")!;
    expect(pill).toHaveTextContent(/^Working for 5s$/);
    expect(pill.querySelector(".composer-pill-detail")).toBeNull();
  });

  it("renders a compaction state with no timer when compaction replaces the run", () => {
    const { container } = render(<ComposerPills compaction={{ reason: "overflow", startedAt: 1_000 }} />);
    const pill = container.querySelector(".composer-pill")!;
    expect(pill).toHaveTextContent("Compacting context…");
    expect(pill).not.toHaveTextContent("Working for");
  });

  it("renders only the todo pill between runs and nothing at all when neither is active", () => {
    const { container, rerender } = render(<ComposerPills todos={todos} />);
    expect(container.querySelectorAll(".composer-pill")).toHaveLength(1);
    expect(container.querySelector(".composer-pill-todo")).not.toBeNull();
    rerender(<ComposerPills />);
    expect(container.firstChild).toBeNull();
    const tombed: TodoTask[] = [{ id: 1, subject: "Gone", status: "deleted" }];
    rerender(<ComposerPills todos={tombed} />);
    expect(container.firstChild).toBeNull();
  });

  it("shows the changed-files count and ± stats last, and clicking toggles the Changes view", () => {
    const onToggle = vi.fn();
    const { container } = render(
      <ComposerPills
        run={{ startedAt: Date.now() - 30_000 }}
        todos={todos}
        changes={gitChanges([changeFile("a.ts", [{ layer: "working", additions: 10, deletions: 2 }]), changeFile("b.ts", [{ layer: "working", additions: 3, deletions: 0 }])])}
        changesOpen={false}
        onToggleChanges={onToggle}
      />
    );
    const pills = container.querySelectorAll(".composer-pill");
    expect(pills).toHaveLength(3);
    const filesPill = pills[2];
    expect(filesPill).toHaveTextContent(/2 files \+13 −2/);
    expect(filesPill).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(filesPill);
    expect(onToggle).toHaveBeenCalledTimes(1);
  });

  it("counts every section of a partially staged file like the Changes view does", () => {
    const { container } = render(
      <ComposerPills changes={gitChanges([changeFile("a.ts", [{ layer: "staged", additions: 4, deletions: 1 }, { layer: "working", additions: 2, deletions: 3 }])])} />
    );
    expect(container.querySelector(".composer-pill-files")).toHaveTextContent(/1 file \+6 −4/);
  });

  it("reads pressed while the panel is on Changes, and hides on a clean or non-Git tree", () => {
    const { container, rerender } = render(
      <ComposerPills changes={gitChanges([changeFile("a.ts", [{ layer: "working", additions: 1, deletions: 0 }])])} changesOpen onToggleChanges={vi.fn()} />
    );
    expect(container.querySelector(".composer-pill-files")).toHaveAttribute("aria-pressed", "true");
    rerender(<ComposerPills changes={gitChanges([])} />);
    expect(container.firstChild).toBeNull();
    rerender(<ComposerPills changes={{ ...gitChanges([changeFile("a.ts", [{ layer: "working", additions: 1, deletions: 0 }])]), isGit: false }} />);
    expect(container.firstChild).toBeNull();
  });

  it("adds GlassPlatter to every pill and the todo popover only when glass is enabled", () => {
    const changes = gitChanges([changeFile("a.ts", [{ layer: "working", additions: 2, deletions: 1 }])]);
    const { container, rerender } = render(
      <ComposerPills run={{ startedAt: Date.now() - 5_000 }} todos={todos} changes={changes} glass />
    );
    expect(container.querySelectorAll(".composer-pill > .glass-platter")).toHaveLength(3);
    fireEvent.click(screen.getByRole("button", { name: /Todos: 1 of 3/ }));
    expect(document.body.querySelector(".todo-popover")).toHaveClass("glass");

    rerender(<ComposerPills run={{ startedAt: Date.now() - 5_000 }} todos={todos} changes={changes} />);
    expect(container.querySelectorAll(".glass-platter")).toHaveLength(0);
    expect(document.body.querySelector(".todo-popover")).not.toHaveClass("glass");
  });
});
