import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProviderRecord, SubagentConfig, SubagentRecord } from "../types";
import { SubagentsSection } from "./SubagentsSection";

afterEach(cleanup);

const scout: SubagentRecord = {
  id: "builtin:scout", builtin: true, enabled: true, name: "scout",
  description: "Fast read-only reconnaissance.", prompt: "You are Scout.",
  tools: ["read", "grep", "find", "ls", "bash"], readOnly: true, model: null
};
const docs: SubagentRecord = {
  id: "custom-1", builtin: false, enabled: true, name: "docs",
  description: "Checks the docs.", prompt: "Read the docs.", tools: ["read"], readOnly: true, model: null
};
const config: SubagentConfig = { enabled: true, trigger: "on_request", maxConcurrency: 4, agents: [scout, docs] };

const provider: ProviderRecord = {
  id: "custom-a", name: "Cheap", kind: "custom", baseUrl: "https://example.test/v1", apiFormat: "openai-completions",
  models: [{ id: "mini", name: "Mini", contextWindow: 64_000, maxTokens: 4_096, reasoning: true, thinkingLevels: ["off", "low", "medium"], thinkingLevelMap: {}, vision: false }],
  createdAt: "now", updatedAt: "now", hasApiKey: true, connected: true
};

function renderSection(overrides: Partial<SubagentConfig> = {}, providers: ProviderRecord[] = [provider], webFetchEnabled = true) {
  const onChange = vi.fn().mockResolvedValue(undefined);
  render(<SubagentsSection config={{ ...config, ...overrides }} providers={providers} onChange={onChange} webFetchEnabled={webFetchEnabled} />);
  return { onChange };
}

describe("SubagentsSection", () => {
  it("offers web_fetch to read-only agents, and says when Web Fetch is switched off", () => {
    const webScout = { ...scout, tools: [...scout.tools, "web_fetch"] };
    renderSection({ agents: [webScout, docs] });
    fireEvent.click(screen.getByRole("button", { name: /scout/ }));
    expect(screen.getByText("read, grep, find, ls, bash, web_fetch")).toBeInTheDocument();
    expect(screen.queryByText(/Web Fetch is off/)).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /New agent/ }));
    const chip = screen.getByRole("button", { name: "web_fetch" });
    expect(chip).toBeEnabled();
    fireEvent.click(chip);
    expect(chip).toHaveAttribute("aria-pressed", "true");

    cleanup();
    renderSection({ agents: [webScout, docs] }, [provider], false);
    fireEvent.click(screen.getByRole("button", { name: /scout/ }));
    expect(screen.getByText(/Web Fetch is off in Settings › Packages/)).toBeInTheDocument();
  });

  it("switches an agent off and saves the change at once", async () => {
    const { onChange } = renderSection();
    fireEvent.click(screen.getByRole("switch", { name: "Use scout" }));
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    expect(onChange.mock.calls[0][0].agents[0]).toMatchObject({ id: "builtin:scout", enabled: false });
  });

  it("gives an agent its own model, and back to the chat's", async () => {
    const { onChange } = renderSection();
    fireEvent.click(screen.getByRole("button", { name: /scout/ }));
    const group = screen.getByRole("radiogroup", { name: "scout model" });
    fireEvent.click(within(group).getByRole("radio", { name: "Specific model" }));
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    expect(onChange.mock.calls[0][0].agents[0].model).toEqual({ providerId: "custom-a", modelId: "mini", thinkingLevel: "medium" });
  });

  it("warns when an agent's own model can no longer run", () => {
    renderSection({ agents: [{ ...scout, model: { providerId: "custom-a", modelId: "mini", thinkingLevel: "low" } }] }, [{ ...provider, connected: false }]);
    expect(screen.getByText(/Cheap has no API key\. It will refuse to run/)).toBeInTheDocument();
  });

  it("shows a built-in's instructions read-only and duplicates it as an editable custom agent", async () => {
    const { onChange } = renderSection();
    fireEvent.click(screen.getByRole("button", { name: /scout/ }));
    expect(screen.getByText("You are Scout.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Edit$/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Duplicate as custom/ }));
    expect(screen.getByDisplayValue("scout-custom")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Add agent" }));
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    const added = onChange.mock.calls[0][0].agents.at(-1);
    expect(added).toMatchObject({ id: "", builtin: false, name: "scout-custom", prompt: "You are Scout.", readOnly: true });
  });

  it("creates a custom agent, keeping edit tools off until it may edit files", async () => {
    const { onChange } = renderSection();
    fireEvent.click(screen.getByRole("button", { name: /New agent/ }));
    const add = screen.getByRole("button", { name: "Add agent" });
    expect(add).toBeDisabled();
    fireEvent.change(screen.getByPlaceholderText("docs-checker"), { target: { value: "Fixer" } });
    fireEvent.change(screen.getByPlaceholderText("Checks the docs match the code"), { target: { value: "Fixes lint" } });
    fireEvent.change(screen.getByPlaceholderText("You are a sub-agent that…"), { target: { value: "Fix the lint errors." } });
    expect(screen.getByRole("button", { name: "write" })).toBeDisabled();
    fireEvent.click(screen.getByRole("switch", { name: "Can edit files" }));
    fireEvent.click(screen.getByRole("button", { name: "write" }));
    fireEvent.click(add);
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    expect(onChange.mock.calls[0][0].agents.at(-1)).toMatchObject({
      name: "fixer", description: "Fixes lint", readOnly: false, tools: ["read", "grep", "find", "ls", "bash", "write"]
    });
  });

  it("deletes a custom agent only after confirmation, and shows why a save failed", async () => {
    const { onChange } = renderSection();
    fireEvent.click(screen.getByRole("button", { name: /docs/ }));
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    const dialog = screen.getByRole("alertdialog", { name: "Delete docs?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    expect(onChange.mock.calls[0][0].agents.map((agent: SubagentRecord) => agent.name)).toEqual(["scout"]);

    cleanup();
    render(<SubagentsSection config={config} providers={[provider]} onChange={vi.fn().mockRejectedValue("There is already a sub-agent called scout.")} />);
    fireEvent.click(screen.getByRole("switch", { name: "Use docs" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("There is already a sub-agent called scout.");
  });
});
