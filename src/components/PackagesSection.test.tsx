import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { PackageRecord } from "../types";
import { PackagesSection } from "./PackagesSection";

afterEach(cleanup);

const installed: PackageRecord = {
  source: "npm:pi-web-access",
  displayName: "pi-web-access",
  kind: "npm",
  version: "0.30.0",
  installedPath: "/pkgs/pi-web-access",
  extensions: [
    { path: "/pkgs/pi-web-access/index.ts", name: "index.ts", enabled: true },
    { path: "/pkgs/pi-web-access/legacy.ts", name: "legacy.ts", enabled: false }
  ],
  skills: [{ path: "/pkgs/pi-web-access/skills/search.md", name: "skills/search.md", enabled: true }],
  prompts: [],
  themes: [],
  errors: [],
  trustedAt: "2026-01-01T00:00:00Z",
  installedAt: "2026-01-01T00:00:00Z"
};

function renderSection(packages: PackageRecord[], overrides: Partial<Parameters<typeof PackagesSection>[0]> = {}) {
  const actions = {
    onRefresh: vi.fn().mockResolvedValue(undefined),
    onInstall: vi.fn().mockResolvedValue(undefined),
    onTrust: vi.fn().mockResolvedValue(undefined),
    onSearch: vi.fn().mockResolvedValue([]),
    onRemove: vi.fn().mockResolvedValue(undefined),
    onUpdate: vi.fn().mockResolvedValue(undefined),
    onSetResources: vi.fn().mockResolvedValue(undefined),
    ...overrides
  };
  render(<PackagesSection packages={packages} {...actions} />);
  return actions;
}

describe("PackagesSection", () => {
  it("will not install until the trust warning is acknowledged", async () => {
    const { onInstall } = renderSection([]);
    fireEvent.change(screen.getByLabelText("Package source"), { target: { value: "npm:pi-subagents" } });
    fireEvent.click(screen.getByRole("button", { name: /Add package/ }));

    const confirm = screen.getByRole("button", { name: "Trust and install" });
    expect(confirm).toBeDisabled();
    expect(screen.getByText("npm:pi-subagents")).toBeInTheDocument();
    expect(screen.getByText(/read WackCode’s API keys and subscription credentials/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("checkbox"));
    expect(confirm).toBeEnabled();
    fireEvent.click(confirm);
    await waitFor(() => expect(onInstall).toHaveBeenCalledWith("npm:pi-subagents"));
  });

  it("keeps the dialog open and shows why when an install fails", async () => {
    renderSection([], { onInstall: vi.fn().mockRejectedValue(new Error("the npm registry returned HTTP 404")) });
    fireEvent.change(screen.getByLabelText("Package source"), { target: { value: "npm:nope" } });
    fireEvent.click(screen.getByRole("button", { name: /Add package/ }));
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Trust and install" }));

    expect(await screen.findByText(/HTTP 404/)).toBeInTheDocument();
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
  });

  it("sends the full enabled set for a kind when one resource is toggled", async () => {
    const { onSetResources } = renderSection([installed]);
    fireEvent.click(screen.getByRole("button", { name: "pi-web-access" }));

    // legacy.ts is currently off; switching it on must not drop index.ts.
    fireEvent.click(screen.getByRole("switch", { name: "Extensions: legacy.ts" }));
    await waitFor(() => expect(onSetResources).toHaveBeenCalledWith("npm:pi-web-access", "extensions", ["index.ts", "legacy.ts"]));
  });

  it("toggling one kind leaves the others untouched", async () => {
    const { onSetResources } = renderSection([installed]);
    fireEvent.click(screen.getByRole("button", { name: "pi-web-access" }));
    fireEvent.click(screen.getByRole("switch", { name: "Skills: skills/search.md" }));
    await waitFor(() => expect(onSetResources).toHaveBeenCalledWith("npm:pi-web-access", "skills", []));
  });

  it("shows a package's load errors without hiding the package", () => {
    renderSection([{ ...installed, errors: ["Extension failed to load: SyntaxError"] }]);
    expect(screen.getByText("Extension failed to load: SyntaxError")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "pi-web-access" })).toBeInTheDocument();
  });

  it("marks a package that was never confirmed here as not enabled, and routes it through the trust dialog", async () => {
    const { onTrust, onInstall } = renderSection([{ ...installed, trustedAt: "" }]);
    expect(screen.getByText(/Not enabled/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Review and enable" }));
    expect(screen.getByRole("button", { name: "Trust and enable" })).toBeDisabled();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Trust and enable" }));

    await waitFor(() => expect(onTrust).toHaveBeenCalledWith("npm:pi-web-access"));
    // Enabling must not re-download anything.
    expect(onInstall).not.toHaveBeenCalled();
  });

  it("does not offer Review and enable for a package already trusted", () => {
    renderSection([installed]);
    expect(screen.queryByRole("button", { name: "Review and enable" })).toBeNull();
    expect(screen.queryByText(/Not enabled/)).toBeNull();
  });

  it("confirms before removing", async () => {
    const { onRemove } = renderSection([installed]);
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(screen.getByText(/Remove pi-web-access\?/)).toBeInTheDocument();
    // Both the card and the dialog have a Remove button; confirm inside the dialog.
    fireEvent.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(onRemove).toHaveBeenCalledWith("npm:pi-web-access"));
  });
});

