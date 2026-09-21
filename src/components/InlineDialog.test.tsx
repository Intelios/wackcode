import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ExtensionUIRequest } from "../types";
import { InlineDialog } from "./InlineDialog";

afterEach(cleanup);

const questionRequest: ExtensionUIRequest = {
  taskId: "task-1",
  requestId: "req-1",
  method: "questions",
  title: "Questions",
  questions: [{
    id: "q1",
    header: "Q1",
    question: "Pick one",
    options: [{ label: "A", description: "Option A" }],
  }],
};

const confirmRequest: ExtensionUIRequest = {
  taskId: "task-2",
  requestId: "req-2",
  method: "confirm",
  title: "Confirm action",
  message: "Are you sure?",
};

describe("InlineDialog", () => {
  it("renders nothing when no requests match the selected task", () => {
    const onRespond = vi.fn();
    const { container } = render(
      <InlineDialog requests={[questionRequest]} selectedTaskId="other-task" onRespond={onRespond} />
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders a QuestionCard for a questions request", () => {
    const onRespond = vi.fn();
    render(
      <InlineDialog requests={[questionRequest]} selectedTaskId="task-1" onRespond={onRespond} />
    );
    expect(screen.getByText("Pick one")).toBeInTheDocument();
    expect(screen.getByText("Question")).toBeInTheDocument();
  });

  it("renders an ExtensionCard for a non-questions request", () => {
    const onRespond = vi.fn();
    render(
      <InlineDialog requests={[confirmRequest]} selectedTaskId="task-2" onRespond={onRespond} />
    );
    expect(screen.getByText("Are you sure?")).toBeInTheDocument();
    expect(screen.getByText("Extension")).toBeInTheDocument();
  });

  it("only shows the request for the selected task", () => {
    const onRespond = vi.fn();
    render(
      <InlineDialog requests={[questionRequest, confirmRequest]} selectedTaskId="task-2" onRespond={onRespond} />
    );
    // Should show the confirm for task-2, not the question for task-1
    expect(screen.getByText("Are you sure?")).toBeInTheDocument();
    expect(screen.queryByText("Pick one")).not.toBeInTheDocument();
  });

  it("calls onRespond with the request and response when answered", () => {
    const onRespond = vi.fn();
    render(
      <InlineDialog requests={[confirmRequest]} selectedTaskId="task-2" onRespond={onRespond} />
    );
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    expect(onRespond).toHaveBeenCalledWith(confirmRequest, { confirmed: true });
  });

  it("calls onRespond with cancelled on Escape", () => {
    const onRespond = vi.fn();
    render(
      <InlineDialog requests={[confirmRequest]} selectedTaskId="task-2" onRespond={onRespond} />
    );
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onRespond).toHaveBeenCalledWith(confirmRequest, { cancelled: true });
  });

  it("does not fire Escape handler when no request matches", () => {
    const onRespond = vi.fn();
    render(
      <InlineDialog requests={[confirmRequest]} selectedTaskId="other-task" onRespond={onRespond} />
    );
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onRespond).not.toHaveBeenCalled();
  });
});
