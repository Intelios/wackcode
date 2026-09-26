import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SlashCommandEntry, SlashCommandsOverview } from "../types";
import { CommandsSection, commandDraftIssue } from "./CommandsSection";

afterEach(cleanup);

function entry(name: string, patch: Partial<SlashCommandEntry> = {}): SlashCommandEntry {
  return {
    key: `custom:/app/data/commands/${name}.md`, name, description: `${name} description`,
    kind: "custom", enabled: true, editable: true, filePath: `/app/data/commands/${name}.md`, ...patch
  };
}

const overview: SlashCommandsOverview = {
  customPath: "/app/data/commands",
  disabled: [],
  groups: [
    { id: "custom", label: "Your commands", kind: "custom", entries: [entry("review"), entry("standup", { enabled: false })], diagnostics: [] },
    {
      id: "npm:pack", label: "pack", kind: "package",
      entries: [
        entry("weekly", { key: "prompt:/pkg/weekly.md", kind: "prompt", editable: false, filePath: "/pkg/weekly.md" }),
        entry("deploy", { key: "extension:/pkg/ext.ts#deploy", kind: "extension", editable: false, filePath: "/pkg/ext.ts" })
      ],
      diagnostics: []
    }
  ]
};

function renderSection(start: SlashCommandsOverview = overview) {
  const change = { overview: start, config: { disabled: [] } };
  const actions = {
    onList: vi.fn().mockResolvedValue(start),
    onRead: vi.fn().mockResolvedValue({ body: "Look at $ARGUMENTS closely." }),
    onSave: vi.fn().mockResolvedValue(change),
    onDelete: vi.fn().mockResolvedValue(change),
    onSetEnabled: vi.fn().mockResolvedValue(change),
    onReveal: vi.fn().mockResolvedValue(undefined),
    onChanged: vi.fn()
  };
  render(<CommandsSection {...actions} />);
  return actions;
}

describe("commandDraftIssue", () => {
  const draft = { name: "review", description: "Reviews.", argumentHint: "", body: "Do it." };

  it("follows the name rules, refuses taken names, and needs a body", () => {
    const taken = new Set(["review"]);
    expect(commandDraftIssue({ ...draft, name: "ship-it" }, taken)).toBeUndefined();
    expect(commandDraftIssue({ ...draft, name: "Review" }, taken)).toMatch(/lowercase/);
    expect(commandDraftIssue({ ...draft, name: "-lead" }, taken)).toMatch(/lowercase/);
    expect(commandDraftIssue({ ...draft, name: "review" }, taken)).toMatch(/already exists/);
    expect(commandDraftIssue({ ...draft, name: "review" }, taken, "review")).toBeUndefined();
    expect(commandDraftIssue({ ...draft, name: "other", body: " " }, taken)).toMatch(/instructions/);
  });
});

describe("CommandsSection", () => {
  it("lists WackCode's own commands, yours, and each package's", async () => {
    renderSection();
    const wackcode = await screen.findByRole("region", { name: "WackCode commands" });
    expect(within(wackcode).getByText("/compact")).toBeInTheDocument();
    expect(within(wackcode).getByText("/copy")).toBeInTheDocument();

    const yours = screen.getByRole("region", { name: "Your commands" });
    expect(within(yours).getByText("/review")).toBeInTheDocument();
    expect(within(yours).getByText("/standup")).toBeInTheDocument();

    const pack = screen.getByRole("region", { name: "pack commands" });
    expect(within(pack).getByText("/weekly")).toBeInTheDocument();
    expect(within(pack).getByText("/deploy")).toBeInTheDocument();
  });

  it("toggles a command through its stable key", async () => {
    const actions = renderSection();
    const yours = await screen.findByRole("region", { name: "Your commands" });
    fireEvent.click(within(yours).getByRole("switch", { name: "Use /review" }));
    await waitFor(() => expect(actions.onSetEnabled).toHaveBeenCalledWith("custom:/app/data/commands/review.md", false));
    expect(actions.onChanged).toHaveBeenCalledWith({ disabled: [] });
  });

  it("marks switched-off commands, including WackCode's own", async () => {
    renderSection({ ...overview, disabled: ["app:copy", "custom:/app/data/commands/review.md"] });
    const wackcode = await screen.findByRole("region", { name: "WackCode commands" });
    const row = within(wackcode).getByRole("switch", { name: "Use /copy" });
    expect(row).toHaveAttribute("aria-checked", "false");
  });

  it("creates a command once the name and instructions are valid", async () => {
    const actions = renderSection();
    fireEvent.click(await screen.findByRole("button", { name: /New command/ }));
    const create = screen.getByRole("button", { name: "Create command" });
    expect(create).toBeDisabled();

    fireEvent.change(screen.getByPlaceholderText("review"), { target: { value: "My Cmd" } });
    expect(screen.getByPlaceholderText("review")).toHaveValue("my-cmd");
    fireEvent.change(screen.getByRole("textbox", { name: /Description/ }), { target: { value: "Runs my flow." } });
    fireEvent.change(screen.getByRole("textbox", { name: /Instructions/ }), { target: { value: "Do $1." } });
    expect(create).toBeEnabled();
    fireEvent.click(create);
    await waitFor(() => expect(actions.onSave).toHaveBeenCalledWith({
      path: undefined, name: "my-cmd", description: "Runs my flow.", argumentHint: "", body: "Do $1."
    }));
  });

  it("expands the editor's preview with Pi's argument rules", async () => {
    renderSection();
    fireEvent.click(await screen.findByRole("button", { name: /New command/ }));
    fireEvent.change(screen.getByRole("textbox", { name: /Instructions/ }), { target: { value: "Check $1 and $2." } });
    fireEvent.change(screen.getByRole("textbox", { name: /Sample arguments/ }), { target: { value: "a.ts b.ts" } });
    expect(await screen.findByText("Check a.ts and b.ts.")).toBeInTheDocument();
  });

  it("opens a custom command for editing with its saved body, and deletes it after confirming", async () => {
    const actions = renderSection();
    const yours = await screen.findByRole("region", { name: "Your commands" });
    fireEvent.click(within(yours).getByRole("button", { name: /^\/review/ }));
    await waitFor(() => expect(actions.onRead).toHaveBeenCalledWith("/app/data/commands/review.md"));
    expect(screen.getByRole("textbox", { name: /Instructions/ })).toHaveValue("Look at $ARGUMENTS closely.");

    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    fireEvent.click(await screen.findByRole("button", { name: "Move to Trash" }));
    await waitFor(() => expect(actions.onDelete).toHaveBeenCalledWith("/app/data/commands/review.md"));
  });

  it("filters rows with the composer-style / input", async () => {
    renderSection();
    const filter = await screen.findByRole("textbox", { name: "Filter commands" });
    fireEvent.change(filter, { target: { value: "/week" } });
    await waitFor(() => expect(screen.queryByText("/review")).not.toBeInTheDocument());
    const pack = screen.getByRole("region", { name: "pack commands" });
    expect(within(pack).getByText("/weekly")).toBeInTheDocument();
  });
});
