import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RestoreDialog } from "./RestoreDialog";

afterEach(cleanup);

const changes = [
  { path: "app/main.ts", status: "revert" as const },
  { path: "other/notes.md", status: "delete" as const }
];

describe("RestoreDialog", () => {
  it("restores only the selected files and closes once the choice has run", async () => {
    const onChoose = vi.fn().mockResolvedValue(undefined);
    const onCancel = vi.fn();
    render(
      <RestoreDialog
        title="Retry this message"
        changes={changes}
        initialSelection={["app/main.ts"]}
        sharedWith="“Other chat”"
        choices={[{ id: "keep", label: "Keep files" }, { id: "restore", label: "Restore & send", files: true, danger: true }]}
        onChoose={onChoose}
        onCancel={onCancel}
      />
    );
    expect(screen.getByRole("dialog", { name: "Retry this message" })).toBeInTheDocument();
    expect(screen.getByText(/“Other chat” also works in this folder/)).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: /other\/notes\.md/ })).not.toBeChecked();
    expect(screen.getByText("Deleted")).toBeInTheDocument();
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Restore & send" })); });
    expect(onChoose).toHaveBeenCalledWith("restore", ["app/main.ts"]);
    expect(onCancel).toHaveBeenCalled();
  });

  it("needs a selection for file choices, selects everything at once, and shows failures in place", async () => {
    const onChoose = vi.fn().mockRejectedValue("Git took too long to answer.");
    const onCancel = vi.fn();
    render(
      <RestoreDialog
        title="Rewind"
        changes={changes}
        initialSelection={[]}
        choices={[{ id: "files", label: "Files only", files: true }]}
        onChoose={onChoose}
        onCancel={onCancel}
      />
    );
    expect(screen.getByRole("button", { name: "Files only" })).toBeDisabled();
    fireEvent.click(screen.getByRole("checkbox", { name: "2 changed files" }));
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Files only" })); });
    expect(onChoose).toHaveBeenCalledWith("files", ["app/main.ts", "other/notes.md"]);
    expect(screen.getByText("Git took too long to answer.")).toBeInTheDocument();
    expect(onCancel).not.toHaveBeenCalled();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onCancel).toHaveBeenCalled();
  });
});
