import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProjectRecord, RunInfo } from "../types";
import { RunButton } from "./RunButton";

afterEach(cleanup);
const project: ProjectRecord = { id: "p", name: "Project", path: "/project", gitRoot: null, gitHasHead: false, runCommand: null, branch: null, createdAt: "now" };
const run: RunInfo = { sessionId: "one", generation: 1, revision: 1, cwd: "/tree", command: "pnpm dev", status: "running", exit: null };
function props() {
  return { project, workspacePath: "/tree", onSave: vi.fn(async () => {}), onRun: vi.fn(async () => {}), onStop: vi.fn(async () => {}), onShowOutput: vi.fn() };
}

describe("Run control", () => {
  it("opens setup on first use and saves before launching", async () => {
    const actions = props();
    render(<RunButton {...actions} />);
    fireEvent.click(screen.getByRole("button", { name: "Run" }));
    expect(screen.getByRole("dialog", { name: "Project run command" })).toBeInTheDocument();
    expect(screen.getByLabelText("Command")).toHaveFocus();
    expect(screen.getByRole("button", { name: "Save & Run" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Command"), { target: { value: "pnpm dev" } });
    fireEvent.click(screen.getByRole("button", { name: "Save & Run" }));
    await waitFor(() => expect(actions.onRun).toHaveBeenCalledOnce());
    expect(actions.onSave).toHaveBeenCalledWith("pnpm dev");
    expect(actions.onSave.mock.invocationCallOrder[0]).toBeLessThan(actions.onRun.mock.invocationCallOrder[0]);
  });

  it("launches a configured command, disabling repeat clicks while starting", async () => {
    const actions = props();
    let resolve!: () => void;
    actions.onRun.mockImplementation(() => new Promise<void>((done) => { resolve = done; }));
    render(<RunButton {...actions} project={{ ...project, runCommand: "pnpm dev" }} />);
    fireEvent.click(screen.getByRole("button", { name: "Run" }));
    expect(screen.getByRole("button", { name: "Starting…" })).toBeDisabled();
    expect(actions.onRun).toHaveBeenCalledOnce();
    await act(async () => resolve());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("stops the shared run and lets edits apply only to the next launch", async () => {
    const actions = props();
    render(<RunButton {...actions} run={run} project={{ ...project, runCommand: "pnpm dev" }} />);
    fireEvent.click(screen.getByRole("button", { name: "Configure run command" }));
    expect(screen.queryByRole("button", { name: "Save & Run" })).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Command"), { target: { value: "pnpm test" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(actions.onSave).toHaveBeenCalledWith("pnpm test"));
    expect(actions.onRun).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(actions.onStop).toHaveBeenCalledOnce());
  });

  it("reopens output, permits clearing configuration, and closes setup with Escape", async () => {
    const actions = props();
    render(<RunButton {...actions} run={{ ...run, status: "finished" }} project={{ ...project, runCommand: "pnpm dev" }} />);
    fireEvent.click(screen.getByRole("button", { name: "Configure run command" }));
    fireEvent.click(screen.getByRole("button", { name: "Show output" }));
    expect(actions.onShowOutput).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "Configure run command" }));
    fireEvent.change(screen.getByLabelText("Command"), { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(actions.onSave).toHaveBeenCalledWith(""));
    fireEvent.click(screen.getByRole("button", { name: "Configure run command" }));
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Configure run command" })).toHaveFocus();
  });

  it("shows errors without launching after a failed save", async () => {
    const actions = props();
    actions.onSave.mockRejectedValue(new Error("Could not save."));
    render(<RunButton {...actions} />);
    fireEvent.click(screen.getByRole("button", { name: "Run" }));
    fireEvent.change(screen.getByLabelText("Command"), { target: { value: "pnpm dev" } });
    fireEvent.click(screen.getByRole("button", { name: "Save & Run" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save.");
    expect(actions.onRun).not.toHaveBeenCalled();
  });
});
