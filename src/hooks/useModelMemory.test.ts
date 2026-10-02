import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ModelChoice } from "../model-utils";
import type { ProviderRecord } from "../types";
import { useModelMemory } from "./useModelMemory";

const providers: ProviderRecord[] = [{
  id: "p", name: "Provider", kind: "custom", baseUrl: "", apiFormat: "openai-completions",
  createdAt: "", updatedAt: "", hasApiKey: true, connected: true,
  models: ["first", "preferred"].map((id) => ({
    id, name: id, contextWindow: 10, maxTokens: 5, reasoning: true,
    thinkingLevels: ["low", "medium", "high"], thinkingLevelMap: {}, vision: false
  }))
}];
const preferred: ModelChoice = { providerId: "p", modelId: "preferred", thinkingLevel: "high" };

beforeEach(() => localStorage.clear());

describe("new-chat model memory", () => {
  it("carries a pick to every project, projectless chats and the next launch without a send", () => {
    const { result, unmount } = renderHook(() => useModelMemory(providers));
    act(() => result.current.rememberModel(preferred));
    for (const project of ["project-a", "project-b", null]) {
      expect(result.current.defaultChoice(project)).toEqual(preferred);
    }
    unmount();
    const relaunched = renderHook(() => useModelMemory(providers));
    expect(relaunched.result.current.defaultChoice("project-c")).toEqual(preferred);
  });

  it("lets later usage replace the pick, including the reasoning level", () => {
    const { result } = renderHook(() => useModelMemory(providers));
    act(() => result.current.rememberModel(preferred));
    const used = { ...preferred, modelId: "first", thinkingLevel: "low" as const };
    act(() => result.current.rememberModel(used));
    expect(result.current.defaultChoice(null)).toEqual(used);
  });

  it("preserves old per-project preferences until the first global pick", () => {
    localStorage.setItem("wackcode:lastModel", JSON.stringify({ "project-a": preferred, none: preferred }));
    const { result } = renderHook(() => useModelMemory(providers));
    expect(result.current.defaultChoice("project-a")).toEqual(preferred);
    expect(result.current.defaultChoice(null)).toEqual(preferred);
    expect(result.current.defaultChoice("project-b")?.modelId).toBe("first");
    const picked = { ...preferred, modelId: "first" };
    act(() => result.current.rememberModel(picked));
    expect(result.current.defaultChoice("project-a")).toEqual(picked);
  });

  it("keeps the preference while its connection is unavailable and restores it when available", () => {
    const { result, rerender } = renderHook(({ available }) => useModelMemory(available), { initialProps: { available: providers } });
    act(() => result.current.rememberModel(preferred));
    rerender({ available: [{ ...providers[0], connected: false }] });
    expect(result.current.defaultChoice(null)).toBeUndefined();
    rerender({ available: providers });
    expect(result.current.defaultChoice(null)).toEqual(preferred);
  });

  it("handles malformed saved JSON", () => {
    localStorage.setItem("wackcode:lastModelChoice", "{");
    localStorage.setItem("wackcode:lastModel", "null");
    const { result } = renderHook(() => useModelMemory(providers));
    expect(result.current.defaultChoice(null)?.modelId).toBe("first");
  });

  it("keeps the current pick even when storage is unavailable", () => {
    const { result } = renderHook(() => useModelMemory(providers));
    const write = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("Unavailable"); });
    try {
      act(() => result.current.rememberModel(preferred));
      expect(result.current.defaultChoice(null)).toEqual(preferred);
    } finally { write.mockRestore(); }
  });
});
