import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GitChangeFile, GitCommit } from "../types";
import { GitChangesList, GitHistoryList, GitPanelTop } from "./GitSidebar";

afterEach(cleanup);

function file(path: string, status = "modified", additions = 2, deletions = 1): GitChangeFile {
  return {
    path, oldPath: null, status, staged: false, unstaged: true, untracked: false, binary: false, hunkable: true, truncated: false,
    sections: [{ layer: "working", revision: `rev-${path}`, diff: "", hunks: [], truncated: false, additions, deletions }]
  };
}

function commit(sha: string, subject: string, patch: Partial<GitCommit> = {}): GitCommit {
  return {
    sha, shortSha: sha.slice(0, 7), parents: ["p"], authorName: "Demo Dev", authorEmail: "demo@example.com",
    authoredAt: "2026-09-29T10:00:00Z", subject, body: "", unpushed: false, ...patch
  };
}

function changesProps(excluded: string[] = []) {
  return {
    files: [file("src/a.ts"), file("src/b.ts"), file("merge.txt", "conflict")],
    excluded: new Set(excluded), selectedPath: "src/a.ts", commentCounts: new Map([["src/b.ts", 2]]), disabled: false,
    onSelect: vi.fn(), onToggle: vi.fn(), onToggleAll: vi.fn(), onDiscard: vi.fn(), onCopyPath: vi.fn(), onReveal: vi.fn()
  };
}

describe("GitChangesList", () => {
  it("ticks every committable file by default and shows the tri-state box as mixed once one is unticked", () => {
    const props = changesProps();
    const { rerender } = render(<GitChangesList {...props} />);
    expect(screen.getByRole("checkbox", { name: "Include all files" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Include src/a.ts in commit" })).toBeChecked();
    // A conflicted file can't be committed, so its box is locked off.
    expect(screen.getByRole("checkbox", { name: "Include merge.txt in commit" })).toBeDisabled();
    expect(screen.getByText("3 changed files")).toBeInTheDocument();

    rerender(<GitChangesList {...props} excluded={new Set(["src/b.ts"])} />);
    expect(screen.getByRole("checkbox", { name: "Include all files" })).toBePartiallyChecked();
    expect(screen.getByRole("checkbox", { name: "Include src/b.ts in commit" })).not.toBeChecked();
  });

  it("toggles with the checkbox or Space, and moves the selection with the arrow keys", () => {
    const props = changesProps();
    render(<GitChangesList {...props} />);
    fireEvent.click(screen.getByRole("checkbox", { name: "Include src/b.ts in commit" }));
    expect(props.onToggle).toHaveBeenCalledWith("src/b.ts");
    expect(props.onSelect).not.toHaveBeenCalled();

    const selected = screen.getByRole("button", { name: /a\.ts/, current: true });
    fireEvent.keyDown(selected, { key: " " });
    expect(props.onToggle).toHaveBeenLastCalledWith("src/a.ts");
    fireEvent.keyDown(selected, { key: "ArrowDown" });
    expect(props.onSelect).toHaveBeenCalledWith("src/b.ts");

    fireEvent.click(screen.getByRole("checkbox", { name: "Include all files" }));
    expect(props.onToggleAll).toHaveBeenCalled();
  });

  it("offers Discard, Copy path and Reveal from each row's menu", () => {
    const props = changesProps();
    render(<GitChangesList {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Actions for src/b.ts" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /Discard changes/ }));
    expect(props.onDiscard).toHaveBeenCalledWith(props.files[1]);
    fireEvent.click(screen.getByRole("button", { name: "Actions for src/b.ts" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Copy path" }));
    expect(props.onCopyPath).toHaveBeenCalledWith("src/b.ts");
  });
});

describe("GitPanelTop", () => {
  it("shows the change count on its tab and switches tabs", () => {
    const onTab = vi.fn();
    render(<GitPanelTop switcher={<span>switcher</span>} tab="changes" changesCount={3} commitPulse={0} onTab={onTab} />);
    expect(screen.getByRole("tab", { name: "Changes 3" })).toHaveAttribute("aria-selected", "true");
    fireEvent.click(screen.getByRole("tab", { name: "History" }));
    expect(onTab).toHaveBeenCalledWith("history");
  });
});

describe("GitHistoryList", () => {
  const commits = [commit("aaaaaaa1", "Newest", { unpushed: true }), commit("bbbbbbb2", "Merge it", { parents: ["p", "q"] }), commit("ccccccc3", "Oldest")];

  it("marks unpushed commits only when there is a remote, and selects with click or arrows", () => {
    const onSelect = vi.fn();
    const { rerender } = render(<GitHistoryList commits={commits} hasMore={false} loading={false} showUnpushed selectedSha="aaaaaaa1" onSelect={onSelect} onLoadMore={vi.fn()} />);
    expect(screen.getAllByTitle("Not pushed yet")).toHaveLength(1);
    expect(screen.getByText("merge")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Oldest/ }));
    expect(onSelect).toHaveBeenCalledWith("ccccccc3");
    fireEvent.keyDown(screen.getByRole("button", { name: /Newest/ }), { key: "ArrowDown" });
    expect(onSelect).toHaveBeenLastCalledWith("bbbbbbb2");

    rerender(<GitHistoryList commits={commits} hasMore={false} loading={false} showUnpushed={false} selectedSha="aaaaaaa1" onSelect={onSelect} onLoadMore={vi.fn()} />);
    expect(screen.queryByTitle("Not pushed yet")).toBeNull();
  });

  it("says so when the repository has no commits", () => {
    render(<GitHistoryList commits={[]} hasMore={false} loading={false} showUnpushed onSelect={vi.fn()} onLoadMore={vi.fn()} />);
    expect(screen.getByText("No commits yet")).toBeInTheDocument();
  });

  it("shows a refresh failure even while the previous commits are still listed", () => {
    render(<GitHistoryList commits={commits} hasMore={false} loading={false} error="Git did not finish in time" showUnpushed onSelect={vi.fn()} onLoadMore={vi.fn()} />);
    expect(screen.getByRole("alert")).toHaveTextContent("Git did not finish in time");
    expect(screen.getByRole("button", { name: /Newest/ })).toBeInTheDocument();
  });
});
