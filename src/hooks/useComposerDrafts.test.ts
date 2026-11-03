import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { EMPTY_DRAFT, useComposerDrafts, useComposerDraftValue, type ComposerDraft } from "./useComposerDrafts";

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

describe("draft subscriptions", () => {
  it("notifies only the edited composer without rendering the owner, and reads the latest value synchronously", () => {
    let ownerRenders = 0;
    const owner = renderHook(() => { ownerRenders++; return useComposerDrafts(); });
    const a = owner.result.current.forChat("a");
    const b = owner.result.current.forChat("b");
    expect(owner.result.current.forChat("a")).toBe(a);
    let aRenders = 0;
    let bRenders = 0;
    const first = renderHook(() => { aRenders++; return useComposerDraftValue(a); });
    renderHook(() => { bRenders++; return useComposerDraftValue(b); });
    act(() => {
      a.update((current) => ({ ...current, text: "First" }));
      a.update((current) => ({ ...current, text: `${current.text} second` }));
      expect(owner.result.current.get("a").text).toBe("First second");
    });
    expect(first.result.current?.text).toBe("First second");
    expect(ownerRenders).toBe(1);
    expect(aRenders).toBe(2);
    expect(bRenders).toBe(1);
    act(() => a.update((current) => current));
    expect(aRenders).toBe(2);
    first.unmount();
    act(() => owner.result.current.remove("a"));
    act(() => a.update({ ...EMPTY_DRAFT, text: "Late" }));
    expect(owner.result.current.get("a")).toBe(EMPTY_DRAFT);
  });
});
