import { describe, expect, it } from "vitest";
import { MAX_VISUAL_SEND_CHARS, VISUAL_ACTION_CLOSE, VISUAL_ACTION_OPEN, composeVisualAction, splitVisualAction } from "./visual-action";

describe("composeVisualAction", () => {
  it("appends the versioned tail the model reads", () => {
    expect(composeVisualAction("Pick option B", "Latency by region")).toBe(
      `Pick option B\n\n${VISUAL_ACTION_OPEN}\nSent from the visual "Latency by region".\n${VISUAL_ACTION_CLOSE}`
    );
  });

  it("keeps the title on one line, without quotes that would end its phrase", () => {
    const composed = composeVisualAction("Go", 'He said "hi"\nnow');
    expect(composed).toContain('Sent from the visual "He said  hi now".');
    expect(splitVisualAction(composed).title).toBe("He said  hi now");
  });

  it("trims the words and caps them", () => {
    const composed = composeVisualAction(`  ${"x".repeat(MAX_VISUAL_SEND_CHARS + 50)}  `, "Big");
    expect(splitVisualAction(composed).text).toHaveLength(MAX_VISUAL_SEND_CHARS);
  });

  it("still sends when the visual has no title", () => {
    expect(composeVisualAction("Go", "   ")).toContain("Sent from a visual.");
  });
});

describe("splitVisualAction", () => {
  it("round-trips a message a visual sent", () => {
    expect(splitVisualAction(composeVisualAction("Pick option B", "Latency by region"))).toEqual({
      text: "Pick option B",
      title: "Latency by region",
    });
  });

  it("leaves a typed message untouched", () => {
    expect(splitVisualAction("just words")).toEqual({ text: "just words" });
    // A tail that is not at the end of the message is the user's own text.
    expect(splitVisualAction("words\n\n<visual-action-v1>\nand more")).toEqual({ text: "words\n\n<visual-action-v1>\nand more" });
  });

  it("splits a message edited down to its tail", () => {
    expect(splitVisualAction(`${VISUAL_ACTION_OPEN}\nSent from the visual "Chart".\n${VISUAL_ACTION_CLOSE}`)).toEqual({
      text: "",
      title: "Chart",
    });
  });

  it("keeps the words when the tail names no visual", () => {
    expect(splitVisualAction(`Go\n\n${VISUAL_ACTION_OPEN}\nSent from a visual.\n${VISUAL_ACTION_CLOSE}`)).toEqual({ text: "Go" });
  });
});
