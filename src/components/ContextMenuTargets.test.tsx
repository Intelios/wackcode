import type { ReactNode } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GitChangeFile, NormalizedMessage, ProjectRecord, TaskRecord } from "../types";
import { Sidebar } from "./Sidebar";
import { ArchivedList } from "./ArchivedList";
import { Transcript } from "./Transcript";
import { GitChangesList } from "./GitSidebar";
import { ContextMenuProvider } from "./ui/ContextMenu";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import { MenuButton } from "./ui/MenuButton";

afterEach(cleanup);

const project: ProjectRecord = { id: "p", name: "Project", path: "/code", gitRoot: "/code", gitHasHead: true, runCommand: null, branch: "main", createdAt: "now" };
const chat: TaskRecord = {
  id: "other", projectId: "p", name: "Background chat", workspacePath: "/code", worktreePath: null, branch: null, usesWorktree: false,
  autoTitleEligible: false, autoTitleAttemptId: null, providerId: "prov", modelId: "m", thinkingLevel: "off", sessionFile: "session.json", status: "idle",
  mode: "build", archived: false, archivedAt: null, lastError: null, createdAt: "now", updatedAt: "now"
};
function menus(children: ReactNode) {
  return <ContextMenuProvider scope="test" items={[]} copyText={vi.fn()} readText={vi.fn()} openLink={vi.fn()} onError={vi.fn()}>{children}</ContextMenuProvider>;
}

describe("context menu targets", () => {
  it("acts on a background chat without selecting it, and dispatches Delete through confirmation", () => {
    const action = vi.fn(); const select = vi.fn();
    render(menus(<Sidebar projects={[project]} tasks={[chat]} selectedTaskId="selected" pinnedProjectIds={new Set()} pendingDialogTaskIds={new Set()} titlePulses={{}} collapsedProjectIds={new Set()}
      archivedOpen={false} onSelectTask={select} onTaskAction={action} onNewChat={vi.fn()} onNewDraft={vi.fn()} onAddProject={vi.fn()} onToggleArchived={vi.fn()}
      onToggleProjectCollapsed={vi.fn()} onOpenSettings={vi.fn()} onProjectAction={vi.fn()} onRenameTask={vi.fn()} onArchiveAll={vi.fn()} onDeleteAllArchived={vi.fn()} onToggleGit={null} />));
    const row = screen.getByRole("button", { name: "Background chat" });
    fireEvent.contextMenu(row);
    expect(screen.getByRole("menuitem", { name: "Move to worktree" })).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: "Fork chat" })).not.toBeDisabled();
    fireEvent.click(screen.getByRole("menuitem", { name: "Copy path" }));
    expect(action).toHaveBeenLastCalledWith(chat, "copy");
    fireEvent.contextMenu(row);
    fireEvent.click(screen.getByRole("menuitem", { name: "Delete" }));
    expect(action).toHaveBeenLastCalledWith(chat, "delete");
    expect(select).not.toHaveBeenCalled();
  });

  it("keeps archived-chat Delete on the confirmation path too", () => {
    const archived = { ...chat, archived: true };
    const action = vi.fn();
    render(menus(<ArchivedList tasks={[archived]} projects={[project]} onSelectTask={vi.fn()} onTaskAction={action} />));
    fireEvent.contextMenu(screen.getByText("Background chat"));
    fireEvent.click(screen.getByRole("menuitem", { name: "Delete" }));
    expect(action).toHaveBeenCalledWith(archived, "delete");
  });

  it("offers the message's existing actions and only Copy while actions are locked", () => {
    const user: NormalizedMessage = { id: "u", entryId: "u", role: "user", blocks: [{ type: "text", text: "Make it blue" }] };
    const answer: NormalizedMessage = { id: "a", entryId: "a", role: "assistant", blocks: [{ type: "text", text: "Done." }], turn: { userEntryId: "u", endEntryId: "a" } };
    const action = vi.fn();
    const transcript = (enabled: boolean) => menus(<Transcript messages={[user, answer]} running={!enabled} actionsEnabled={enabled} onMessageAction={action} />);
    const { rerender } = render(transcript(true));
    fireEvent.contextMenu(screen.getByText("Make it blue"));
    expect(screen.getByRole("menuitem", { name: "Edit" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("menuitem", { name: "Rewind to here" }));
    expect(action).toHaveBeenLastCalledWith({ type: "rewind", message: user });
    fireEvent.contextMenu(screen.getByText("Done."));
    fireEvent.click(screen.getByRole("menuitem", { name: "Fork from here" }));
    expect(action).toHaveBeenLastCalledWith({ type: "fork", message: answer });
    rerender(transcript(false));
    fireEvent.contextMenu(screen.getByText("Make it blue"));
    expect(screen.getAllByRole("menuitem")).toHaveLength(1);
    expect(screen.getByRole("menuitem", { name: "Copy" })).toBeInTheDocument();
  });

  it("locks discard and commit changes on conflicted Git files", () => {
    const conflict: GitChangeFile = { path: "merge.txt", oldPath: null, status: "conflict", staged: false, unstaged: true, untracked: false, binary: false, hunkable: false, truncated: false, sections: [] };
    const discard = vi.fn(); const copy = vi.fn();
    render(menus(<GitChangesList files={[conflict]} excluded={new Set()} commentCounts={new Map()} disabled={false}
      onSelect={vi.fn()} onToggle={vi.fn()} onToggleAll={vi.fn()} onDiscard={discard} onCopyPath={copy} onReveal={vi.fn()} />));
    fireEvent.contextMenu(screen.getByRole("button", { name: /merge\.txt/, current: true }));
    expect(screen.getByRole("menuitem", { name: /Discard/ })).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: /commit/ })).toBeDisabled();
    fireEvent.click(screen.getByRole("menuitem", { name: "Copy path" }));
    expect(copy).toHaveBeenCalledWith("merge.txt");
    expect(discard).not.toHaveBeenCalled();
  });

  it("keeps confirmation dialogs open on right-click and suppresses app navigation inside them", () => {
    const cancel = vi.fn();
    render(<ContextMenuProvider scope="modal" items={[{ label: "New chat" }]} copyText={vi.fn()} readText={vi.fn()} openLink={vi.fn()} onError={vi.fn()}>
      <ConfirmDialog title="Delete chat?" onConfirm={vi.fn()} onCancel={cancel} />
    </ContextMenuProvider>);
    const backdrop = screen.getByRole("presentation");
    fireEvent.mouseDown(backdrop, { button: 2 });
    fireEvent.contextMenu(screen.getByRole("alertdialog"));
    expect(cancel).not.toHaveBeenCalled();
    expect(screen.queryByRole("menu")).toBeNull();
    fireEvent.mouseDown(backdrop, { button: 0 });
    expect(cancel).toHaveBeenCalledOnce();
  });

  it("still closes existing dropdown menus with Escape and returns focus to their button", () => {
    render(menus(<MenuButton label="Dropdown" items={[{ label: "Action" }]} />));
    const button = screen.getByRole("button", { name: "Dropdown" });
    fireEvent.click(button);
    fireEvent.keyDown(screen.getByRole("menuitem"), { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(button).toHaveFocus();
  });
});
