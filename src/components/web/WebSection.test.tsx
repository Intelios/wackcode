import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CustomProviderRecord, SearchProviderRecord, WebConfig } from "../../types";
import { WebSection } from "./WebSection";

afterEach(cleanup);

const deepseek: SearchProviderRecord = {
  id: "s1", name: "DeepSeek", kind: "deepseek", keySource: { type: "connection", providerId: "ds" }, hasKey: true,
  priceSearchPer1k: 0.75, priceFetchPer1k: 0.75, createdAt: "", updatedAt: "",
};
const brave: SearchProviderRecord = { ...deepseek, id: "s2", name: "Brave Search", kind: "brave", keySource: { type: "own" }, priceSearchPer1k: null, priceFetchPer1k: null };
const dsConnection: CustomProviderRecord = {
  id: "ds", name: "DeepSeek chat", kind: "custom", baseUrl: "https://api.deepseek.com/v1", apiFormat: "openai-completions", models: [],
  createdAt: "", updatedAt: "", hasApiKey: true, connected: true, enabled: true,
};
const OFF: WebConfig = { enabled: false, providers: [], activeProviderId: null, routeFetch: false };

function renderSection(config: WebConfig, overrides: Partial<Parameters<typeof WebSection>[0]> = {}) {
  const props = {
    config,
    providers: [dsConnection],
    agentName: "Quill",
    webFetchEnabled: true,
    onToggleWebFetch: vi.fn().mockResolvedValue(undefined),
    onChanged: vi.fn(),
    onSave: vi.fn(),
    onDelete: vi.fn(),
    onSetActive: vi.fn(),
    onSetConfig: vi.fn(),
    onTest: vi.fn(),
    onUsage: vi.fn().mockResolvedValue({ month: "2026-10", providers: [{ id: "s1", searches: 142, fetches: 3, billedSearches: 140, billedFetches: 3, failed: 2 }] }),
    onResetUsage: vi.fn().mockResolvedValue({ month: "2026-10", providers: [] }),
    onOpenLink: vi.fn(),
    ...overrides,
  };
  render(<WebSection {...props} />);
  return props;
}