describe("PackagesSection built-ins", () => {
  it("requires model setup before the automatic titles switch can be used", async () => {
    const onConfigureAutoTitles = vi.fn();
    const onToggleAutoTitles = vi.fn().mockResolvedValue(undefined);
    renderSection([], { onConfigureAutoTitles, onToggleAutoTitles });
    expect(screen.getByRole("switch", { name: "Sub-agents" })).toHaveAttribute("aria-checked", "false");
    const card = screen.getByText("Auto chat titles").closest("article")!;
    expect(within(card).getByText("Off")).toBeInTheDocument();
    expect(within(card).getByRole("switch", { name: "Auto chat titles" })).toBeDisabled();
    fireEvent.click(within(card).getByRole("button", { name: /Set up/ }));
    expect(onConfigureAutoTitles).toHaveBeenCalledOnce();

    cleanup();
    renderSection([], { autoTitlesConfigured: true, onToggleAutoTitles });
    fireEvent.click(screen.getByRole("switch", { name: "Auto chat titles" }));
    await waitFor(() => expect(onToggleAutoTitles).toHaveBeenCalledWith(true));
  });

  it("lists the compiled-in extensions with pinned-on, disabled toggles even with no packages", () => {
    renderSection([]);
    expect(screen.getByRole("heading", { name: "Built-In" })).toBeInTheDocument();
    for (const name of ["Plan Mode", "Ask User Questions", "Todo List", "plan_mode_complete", "ask_user_question", "todo"]) {
      const toggle = screen.getByRole("switch", { name });
      expect(toggle).toBeDisabled();
      expect(toggle).toHaveAttribute("aria-checked", "true");
    }
  });

  it("lets sub-agents be switched on, and links to its settings only while on", async () => {
    const onToggleSubagents = vi.fn().mockResolvedValue(undefined);
    const onConfigureSubagents = vi.fn();
    renderSection([], { onToggleSubagents, onConfigureSubagents });
    const toggle = screen.getByRole("switch", { name: "Sub-agents" });
    expect(toggle).toBeEnabled();
    expect(toggle).toHaveAttribute("aria-checked", "false");
    expect(screen.getByRole("switch", { name: "subagent" })).toHaveAttribute("aria-checked", "false");
    expect(screen.getByText(/every sub-agent is extra model usage/)).toBeInTheDocument();
    const subagentCard = screen.getByRole("switch", { name: "Sub-agents" }).closest("article")!;
    expect(within(subagentCard).queryByRole("button", { name: /Configure/ })).not.toBeInTheDocument();
    fireEvent.click(toggle);
    await waitFor(() => expect(onToggleSubagents).toHaveBeenCalledWith(true));

    cleanup();
    renderSection([], { subagentsEnabled: true, onToggleSubagents, onConfigureSubagents });
    expect(screen.getByRole("switch", { name: "Sub-agents" })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(within(screen.getByRole("switch", { name: "Sub-agents" }).closest("article")!).getByRole("button", { name: /Configure/ }));
    expect(onConfigureSubagents).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("switch", { name: "Sub-agents" }));
    await waitFor(() => expect(onToggleSubagents).toHaveBeenLastCalledWith(false));
  });

  it("shows Web Fetch on by default and lets it be switched off", async () => {
    const onToggleWebFetch = vi.fn().mockResolvedValue(undefined);
    renderSection([], { onToggleWebFetch });
    const card = screen.getByText("Web Fetch").closest("article")!;
    const toggle = within(card).getByRole("switch", { name: "Web Fetch" });
    expect(toggle).toBeEnabled();
    expect(toggle).toHaveAttribute("aria-checked", "true");
    expect(within(card).getByText("On")).toBeInTheDocument();
    expect(within(card).getByRole("switch", { name: "web_fetch" })).toHaveAttribute("aria-checked", "true");
    expect(within(card).queryByRole("button", { name: /Configure/ })).not.toBeInTheDocument();
    fireEvent.click(toggle);
    await waitFor(() => expect(onToggleWebFetch).toHaveBeenCalledWith(false));

    cleanup();
    renderSection([], { webFetchEnabled: false, onToggleWebFetch });
    expect(screen.getByRole("switch", { name: "Web Fetch" })).toHaveAttribute("aria-checked", "false");
    fireEvent.click(screen.getByRole("switch", { name: "Web Fetch" }));
    await waitFor(() => expect(onToggleWebFetch).toHaveBeenLastCalledWith(true));
  });

  it("still shows them alongside installed packages", () => {
    renderSection([installed]);
    expect(screen.getByRole("button", { name: "pi-web-access" })).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Plan Mode" })).toBeInTheDocument();
  });
});

