import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { McpServerRecord, McpTestResult } from "../types";
import { McpSection } from "./McpSection";

afterEach(cleanup);

const github: McpServerRecord = {
  id: "mcp-1", name: "GitHub", enabled: true, transport: "stdio", timeoutMs: 120_000,
  command: "npx", args: ["-y", "@example/github-mcp"], url: "", headers: [], env: ["GITHUB_TOKEN"],
  disabledTools: ["delete_repo"],
  tools: [
    { name: "search_issues", description: "Search issues.", readOnly: true },
    { name: "delete_repo", description: "Delete a repository.", readOnly: false }
  ]
};
const remote: McpServerRecord = {
  id: "mcp-2", name: "Remote", enabled: false, transport: "http", timeoutMs: 30_000,
  command: "", args: [], url: "https://mcp.example.com/mcp", headers: ["Authorization"], env: [], disabledTools: [], tools: []
};

function renderSection(servers: McpServerRecord[] = [github, remote], testResult?: McpTestResult) {
  const actions = {
    onSaveMcpServer: vi.fn().mockImplementation(async (input) => ({ ...remote, ...input, id: input.id ?? "mcp-new", headers: [], env: [] })),
    onDeleteMcpServer: vi.fn().mockResolvedValue(undefined),
    onSetMcpServerEnabled: vi.fn().mockResolvedValue(undefined),
    onSetMcpServerTools: vi.fn().mockResolvedValue(undefined),
    onTestMcpServer: vi.fn().mockResolvedValue(testResult ?? { ok: true, server: { ...github, tools: github.tools } })
  };
  render(<McpSection servers={servers} {...actions} />);
  return actions;
}

describe("McpSection", () => {
  it("adds a stdio server with one argument per line and a secret variable, then tests it", async () => {
    const actions = renderSection([]);
    expect(screen.getByRole("heading", { name: "No servers yet" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /New server/ }));
    // The empty state gives way to the editor.
    expect(screen.queryByRole("heading", { name: "No servers yet" })).not.toBeInTheDocument();
    const add = screen.getByRole("button", { name: "Add server" });
    expect(add).toBeDisabled();

    fireEvent.change(screen.getByPlaceholderText("GitHub"), { target: { value: " Files " } });
    fireEvent.change(screen.getByPlaceholderText("npx"), { target: { value: "npx" } });
    fireEvent.change(screen.getByRole("textbox", { name: /Arguments/ }), { target: { value: "-y\n\n@modelcontextprotocol/server-filesystem \n/tmp" } });
    fireEvent.click(screen.getByRole("button", { name: /Add variable/ }));
    fireEvent.change(screen.getByRole("textbox", { name: "variable name" }), { target: { value: "API_TOKEN" } });
    fireEvent.change(screen.getByLabelText("API_TOKEN value"), { target: { value: "s3cret" } });
    expect(screen.getByText(/runs on your Mac, with your permissions/)).toBeInTheDocument();
    fireEvent.click(add);

    await waitFor(() => expect(actions.onSaveMcpServer).toHaveBeenCalled());
    expect(actions.onSaveMcpServer.mock.calls[0][0]).toEqual({
      id: undefined, name: "Files", transport: "stdio", timeoutMs: 120_000,
      command: "npx", args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"], url: "",
      headers: [], env: [{ name: "API_TOKEN", value: "s3cret" }]
    });
    await waitFor(() => expect(actions.onTestMcpServer).toHaveBeenCalledWith("mcp-new"));
  });

  it("starts a server from the empty state with the transport its tile names", () => {
    renderSection([]);
    fireEvent.click(screen.getByRole("button", { name: /A server at a URL/ }));
    expect(screen.getByRole("radio", { name: "HTTP" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByPlaceholderText("https://example.com/mcp")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    fireEvent.click(screen.getByRole("button", { name: /A command on your Mac/ }));
    expect(screen.getByRole("radio", { name: "stdio" })).toHaveAttribute("aria-checked", "true");
  });

  it("says in the hero how many servers are switched on", () => {
    renderSection();
    const hero = screen.getByRole("region", { name: "MCP servers overview" });
    expect(within(hero).getByText("1 of 2 on")).toBeInTheDocument();
    expect(within(hero).getByRole("heading", { name: "Plug more tools into WackCode" })).toBeInTheDocument();
    cleanup();
    renderSection([]);
    expect(within(screen.getByRole("region", { name: "MCP servers overview" })).getByText("No servers")).toBeInTheDocument();
  });

  it("keeps a saved header when its value is left blank, and needs a valid timeout", async () => {
    const actions = renderSection();
    fireEvent.click(screen.getByRole("button", { name: /Remote/ }));
    fireEvent.click(screen.getByRole("button", { name: /Edit/ }));
    expect(screen.getByRole("radio", { name: "HTTP" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByLabelText("Authorization value")).toHaveAttribute("placeholder", "Saved — leave blank to keep");

    const timeout = screen.getByRole("spinbutton");
    fireEvent.change(timeout, { target: { value: "500" } });
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(screen.getByText(/Between 1,000 and 3,600,000 ms/)).toBeInTheDocument();
    fireEvent.change(timeout, { target: { value: "60000" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(actions.onSaveMcpServer).toHaveBeenCalled());
    expect(actions.onSaveMcpServer.mock.calls[0][0]).toMatchObject({
      id: "mcp-2", transport: "http", timeoutMs: 60_000, url: "https://mcp.example.com/mcp", command: "", args: [], env: [],
      headers: [{ name: "Authorization", value: undefined }]
    });
  });

  it("switches a server, and a single tool, at once", async () => {
    const actions = renderSection();
    fireEvent.click(screen.getByRole("switch", { name: "Use Remote" }));
    await waitFor(() => expect(actions.onSetMcpServerEnabled).toHaveBeenCalledWith("mcp-2", true));

    fireEvent.click(screen.getByRole("button", { name: /GitHub/ }));
    const tools = screen.getByRole("region", { name: "GitHub tools" });
    expect(within(tools).getByRole("switch", { name: "Use delete_repo" })).toHaveAttribute("aria-checked", "false");
    expect(within(tools).getByText("Read-only")).toBeInTheDocument();
    fireEvent.click(within(tools).getByRole("switch", { name: "Use search_issues" }));
    await waitFor(() => expect(actions.onSetMcpServerTools).toHaveBeenCalledWith("mcp-1", ["delete_repo", "search_issues"]));
  });

  it("shows what a test found, or why it failed", async () => {
    const actions = renderSection([github], { ok: false, error: "Command not found: npx.", server: github });
    fireEvent.click(screen.getByRole("button", { name: /GitHub/ }));
    fireEvent.click(screen.getByRole("button", { name: /Test connection/ }));
    expect(await screen.findByText("Command not found: npx.")).toBeInTheDocument();
    expect(actions.onTestMcpServer).toHaveBeenCalledWith("mcp-1");
  });

  it("shows a refused save and deletes through a confirmation", async () => {
    const actions = renderSection();
    actions.onSaveMcpServer.mockRejectedValueOnce("Give the server a name.");
    fireEvent.click(screen.getByRole("button", { name: /GitHub/ }));
    fireEvent.click(screen.getByRole("button", { name: /Edit/ }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Give the server a name.");

    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    const dialog = screen.getByRole("alertdialog", { name: "Delete GitHub?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(actions.onDeleteMcpServer).toHaveBeenCalledWith("mcp-1"));
  });
});
