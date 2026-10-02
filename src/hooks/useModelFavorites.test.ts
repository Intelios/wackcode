import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { ModelRef } from "../types";
import { useModelFavorites } from "./useModelFavorites";

vi.mock("../api", () => ({ api: { setModelFavorite: vi.fn() } }));

const first: ModelRef = { providerId: "p", modelId: "first" };
const second: ModelRef = { providerId: "q", modelId: "second" };

beforeEach(() => vi.mocked(api.setModelFavorite).mockReset());

describe("shared favourite saves", () => {
  it("serializes rapid requests and stays busy until every queued save has finished", async () => {
    let finishFirst!: (models: ModelRef[]) => void;
    let finishSecond!: (models: ModelRef[]) => void;
    vi.mocked(api.setModelFavorite)
      .mockImplementationOnce(() => new Promise((resolve) => { finishFirst = resolve; }))
      .mockImplementationOnce(() => new Promise((resolve) => { finishSecond = resolve; }));
    const onSaved = vi.fn();
    const { result } = renderHook(() => useModelFavorites(onSaved, vi.fn()));
    let saveFirst!: Promise<void>;
    let saveSecond!: Promise<void>;
    await act(async () => {
      saveFirst = result.current.setModelFavorite(first, true);
      saveSecond = result.current.setModelFavorite(second, true);
    });
    expect(result.current.favoriteSaving).toBe(true);
    expect(api.setModelFavorite).toHaveBeenCalledTimes(1);
    await act(async () => { finishFirst([first]); await saveFirst; });
    expect(api.setModelFavorite).toHaveBeenLastCalledWith(second, true);
    expect(result.current.favoriteSaving).toBe(true);
    await act(async () => { finishSecond([first, second]); await saveSecond; });
    expect(onSaved.mock.calls).toEqual([[[first]], [[first, second]]]);
    expect(result.current.favoriteSaving).toBe(false);
  });

  it("reports a failed save without replacing the list and accepts the next save", async () => {
    vi.mocked(api.setModelFavorite).mockRejectedValueOnce("Could not save favourites.").mockResolvedValueOnce([second]);
    const onSaved = vi.fn();
    const onError = vi.fn();
    const { result } = renderHook(() => useModelFavorites(onSaved, onError));
    await act(async () => {
      await expect(result.current.setModelFavorite(first, true)).rejects.toBe("Could not save favourites.");
    });
    expect(onSaved).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalledWith("Could not save favourites.");
    expect(result.current.favoriteSaving).toBe(false);
    await act(async () => { await result.current.setModelFavorite(second, true); });
    expect(onSaved).toHaveBeenCalledWith([second]);
    expect(result.current.favoriteSaving).toBe(false);
  });
});
