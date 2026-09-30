import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { EMPTY_DRAFT, useComposerDrafts, type ComposerDraft } from "./useComposerDrafts";

describe("useComposerDrafts", () => {
  it("hands the welcome draft to a prepared chat without linking subsequent edits", () => {
    const { result } = renderHook(useComposerDrafts);
    const welcome: ComposerDraft = {
      text: "/skill arguments",
      images: [{ type: "image", mimeType: "image/png", data: "test" }],
      files: [{ name: "notes.txt", text: "Notes" }]
    };
    act(() => result.current.update("new:0", welcome));
    act(() => result.current.update("task:a", result.current.get("new:0")));
    expect(result.current.get("task:a")).toEqual(welcome);
    act(() => result.current.forChat("task:a").update(EMPTY_DRAFT));
    expect(result.current.get("task:a")).toEqual(EMPTY_DRAFT);
    expect(result.current.get("new:0")).toEqual(welcome);
  });

  it("drops deleted drafts and ignores late operations holding their updater", () => {
    const { result } = renderHook(useComposerDrafts);
    act(() => result.current.update("task:a", { ...EMPTY_DRAFT, text: "Old draft" }));
    const pendingUpdate = result.current.forChat("task:a").update;
    act(() => result.current.remove("task:a"));
    act(() => pendingUpdate({ ...EMPTY_DRAFT, files: [{ name: "late.txt", text: "Late" }] }));
    expect(result.current.get("task:a")).toEqual(EMPTY_DRAFT);
  });
});
