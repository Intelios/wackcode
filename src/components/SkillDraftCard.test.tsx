import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SkillDraftStatus, SkillPreviewDetails } from "../types";
import { SkillDraftCard } from "./SkillDraftCard";

afterEach(cleanup);

const details: SkillPreviewDetails = {
  v: 1,
  source: "skill_creator_preview",
  ownerTaskId: "task-1",
  draftId: "draft-1",
  revision: "rev-1",
  name: "pdf-tools",
  description: "Extract text from PDFs.",
  manual: false,
  target: "new",
  bodyPreview: "# PDF tools\n\nRead the references first.",
  bodyTruncated: false,
  files: ["scripts/extract.sh"],
  fileCount: 2,
  totalBytes: 2048,
  warnings: [],
};

const status = (patch: Partial<SkillDraftStatus> = {}): SkillDraftStatus => ({
  draftId: "draft-1",
  name: "pdf-tools",
  draftRoot: "/agent/task-1/skill-creator/draft-1",
  state: "ready",
  revision: "rev-1",
  overwritten: false,
  ...patch,
});

function card(patch: Partial<Parameters<typeof SkillDraftCard>[0]> = {}) {
  return render(
    <SkillDraftCard
      details={details}
      status={status()}
      current
      owned
      {...patch}
    />
  );
}

describe("SkillDraftCard", () => {
  it("shows the reviewed draft with its Save action", () => {
    card();
    expect(screen.getByText("pdf-tools")).toBeInTheDocument();
    expect(screen.getByText("Extract text from PDFs.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save skill" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Open draft folder" })).toBeEnabled();
    expect(screen.getByText("Nothing is installed until you save. Feedback below revises the draft.")).toBeInTheDocument();
  });

  it("labels updates and copies differently, and disables while busy, saving, or unhydrated", () => {
    cleanup();
    card({ details: { ...details, target: "library-update", originLabel: "Your skills" } });
    expect(screen.getByRole("button", { name: "Update skill" })).toBeEnabled();
    expect(screen.getByText("Updates the Your skills copy")).toBeInTheDocument();

    cleanup();
    card({ busy: true });
    expect(screen.getByRole("button", { name: "Save skill" })).toBeDisabled();
    cleanup();
    card({ saving: true });
    expect(screen.getByRole("button", { name: "Saving…" })).toBeDisabled();
    cleanup();
    card({ status: undefined });
    expect(screen.getByRole("button", { name: "Save skill" })).toBeDisabled();
  });

  it("save publishes the reviewed revision through the app", () => {
    const onAction = vi.fn();
    card({ onAction });
    fireEvent.click(screen.getByRole("button", { name: "Save skill" }));
    expect(onAction).toHaveBeenCalledWith({ type: "save", draftId: "draft-1", revision: "rev-1", name: "pdf-tools" });
  });

  it("shows the saved state with its library path", () => {
    card({ status: status({ state: "saved", path: "~/.agents/skills/pdf-tools", overwritten: true }) });
    expect(screen.getByText("Saved")).toBeInTheDocument();
    expect(screen.getByText("~/.agents/skills/pdf-tools")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save skill" })).not.toBeInTheDocument();
  });

  it("explains stale revisions and interrupted saves instead of offering Save", () => {
    cleanup();
    card({ status: status({ state: "stale" }) });
    expect(screen.getByText(/changed since this review/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save skill" })).not.toBeInTheDocument();

    cleanup();
    card({ status: status({ state: "unknown" }) });
    expect(screen.getByText(/last save was interrupted/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save skill" })).not.toBeInTheDocument();

    // A hydrated newer revision also means this card is outdated.
    cleanup();
    card({ status: status({ revision: "rev-2" }) });
    expect(screen.getByText(/changed since this review/i)).toBeInTheDocument();
  });

  it("keeps a forked chat's card read-only", () => {
    card({ owned: false });
    expect(screen.getByText(/belongs to the chat that created it/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save skill" })).not.toBeInTheDocument();
  });

  it("marks superseded previews inert", () => {
    card({ current: false });
    expect(screen.getByText("Replaced by a newer preview.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save skill" })).not.toBeInTheDocument();
  });

  it("loads the full instructions on demand", async () => {
    const loadDocument = vi.fn().mockResolvedValue("# PDF tools\n\nThe complete instructions.");
    card({ loadDocument });
    fireEvent.click(screen.getByRole("button", { name: "View full instructions" }));
    await waitFor(() => expect(screen.getByText("The complete instructions.")).toBeInTheDocument());
    expect(loadDocument).toHaveBeenCalledWith("draft-1");
    fireEvent.click(screen.getByRole("button", { name: "Hide full instructions" }));
    expect(screen.queryByText("The complete instructions.")).not.toBeInTheDocument();
  });
});
