import { afterEach, describe, expect, it } from "vitest";
import { BEST_SCORES_KEY, bestScore, recordScore } from "./scores";

afterEach(() => localStorage.clear());

describe("best scores", () => {
  it("only ever goes up", () => {
    expect(bestScore("quack")).toBe(0);
    expect(recordScore("quack", 50)).toEqual({ best: 50, isBest: true });
    expect(recordScore("quack", 30)).toEqual({ best: 50, isBest: false });
    expect(recordScore("quack", 50)).toEqual({ best: 50, isBest: false });
    expect(bestScore("quack")).toBe(50);
  });

  it("reads corrupt or foreign storage as no score", () => {
    localStorage.setItem(BEST_SCORES_KEY, "{not json");
    expect(bestScore("quack")).toBe(0);
    localStorage.setItem(BEST_SCORES_KEY, JSON.stringify({ quack: "lots" }));
    expect(bestScore("quack")).toBe(0);
    localStorage.setItem(BEST_SCORES_KEY, JSON.stringify([1, 2]));
    expect(recordScore("quack", 5)).toEqual({ best: 5, isBest: true });
  });

  it("keeps the pre-rename Swarm best until a new one beats it", () => {
    localStorage.setItem(BEST_SCORES_KEY, JSON.stringify({ swarm: 120 }));
    expect(bestScore("quack")).toBe(120);
    expect(recordScore("quack", 30)).toEqual({ best: 120, isBest: false });
    expect(recordScore("quack", 140)).toEqual({ best: 140, isBest: true });
    expect(bestScore("quack")).toBe(140);
  });
});
