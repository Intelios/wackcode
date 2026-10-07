import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
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

  it("scopes digit shortcuts to the card and ignores modified or repeated keys", () => {
    render(<><button>Outside</button><QuestionCard request={request} onRespond={vi.fn()} /></>);
    const card = screen.getByRole("region", { name: "Questions" });
    const sqlite = screen.getByRole("radio", { name: /SQLite/ });
    fireEvent.keyDown(screen.getByRole("button", { name: "Outside" }), { key: "1" });
    for (const modifier of [{ metaKey: true }, { ctrlKey: true }, { altKey: true }, { repeat: true }, { isComposing: true }]) {
      fireEvent.keyDown(card, { key: "1", ...modifier });
    }
    expect(sqlite).toHaveAttribute("aria-checked", "false");
    expect(fireEvent.keyDown(card, { key: "1" })).toBe(false);
    expect(sqlite).toHaveAttribute("aria-checked", "true");
    fireEvent.keyDown(card, { key: "2" });
    expect(screen.getByRole("radio", { name: /Postgres/ })).toHaveAttribute("aria-checked", "true");
    expect(sqlite).toHaveAttribute("aria-checked", "false");
    expect(screen.getByRole("heading", { name: questions[0].question })).toBeInTheDocument();
  });

  it("advances from the heading with Enter only after answering and focuses the new heading", () => {
    const onRespond = vi.fn();
    render(<QuestionCard request={request} onRespond={onRespond} />);
    const heading = screen.getByRole("heading", { name: questions[0].question });
    heading.focus();
    fireEvent.keyDown(heading, { key: "Enter" });
    expect(heading).toHaveFocus();
    expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
    fireEvent.keyDown(heading, { key: "1" });
    fireEvent.keyDown(heading, { key: "Enter" });
    const nextHeading = screen.getByRole("heading", { name: questions[1].question });
    expect(nextHeading).toHaveFocus();
    fireEvent.keyDown(nextHeading, { key: "Enter" });
    expect(onRespond).not.toHaveBeenCalled();
    fireEvent.keyDown(nextHeading, { key: "2" });
    fireEvent.keyDown(nextHeading, { key: "Enter" });
    expect(onRespond).toHaveBeenCalledWith({ answers: [
      { questionId: "approach", selected: ["SQLite"] },
      { questionId: "scope", selected: ["Worker"] },
    ] });
  });

  it("does not intercept typing, newline, or command-Enter in Other", () => {
    const onRespond = vi.fn();
    render(<QuestionCard request={{ ...request, questions: [questions[0]] }} onRespond={onRespond} />);
    const other = screen.getByRole("textbox", { name: /Other/ });
    other.focus();
    fireEvent.change(other, { target: { value: "Custom\nanswer" } });
    for (const event of [{ key: "1" }, { key: "Enter" }, { key: "Enter", metaKey: true }]) {
      expect(fireEvent.keyDown(other, event)).toBe(true);
    }
    expect(other).toHaveValue("Custom\nanswer");
    expect(screen.getByRole("radio", { name: /SQLite/ })).toHaveAttribute("aria-checked", "false");
    expect(onRespond).not.toHaveBeenCalled();
  });

  it("leaves native button Enter activation to the button", () => {
    const onRespond = vi.fn();
    render(<QuestionCard request={request} onRespond={onRespond} />);
    const option = screen.getByRole("radio", { name: /SQLite/ });
    fireEvent.click(option);
    option.focus();
    expect(fireEvent.keyDown(option, { key: "Enter" })).toBe(true);
    expect(screen.getByRole("heading", { name: questions[0].question })).toBeInTheDocument();
    const cancel = screen.getByRole("button", { name: "Cancel" });
    cancel.focus();
    expect(fireEvent.keyDown(cancel, { key: "Enter" })).toBe(true);
    expect(onRespond).not.toHaveBeenCalled();
    // jsdom does not synthesize a native click from a keyboard event.
    fireEvent.click(cancel);
    expect(onRespond).toHaveBeenCalledExactlyOnceWith({ cancelled: true });
  });

  it("gates command-Enter on every answer, even when invoked from an earlier question", () => {
    const onRespond = vi.fn();
    render(<QuestionCard request={request} onRespond={onRespond} />);
    const card = screen.getByRole("region", { name: "Questions" });
    expect(fireEvent.keyDown(card, { key: "Enter", metaKey: true })).toBe(true);
    fireEvent.click(screen.getByRole("radio", { name: /SQLite/ }));
    expect(fireEvent.keyDown(card, { key: "Enter", metaKey: true })).toBe(true);
    expect(onRespond).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("tab", { name: /Scope/ }));
    fireEvent.click(screen.getByRole("checkbox", { name: /Worker/ }));
    fireEvent.click(screen.getByRole("tab", { name: /Approach/ }));
    expect(fireEvent.keyDown(card, { key: "Enter", metaKey: true })).toBe(false);
    expect(onRespond).toHaveBeenCalledExactlyOnceWith({ answers: [
      { questionId: "approach", selected: ["SQLite"] },
      { questionId: "scope", selected: ["Worker"] },
    ] });
  });

  it("replaces multiple presets with Other and clears Other when a preset is picked", () => {
    const onRespond = vi.fn();
    render(<QuestionCard request={{ ...request, questions: [questions[1]] }} onRespond={onRespond} />);
    const ui = screen.getByRole("checkbox", { name: /UI/ });
    const worker = screen.getByRole("checkbox", { name: /Worker/ });
    fireEvent.click(ui);
    fireEvent.click(worker);
    expect(ui).toHaveAttribute("aria-checked", "true");
    expect(worker).toHaveAttribute("aria-checked", "true");
    const other = screen.getByRole("textbox", { name: /Other/ });
    fireEvent.focus(other);
    expect(ui).toHaveAttribute("aria-checked", "false");
    expect(worker).toHaveAttribute("aria-checked", "false");
    expect(screen.getByRole("button", { name: "Submit" })).toBeDisabled();
    fireEvent.change(other, { target: { value: "A different layer" } });
    expect(screen.getByRole("button", { name: "Submit" })).toBeEnabled();
    fireEvent.click(worker);
    expect(other).toHaveValue("");
    expect(worker).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    expect(onRespond).toHaveBeenCalledWith({ answers: [{ questionId: "scope", selected: ["Worker"] }] });
  });

  it("supports wrapping arrow, Home, and End tab navigation with roving focus", () => {
    render(<QuestionCard request={request} onRespond={vi.fn()} />);
    const approach = screen.getByRole("tab", { name: /Approach/ });
    const scope = screen.getByRole("tab", { name: /Scope/ });
    approach.focus();
    for (const [key, target] of [
      ["ArrowRight", scope], ["ArrowRight", approach], ["ArrowLeft", scope],
      ["Home", approach], ["End", scope],
    ] as const) {
      expect(fireEvent.keyDown(document.activeElement!, { key })).toBe(false);
      expect(target).toHaveFocus();
      expect(target).toHaveAttribute("aria-selected", "true");
      expect(target).toHaveAttribute("tabindex", "0");
      const inactive = target === approach ? scope : approach;
      expect(inactive).toHaveAttribute("aria-selected", "false");
      expect(inactive).toHaveAttribute("tabindex", "-1");
      expect(screen.getByRole("tabpanel")).toHaveAttribute("id", target.getAttribute("aria-controls"));
    }
    fireEvent.click(approach);
    expect(screen.getByRole("heading", { name: questions[0].question })).toHaveFocus();
  });

  it("submits only once while the answered card remains mounted", () => {
    const onRespond = vi.fn();
    render(<QuestionCard request={{ ...request, questions: [questions[0]], offerWrapUp: true }} onRespond={onRespond} />);
    fireEvent.click(screen.getByRole("radio", { name: /SQLite/ }));
    const submit = screen.getByRole("button", { name: "Submit" });
    const card = screen.getByRole("region", { name: "Questions" });
    act(() => {
      fireEvent.click(submit);
      fireEvent.click(submit);
      fireEvent.keyDown(card, { key: "Enter", metaKey: true });
      fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
      fireEvent.click(screen.getByRole("button", { name: "Write the plan now" }));
    });
    expect(onRespond).toHaveBeenCalledExactlyOnceWith({ answers: [{ questionId: "approach", selected: ["SQLite"] }] });
    expect(card).toHaveAttribute("inert");
  });

  it("omits the tab rail for a single question", () => {
    render(<QuestionCard request={{ ...request, questions: [questions[0]] }} onRespond={vi.fn()} />);
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
    expect(screen.queryByRole("tab")).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: questions[0].question })).toBeInTheDocument();
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
    expect(screen.getByRole("region", { name: "Questions" })).not.toHaveClass("question-ultra");
    cleanup();

    render(<QuestionCard request={{ ...request, offerWrapUp: true }} onRespond={onRespond} />);
    expect(screen.getByText("Ultra Plan · Question")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Questions" })).toHaveClass("question-ultra");
    fireEvent.click(screen.getByRole("button", { name: "Write the plan now" }));
    expect(onRespond).toHaveBeenCalledWith({ wrapUp: true });
  });
});
