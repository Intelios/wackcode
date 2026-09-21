import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ToolCatalogEntry } from "../types";
import { SettingsPage } from "./SettingsPage";

vi.mock("../api", () => ({ api: { revealPath: vi.fn().mockResolvedValue(undefined) } }));

afterEach(cleanup);

const catalog: ToolCatalogEntry[] = [
  { name: "read", description: "Read a file", source: { kind: "builtin" }, available: true },
  { name: "bash", description: "Run a command", source: { kind: "builtin" }, available: true },
  { name: "find", description: "Find files", source: { kind: "builtin" }, available: false, unavailableReason: "Requires fd, which is not installed" },
  { name: "web_search", description: "Search the web", source: { kind: "package", packageId: "npm:pi-web-access" }, available: true }
];

function renderTools(overrides: { disabled?: string[]; onSetDisabledTools?: (next: string[]) => Promise<void> } = {}) {
  const onSetDisabledTools = overrides.onSetDisabledTools ?? vi.fn().mockResolvedValue(undefined);
  render(
    <SettingsPage
      providers={[]}
      packages={[]}
      toolCatalog={catalog}
      disabledTools={overrides.disabled ?? []}
      appDataPath="/tmp/wackcode"
      onClose={vi.fn()}
      onSave={vi.fn()}
      onDelete={vi.fn()}
      onSetDisabledTools={onSetDisabledTools}
      onRefresh={vi.fn().mockResolvedValue(undefined)}
      onInstall={vi.fn()}
      onTrust={vi.fn()}
      onSearch={vi.fn().mockResolvedValue([])}
      onRemove={vi.fn()}
      onUpdate={vi.fn()}
      onSetResources={vi.fn()}
    />
  );
  fireEvent.click(screen.getByRole("button", { name: "Tools" }));
  return { onSetDisabledTools };
}

describe("SettingsPage tools section", () => {
  it("groups built-ins separately from package tools", () => {
    renderTools();
    expect(screen.getByRole("heading", { name: "Built-in" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "npm:pi-web-access" })).toBeInTheDocument();
  });

  it("switching a tool off sends it as a denylist entry", async () => {
    const { onSetDisabledTools } = renderTools();
    fireEvent.click(screen.getByRole("switch", { name: "bash" }));
    await waitFor(() => expect(onSetDisabledTools).toHaveBeenCalledWith(["bash"]));
  });

  it("switching an already-disabled tool back on removes it from the denylist", async () => {
    const { onSetDisabledTools } = renderTools({ disabled: ["bash"] });
    expect(screen.getByRole("switch", { name: "bash" })).toHaveAttribute("aria-checked", "false");
    fireEvent.click(screen.getByRole("switch", { name: "bash" }));
    await waitFor(() => expect(onSetDisabledTools).toHaveBeenCalledWith([]));
  });

  it("a tool whose binary is missing is off, locked, and explains why", () => {
    renderTools();
    const find = screen.getByRole("switch", { name: "find" });
    expect(find).toBeDisabled();
    expect(find).toHaveAttribute("aria-checked", "false");
    expect(screen.getByText("Requires fd, which is not installed")).toBeInTheDocument();
  });

  it("surfaces a failed save instead of silently reverting", async () => {
    const onSetDisabledTools = vi.fn().mockRejectedValue(new Error("Metadata lock was poisoned"));
    renderTools({ onSetDisabledTools });
    fireEvent.click(screen.getByRole("switch", { name: "read" }));
    expect(await screen.findByText(/Metadata lock was poisoned/)).toBeInTheDocument();
  });
});
