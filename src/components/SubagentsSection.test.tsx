import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AutoTitleConfig, ProviderRecord, SubagentConfig, SubagentRecord } from "../types";
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

const noAutoTitle: AutoTitleConfig = { enabled: false, providerId: null, modelId: null };

function renderSection(
  overrides: Partial<SubagentConfig> = {},
  providers: ProviderRecord[] = [provider],
  webFetchEnabled = true,
  autoTitle: AutoTitleConfig = noAutoTitle
) {
  const onChange = vi.fn().mockResolvedValue(undefined);
  const onSetAutoTitle = vi.fn().mockResolvedValue(undefined);
  const onOpenProviders = vi.fn();
  render(
    <SubagentsSection
      config={{ ...config, ...overrides }}
      providers={providers}
      onChange={onChange}
      webFetchEnabled={webFetchEnabled}
      autoTitle={autoTitle}
      onSetAutoTitle={onSetAutoTitle}
      onOpenProviders={onOpenProviders}
    />
  );
  return { onChange, onSetAutoTitle, onOpenProviders };
}

describe("SubagentsSection", () => {
  it("sets when to delegate and how many run at once with one click each", async () => {
    const { onChange } = renderSection();
    const when = screen.getByRole("radiogroup", { name: "When to use sub-agents" });
    expect(within(when).getByRole("radio", { name: /Only when I ask/ })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(within(when).getByRole("radio", { name: /Whenever useful/ }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ trigger: "auto" })));
    const count = screen.getByRole("radiogroup", { name: "Sub-agents running at the same time" });
    expect(within(count).getByRole("radio", { name: "4" })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(within(count).getByRole("radio", { name: "2" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ maxConcurrency: 2 })));
  });

  it("counts switched-on agents in the hero", () => {
    renderSection({ agents: [scout, { ...docs, enabled: false }] });
    const hero = screen.getByRole("region", { name: "Sub-agents overview" });
    expect(within(hero).getByText("1 of 2 on")).toBeInTheDocument();
    expect(within(hero).getByRole("heading", { name: "Helpers WackCode can hand work to" })).toBeInTheDocument();
  });

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
    render(<SubagentsSection config={config} providers={[provider]} onChange={vi.fn().mockRejectedValue("There is already a sub-agent called scout.")} autoTitle={noAutoTitle} onSetAutoTitle={vi.fn()} onOpenProviders={vi.fn()} />);
    fireEvent.click(screen.getByRole("switch", { name: "Use docs" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("There is already a sub-agent called scout.");
  });
});

describe("SubagentsSection auto titles", () => {
  const titleModelProvider: ProviderRecord = {
    id: "p", name: "Test connection", kind: "custom", connected: true, hasApiKey: true,
    baseUrl: "https://example.test/v1", apiFormat: "openai-completions", createdAt: "now", updatedAt: "now",
    models: [{ id: "small", name: "Small model", contextWindow: 16_000, maxTokens: 2_000,
      reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: {}, vision: false }]
  };

  it("requires an explicit model before enabling and saves the choice first", async () => {
    const { onSetAutoTitle } = renderSection({}, [titleModelProvider]);
    expect(screen.getByRole("switch", { name: "Automatic titles" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: /auto-titles/ }));
    fireEvent.click(screen.getByRole("button", { name: "Choose model" }));
    fireEvent.click(screen.getByRole("button", { name: /Test connection/ }));
    fireEvent.click(screen.getByRole("button", { name: "Small model" }));
    await waitFor(() => expect(onSetAutoTitle).toHaveBeenCalledWith({ enabled: false, providerId: "p", modelId: "small" }));

    cleanup();
    const again = renderSection({}, [titleModelProvider], true, { enabled: false, providerId: "p", modelId: "small" });
    fireEvent.click(screen.getByRole("switch", { name: "Automatic titles" }));
    await waitFor(() => expect(again.onSetAutoTitle).toHaveBeenLastCalledWith({ enabled: true, providerId: "p", modelId: "small" }));
  });

  it("offers connection setup when no model is available", () => {
    const { onOpenProviders } = renderSection({}, []);
    expect(screen.getByRole("switch", { name: "Automatic titles" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: /auto-titles/ }));
    fireEvent.click(screen.getByRole("button", { name: "Add a connection" }));
    expect(onOpenProviders).toHaveBeenCalledOnce();
  });

  it("keeps the existing setting when a save fails and shows the error", async () => {
    const onSetAutoTitle = vi.fn().mockRejectedValue(new Error("Could not save"));
    render(
      <SubagentsSection
        config={config} providers={[titleModelProvider]} onChange={vi.fn()}
        autoTitle={{ enabled: false, providerId: "p", modelId: "small" }}
        onSetAutoTitle={onSetAutoTitle} onOpenProviders={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole("switch", { name: "Automatic titles" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save");
    expect(screen.getByRole("switch", { name: "Automatic titles" })).toHaveAttribute("aria-checked", "false");
  });

  it("warns when the chosen title model can no longer run", () => {
    renderSection({}, [{ ...titleModelProvider, connected: false }], true, { enabled: false, providerId: "p", modelId: "small" });
    expect(screen.getByText(/Test connection has no API key\. Automatic titles can't run/)).toBeInTheDocument();
  });
});
