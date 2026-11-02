import { describe, expect, it } from "vitest";
import { streamSegments } from "./markdown-segments";

/** Every segment but the last is settled; joining the pieces must return the exact source. */
function joined(source: string): string {
  return streamSegments(source).join("");
}

describe("streamSegments", () => {
  it("leaves a source with no stable boundary as one open tail", () => {
    expect(streamSegments("One paragraph\nstill going")).toEqual(["One paragraph\nstill going"]);
  });

  it("returns nothing for an empty or whitespace-only source", () => {
    expect(streamSegments("")).toEqual([]);
    expect(streamSegments("\n\n")).toEqual([]);
  });

  it("always joins back to the exact source", () => {
    const corpus = [
      "One.\n\nTwo.",
      "One.\n\n\n\nTwo.",
      "# Head\n\n- a\n- b\n\npara `code` [l](https://x)",
      "```ts\nconst x = 1\n\nconst y = 2\n```\n\nafter",
      "~~~\nfence\n~~~\ntail",
      "```\nopen fence\n\nkeeps\n\ngoing",
      "> quote\n\nnext",
      "setext\n===\n\npara",
      "para\n   \n\t\nnext"
    ];
    for (const source of corpus) expect(joined(source)).toBe(source);
  });

  it("cuts at the blank line's start, leaving its whitespace to the tail", () => {
    expect(streamSegments("One.\n\nTwo.")).toEqual(["One.\n", "\nTwo."]);
  });

  it("lets a run of blank lines lead the tail", () => {
    expect(streamSegments("One.\n\n\n\nTwo.")).toEqual(["One.\n", "\n\n\nTwo."]);
  });

  it("treats whitespace-only lines as blank", () => {
    expect(streamSegments("One.\n   \n\t\nTwo.")).toEqual(["One.\n", "   \n\t\nTwo."]);
  });

  it("does not cut while only blank lines have streamed past the last boundary", () => {
    expect(streamSegments("One.\n\n")).toEqual(["One.\n\n"]);
    expect(streamSegments("One.\n\n\n")).toEqual(["One.\n\n\n"]);
  });

  it("cuts several settled blocks while the tail stays open", () => {
    expect(streamSegments("A.\n\nB.\n\nC.")).toEqual(["A.\n", "\nB.\n", "\nC."]);
  });

  it("cuts after a closing fence even without a blank line", () => {
    expect(streamSegments("```ts\ncode\n```\nNext.")).toEqual(["```ts\ncode\n```\n", "Next."]);
  });

  it("never cuts inside an open fence, however blank it gets", () => {
    const source = "```ts\nconst a = 1\n\nconst b = 2\n\nconst c = 3";
    expect(streamSegments(source)).toEqual([source]);
  });

  it("keeps a fence opener that follows blank lines in the open tail", () => {
    expect(streamSegments("Para.\n\n```\ncode")).toEqual(["Para.\n", "\n```\ncode"]);
  });

  it("honours the same fence rules as the settled parser", () => {
    // A closer must match the character and reach the opener's length.
    expect(streamSegments("```\na\n~~~\nb\n```\n\ntail")).toEqual(["```\na\n~~~\nb\n```\n", "\ntail"]);
    // A backtick opener whose info contains a backtick never opens a fence.
    expect(streamSegments("``` foo ` bar\n\nTwo.")).toEqual(["``` foo ` bar\n", "\nTwo."]);
    // Four leading spaces are indented code, not a fence: the blank still cuts.
    expect(streamSegments("    ```\n\ncode")).toEqual(["    ```\n", "\ncode"]);
  });
});