describe("PackagesSection browse tab", () => {
  const hit = {
    name: "pi-web-access",
    version: "0.30.0",
    description: "Web search, URL fetching, GitHub cloning",
    publisher: "nicopreme",
    npmUrl: "https://www.npmjs.com/package/pi-web-access",
    publishedAt: "2026-09-19T14:25:30.716Z",
    declares: ["extensions"]
  };

  it("searches on open and routes Install through the trust dialog", async () => {
    const onSearch = vi.fn().mockResolvedValue([hit]);
    const { onInstall } = renderSection([], { onSearch });
    fireEvent.click(screen.getByRole("tab", { name: "Browse" }));

    expect(await screen.findByText("pi-web-access")).toBeInTheDocument();
    expect(onSearch).toHaveBeenCalledWith("");

    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    // Browsing must not bypass the trust gate.
    expect(onInstall).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Trust and install" }));
    await waitFor(() => expect(onInstall).toHaveBeenCalledWith("npm:pi-web-access"));
  });

  it("marks an already-installed package instead of offering it again", async () => {
    renderSection([installed], { onSearch: vi.fn().mockResolvedValue([hit]) });
    fireEvent.click(screen.getByRole("tab", { name: /Browse/ }));
    expect(await screen.findByText("Installed")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();
  });

  it("reports a registry failure without emptying the tab", async () => {
    renderSection([], { onSearch: vi.fn().mockRejectedValue(new Error("Could not reach the npm registry")) });
    fireEvent.click(screen.getByRole("tab", { name: "Browse" }));
    expect(await screen.findByText(/Could not reach the npm registry/)).toBeInTheDocument();
    expect(screen.getByLabelText("Search packages")).toBeInTheDocument();
  });

  it("renders search results in a grid container with metadata and actions", async () => {
    const hits = [
      hit,
      {
        name: "pi-subagents",
        version: "0.12.0",
        description: "Orchestrate subagent tasks",
        publisher: "jsmith",
        npmUrl: "https://www.npmjs.com/package/pi-subagents",
        publishedAt: "2026-09-18T10:00:00.000Z",
        declares: ["skills"]
      }
    ];
    renderSection([], { onSearch: vi.fn().mockResolvedValue(hits) });
    fireEvent.click(screen.getByRole("tab", { name: "Browse" }));

    expect(await screen.findByText("pi-web-access")).toBeInTheDocument();
    expect(screen.getByText("pi-subagents")).toBeInTheDocument();

    const grid = document.querySelector(".search-results");
    expect(grid).toBeInTheDocument();
    expect(grid?.querySelectorAll(".search-result")).toHaveLength(2);
  });
});
