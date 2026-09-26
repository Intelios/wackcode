import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { SubagentDetails, SubagentResult } from "../types";
import { SubagentGroup, childStatus, stepLabel } from "./SubagentChip";

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

describe("SubAgent chips", () => {
  it("read a child's status, stopping one its call left running", () => {
    expect(childStatus(child({ status: "running" }), true)).toBe("running");
    expect(childStatus(child({ status: "queued" }), true)).toBe("queued");
    expect(childStatus(child({ status: "running" }), false)).toBe("stopped");
    expect(childStatus(child({ status: "aborted" }), true)).toBe("stopped");
    expect(childStatus(child({ status: "done" }), false)).toBe("done");
    expect(childStatus(child({ status: "failed" }), true)).toBe("failed");
  });

  it("put a tool call into words, naming files by their base name", () => {
    expect(stepLabel({ tool: "read", subject: "src/session/restore.ts" })).toEqual({ verb: "Reading", subject: "restore.ts" });
    expect(stepLabel({ tool: "bash", subject: "pnpm test" })).toEqual({ verb: "Running", subject: "pnpm test" });
    expect(stepLabel({ tool: "mystery", subject: "x" })).toEqual({ verb: "mystery", subject: "x" });
  });

  it("show the latest step while a child works, and flag one that can edit", () => {
    render(<SubagentGroup toolCallId="call-1" live details={details([
      child({ activity: [{ tool: "grep", subject: "restore" }, { tool: "read", subject: "src/session.ts" }] })
    ])} />);
    const chip = screen.getByRole("button", { name: /^SubAgent Scout, working/ });
    expect(within(chip).getByText("Reading")).toBeInTheDocument();
    expect(within(chip).getByText("session.ts")).toBeInTheDocument();
    expect(within(chip).queryByText("edits")).not.toBeInTheDocument();
    cleanup();

    render(<SubagentGroup toolCallId="call-1" live details={details([child({ agent: "worker", readOnly: false, status: "queued" })])} />);
    const worker = screen.getByRole("button", { name: /^SubAgent Worker, queued/ });
    expect(within(worker).getByText("edits")).toBeInTheDocument();
    expect(within(worker).getByText("Queued")).toBeInTheDocument();
  });

  it("say how a finished call's children ended", () => {
    render(<SubagentGroup toolCallId="call-1" live={false} details={details([
      child({ status: "failed", error: "Scout's model isn't connected." }),
      child({ agent: "reviewer", status: "running" })
    ])} />);
    expect(within(screen.getByRole("button", { name: /^SubAgent Scout, failed/ })).getByText("Failed")).toBeInTheDocument();
    expect(within(screen.getByRole("button", { name: /^SubAgent Reviewer, stopped/ })).getByText("Stopped")).toBeInTheDocument();
    expect(screen.getByText("1 failed · 1 stopped")).toBeInTheDocument();
    // Without a panel to open, a chip is inert.
    expect(screen.getByRole("button", { name: /^SubAgent Scout/ })).toBeDisabled();
  });
});
