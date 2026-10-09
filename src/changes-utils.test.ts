import { describe, expect, it } from "vitest";
import type { DiffComment, GitChangeFile, GitDiffSection } from "./types";
import { changeEntries, hunkLabel, isFresh, lineAnchor, sectionComments, splitPath } from "./changes-utils";

const section: GitDiffSection = {
  layer: "working", revision: "rev-1", diff: "", truncated: false, additions: 2, deletions: 1,
  hunks: [{
    id: 0, header: "@@ -10,3 +10,4 @@", oldStart: 10, newStart: 10,
    lines: [
      { kind: "context", text: " keep", oldLine: 10, newLine: 10 },
      { kind: "deletion", text: "-old", oldLine: 11, newLine: null },
      { kind: "addition", text: "+new", oldLine: null, newLine: 11 },
      { kind: "addition", text: "+extra", oldLine: null, newLine: 12 }
    ]
  }]
};

const file: GitChangeFile = {
  path: "src/deep/file.ts", oldPath: null, status: "modified", staged: false, unstaged: true,
  untracked: false, binary: false, hunkable: true, truncated: false, sections: [section]
};

const comment = (over: Partial<DiffComment>): DiffComment => ({
  id: "1", path: "src/deep/file.ts", layer: "working", side: "new", line: 11,
  excerpt: "+new", revision: "rev-1", text: "check", ...over
});

describe("changes-utils", () => {
  it("flattens files into per-layer entries", () => {
    const both = { ...file, sections: [section, { ...section, layer: "staged" as const }] };
    expect(changeEntries([both])).toHaveLength(2);
  });

  it("splits a path into dimmable directory and basename", () => {
    expect(splitPath("src/deep/file.ts")).toEqual({ dir: "src/deep/", base: "file.ts" });
    expect(splitPath("file.ts")).toEqual({ dir: "", base: "file.ts" });
  });

  it("labels hunks by their new-side range and deletions by old", () => {
    expect(hunkLabel(section.hunks[0])).toBe("Lines 10–12");
    const deletionOnly = { id: 1, header: "@@ -5,2 +4,0 @@", oldStart: 5, newStart: 4, lines: [
      { kind: "deletion", text: "-a", oldLine: 5, newLine: null },
      { kind: "deletion", text: "-b", oldLine: 6, newLine: null }
    ]};
    expect(hunkLabel(deletionOnly)).toBe("Removed lines 5–6");
    const single = { id: 2, header: "@@ -0,0 +7 @@", oldStart: 0, newStart: 7, lines: [
      { kind: "addition", text: "+only", oldLine: null, newLine: 7 }
    ]};
    expect(hunkLabel(single)).toBe("Line 7");
  });

  it("anchors deletions on the old side and everything else on the new", () => {
    expect(lineAnchor(section.hunks[0].lines[1])).toEqual({ side: "old", line: 11 });
    expect(lineAnchor(section.hunks[0].lines[2])).toEqual({ side: "new", line: 11 });
    expect(lineAnchor(section.hunks[0].lines[0])).toEqual({ side: "new", line: 10 });
    expect(lineAnchor({ kind: "meta", text: "\\ No newline", oldLine: null, newLine: null })).toBeNull();
  });

  it("is fresh only while the section revision still exists", () => {
    expect(isFresh(comment({}), file)).toBe(true);
    expect(isFresh(comment({ revision: "old-rev" }), file)).toBe(false);
    expect(isFresh(comment({}), undefined)).toBe(false);
  });

  it("partitions comments into inline groups and orphans", () => {
    const comments = [
      comment({ id: "a" }),
      comment({ id: "b", line: 12 }),
      comment({ id: "c", side: "old" }),
      comment({ id: "d", line: 99 }),
      comment({ id: "e", revision: "stale" }),
      comment({ id: "f", path: "other.ts" })
    ];
    const { inline, orphans } = sectionComments(file.path, section, comments);
    expect(inline.get("new:11")?.map((item) => item.id)).toEqual(["a"]);
    expect(inline.get("new:12")?.map((item) => item.id)).toEqual(["b"]);
    expect(inline.get("old:11")?.map((item) => item.id)).toEqual(["c"]);
    // Same revision but a line that left the diff becomes an orphan.
    expect(orphans.map((item) => item.id)).toEqual(["d"]);
    // Stale revisions and other files are neither inline nor orphan here.
    expect([...inline.values()].flat().length + orphans.length).toBe(4);
  });


});
