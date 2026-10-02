import { describe, expect, it } from "vitest";
import { editArgumentDiff, focusedEditDiff } from "./edit-preview";

describe("focusedEditDiff", () => {
  it("focuses a change near the end of a full-file patch", () => {
    const context = Array.from({ length: 100 }, (_, i) => ` line ${i + 1}`);
    const diff = `--- a/file\n+++ b/file\n@@ -1,101 +1,101 @@\n${context.join("\n")}\n-old\n+new\n tail`;
    const preview = focusedEditDiff(diff);
    expect(preview).not.toContain(" line 1\n");
    expect(preview).toContain(" …\n line 98\n line 99\n line 100\n-old\n+new\n tail");
    expect(preview).toContain("--- a/file\n+++ b/file\n@@ -1,101 +1,101 @@");
  });

  it("keeps separate changes and three context lines on either side of a gap", () => {
    const context = Array.from({ length: 20 }, (_, i) => ` line ${i + 1}`);
    expect(focusedEditDiff(`-first\n+first changed\n${context.join("\n")}\n-last\n+last changed`))
      .toBe("-first\n+first changed\n line 1\n line 2\n line 3\n …\n line 18\n line 19\n line 20\n-last\n+last changed");
  });

  it("preserves Pi line numbers and its existing omitted-line markers", () => {
    const diff = "    ...\n  97 before\n  98 before\n  99 before\n-100 old\n+100 new\n 101 after\n    ...";
    expect(focusedEditDiff(diff)).toBe(diff);
  });

  it("keeps small diffs and unrecognized output intact", () => {
    for (const diff of ["@@ -1 +1 @@\n-old\n+new\n", " before\n-old\n+new\n after", "No changes\n more details"]) {
      expect(focusedEditDiff(diff)).toBe(diff);
    }
  });
});

describe("editArgumentDiff", () => {
  it.each([
    ["a\nb\nc", "a\nc"],
    ["a\nc", "a\nb\nc"],
    ["a\nb\na", "a\na\nb"],
    ["a\na\nb", "b\na\na"],
    ["a\nb\nc", "d\ne\nf"],
    ["", "a\nb"],
    ["a\nb", ""]
  ])("preserves both versions when comparing %j to %j", (oldText, newText) => {
    const lines = editArgumentDiff({ oldText, newText })!.split("\n").slice(1);
    expect(lines.filter(line => /^[ -]/.test(line)).map(line => line.slice(1)).join("\n")).toBe(oldText);
    expect(lines.filter(line => /^[ +]/.test(line)).map(line => line.slice(1)).join("\n")).toBe(newText);
  });

  it("finds sparse changes inside whole-file replacements", () => {
    const oldText = Array.from({ length: 10_000 }, (_, i) => `line ${i + 1}`).join("\n");
    const newText = oldText.replace("line 20\n", "changed 20\n").replace("line 9000\n", "changed 9000\n");
    const diff = editArgumentDiff({ edits: [{ oldText, newText }] });
    expect(diff).toContain("-line 20\n+changed 20");
    expect(diff).toContain("-line 9000\n+changed 9000");
    expect(diff).not.toContain("line 1\n");
    expect(diff).not.toContain("line 50\n");
    expect(diff).not.toContain("line 10000");
    expect(diff).toContain(" …");
  });

  it("handles multiple edits and legacy single-edit arguments", () => {
    expect(editArgumentDiff({ edits: [{ oldText: "old", newText: "new" }, { oldText: "remove", newText: "" }] }))
      .toBe("@@ Edit 1 @@\n-old\n+new\n@@ Edit 2 @@\n-remove");
    expect(editArgumentDiff({ oldText: "", newText: "added\n" })).toContain("+added");
    expect(editArgumentDiff({ oldText: "old\r\n", newText: "new\r\n" })).toBe("@@ Edit 1 @@\n-old\n+new");
    expect(editArgumentDiff({ oldText: "same", newText: "same\n" })).toContain("\\ Final newline changed");
  });

  it("falls back to a valid replacement for very large changed blocks", () => {
    const oldText = "old\n".repeat(501);
    const newText = "new\n".repeat(501);
    const diff = editArgumentDiff({ oldText, newText })!;
    expect(diff.split("\n").filter((line) => line === "-old")).toHaveLength(501);
    expect(diff.split("\n").filter((line) => line === "+new")).toHaveLength(501);
  });

  it("leaves malformed or unknown edit arguments to the ordinary tool fallback", () => {
    for (const args of [{}, { edits: [] }, { edits: [null] }, { edits: [{ oldText: "old" }] }]) {
      expect(editArgumentDiff(args)).toBeUndefined();
    }
  });
});
