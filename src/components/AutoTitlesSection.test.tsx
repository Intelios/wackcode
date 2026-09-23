import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AutoTitleConfig, ProviderRecord } from "../types";
import { AutoTitlesSection } from "./AutoTitlesSection";

afterEach(cleanup);

const off: AutoTitleConfig = { enabled: false, providerId: null, modelId: null };
const providers: ProviderRecord[] = [{
  id: "p", name: "Test connection", kind: "custom", connected: true, hasApiKey: true,
  baseUrl: "https://example.test/v1", apiFormat: "openai-completions", createdAt: "now", updatedAt: "now",
  models: [{ id: "small", name: "Small model", contextWindow: 16_000, maxTokens: 2_000,
    reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: {}, vision: false }]
}];

it("requires an explicit model before enabling and saves the choice first", async () => {
  const onChange = vi.fn().mockResolvedValue(undefined);
  const { rerender } = render(<AutoTitlesSection config={off} providers={providers} onChange={onChange} onOpenProviders={vi.fn()} />);
  expect(screen.getByRole("switch", { name: "Automatic titles" })).toBeDisabled();
  expect(screen.getByText(/Choose a connected model/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Choose model" }));
  fireEvent.click(screen.getByRole("button", { name: /Test connection/ }));
  fireEvent.click(screen.getByRole("button", { name: "Small model" }));
  await waitFor(() => expect(onChange).toHaveBeenCalledWith({ enabled: false, providerId: "p", modelId: "small" }));

  rerender(<AutoTitlesSection config={{ ...off, providerId: "p", modelId: "small" }} providers={providers} onChange={onChange} onOpenProviders={vi.fn()} />);
  fireEvent.click(screen.getByRole("switch", { name: "Automatic titles" }));
  await waitFor(() => expect(onChange).toHaveBeenLastCalledWith({ enabled: true, providerId: "p", modelId: "small" }));
});

it("offers connection setup when no model is available", () => {
  const onOpenProviders = vi.fn();
  render(<AutoTitlesSection config={off} providers={[]} onChange={vi.fn()} onOpenProviders={onOpenProviders} />);
  expect(screen.getByRole("switch", { name: "Automatic titles" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Add a connection" }));
  expect(onOpenProviders).toHaveBeenCalledOnce();
});

it("keeps the existing setting when a save fails and shows the error", async () => {
  const onChange = vi.fn().mockRejectedValue(new Error("Could not save"));
  render(<AutoTitlesSection config={{ ...off, providerId: "p", modelId: "small" }} providers={providers} onChange={onChange} onOpenProviders={vi.fn()} />);
  fireEvent.click(screen.getByRole("switch", { name: "Automatic titles" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not save");
  expect(screen.getByRole("switch", { name: "Automatic titles" })).toHaveAttribute("aria-checked", "false");
});
