import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ModelCatalogEntry, ModelCatalogProfile } from "../types";
import { ModelCatalogSearch } from "./ModelCatalogSearch";

afterEach(cleanup);
const profile: ModelCatalogProfile = { key: "lab:model", id: "model", provider: "lab", providerName: "Lab",
  baseUrl: "https://example.test/v1", apiFormats: ["openai-completions"], contextWindow: 128000, maxTokens: 16000,
  reasoning: true, thinkingLevels: null, thinkingLevelMap: {}, vision: true };
const gateway: ModelCatalogProfile = { ...profile, key: "gateway:model", provider: "gateway", providerName: "Gateway",
  id: "vendor/model", baseUrl: "https://example.test/plan/v1", maxTokens: 8000 };
const entry: ModelCatalogEntry = { id: "lab/model", name: "Model Four", originalProvider: "lab", profiles: [gateway, profile] };

describe("model specification preview", () => {
  it("preselects a matching host profile and applies only on the explicit button", () => {
    const onSelect = vi.fn();
    render(<ModelCatalogSearch modelLabel="new model" catalog={[entry]} baseUrl={gateway.baseUrl!} apiFormat="openai-completions" loading={false} onSelect={onSelect} />);
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "model" } });
    fireEvent.click(screen.getByRole("option"));
    const preview = screen.getByRole("region", { name: "Specifications for Model Four" });
    expect(preview).toHaveTextContent("Source matched to your connection.");
    expect(preview).toHaveTextContent("8K tokens");
    expect(preview).toHaveTextContent("Set efforts manually");
    expect(onSelect).not.toHaveBeenCalled();
    fireEvent.click(within(preview).getByRole("button", { name: "Use these specifications" }));
    expect(onSelect).toHaveBeenCalledWith(entry, gateway);
  });

  it("defaults to the original provider on an unknown gateway and lets you choose another source", () => {
    const onSelect = vi.fn();
    render(<ModelCatalogSearch modelLabel="new model" modelId="keep-this" catalog={[entry]} baseUrl="https://unknown.test/v1" apiFormat="openai-completions" loading={false} onSelect={onSelect} />);
    const search = screen.getByRole("combobox");
    fireEvent.change(search, { target: { value: "model" } });
    fireEvent.keyDown(search, { key: "ArrowDown" });
    expect(search).toHaveAttribute("aria-activedescendant");
    fireEvent.keyDown(search, { key: "Enter" });
    expect(screen.getByRole("button", { name: "Metadata source for Model Four" })).toHaveTextContent("Lab");
    expect(screen.getByText(/keep-this.*will be kept/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Metadata source for Model Four" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /Gateway/ }));
    fireEvent.click(screen.getByRole("button", { name: "Use these specifications" }));
    expect(onSelect).toHaveBeenCalledWith(entry, gateway);
  });

  it("keeps a working catalog searchable during a failed refresh and explains no match", () => {
    render(<ModelCatalogSearch modelLabel="new model" catalog={[entry]} baseUrl="" apiFormat="openai-completions" loading={false} error="Offline" onSelect={vi.fn()} />);
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "vendor/model" } });
    expect(screen.getByRole("option")).toHaveTextContent("Model Four");
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "unlisted" } });
    expect(screen.getByText(/No model catalog matches/)).toBeInTheDocument();
  });
});
