import { useState } from "react";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CustomProviderRecord, ModelRecord, ProviderRecord, SaveProviderInput } from "../types";
import { ProvidersSection, type ConnectionMethod } from "./ProvidersSection";

afterEach(cleanup);

const ready: ModelRecord = {
  id: "vendor/big-model", name: "Big Model", contextWindow: 200_000, maxTokens: 32_000,
  reasoning: true, thinkingLevels: ["off", "high"], thinkingLevelMap: { off: null, high: "high" }, vision: true
};

const gateway: CustomProviderRecord = {
  id: "p1", name: "Gateway", kind: "custom", baseUrl: "https://api.gateway.test/v1", apiFormat: "openai-completions",
  models: [ready, { ...ready, id: "vendor/raw", name: "vendor/raw", contextWindow: null, maxTokens: null, reasoning: false, vision: false }],
  createdAt: "now", updatedAt: "now", hasApiKey: true, connected: true
};

const signedOut: ProviderRecord = {
  id: "anthropic", name: "Anthropic", kind: "subscription", baseUrl: "", apiFormat: "", models: [],
  createdAt: "now", updatedAt: "now", hasApiKey: false, connected: false, enabled: false
};

/** ProvidersSection with the navigation SettingsPage gives it. */
function Harness({ providers, start, onSave, onDiscover }: {
  providers: ProviderRecord[];
  start?: string;
  onSave: (input: SaveProviderInput) => Promise<ProviderRecord>;
  onDiscover: (id: string) => Promise<string[]>;
}) {
  const [selected, setSelected] = useState(start);
  const [editor, setEditor] = useState(0);
  const [method, setMethod] = useState<ConnectionMethod>("apiKey");
  const open = (id?: string) => { setSelected(id); setEditor((value) => value + 1); };
  return (
    <ProvidersSection
      providers={providers}
      selectedId={selected}
      editorKey={editor}
      newMethod={method}
      agentName="Quack"
      builtinModels={[]}
      catalogLoading={false}
      subscriptionProviders={[{ id: "anthropic", name: "Anthropic", guidance: "Uses your Claude plan." }]}
      onSelect={open}
      onCreated={setSelected}
      onStartNew={(next) => { setMethod(next); open("new"); }}
      onSave={onSave}
      onDelete={vi.fn()}
      onSetProviderEnabled={vi.fn().mockResolvedValue(undefined)}
      onConnectSubscription={vi.fn()}
      onSignOutSubscription={vi.fn()}
      onDiscover={onDiscover}
      onOpenAuthUrl={vi.fn()}
    />
  );
}

function renderProviders(providers: ProviderRecord[] = [gateway, signedOut], start?: string, discovered: string[] = []) {
  const onSave = vi.fn().mockImplementation(async (input: SaveProviderInput) => ({ ...gateway, ...input, id: input.id ?? "p-new", apiKey: undefined }));
  const onDiscover = vi.fn().mockResolvedValue(discovered);
  render(<Harness providers={providers} start={start} onSave={onSave} onDiscover={onDiscover} />);
  return { onSave, onDiscover };
}

