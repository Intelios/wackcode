import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GitSyncStatus } from "../types";
import { GitSyncButton } from "./GitSyncButton";

afterEach(cleanup);

const now = new Date("2026-09-29T12:00:00Z");

function sync(patch: Partial<GitSyncStatus> = {}): GitSyncStatus {
  return { branch: "main", upstream: "origin/main", remotes: ["origin"], fetchRemote: "origin", ahead: 0, behind: 0, fetchedAt: "2026-09-29T11:56:00Z", head: "abc", hasHead: true, ...patch };
}

function callbacks() {
  return { onFetch: vi.fn(), onPull: vi.fn(), onPush: vi.fn(), onPublish: vi.fn(), onCreatePr: vi.fn(), onDismissError: vi.fn() };
}

describe("GitSyncButton", () => {
  it("fetches when the branch is level, saying when it last did", () => {
    const actions = callbacks();
    render(<GitSyncButton sync={sync()} now={now} {...actions} />);
    fireEvent.click(screen.getByRole("button", { name: "Fetch origin. Fetched 4m ago" }));
    expect(actions.onFetch).toHaveBeenCalled();
  });

  it("offers Pull when behind, Push when ahead and Publish with no upstream", () => {
    const actions = callbacks();
    const { rerender } = render(<GitSyncButton sync={sync({ behind: 3 })} now={now} {...actions} />);
    fireEvent.click(screen.getByRole("button", { name: /^Pull origin/ }));
    expect(actions.onPull).toHaveBeenCalled();
    expect(screen.getByText("↓3")).toBeInTheDocument();

    rerender(<GitSyncButton sync={sync({ ahead: 2 })} now={now} {...actions} />);
    fireEvent.click(screen.getByRole("button", { name: /^Push origin/ }));
    expect(actions.onPush).toHaveBeenCalled();
    expect(screen.getByText("↑2")).toBeInTheDocument();

    rerender(<GitSyncButton sync={sync({ upstream: null })} now={now} {...actions} />);
    fireEvent.click(screen.getByRole("button", { name: /^Publish branch/ }));
    expect(actions.onPublish).toHaveBeenCalledWith("origin");
  });

  it("is inert without a remote and while Git is talking to one", () => {
    const actions = callbacks();
    const { rerender } = render(<GitSyncButton sync={sync({ remotes: [], fetchRemote: null, upstream: null })} now={now} {...actions} />);
    expect(screen.getByRole("button", { name: /^No remote/ })).toBeDisabled();
    rerender(<GitSyncButton sync={sync({ ahead: 1 })} network={{ kind: "push", background: false }} now={now} {...actions} />);
    expect(screen.getByRole("button", { name: /^Pushing origin/ })).toBeDisabled();
  });

  it("won't pull under a running chat, but still fetches from the menu", () => {
    const actions = callbacks();
    render(<GitSyncButton sync={sync({ behind: 1 })} busyReason="Wait for Fix auth to finish before changing Git files" now={now} {...actions} />);
    expect(screen.getByRole("button", { name: /^Pull origin/ })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "More sync actions" }));
    expect(screen.getByRole("menuitem", { name: /^Pull/ })).toBeDisabled();
    fireEvent.click(screen.getByRole("menuitem", { name: "Fetch origin" }));
    expect(actions.onFetch).toHaveBeenCalled();
  });

  it("opens the pull request form from the menu and shows a clicked action's failure", () => {
    const actions = callbacks();
    render(<GitSyncButton sync={sync()} error={{ message: "Could not fetch from origin: offline\nmore detail", background: false }} now={now} {...actions} />);
    expect(screen.getByRole("alert")).toHaveTextContent("Could not fetch from origin: offline");
    fireEvent.click(screen.getByRole("button", { name: "Dismiss sync error" }));
    expect(actions.onDismissError).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "More sync actions" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Create pull request…" }));
    expect(actions.onCreatePr).toHaveBeenCalled();
  });
});
