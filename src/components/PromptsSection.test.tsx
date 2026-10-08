import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_CHAT_PROMPT, DEFAULT_PLAN_PROMPT, DEFAULT_SYSTEM_PROMPT } from "../promptDefaults";
import type { PromptConfig } from "../types";
import { PromptsSection } from "./PromptsSection";

afterEach(cleanup);

function renderPrompts(config: PromptConfig = {}, onChange = vi.fn().mockResolvedValue(undefined)) {
  render(<PromptsSection config={config} agentName="Nova" onChange={onChange} />);
  return { onChange };
}

function customise(title: string) {
  fireEvent.click(within(screen.getByRole("article", { name: title })).getByRole("button", { name: /Customise|Edit/ }));
}

describe("PromptsSection", () => {
  it("introduces the prompts under the agent's name and shows the built-in text", () => {
    renderPrompts();
    expect(screen.getByRole("heading", { name: "Shape how Nova thinks" })).toBeInTheDocument();
    expect(screen.getByText(/restore the built-in text whenever you like/i)).toBeInTheDocument();
    expect(screen.getByText(/including chats that are already planning/i)).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Prompts overview" })).getByText("Built-in")).toBeInTheDocument();
    // The coding persona, Plan, Ultra Plan and Chat mode.
    expect(screen.getAllByText("Default")).toHaveLength(4);
    // Nothing to restore while every prompt is at its default.
    expect(screen.queryByRole("button", { name: "Restore default" })).not.toBeInTheDocument();
    // The default texts are on screen for review, starting with the persona paragraph.
    expect(document.querySelector(".prompt-view")?.textContent).toContain(DEFAULT_SYSTEM_PROMPT);
  });

  it("offers Chat mode's persona with its own default and says a guide follows it", async () => {
    const { onChange } = renderPrompts();
    const card = screen.getByRole("article", { name: "Chat mode" });
    expect(card.textContent).toContain(DEFAULT_CHAT_PROMPT.split("\n")[0]);
    customise("Chat mode");
    expect(screen.getByRole("textbox", { name: "Chat mode text" })).toHaveValue(DEFAULT_CHAT_PROMPT);
    expect(within(card).getByText(/adds a short guide after this text/i)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "Chat mode text" }), { target: { value: "You are Quill." } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith({ chatPrompt: "You are Quill." }));
  });

  it("counts customised prompts in the hero", () => {
    renderPrompts({ planPrompt: "My plan rules.", ultraPlanPrompt: "My ultra rules." });
    expect(within(screen.getByRole("region", { name: "Prompts overview" })).getByText("2 customised")).toBeInTheDocument();
  });

  it("clamps a long prompt's preview until asked to show the full text", () => {
    renderPrompts({ planPrompt: Array.from({ length: 12 }, (_, line) => `Rule ${line + 1}.`).join("\n") });
    const planCard = screen.getByRole("article", { name: "Plan mode" });
    const preview = planCard.querySelector(".prompt-view");
    expect(preview).toHaveClass("clamped");
    fireEvent.click(within(planCard).getByRole("button", { name: "Show full text" }));
    expect(preview).not.toHaveClass("clamped");
    expect(within(planCard).getByRole("button", { name: "Show less" })).toHaveAttribute("aria-expanded", "true");
    // The persona is two sentences: nothing to clamp, so no toggle.
    const personaCard = screen.getByRole("article", { name: "Default system prompt" });
    expect(within(personaCard).queryByRole("button", { name: /Show/ })).not.toBeInTheDocument();
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
    // The meter tops out at the limit and turns to the over-limit state.
    const meter = screen.getByRole("meter", { name: "Plan mode length" });
    expect(meter).toHaveAttribute("aria-valuenow", "20000");
    expect(meter).toHaveClass("over");
  });

  it("fills the meter as the draft grows", () => {
    renderPrompts();
    customise("Plan mode");
    const editor = screen.getByRole("textbox", { name: "Plan mode text" });
    fireEvent.change(editor, { target: { value: "x".repeat(5_000) } });
    const meter = screen.getByRole("meter", { name: "Plan mode length" });
    expect(meter).toHaveAttribute("aria-valuenow", "5000");
    expect(meter).not.toHaveClass("warn");
    fireEvent.change(editor, { target: { value: "x".repeat(18_500) } });
    expect(meter).toHaveClass("warn");
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
