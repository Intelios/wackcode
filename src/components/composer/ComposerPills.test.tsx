import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { NormalizedBlock, TodoTask } from "../../types";
import { ComposerPills, runDetail } from "./ComposerPills";

afterEach(cleanup);

const bashCall: NormalizedBlock = { type: "tool-call", toolName: "bash", toolCallId: "b", arguments: { command: "pnpm test providers" } };
const readCall: NormalizedBlock = { type: "tool-call", toolName: "read", toolCallId: "r", arguments: { path: "src/App.tsx" } };

describe("runDetail", () => {
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
});
