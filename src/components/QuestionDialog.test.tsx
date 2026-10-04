import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AskQuestion, ExtensionUIRequest } from "../types";
import { QuestionCard } from "./QuestionDialog";

afterEach(cleanup);

const base = { taskId: "task", requestId: "req-1" };

const questions: AskQuestion[] = [
  {
    id: "approach",
    header: "Approach",
    question: "Which storage engine?",
    options: [
      { label: "SQLite", description: "Simple, local file." },
      { label: "Postgres", description: "Shared, heavier to run." },
    ],
  },
  {
    id: "scope",
    header: "Scope",
    question: "Which layers change?",
    multiSelect: true,
    options: [
      { label: "UI", description: "Dialog and transcript." },
      { label: "Worker", description: "Protocol and tools." },
      { label: "Rust", description: "Commands and storage." },
    ],
  },
];

const request: Extract<ExtensionUIRequest, { method: "questions" }> = {
  ...base, method: "questions", title: "Questions", questions
};

describe("QuestionCard", () => {
  it("submits one answer per question with the picked option labels", () => {
    const onRespond = vi.fn();
    render(<QuestionCard request={request} onRespond={onRespond} />);

    // Picking an answer stays on the question; advancing is a deliberate Next.
    fireEvent.click(screen.getByText("Postgres"));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    fireEvent.click(screen.getByText("UI"));
    fireEvent.click(screen.getByText("Rust"));
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));

    expect(onRespond).toHaveBeenCalledWith({
      answers: [
        { questionId: "approach", selected: ["Postgres"] },
        { questionId: "scope", selected: ["UI", "Rust"] },
      ],
    });
  });

  it("keeps Next and Submit disabled until their questions have answers", () => {
    const onRespond = vi.fn();
    render(<QuestionCard request={request} onRespond={onRespond} />);
    const next = screen.getByRole("button", { name: "Next" });
    expect(next).toBeDisabled();
    fireEvent.click(screen.getByText("SQLite"));
    expect(next).toBeEnabled();
    fireEvent.click(next);
    expect(screen.getByRole("button", { name: "Submit" })).toBeDisabled();
    fireEvent.click(screen.getByText("Worker"));
    expect(screen.getByRole("button", { name: "Submit" })).toBeEnabled();
  });

  it("lets a free-form Other answer replace the preset choices", () => {
    const onRespond = vi.fn();
    const single = { ...request, questions: [questions[0]] };
    render(<QuestionCard request={single} onRespond={onRespond} />);

    fireEvent.click(screen.getByText("SQLite"));
    fireEvent.change(screen.getByPlaceholderText(/Other/), { target: { value: "Just use files" } });
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));

    expect(onRespond).toHaveBeenCalledWith({
      answers: [{ questionId: "approach", selected: [], custom: "Just use files" }],
    });
  });

  it("navigates between question tabs without losing answers", () => {
    const onRespond = vi.fn();
    render(<QuestionCard request={request} onRespond={onRespond} />);
    fireEvent.click(screen.getByText("SQLite"));
    fireEvent.click(screen.getByRole("tab", { name: /Approach/ }));
    expect(screen.getByText("SQLite").closest("button")).toHaveClass("selected");
    fireEvent.click(screen.getByRole("tab", { name: /Scope/ }));
    fireEvent.click(screen.getByText("UI"));
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    expect(onRespond).toHaveBeenCalled();
  });

  it("sends cancelled when the Cancel button is clicked", () => {
    const onRespond = vi.fn();
    render(<QuestionCard request={request} onRespond={onRespond} />);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onRespond).toHaveBeenCalledWith({ cancelled: true });
  });

  it("offers \"Write the plan now\" only for Ultra Plan's interview", () => {
    const onRespond = vi.fn();
    render(<QuestionCard request={request} onRespond={onRespond} />);
    expect(screen.queryByRole("button", { name: "Write the plan now" })).toBeNull();
    expect(screen.getByText("Question")).toBeInTheDocument();
    cleanup();

    render(<QuestionCard request={{ ...request, offerWrapUp: true }} onRespond={onRespond} />);
    expect(screen.getByText("Ultra Plan · Question")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Write the plan now" }));
    expect(onRespond).toHaveBeenCalledWith({ wrapUp: true });
  });
});
