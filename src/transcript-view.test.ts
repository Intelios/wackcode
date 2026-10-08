import { describe, expect, it } from "vitest";
import { captureScroll, restoreScroll } from "./transcript-view";

function scroller(rows: [string, number][], height = 2000) {
  const el = document.createElement("div");
  Object.defineProperties(el, { clientHeight: { value: 300 }, scrollHeight: { value: height } });
  el.getBoundingClientRect = () => ({ top: 50 } as DOMRect);
  for (const [id, offset] of rows) {
    const row = document.createElement("div");
    row.dataset.transcriptAnchor = id;
    row.getBoundingClientRect = () => ({ top: 50 + offset - el.scrollTop } as DOMRect);
    el.append(row);
  }
  return el;
}

describe("transcript reading memory", () => {
  it("restores the reading row after preceding content changes height", () => {
    const before = scroller([["u", 100], ["reading", 700], ["later", 1200]]);
    before.scrollTop = 675;
    const saved = captureScroll(before, false);
    const after = scroller([["u", 100], ["reading", 400], ["later", 900]]);
    restoreScroll(after, saved);
    expect(after.scrollTop).toBe(375);
  });
  it("uses the nearest surviving row after a branch/fold removes the preferred anchor", () => {
    const before = scroller([["previous", 200], ["removed", 500], ["next", 700]]);
    before.scrollTop = 490;
    const after = scroller([["previous", 200], ["next", 400]]);
    restoreScroll(after, captureScroll(before, false));
    expect(after.scrollTop).toBe(190);
  });
  it("follows background output only when the tab was following latest", () => {
    const before = scroller([["u", 100]]);
    before.scrollTop = 1700;
    const after = scroller([["u", 100]], 3000);
    restoreScroll(after, captureScroll(before, true));
    expect(after.scrollTop).toBe(2700);
  });
  it("clamps the scroll fallback when no anchors survive", () => {
    const before = scroller([["gone", 1900]]);
    before.scrollTop = 1500;
    const after = scroller([], 600);
    restoreScroll(after, captureScroll(before, false));
    expect(after.scrollTop).toBe(300);
  });
});