describe("Settings › Web", () => {
  it("explains itself and lists the presets while there are no providers", () => {
    renderSection(OFF);
    expect(screen.getByRole("heading", { name: "Let Quill search the web" })).toBeInTheDocument();
    expect(screen.getByText("No providers")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Web search" })).toBeDisabled();
    for (const name of ["DeepSeek", "Brave Search", "Tavily", "SearXNG", "Custom"]) {
      expect(screen.getByRole("button", { name: new RegExp(`^${name}`) })).toBeInTheDocument();
    }
    expect(screen.getByRole("switch", { name: "Web Fetch" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("switch", { name: /Read pages through/ })).toBeDisabled();
  });

  it("shows the active provider, its usage, and swaps another in with one click", async () => {
    const next: WebConfig = { enabled: true, providers: [deepseek, brave], activeProviderId: "s2", routeFetch: false };
    const props = renderSection({ enabled: true, providers: [deepseek, brave], activeProviderId: "s1", routeFetch: false }, { onSetActive: vi.fn().mockResolvedValue(next) });
    expect(await screen.findByText("DeepSeek · 142 searches")).toBeInTheDocument();
    expect(screen.getByText("Uses ‘DeepSeek chat’ key")).toBeInTheDocument();
    expect(screen.getByText("≈ $0.11")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Make Brave Search the active provider" }));
    await waitFor(() => expect(props.onSetActive).toHaveBeenCalledWith("s2"));
    expect(props.onChanged).toHaveBeenCalledWith(next);
  });

  it("routes pages only through a provider that can read them", async () => {
    const config: WebConfig = { enabled: true, providers: [deepseek, brave], activeProviderId: "s1", routeFetch: false };
    const props = renderSection(config, { onSetConfig: vi.fn().mockResolvedValue({ ...config, routeFetch: true }) });
    fireEvent.click(screen.getByRole("switch", { name: "Read pages through DeepSeek" }));
    await waitFor(() => expect(props.onSetConfig).toHaveBeenCalledWith({ enabled: true, routeFetch: true }));
    cleanup();
    renderSection({ ...config, activeProviderId: "s2" });
    expect(screen.getByRole("switch", { name: /Read pages through/ })).toBeDisabled();
    expect(screen.getByText(/Brave Search can't read pages/)).toBeInTheDocument();
  });

  it("switches Web Fetch and asks before resetting the counts", async () => {
    const props = renderSection({ enabled: true, providers: [deepseek], activeProviderId: "s1", routeFetch: false });
    fireEvent.click(screen.getByRole("switch", { name: "Web Fetch" }));
    await waitFor(() => expect(props.onToggleWebFetch).toHaveBeenCalledWith(false));
    fireEvent.click(await screen.findByRole("button", { name: "Reset counts" }));
    const dialog = screen.getByRole("alertdialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Reset" }));
    await waitFor(() => expect(props.onResetUsage).toHaveBeenCalled());
  });

  it("adds DeepSeek on a connection's key, tests the draft, and shows the save bar only while unsaved", async () => {
    const saved: WebConfig = { enabled: false, providers: [{ ...deepseek, id: "new-id" }], activeProviderId: "new-id", routeFetch: false };
    const props = renderSection(OFF, {
      onSave: vi.fn().mockResolvedValue(saved),
      onTest: vi.fn().mockResolvedValue({ ok: true, results: [{ title: "Rust 1.95", url: "https://blog.rust-lang.org/", snippet: "Released", summary: "" }], elapsedMs: 420 }),
    });
    fireEvent.click(screen.getByRole("button", { name: /^DeepSeek/ }));
    expect(screen.getByRole("radio", { name: "From a connection" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getAllByText("Not saved yet").length).toBeGreaterThan(0);

    fireEvent.click(screen.getByRole("button", { name: "Test search" }));
    await waitFor(() => expect(props.onTest).toHaveBeenCalledWith(expect.objectContaining({
      query: "latest Rust release", draft: expect.objectContaining({ kind: "deepseek", keySource: { type: "connection", providerId: "ds" } }),
    })));
    const result = await screen.findByRole("link", { name: "Rust 1.95" });
    fireEvent.click(result);
    expect(props.onOpenLink).toHaveBeenCalledWith("https://blog.rust-lang.org/");

    fireEvent.click(screen.getByRole("button", { name: "Add provider" }));
    await waitFor(() => expect(props.onSave).toHaveBeenCalledWith(expect.objectContaining({ name: "DeepSeek", kind: "deepseek" })));
    expect(props.onChanged).toHaveBeenCalledWith(saved);
  });

  it("asks for its own key when switched off the connection, and reports a failed test", async () => {
    const props = renderSection(OFF, { onTest: vi.fn().mockResolvedValue({ ok: false, error: "DeepSeek rejected the API key.", results: [], elapsedMs: 80 }) });
    fireEvent.click(screen.getByRole("button", { name: /^DeepSeek/ }));
    fireEvent.click(screen.getByRole("radio", { name: "Its own key" }));
    expect(screen.getByRole("button", { name: "Add provider" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("API key"), { target: { value: "sk-test" } });
    fireEvent.click(screen.getByRole("button", { name: "Test search" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("DeepSeek rejected the API key.");
    expect(props.onTest).toHaveBeenCalledWith(expect.objectContaining({ draft: expect.objectContaining({ apiKey: "sk-test", keySource: { type: "own" } }) }));
  });

  it("deletes a provider only after a danger confirmation", async () => {
    const props = renderSection({ enabled: true, providers: [brave], activeProviderId: "s2", routeFetch: false }, { onDelete: vi.fn().mockResolvedValue(OFF) });
    fireEvent.click(within(screen.getByRole("list", { name: "Search providers" })).getByRole("button", { name: /^Brave Search/ }));
    expect(screen.queryByText("Unsaved changes")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Delete provider" }));
    const dialog = screen.getByRole("alertdialog");
    expect(dialog).toHaveTextContent("web search stops");
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(props.onDelete).toHaveBeenCalledWith("s2"));
    expect(props.onChanged).toHaveBeenCalledWith(OFF);
  });
});
