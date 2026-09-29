import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ComputerUseStatus } from "../types";
import { ComputerUseSection, type ComputerUseActions } from "./ComputerUseSection";

afterEach(cleanup);

const base: ComputerUseStatus = { supported: true, accessibility: true, screenRecording: true, hotkeyAvailable: true, devBuild: false };

function setup(status: Partial<ComputerUseStatus> = {}, neverAllow: string[] = []) {
  const actions: ComputerUseActions = {
    onStatus: vi.fn().mockResolvedValue({ ...base, ...status }),
    onRequestPermission: vi.fn().mockResolvedValue(undefined),
    onOpenSettings: vi.fn().mockResolvedValue(undefined),
    onResetPermissions: vi.fn().mockResolvedValue(undefined),
    onRelaunch: vi.fn().mockResolvedValue(undefined),
    onListApps: vi.fn().mockResolvedValue([{ name: "Notes", bundleId: "com.apple.Notes" }])
  };
  const onChange = vi.fn().mockResolvedValue(undefined);
  render(<ComputerUseSection config={{ enabled: true, neverAllow }} actions={actions} agentName="Nova" onChange={onChange} />);
  return { actions, onChange };
}

describe("ComputerUseSection", () => {
  it("says it is ready once both permissions are allowed", async () => {
    setup();
    expect(await screen.findByText("Ready")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Nova can use the apps you build" })).toBeInTheDocument();
    expect(screen.getAllByText("Allowed")).toHaveLength(2);
  });

  it("counts the permissions still to grant", async () => {
    setup({ screenRecording: false });
    expect(await screen.findByText("1 permission to go")).toBeInTheDocument();
    cleanup();
    setup({ accessibility: false, screenRecording: false });
    expect(await screen.findByText("2 permissions to go")).toBeInTheDocument();
  });

  it("warns when another app holds the stop shortcut", async () => {
    setup({ hotkeyAvailable: false });
    expect(await screen.findByRole("alert")).toHaveTextContent("Another app is using");
    expect(screen.getByRole("img", { name: "Control Option Command Period" })).toBeInTheDocument();
  });

  it("lists the always-blocked apps and an empty list of your own", async () => {
    setup();
    expect(await screen.findByRole("list", { name: "Always blocked" })).toHaveTextContent("Password managers");
    expect(screen.getByText(/Nothing added yet/)).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "Apps you never allow" })).toBeNull();
  });

  it("adds and removes apps through onChange", async () => {
    const { onChange } = setup({}, ["com.apple.Notes"]);
    expect(await screen.findByText("Notes")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Allow asking for Notes again" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith({ enabled: true, neverAllow: [] }));
    fireEvent.change(screen.getByRole("textbox", { name: "Bundle id to never allow" }), { target: { value: "com.example.Secret" } });
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Bundle id to never allow" }), { key: "Enter" });
    await waitFor(() => expect(onChange).toHaveBeenLastCalledWith({ enabled: true, neverAllow: ["com.apple.Notes", "com.example.Secret"] }));
  });

  it("rechecks the permissions on demand", async () => {
    const { actions } = setup();
    await screen.findByText("Ready");
    const calls = vi.mocked(actions.onStatus).mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: /Recheck/ }));
    await waitFor(() => expect(vi.mocked(actions.onStatus).mock.calls.length).toBeGreaterThan(calls));
  });
});
