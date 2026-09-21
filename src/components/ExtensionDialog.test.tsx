import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ExtensionUIRequest } from "../types";
import { ExtensionCard } from "./ExtensionDialog";

afterEach(cleanup);

const base = { taskId: "task", requestId: "req-1" };

describe("ExtensionCard", () => {
  it("returns the chosen option from a select", () => {
    const onRespond = vi.fn();
    const request: ExtensionUIRequest = { ...base, method: "select", title: "Pick a branch", options: ["main", "develop"] };
    render(<ExtensionCard request={request} onRespond={onRespond} />);

    fireEvent.click(screen.getByRole("radio", { name: "develop" }));
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    expect(onRespond).toHaveBeenCalledWith({ value: "develop" });
  });

  it("defaults a select to the first option so Submit is never ambiguous", () => {
    const onRespond = vi.fn();
    render(<ExtensionCard request={{ ...base, method: "select", title: "T", options: ["a", "b"] }} onRespond={onRespond} />);
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    expect(onRespond).toHaveBeenCalledWith({ value: "a" });
  });

  it("returns true only when a confirm is actually confirmed", () => {
    const onRespond = vi.fn();
    render(<ExtensionCard request={{ ...base, method: "confirm", title: "Delete?", message: "This cannot be undone" }} onRespond={onRespond} />);
    expect(screen.getByText("This cannot be undone")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    expect(onRespond).toHaveBeenCalledWith({ confirmed: true });
  });

  it("submits typed input on Enter", () => {
    const onRespond = vi.fn();
    render(<ExtensionCard request={{ ...base, method: "input", title: "Branch name" }} onRespond={onRespond} />);
    const field = screen.getByRole("textbox", { name: "Branch name" });
    fireEvent.change(field, { target: { value: "feature/x" } });
    fireEvent.keyDown(field, { key: "Enter" });
    expect(onRespond).toHaveBeenCalledWith({ value: "feature/x" });
  });

  it("prefills an editor and returns the edited text", () => {
    const onRespond = vi.fn();
    render(<ExtensionCard request={{ ...base, method: "editor", title: "Message", prefill: "draft" }} onRespond={onRespond} />);
    const field = screen.getByRole("textbox", { name: "Message" });
    expect(field).toHaveValue("draft");
    fireEvent.change(field, { target: { value: "final" } });
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    expect(onRespond).toHaveBeenCalledWith({ value: "final" });
  });

  it("sends cancelled when the Cancel button is clicked", () => {
    const onRespond = vi.fn();
    render(<ExtensionCard request={{ ...base, method: "confirm", title: "T", message: "M" }} onRespond={onRespond} />);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onRespond).toHaveBeenCalledWith({ cancelled: true });
  });
});
