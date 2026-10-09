import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GitChangeFile, GitDiffSection } from "../types";
import { DiffView } from "./DiffView";

afterEach(cleanup);

function section(layer: GitDiffSection["layer"], revision: string): GitDiffSection {
  return {
    layer, revision, diff: "", truncated: false, additions: 2, deletions: 1,
    hunks: [{
      id: 0, header: "@@ -1,2 +1,3 @@", oldStart: 1, newStart: 1,
      lines: [
        { kind: "context", text: " const keep = 1;", oldLine: 1, newLine: 1 },
        { kind: "deletion", text: "-const gone = 2;", oldLine: 2, newLine: null },
        { kind: "addition", text: "+const now = 2;", oldLine: null, newLine: 2 },
        { kind: "addition", text: "+const extra = 3;", oldLine: null, newLine: 3 }
      ]
    }]
  };
}

function file(sections: GitDiffSection[]): GitChangeFile {
  return { path: "src/file.ts", oldPath: null, status: "modified", staged: false, unstaged: true, untracked: false, binary: false, hunkable: true, truncated: false, sections };
}

describe("DiffView", () => {
  it("anchors a comment to the line's own section", () => {
    const working = section("working", "w1");
    const onAddComment = vi.fn();
    render(<DiffView file={file([working])} sections={[working]} disabled={false} onAddComment={onAddComment} />);
    fireEvent.click(screen.getByRole("button", { name: "Comment on src/file.ts line 3" }));
    expect(onAddComment).toHaveBeenCalledWith(working, working.hunks[0].lines[3]);
  });

  it("stacks a file's staged and unstaged sections, each with its own discard", () => {
    const staged = section("staged", "s1");
    const working = section("working", "w1");
    const onAction = vi.fn();
    render(<DiffView file={file([staged, working])} sections={[staged, working]} disabled={false} onAction={onAction} />);
    expect(screen.getByText("Staged")).toBeInTheDocument();
    expect(screen.getByText("Not staged")).toBeInTheDocument();
    const discards = screen.getAllByRole("button", { name: "Discard" });
    expect(discards).toHaveLength(2);
    fireEvent.click(discards[1]);
    expect(onAction).toHaveBeenCalledWith(expect.objectContaining({ path: "src/file.ts" }), working, undefined);
  });

  it("is read-only for a past commit: no comment gutter and nothing to discard", () => {
    const commit = section("commit", "c1");
    render(<DiffView file={file([commit])} sections={[commit]} readOnly disabled onAddComment={vi.fn()} onAction={vi.fn()} />);
    expect(screen.queryByRole("button", { name: /Comment on/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /Discard/ })).toBeNull();
    expect(document.querySelectorAll(".diff-line")).toHaveLength(4);
  });
});
