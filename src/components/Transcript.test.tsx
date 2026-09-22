import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { NormalizedMessage } from "../types";
import { Transcript } from "./Transcript";

afterEach(cleanup);

describe("Transcript tool output", () => {
  it("keeps live output collapsed, replaces updates, then shows the final result", () => {
    const call: NormalizedMessage = {
      id: "assistant-1",
      role: "assistant",
      blocks: [{ type: "tool-call", toolName: "bash", toolCallId: "call-1", arguments: { command: "slow-command" } }]
    };
    const result: NormalizedMessage = {
      id: "tool-2",
      role: "tool",
      blocks: [{ type: "tool-result", toolName: "bash", toolCallId: "call-1", text: "final output" }]
    };
    const view = render(<Transcript messages={[call]} running liveToolText={{ "call-1": "first update" }} />);
    const row = screen.getByRole("button", { name: /slow-command/ });
    expect(row).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("first update")).not.toBeInTheDocument();

    fireEvent.click(row);
    expect(screen.getByText("first update")).toBeInTheDocument();

    view.rerender(<Transcript messages={[call]} running liveToolText={{ "call-1": "first update\nsecond update" }} />);
    expect(screen.getByText("first update second update")).toBeInTheDocument();
    expect(screen.queryByText("first update", { exact: true })).not.toBeInTheDocument();

    view.rerender(<Transcript messages={[call, result]} running={false} liveToolText={{}} />);
    expect(screen.getByText("final output")).toBeInTheDocument();
    expect(screen.queryByText("second update", { exact: false })).not.toBeInTheDocument();
  });
});
