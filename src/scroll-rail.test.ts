import { describe, expect, it } from "vitest";
import { currentTurnIndex, hasOverflow, readLineY, scrollTopFor, scrollTopForRailPoint, tickOffset, turnExcerpt } from "./scroll-rail";

const metrics = { scrollTop: 0, clientHeight: 500, scrollHeight: 2000, railHeight: 400 };

describe("hasOverflow", () => {
  it("is false until content exceeds the viewport by more than a rounding pixel", () => {
    expect(hasOverflow(500, 500)).toBe(false);
    expect(hasOverflow(501, 500)).toBe(false);
    expect(hasOverflow(502, 500)).toBe(true);
  });
});

describe("readLineY", () => {
  it("maps the viewport's bottom edge onto the rail", () => {
    // The first viewport covers 500 of 2000 → 100px on a 400px rail.
    expect(readLineY(metrics)).toBe(100);
    // At the bottom the whole line is lit.
    expect(readLineY({ ...metrics, scrollTop: 1500 })).toBe(400);
  });
});

describe("scrollTopForRailPoint", () => {
  it("centres the viewport on the scrubbed rail position", () => {
    // Halfway down the rail → content 1000, viewport 500 → scrollTop 750.
    expect(scrollTopForRailPoint(200, 400, 2000, 500)).toBe(750);
  });

  it("clamps to the scrollable range", () => {
    expect(scrollTopForRailPoint(-10, 400, 2000, 500)).toBe(0);
    expect(scrollTopForRailPoint(10_000, 400, 2000, 500)).toBe(1500);
    expect(scrollTopForRailPoint(100, 0, 2000, 500)).toBe(0);
  });
});

describe("tickOffset", () => {
  it("maps content offsets onto the rail, clamped", () => {
    expect(tickOffset(0, 2000, 400)).toBe(0);
    expect(tickOffset(1000, 2000, 400)).toBe(200);
    expect(tickOffset(2000, 2000, 400)).toBe(400);
    expect(tickOffset(5000, 2000, 400)).toBe(400);
    expect(tickOffset(-10, 2000, 400)).toBe(0);
  });
});

describe("currentTurnIndex", () => {
  const tops = [0, 800, 1600, 2400];

  it("highlights the last turn whose top passed the reading line", () => {
    // Viewport is 500 tall; the line sits at scrollTop + 175.
    expect(currentTurnIndex(tops, 0, 500)).toBe(0);
    expect(currentTurnIndex(tops, 700, 500)).toBe(1);
    expect(currentTurnIndex(tops, 1600, 500)).toBe(2);
  });

  it("returns -1 before the first turn and clamps at the last", () => {
    expect(currentTurnIndex([900], 0, 500)).toBe(-1);
    expect(currentTurnIndex(tops, 5000, 500)).toBe(3);
  });
});

describe("scrollTopFor", () => {
  it("aims just above the turn's top, clamped to the scrollable range", () => {
    expect(scrollTopFor(800, 2000, 500)).toBe(780);
    expect(scrollTopFor(10, 2000, 500)).toBe(0);
    expect(scrollTopFor(1990, 2000, 500)).toBe(1500);
  });
});

describe("turnExcerpt", () => {
  it("collapses whitespace and truncates with an ellipsis", () => {
    expect(turnExcerpt("  hello\n\tworld  ")).toBe("hello world");
    const long = "word ".repeat(30).trim();
    const excerpt = turnExcerpt(long, 60);
    expect(excerpt.endsWith("…")).toBe(true);
    expect(excerpt.length).toBeLessThanOrEqual(60);
  });

  it("passes short text through unchanged", () => {
    expect(turnExcerpt("fix the bug")).toBe("fix the bug");
    expect(turnExcerpt("")).toBe("");
  });
});
