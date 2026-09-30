import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ComponentProps } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { GitWorkspace } from "./GitWorkspace";

afterEach(cleanup);

function renderClean(patch: Partial<ComponentProps<typeof GitWorkspace>> = {}) {
  const onOpenRepo = vi.fn();
  const props: ComponentProps<typeof GitWorkspace> = {
    isGit: true,
    loading: false,
    comments: [],
    disabled: false,
    layout: "unified",
    direction: 1,
    agentName: "WackCode",
    onComments: vi.fn(),
    onDiscard: vi.fn(),
    onPush: vi.fn(),
    onPull: vi.fn(),
    onPublish: vi.fn(),
    onCreatePr: vi.fn(),
    onOpenRepo,
    onOpenChat: vi.fn(),
    onReveal: vi.fn(),
    onAskIntegrate: vi.fn(),
    onDismissDivergence: vi.fn(),
    onDismissError: vi.fn(),
    ...patch
  };
  render(<GitWorkspace {...props} />);
  return { onOpenRepo };
}

describe("GitWorkspace clean state", () => {
  it("opens the repository on GitHub from the clean tree", () => {
    const { onOpenRepo } = renderClean({ repoUrl: "https://github.com/owner/repo" });
    fireEvent.click(screen.getByRole("button", { name: "Open in GitHub" }));
    expect(onOpenRepo).toHaveBeenCalledWith("https://github.com/owner/repo");
  });

  it("names other hosts instead of GitHub", () => {
    renderClean({ repoUrl: "https://gitlab.com/owner/repo" });
    expect(screen.getByRole("button", { name: "Open on gitlab.com" })).toBeInTheDocument();
  });

  it("shows no web card when the remote has no page", () => {
    renderClean({ repoUrl: null });
    expect(screen.queryByRole("button", { name: /^Open in GitHub|^Open on / })).toBeNull();
    expect(screen.getByRole("button", { name: "Reveal in Finder" })).toBeInTheDocument();
  });
});
