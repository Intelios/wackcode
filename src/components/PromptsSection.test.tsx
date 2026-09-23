import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_PLAN_PROMPT, DEFAULT_SYSTEM_PROMPT } from "../promptDefaults";
import type { PromptConfig } from "../types";
import { PromptsSection } from "./PromptsSection";

afterEach(cleanup);

function renderPrompts(config: PromptConfig = {}, onChange = vi.fn().mockResolvedValue(undefined)) {
  render(<PromptsSection config={config} onChange={onChange} />);
  return { onChange };
}

function customise(title: string) {
  fireEvent.click(within(screen.getByRole("article", { name: title })).getByRole("button", { name: /Customise|Edit/ }));
}

describe("PromptsSection", () => {
  it("shows the built-in prompts with the tuned-for-WackCode notice", () => {
    renderPrompts();
    expect(screen.getByRole("heading", { name: "Built-in prompts" })).toBeInTheDocument();
    expect(screen.getByText(/designed to work best with Pi and WackCode/i)).toBeInTheDocument();
    expect(screen.getByText(/restore the built-in text at any time/i)).toBeInTheDocument();
    expect(screen.getAllByText("Default")).toHaveLength(3);
    // Nothing to restore while every prompt is at its default.
    expect(screen.queryByRole("button", { name: "Restore default" })).not.toBeInTheDocument();
    // The default texts are on screen for review, starting with the persona paragraph.
    expect(document.querySelector(".prompt-view")?.textContent).toContain(DEFAULT_SYSTEM_PROMPT);
  });

  it("marks a customised prompt and offers restore only for it", () => {
    renderPrompts({ planPrompt: "My plan rules." });
    const planCard = screen.getByRole("article", { name: "Plan mode" });
    expect(within(planCard).getByText("Customised")).toBeInTheDocument();
    expect(within(planCard).getByRole("button", { name: "Restore default" })).toBeInTheDocument();
    const personaCard = screen.getByRole("article", { name: "Default system prompt" });
    expect(within(personaCard).getByText("Default")).toBeInTheDocument();
    expect(within(personaCard).queryByRole("button", { name: "Restore default" })).not.toBeInTheDocument();
  });

  it("saves an edited prompt and reports a failed save", async () => {
    const onChange = vi.fn().mockResolvedValue(undefined);
    renderPrompts({}, onChange);
    customise("Default system prompt");
    const editor = screen.getByRole("textbox", { name: "Default system prompt text" });
    // The editor starts from the effective text, so a light edit is a real customization.
    expect(editor).toHaveValue(DEFAULT_SYSTEM_PROMPT);
    fireEvent.change(editor, { target: { value: "You are my bespoke agent." } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith({ systemPrompt: "You are my bespoke agent." }));

    // A failed save keeps the editor open and surfaces the error.
    const failing = vi.fn().mockRejectedValue("Could not save settings.");
    cleanup();
    renderPrompts({ systemPrompt: "kept" }, failing);
    customise("Default system prompt");
    fireEvent.change(screen.getByRole("textbox", { name: "Default system prompt text" }), { target: { value: "new text" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Could not save settings.")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Default system prompt text" })).toBeInTheDocument();
  });

  it("refuses to save a blank or over-limit prompt", () => {
    renderPrompts();
    customise("Plan mode");
    const editor = screen.getByRole("textbox", { name: "Plan mode text" }) as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: "   " } });
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();

    fireEvent.change(editor, { target: { value: "x".repeat(20_001) } });
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(screen.getByText(/20,001 \/ 20,000 characters/)).toBeInTheDocument();
  });

  it("restores a customised prompt to the built-in text after confirming", async () => {
    const onChange = vi.fn().mockResolvedValue(undefined);
    renderPrompts({ planPrompt: "My plan rules." }, onChange);
    fireEvent.click(screen.getByRole("button", { name: "Restore default" }));
    // Destructive: only the confirmed dialog clears the customization.
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Restore" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith({ planPrompt: null }));
  });

  it("starts a fresh customization from the built-in default text", () => {
    renderPrompts();
    customise("Plan mode");
    expect(screen.getByRole("textbox", { name: "Plan mode text" })).toHaveValue(DEFAULT_PLAN_PROMPT);
  });
});
