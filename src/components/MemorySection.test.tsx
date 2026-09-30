import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MemoriesOverview, MemoryProject } from "../types";
import { MemorySection, memoryDraftIssue } from "./MemorySection";

afterEach(cleanup);

function project(patch: Partial<MemoryProject> = {}): MemoryProject {
  return {
    key: "abc123456789",
    name: "wackcode",
    path: "/Users/jack/repos/wackcode",
    dir: "/app data/memory/wackcode-abc123456789",
    enabled: true,
    entries: [
      {
        name: "feedback_run-worker-tests", filePath: "/app data/memory/wackcode-abc123456789/feedback_run-worker-tests.md",
        kind: "feedback", title: "Run worker tests", description: "Protocol edits need pnpm test:worker",
        modified: "2026-09-28T10:12:00.000Z"
      },
      {
        name: "user_prefers", filePath: "/app data/memory/wackcode-abc123456789/user_prefers.md",
        kind: "user", title: "Prefers terse answers", description: "", modified: undefined
      }
    ],
    ...patch
  };
}

const overview: MemoriesOverview = { enabled: true, projects: [project()] };

function change(next: MemoriesOverview = overview) {
  return { overview: next, config: { enabled: next.enabled, disabledProjects: [] } };
}

function renderSection(start: MemoriesOverview = overview) {
  const actions = {
    onList: vi.fn().mockResolvedValue(start),
    onRead: vi.fn().mockResolvedValue({ body: "Protocol edits need pnpm test:worker." }),
    onSave: vi.fn().mockResolvedValue(change()),
    onDelete: vi.fn().mockResolvedValue(change()),
    onRemoveProject: vi.fn().mockResolvedValue(change({ enabled: true, projects: [] })),
    onSetProjectEnabled: vi.fn().mockResolvedValue(change()),
    onReveal: vi.fn().mockResolvedValue(undefined),
    onFindFile: vi.fn().mockResolvedValue(undefined),
    onSetEnabled: vi.fn().mockResolvedValue(undefined)
  };
  render(<MemorySection {...actions} />);
  return actions;
}

