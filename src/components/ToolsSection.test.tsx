import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { PackageRecord, ToolCatalogEntry } from "../types";
import { ToolsSection } from "./ToolsSection";

afterEach(cleanup);

const pi = (name: string, description: string): ToolCatalogEntry => ({ name, description, source: { kind: "builtin" }, available: true });

const catalog: ToolCatalogEntry[] = [
  pi("read", "Read the contents of a file. Output is truncated to 2000 lines."),
  pi("edit", "Edit a single file using exact text replacement."),
  pi("write", "Write content to a file."),
  pi("bash", "Execute a bash command in the current working directory."),
  pi("lsp", "Query the language server. Returns symbols."),
  { name: "web_search", description: "Search the web. Returns ten results as JSON.", source: { kind: "package", packageId: "npm:pi-web-access" }, available: true },
  { name: "todo", description: "Manage a task list.", source: { kind: "wackcode" }, available: true }
];

const webAccess = { source: "npm:pi-web-access", displayName: "Web Access" } as PackageRecord;

function renderTools(props: Partial<React.ComponentProps<typeof ToolsSection>> = {}) {
  const onSetDisabled = vi.fn().mockResolvedValue(undefined);
  const onOpen = vi.fn();
  render(
    <ToolsSection catalog={catalog} disabled={[]} packages={[webAccess]} agentName="Quack" onSetDisabled={onSetDisabled} onOpen={onOpen} mcpAvailable {...props} />
  );
  return { onSetDisabled, onOpen };
}

describe("Settings › Tools", () => {
  it("describes Pi's tools for people and keeps the model's text behind a disclosure", () => {
    renderTools();
    expect(screen.getByText("Opens a file to look at it, images included. Long files are read a page at a time.")).toBeInTheDocument();
    expect(screen.queryByText(/Output is truncated to 2000 lines/)).not.toBeInTheDocument();
    const read = screen.getByText("Read files").closest("li")!;
    fireEvent.click(within(read).getByRole("button", { name: /What the model is told/ }));
    expect(within(read).getByRole("button", { name: /What the model is told/ })).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText(/Output is truncated to 2000 lines/)).toBeInTheDocument();
  });

  it("names a package's group by its display name and shows each tool's opening sentence", () => {
    renderTools();
    const group = screen.getByRole("region", { name: "Web Access tools" });
    expect(within(group).getByText("npm:pi-web-access")).toBeInTheDocument();
    expect(within(group).getByText("Search the web.")).toBeInTheDocument();
    // A Pi tool with no copy of ours still shows, under its own name.
    expect(within(screen.getByRole("region", { name: "More from Pi" })).getByText("Query the language server.")).toBeInTheDocument();
    // WackCode's own tools are set up elsewhere.
    expect(screen.queryByText("todo")).not.toBeInTheDocument();
  });

  it("counts what is on in the hero, and says when the agent can no longer change anything", () => {
    renderTools({ disabled: ["edit", "write"] });
    const hero = screen.getByRole("region", { name: "Tools overview" });
    expect(within(hero).getByText("4 of 6 on")).toBeInTheDocument();
    expect(within(hero).getByRole("heading", { name: "The tools Quack works with" })).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    cleanup();
    renderTools({ disabled: ["edit", "write", "bash"] });
    expect(screen.getByRole("status")).toHaveTextContent("Quack can look but not touch");
  });

  it("explains an empty catalogue instead of showing an empty list", () => {
    renderTools({ catalog: [] });
    expect(within(screen.getByRole("region", { name: "Tools overview" })).getByText("Not loaded yet")).toBeInTheDocument();
    expect(screen.getByText("Tools appear after your first chat")).toBeInTheDocument();
  });

  it("links to where the app's own tools are set up, leaving out MCP until it is wired", () => {
    const { onOpen } = renderTools();
    fireEvent.click(screen.getByRole("button", { name: /^MCP servers/ }));
    expect(onOpen).toHaveBeenCalledWith("mcp");
    cleanup();
    renderTools({ mcpAvailable: false });
    expect(screen.queryByRole("button", { name: /^MCP servers/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Memory/ })).toBeInTheDocument();
  });
});
