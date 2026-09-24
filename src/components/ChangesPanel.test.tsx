import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GitChanges } from "../types";
import { ChangesPanel } from "./ChangesPanel";

afterEach(cleanup);

const changes: GitChanges = {
  isGit: true, root: "/tmp/project", branch: "topic", stagedRevision: "index-1",
  files: [{
    path: "file.ts", oldPath: null, status: "modified", staged: true, unstaged: true,
    untracked: false, binary: false, hunkable: true, truncated: false, diff: "",
    sections: [
      { layer: "staged", revision: "staged-1", diff: "", truncated: false, hunks: [{
        id: 0, header: "@@ -1 +1 @@", oldStart: 1, newStart: 1,
        lines: [{ kind: "deletion", text: "-old", oldLine: 1, newLine: null }, { kind: "addition", text: "+new", oldLine: null, newLine: 1 }]
      }] },
      { layer: "working", revision: "working-1", diff: "", truncated: false, hunks: [{
        id: 0, header: "@@ -3 +3 @@", oldStart: 3, newStart: 3,
        lines: [{ kind: "deletion", text: "-more", oldLine: 3, newLine: null }, { kind: "addition", text: "+better", oldLine: null, newLine: 3 }]
      }] }
    ]
  }]
};

function props() {
  return {
    changes, loading: false, busy: false, width: 430, mode: "build" as const,
    canReview: true, comments: [],
    onWidthChange: vi.fn(), onClose: vi.fn(), onRefresh: vi.fn(), onSettings: vi.fn(),
    onReview: vi.fn(async () => true), onAction: vi.fn(async () => {}),
    onCommit: vi.fn(async () => {}), onGenerate: vi.fn(async () => ({ message: "Update file", revision: "index-1" })),
    onPublishInfo: vi.fn(async () => ({ branch: "topic", upstream: null, remotes: ["origin"] })),
    onPush: vi.fn(async () => {}), onPreparePr: vi.fn(async () => ({ repo: "github.com/o/r", base: "main", head: "topic", title: "Title", body: "", existingUrl: null })),
    onCreatePr: vi.fn(async () => "https://github.com/o/r/pull/1"), onOpenPr: vi.fn(),
    onComments: vi.fn(async () => {}), onAddressComments: vi.fn(async () => true)
  };
}

describe("ChangesPanel", () => {
  it("shows separate layers and anchors a comment to a deleted line", async () => {
    const callbacks = props();
    render(<ChangesPanel {...callbacks} />);
    expect(screen.getByRole("heading", { name: "Staged 1" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Working tree 1" })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Unstage" })).toHaveLength(2);
    fireEvent.click(screen.getAllByRole("button", { name: "Comment on file.ts line 1" })[0]);
    fireEvent.change(screen.getByRole("textbox", { name: "Diff comment" }), { target: { value: "Check removal" } });
    fireEvent.click(screen.getByRole("button", { name: "Save comment" }));
    await waitFor(() => expect(callbacks.onComments).toHaveBeenCalledWith([expect.objectContaining({
      path: "file.ts", layer: "staged", side: "old", line: 1, excerpt: "-old", revision: "staged-1", text: "Check removal"
    })]));
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "Diff comment" })).not.toBeInTheDocument());
    fireEvent.click(screen.getAllByRole("button", { name: /file\.ts/ }).filter((button) => button.getAttribute("title") === "file.ts")[1]);
    fireEvent.click(screen.getAllByRole("button", { name: "Stage" })[1]);
    await waitFor(() => expect(callbacks.onAction).toHaveBeenCalledWith(changes.files[0], changes.files[0].sections[1], "stage", 0));
  });

  it("disables review when Reviewer is unavailable", () => {
    render(<ChangesPanel {...props()} canReview={false} reviewReason="Enable Reviewer in Settings" />);
    expect(screen.getByRole("button", { name: "Review changes" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Enable Reviewer in Settings" })).toBeInTheDocument();
  });

  it("commits the staged revision and creates a regular PR from editable fields", async () => {
    const callbacks = props();
    render(<ChangesPanel {...callbacks} />);
    fireEvent.change(screen.getByRole("textbox", { name: "Commit message" }), { target: { value: "Update file" } });
    fireEvent.click(screen.getByRole("button", { name: "Commit" }));
    await waitFor(() => expect(callbacks.onCommit).toHaveBeenCalledWith("Update file", "index-1"));
    await waitFor(() => expect(screen.getByRole("textbox", { name: "Commit message" })).toHaveValue(""));
    fireEvent.click(screen.getByRole("button", { name: "Create PR" }));
    await waitFor(() => expect(screen.getByRole("textbox", { name: "PR title" })).toHaveValue("Title"));
    fireEvent.change(screen.getByRole("textbox", { name: "PR title" }), { target: { value: "Better title" } });
    expect(screen.getByRole("checkbox", { name: "Draft" })).not.toBeChecked();
    fireEvent.click(screen.getAllByRole("button", { name: "Create PR" })[1]);
    await waitFor(() => expect(callbacks.onCreatePr).toHaveBeenCalledWith("origin", "main", "Better title", "", false));
  });
});