describe("Settings › Providers", () => {
  it("lists each connection with its state and what still needs doing", () => {
    renderProviders();
    const hero = screen.getByRole("region", { name: "Providers overview" });
    expect(within(hero).getByText("1 ready")).toBeInTheDocument();
    expect(within(hero).getByRole("heading", { name: "Models Quack can talk to" })).toBeInTheDocument();
    const row = screen.getByRole("button", { name: /^Gateway/ });
    expect(row).toHaveTextContent("Key saved");
    expect(row).toHaveTextContent("2 models");
    expect(row).toHaveTextContent("1 needs limits");
    expect(screen.getByRole("button", { name: /^Anthropic/ })).toHaveTextContent("Off");
    expect(screen.getByRole("switch", { name: "Use Anthropic" })).toHaveAttribute("aria-checked", "false");
  });

  it("opens a connection from its row, with models as rows that open into their settings", () => {
    renderProviders();
    fireEvent.click(screen.getByRole("button", { name: /^Gateway/ }));
    expect(screen.getByRole("region", { name: "Gateway connection" })).toBeInTheDocument();
    const model = screen.getByRole("button", { name: /^Big Model/ });
    expect(model).toHaveTextContent("200K context · 32K out");
    expect(screen.queryByRole("textbox", { name: "Model ID" })).not.toBeInTheDocument();
    fireEvent.click(model);
    expect(screen.getByRole("textbox", { name: "Model ID" })).toHaveValue("vendor/big-model");
    fireEvent.click(screen.getByRole("button", { name: /All connections/ }));
    expect(screen.getByRole("region", { name: "Providers overview" })).toBeInTheDocument();
  });

  it("shows the save bar only while there are changes, and Discard puts them back", async () => {
    const { onSave } = renderProviders(undefined, "p1");
    expect(screen.queryByRole("button", { name: "Save connection" })).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Name", { selector: "input" }), { target: { value: "Renamed" } });
    expect(screen.getByText(/Unsaved changes/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Save & fetch models/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    expect(screen.getByLabelText("Name", { selector: "input" })).toHaveValue("Gateway");
    await waitFor(() => expect(screen.queryByRole("button", { name: "Save connection" })).not.toBeInTheDocument());

    fireEvent.change(screen.getByLabelText("Name", { selector: "input" }), { target: { value: "Renamed" } });
    fireEvent.click(screen.getByRole("button", { name: "Save connection" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ id: "p1", name: "Renamed" })));
  });

  it("lets one model speak another API than its connection, and back again", async () => {
    const { onSave } = renderProviders(undefined, "p1");
    fireEvent.click(screen.getByRole("button", { name: /^Big Model/ }));
    const format = screen.getByRole("button", { name: "API format for Big Model" });
    expect(format).toHaveTextContent("Same as connection");

    fireEvent.click(format);
    fireEvent.click(screen.getByRole("menuitem", { name: "Responses" }));
    expect(screen.getByRole("button", { name: /^Big Model/ })).toHaveTextContent("Responses");
    fireEvent.click(screen.getByRole("button", { name: "Save connection" }));
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    const [big, raw] = onSave.mock.calls[0][0].models as ModelRecord[];
    expect(big.apiFormat).toBe("openai-responses");
    expect(raw.apiFormat).toBeUndefined();

    fireEvent.click(screen.getByRole("button", { name: "API format for Big Model" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Messages" }));
    fireEvent.click(screen.getByRole("button", { name: "Save connection" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(2));
    expect((onSave.mock.calls[1][0].models as ModelRecord[])[0].apiFormat).toBe("anthropic-messages");

    fireEvent.click(screen.getByRole("button", { name: "API format for Big Model" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /^Same as connection/ }));
    expect(screen.getByRole("button", { name: /^Big Model/ })).not.toHaveTextContent("Responses");
  });

  it("offers the Messages API for endpoints like OpenCode Go's, and hints at its base URL", async () => {
    const { onSave } = renderProviders(undefined, "p1");
    expect(screen.queryByText("/v1/messages is added for you")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "API format" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Messages compatible" }));
    expect(screen.getByText("/v1/messages is added for you; a pasted /v1 is trimmed")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("https://api.anthropic.com")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Save connection" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ apiFormat: "anthropic-messages" })));
  });

  it("lets you pick which discovered models to add, leaving out the ones already there", async () => {
    const { onDiscover, onSave } = renderProviders(undefined, "p1", ["vendor/big-model", "vendor/new-a", "vendor/new-b"]);
    fireEvent.click(screen.getByRole("button", { name: "Fetch models" }));
    await waitFor(() => expect(onDiscover).toHaveBeenCalledWith("p1"));
    expect(await screen.findByText(/The provider lists 3 models, 2 new/)).toBeInTheDocument();
    // A short list starts ticked.
    expect(screen.getByRole("checkbox", { name: "vendor/new-a" })).toBeChecked();
    fireEvent.click(screen.getByRole("checkbox", { name: "vendor/new-b" }));
    fireEvent.click(screen.getByRole("button", { name: "Add 1 model" }));
    expect(screen.getByRole("button", { name: /^vendor\/new-a/ })).toHaveTextContent("Needs limits");
    expect(screen.queryByRole("button", { name: /^vendor\/new-b/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Save connection" }));
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave.mock.calls[0][0].models.map((model: ModelRecord) => model.id)).toEqual(["vendor/big-model", "vendor/raw", "vendor/new-a"]);
  });

  it("creates a connection from the overview, and says when fetching will save it first", async () => {
    const { onSave } = renderProviders([]);
    expect(screen.getByRole("heading", { name: "Connect your first model" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^API key/ }));
    expect(screen.getByRole("radio", { name: /API key/ })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByText("Not saved yet")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Create & fetch models/ })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Name", { selector: "input" }), { target: { value: "Local" } });
    fireEvent.change(screen.getByLabelText("Base URL", { selector: "input" }), { target: { value: "http://127.0.0.1:8080/v1" } });
    fireEvent.click(screen.getByRole("button", { name: "Create connection" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ name: "Local", baseUrl: "http://127.0.0.1:8080/v1" })));
    expect(onSave.mock.calls[0][0].id).toBeUndefined();
  });

  it("shows a subscription's sign-in and models on its own page", () => {
    renderProviders(undefined, "anthropic");
    expect(screen.getByRole("region", { name: "Anthropic connection" })).toHaveTextContent("Off");
    expect(screen.getByText("Uses your Claude plan.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sign in" })).toBeInTheDocument();
    expect(screen.getByText(/Sign in to load the models/)).toBeInTheDocument();
  });
});