describe("Settings › Memory", () => {
  it("lists each project's notes with type badges", async () => {
    renderSection();
    expect(await screen.findByText("Run worker tests")).toBeInTheDocument();
    expect(screen.getByText("Prefers terse answers")).toBeInTheDocument();
    expect(screen.getByText("feedback")).toBeInTheDocument();
    expect(screen.getByText("user")).toBeInTheDocument();
    expect(screen.getByTitle("/Users/jack/repos/wackcode · /app data/memory/wackcode-abc123456789")).toHaveTextContent("2 notes");
  });

  it("explains itself when no project has memory yet", async () => {
    renderSection({ enabled: true, projects: [] });
    expect(await screen.findByText("No memories yet")).toBeInTheDocument();
    expect(screen.getByText(/Open a chat in a project and ask it to remember something/i)).toBeInTheDocument();
  });

  it("lists projects still waiting for a note as rows of one card, not a card each", async () => {
    const empty = (name: string, key: string) => project({ name, key, dir: `/app data/memory/${name}-${key}`, path: `/repos/${name}`, entries: [] });
    const actions = renderSection({ enabled: true, projects: [project(), empty("alpha", "a1"), empty("beta", "b2")] });
    const waiting = await screen.findByRole("region", { name: "Waiting for a first note" });
    expect(within(waiting).getAllByRole("listitem")).toHaveLength(2);
    expect(screen.queryByText(/No memories yet/)).not.toBeInTheDocument();
    // Each row keeps a way to write the first note, the project's own folder and its switch.
    fireEvent.click(within(waiting).getByRole("button", { name: "New memory in alpha" }));
    expect(await screen.findByRole("heading", { name: "New memory" })).toBeInTheDocument();
    expect(screen.getByText(/In alpha’s memory folder/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    const again = screen.getByRole("region", { name: "Waiting for a first note" });
    expect(within(again).getByRole("button", { name: "Show beta's memory folder in Finder" })).toBeInTheDocument();
    fireEvent.click(within(again).getByRole("switch", { name: "Use memory in beta" }));
    await waitFor(() => expect(actions.onSetProjectEnabled).toHaveBeenCalledWith("b2", false));
  });

  it("says in the hero how many notes there are, or that memory is off", async () => {
    renderSection();
    const hero = screen.getByRole("region", { name: "Memory overview" });
    expect(await within(hero).findByText("2 notes")).toBeInTheDocument();
    cleanup();
    renderSection({ ...overview, enabled: false });
    expect(await within(screen.getByRole("region", { name: "Memory overview" })).findByText("Off")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Memory is switched off");
  });

  it("switches the master switch and a project's memory through their toggles", async () => {
    const actions = renderSection();
    await screen.findByText("Run worker tests");
    fireEvent.click(screen.getByRole("switch", { name: "Use memory" }));
    await waitFor(() => expect(actions.onSetEnabled).toHaveBeenCalledWith(false));
    fireEvent.click(screen.getByRole("switch", { name: "Use memory in wackcode" }));
    await waitFor(() => expect(actions.onSetProjectEnabled).toHaveBeenCalledWith("abc123456789", false));
  });

  it("opens a note in the editor and saves it", async () => {
    const actions = renderSection();
    fireEvent.click(await screen.findByTitle("Edit Run worker tests"));
    const title = await screen.findByDisplayValue("Run worker tests");
    expect(actions.onRead).toHaveBeenCalledWith("/app data/memory/wackcode-abc123456789/feedback_run-worker-tests.md");
    fireEvent.change(title, { target: { value: "Run worker tests first" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(actions.onSave).toHaveBeenCalledWith(expect.objectContaining({
      path: "/app data/memory/wackcode-abc123456789/feedback_run-worker-tests.md",
      title: "Run worker tests first",
      body: "Protocol edits need pnpm test:worker."
    })));
  });

  it("finds one note's file in Finder from its row", async () => {
    const actions = renderSection();
    fireEvent.click(await screen.findByRole("button", { name: "Find Run worker tests in Finder" }));
    await waitFor(() =>
      expect(actions.onFindFile).toHaveBeenCalledWith("/app data/memory/wackcode-abc123456789/feedback_run-worker-tests.md"));
  });

  it("deletes a note through the confirm dialog", async () => {
    const actions = renderSection();
    fireEvent.click(await screen.findByRole("button", { name: "Delete Prefers terse answers" }));
    fireEvent.click(await screen.findByRole("button", { name: "Move to Trash" }));
    await waitFor(() =>
      expect(actions.onDelete).toHaveBeenCalledWith("/app data/memory/wackcode-abc123456789/user_prefers.md"));
  });

  it("removes a project's whole folder through the confirm dialog", async () => {
    const actions = renderSection();
    fireEvent.click(await screen.findByRole("button", { name: "Delete wackcode's memory folder" }));
    expect(await screen.findByText(/2 notes move to the Trash with the folder/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Move to Trash" }));
    await waitFor(() =>
      expect(actions.onRemoveProject).toHaveBeenCalledWith("/app data/memory/wackcode-abc123456789"));
  });

  it("saves a new note under the name derived from its title", async () => {
    const actions = renderSection();
    fireEvent.click(await screen.findByRole("button", { name: /New memory/ }));
    // Title is the last field touched — the derived name must still carry it whole.
    fireEvent.change(screen.getByPlaceholderText("Any change to worker/src/protocol.ts needs pnpm test:worker before review."), { target: { value: "The body." } });
    fireEvent.change(screen.getByPlaceholderText("Run worker tests after protocol changes"), { target: { value: "Run worker tests" } });
    expect(screen.getByDisplayValue("project_run-worker-tests")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Create memory" }));
    await waitFor(() => expect(actions.onSave).toHaveBeenCalledWith(expect.objectContaining({
      dir: "/app data/memory/wackcode-abc123456789",
      name: "project_run-worker-tests",
      memoryType: "project"
    })));
  });

  it("flags renaming a note to a name another note already holds", async () => {
    renderSection();
    fireEvent.click(await screen.findByTitle("Edit Run worker tests"));
    fireEvent.change(await screen.findByDisplayValue("feedback_run-worker-tests"), { target: { value: "user_prefers" } });
    expect(await screen.findByRole("status")).toHaveTextContent("already exists");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });
});

describe("memoryDraftIssue", () => {
  const taken = new Set(["feedback_taken"]);
  const draft = { name: "feedback_fresh", memoryType: "feedback" as const, title: "A title", description: "", body: "A body" };

  it("accepts a filled draft and refuses empty titles, bodies and taken names", () => {
    expect(memoryDraftIssue(draft, taken)).toBeUndefined();
    expect(memoryDraftIssue({ ...draft, title: "  " }, taken)).toContain("title");
    expect(memoryDraftIssue({ ...draft, body: "" }, taken)).toContain("something to remember");
    expect(memoryDraftIssue({ ...draft, name: "feedback_taken" }, taken, "feedback_other")).toContain("already exists");
    // Renaming to itself (the edit case) is fine.
    expect(memoryDraftIssue({ ...draft, name: "feedback_taken" }, taken, "feedback_taken")).toBeUndefined();
    expect(memoryDraftIssue({ ...draft, name: "Bad Name" }, taken)).toContain("lowercase");
  });
});
