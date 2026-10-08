import { afterEach, describe, expect, it } from "vitest";
import { BEST_SCORES_KEY, bestScore, recordScore } from "./scores";

afterEach(() => localStorage.clear());

describe("best scores", () => {
  it("only ever goes up", () => {
    expect(bestScore("swarm")).toBe(0);
    expect(recordScore("swarm", 50)).toEqual({ best: 50, isBest: true });
    expect(recordScore("swarm", 30)).toEqual({ best: 50, isBest: false });
    expect(recordScore("swarm", 50)).toEqual({ best: 50, isBest: false });
    expect(bestScore("swarm")).toBe(50);
  });

  it("reads corrupt or foreign storage as no score", () => {
    localStorage.setItem(BEST_SCORES_KEY, "{not json");
    expect(bestScore("swarm")).toBe(0);
    localStorage.setItem(BEST_SCORES_KEY, JSON.stringify({ swarm: "lots" }));
    expect(bestScore("swarm")).toBe(0);
    localStorage.setItem(BEST_SCORES_KEY, JSON.stringify([1, 2]));
    expect(recordScore("swarm", 5)).toEqual({ best: 5, isBest: true });
  });
});
