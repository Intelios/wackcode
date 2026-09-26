import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GitChanges } from "../types";
import { ChangesPanel } from "./ChangesPanel";

afterEach(cleanup);

const changes: GitChanges = {
  isGit: true, root: "/tmp/project", branch: "topic", changesRevision: "index-1",
  files: [{
    path: "file.ts", oldPath: null, status: "modified", staged: true, unstaged: true,
    untracked: false, binary: false, hunkable: true, truncated: false, diff: "",
    sections: [
      { layer: "staged", revision: "staged-1", diff: "", truncated: false, additions: 1, deletions: 1, hunks: [{
        id: 0, header: "@@ -1 +1 @@", oldStart: 1, newStart: 1,
        lines: [{ kind: "deletion", text: "-const gone = 1;", oldLine: 1, newLine: null }, { kind: "addition", text: "+const now = 2;", oldLine: null, newLine: 1 }]
      }] },
      { layer: "working", revision: "working-1", diff: "", truncated: false, additions: 1, deletions: 1, hunks: [{
        id: 0, header: "@@ -3 +3 @@", oldStart: 3, newStart: 3,
        lines: [{ kind: "deletion", text: "-more", oldLine: 3, newLine: null }, { kind: "addition", text: "+better", oldLine: null, newLine: 3 }]
      }] }
    ]
  }]
};

function props() {
  return {
    changes, loading: false, busy: false, mode: "build" as const,
    canReview: true, comments: [],
    onClose: vi.fn(), onRefresh: vi.fn(), onSettings: vi.fn(),
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
    // file.ts diffs are syntax-highlighted with the file's own language.
    expect(document.querySelector(".diff-line.addition code .hljs-keyword")).not.toBeNull();
    expect(document.querySelector(".diff-line.deletion code .hljs-keyword")).not.toBeNull();
    // Discard on each file row and the diff header; "Discard hunk" per hunk.
    expect(screen.getAllByRole("button", { name: "Discard" })).toHaveLength(3);
    fireEvent.click(screen.getAllByRole("button", { name: "Comment on file.ts line 1" })[0]);
    fireEvent.change(screen.getByRole("textbox", { name: "Diff comment" }), { target: { value: "Check removal" } });
    fireEvent.click(screen.getByRole("button", { name: "Comment" }));
    await waitFor(() => expect(callbacks.onComments).toHaveBeenCalledWith([expect.objectContaining({
      path: "file.ts", layer: "staged", side: "old", line: 1, excerpt: "-const gone = 1;", revision: "staged-1", text: "Check removal"
    })]));
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "Diff comment" })).not.toBeInTheDocument());
    fireEvent.click(screen.getAllByRole("button", { name: /file\.ts/ }).filter((button) => button.getAttribute("title") === "file.ts")[1]);
    // The diff crossfades (AnimatePresence wait); line 3 exists only in the working section.
    await screen.findAllByRole("button", { name: "Comment on file.ts line 3" });
    fireEvent.click(screen.getByRole("button", { name: "Discard hunk" }));
    await waitFor(() => expect(callbacks.onAction).toHaveBeenCalledWith(changes.files[0], changes.files[0].sections[1], 0));
  });

  it("scopes the commit sheet to a file row and clears back to all changes", async () => {
    const callbacks = props();
    render(<ChangesPanel {...callbacks} />);
    fireEvent.click(screen.getAllByRole("button", { name: "Commit" })[0]);
    // The commit sheet opens with the file as the scope chip.
    const sheet = await screen.findByRole("tabpanel");
    expect(within(sheet).getByText("file.ts")).toBeInTheDocument();
    fireEvent.change(within(sheet).getByRole("textbox", { name: "Commit message" }), { target: { value: "Just this file" } });
    fireEvent.click(within(sheet).getByRole("button", { name: "Commit" }));
    await waitFor(() => expect(callbacks.onCommit).toHaveBeenCalledWith("Just this file", ["file.ts"], "index-1"));
  });

  it("opens Settings when Review is unavailable", async () => {
    const callbacks = props();
    render(<ChangesPanel {...callbacks} canReview={false} reviewReason="Enable Reviewer in Settings" />);
    const review = screen.getByRole("button", { name: "Review" });
    expect(review).not.toBeDisabled();
    expect(review).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(review);
    expect(callbacks.onSettings).toHaveBeenCalled();
  });

  it("commits all changes from the dock and creates a regular PR from editable fields", async () => {
    const callbacks = props();
    render(<ChangesPanel {...callbacks} />);
    fireEvent.click(screen.getByRole("tab", { name: "Commit" }));
    const sheet = await screen.findByRole("tabpanel");
    expect(within(sheet).getByText("All changes")).toBeInTheDocument();
    fireEvent.change(within(sheet).getByRole("textbox", { name: "Commit message" }), { target: { value: "Update file" } });
    fireEvent.click(within(sheet).getByRole("button", { name: "Commit" }));
    await waitFor(() => expect(callbacks.onCommit).toHaveBeenCalledWith("Update file", [], "index-1"));
    await waitFor(() => expect(screen.getByRole("tab", { name: "Pull request" })).toBeInTheDocument());
    fireEvent.click(screen.getByRole("tab", { name: "Pull request" }));
    await waitFor(() => expect(screen.getByRole("textbox", { name: "PR title" })).toHaveValue("Title"));
    fireEvent.change(screen.getByRole("textbox", { name: "PR title" }), { target: { value: "Better title" } });
    expect(screen.getByRole("checkbox", { name: "Draft" })).not.toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: "Create pull request" }));
    await waitFor(() => expect(callbacks.onCreatePr).toHaveBeenCalledWith("origin", "main", "Better title", "", false));
  });
});
