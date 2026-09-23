import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { SubagentDetails, SubagentResult } from "../types";
import { SubagentCard } from "./SubagentCard";

afterEach(cleanup);

function child(overrides: Partial<SubagentResult>): SubagentResult {
  return {
    agent: "scout",
    task: "Find where sessions are restored",
    readOnly: true,
    status: "running",
    activity: [],
    usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 0 },
    ...overrides
  };
}

const details = (results: SubagentResult[]): SubagentDetails => ({ v: 1, mode: results.length > 1 ? "parallel" : "single", results });

describe("SubagentCard", () => {
  it("shows each child's live status and latest tool call while the call runs", () => {
    render(<SubagentCard running details={details([
      child({ activity: [{ tool: "grep", subject: "restore" }, { tool: "read", subject: "src/session.ts" }] }),
      child({ agent: "worker", readOnly: false, status: "queued", task: "Fix it" })
    ])} />);
    expect(screen.getByText("2 sub-agents")).toBeInTheDocument();
    expect(screen.getByText("1 running · 1 queued")).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Running" })).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Queued" })).toBeInTheDocument();
    expect(screen.getByText("src/session.ts")).toBeInTheDocument();
    expect(screen.queryByText("restore")).not.toBeInTheDocument();
    expect(screen.getByText("edits")).toBeInTheDocument();
  });

  it("expands a finished child to its answer, tool calls and usage", () => {
    render(<SubagentCard running={false} details={details([child({
      status: "done",
      model: "OpenAI · Small",
      output: "## Found\n\nIn `restore.ts`.",
      activity: [{ tool: "grep", subject: "restore" }],
      usage: { input: 1200, output: 300, cacheRead: 0, cacheWrite: 0, cost: 0.0123, turns: 3 },
      startedAt: 1_000,
      endedAt: 13_000
    })])} />);
    expect(screen.getByText("Sub-agent")).toBeInTheDocument();
    expect(screen.getByText("1 done")).toBeInTheDocument();
    expect(screen.getByText("1.5k tokens · $0.01")).toBeInTheDocument();
    const row = screen.getByRole("button", { name: /scout/ });
    expect(row).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(row);
    expect(screen.getByRole("heading", { name: "Found" })).toBeInTheDocument();
    expect(within(screen.getByRole("list", { name: "scout tool calls" })).getByText("restore")).toBeInTheDocument();
    expect(screen.getByText("OpenAI · Small")).toBeInTheDocument();
    expect(screen.getByText("3 turns")).toBeInTheDocument();
    expect(screen.getByText("12s")).toBeInTheDocument();
  });

  it("shows failures, and children left running when the call ended as stopped", () => {
    render(<SubagentCard running={false} details={details([
      child({ status: "failed", error: "Scout's model isn't connected." }),
      child({ agent: "reviewer", status: "running" })
    ])} />);
    expect(screen.getByText("Scout's model isn't connected.")).toBeInTheDocument();
    expect(screen.getByText("1 failed · 1 stopped")).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Stopped" })).toBeInTheDocument();
  });
});
