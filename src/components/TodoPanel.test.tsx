import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { TodoTask } from "../types";
import { TodoPanel } from "./TodoPanel";

afterEach(cleanup);

describe("TodoPanel", () => {
  const tasks: TodoTask[] = [
    { id: 1, subject: "Set up renderer", status: "completed" },
    { id: 2, subject: "Wire events", status: "in_progress", activeForm: "wiring the events" },
    { id: 3, subject: "Write tests", status: "pending", blockedBy: [2] },
    { id: 4, subject: "Old idea", status: "deleted" }
  ];

  it("renders counts, markers, activeForm and dependency hints, hiding tombstones", () => {
    render(<TodoPanel tasks={tasks} />);
    expect(screen.getByText("Todos")).toBeInTheDocument();
    expect(screen.getByText("(1/3)")).toBeInTheDocument();
    expect(screen.getByText("Set up renderer")).toBeInTheDocument();
    expect(screen.getByText("(wiring the events)")).toBeInTheDocument();
    expect(screen.getByLabelText("Blocked by #2")).toBeInTheDocument();
    // Ids prefix every row once any task carries dependencies; the blocked hint repeats it.
    expect(screen.getAllByText("#2").length).toBeGreaterThanOrEqual(2);
    expect(screen.getByRole("listitem", { name: "Wire events, in progress" })).toBeInTheDocument();
    expect(screen.queryByText("Old idea")).not.toBeInTheDocument();
  });

  it("draws a ring for open tasks and a check for completed ones", () => {
    const { container } = render(<TodoPanel tasks={tasks} />);
    expect(container.querySelectorAll(".todo-mark-circle.pending")).toHaveLength(1);
    expect(container.querySelectorAll(".todo-mark-circle.in_progress")).toHaveLength(1);
    expect(container.querySelectorAll(".todo-check")).toHaveLength(1);
  });

  it("omits row ids when no task has dependencies", () => {
    render(<TodoPanel tasks={[{ id: 1, subject: "Solo task", status: "pending" }]} />);
    expect(screen.getByText("Solo task")).toBeInTheDocument();
    expect(screen.queryByText("#1")).not.toBeInTheDocument();
  });

  it("hides itself when there is nothing to show", () => {
    expect(render(<TodoPanel tasks={[]} />).container.firstChild).toBeNull();
    expect(render(<TodoPanel />).container.firstChild).toBeNull();
    const tombed: TodoTask[] = [{ id: 1, subject: "Gone", status: "deleted" }];
    expect(render(<TodoPanel tasks={tombed} />).container.firstChild).toBeNull();
  });

  it("folds completed tasks away at the start of a run and reveals them on demand", () => {
    const { rerender } = render(<TodoPanel tasks={tasks} />);
    expect(screen.getByText("Set up renderer")).toBeInTheDocument();

    rerender(<TodoPanel tasks={tasks} busy />);
    expect(screen.queryByText("Set up renderer")).not.toBeInTheDocument();
    expect(screen.getByText("+1 done")).toBeInTheDocument();

    fireEvent.click(screen.getByText("+1 done"));
    expect(screen.getByText("Set up renderer")).toBeInTheDocument();
    expect(screen.queryByText("+1 done")).not.toBeInTheDocument();
  });

  it("keeps a task completed mid-run visible until the next run starts", () => {
    const midRun: TodoTask[] = [...tasks, { id: 5, subject: "Finish docs", status: "completed" }];
    const { rerender } = render(<TodoPanel tasks={tasks} busy />);

    rerender(<TodoPanel tasks={midRun} busy />);
    expect(screen.getByText("Finish docs")).toBeInTheDocument();
    expect(screen.queryByText("Set up renderer")).not.toBeInTheDocument();

    // The next run boundary folds both completions behind the summary.
    rerender(<TodoPanel tasks={midRun} />);
    rerender(<TodoPanel tasks={midRun} busy />);
    expect(screen.queryByText("Finish docs")).not.toBeInTheDocument();
    expect(screen.getByText("+2 done")).toBeInTheDocument();
  });

  it("collapses to just the heading and expands back", () => {
    render(<TodoPanel tasks={tasks} />);
    fireEvent.click(screen.getByRole("button", { name: "Collapse todos" }));
    expect(screen.queryByText("Set up renderer")).not.toBeInTheDocument();
    expect(screen.getByText("(1/3)")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Expand todos" }));
    expect(screen.getByText("Set up renderer")).toBeInTheDocument();
  });
});
