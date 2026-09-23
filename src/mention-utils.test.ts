import { describe, expect, it } from "vitest";
import { activeMention, mentionValue, rankMentions, splitMentions } from "./mention-utils";

describe("activeMention", () => {
  it("finds the @ token the caret is in", () => {
    expect(activeMention("look at @rea", 12)).toEqual({ start: 8, end: 12, query: "rea", quoted: false });
    expect(activeMention("@", 1)).toEqual({ start: 0, end: 1, query: "", quoted: false });
    expect(activeMention("(@src/ma", 8)).toEqual({ start: 1, end: 8, query: "src/ma", quoted: false });
  });

  it("covers the rest of the token after the caret", () => {
    expect(activeMention("@readme now", 3)).toEqual({ start: 0, end: 7, query: "re", quoted: false });
  });

  it("ignores emails, finished tokens and plain text", () => {
    expect(activeMention("mail a@b.com", 12)).toBeNull();
    expect(activeMention("@readme now", 11)).toBeNull();
    expect(activeMention("no mention", 5)).toBeNull();
  });

  it("handles quoted mentions like Pi", () => {
    expect(activeMention("see @\"my fo", 11)).toEqual({ start: 4, end: 11, query: "my fo", quoted: true });
    expect(activeMention("see @\"my file\" and @x", 21)).toEqual({ start: 19, end: 21, query: "x", quoted: false });
  });
});

describe("mentionValue", () => {
  it("quotes paths with spaces", () => {
    expect(mentionValue("README.md")).toBe("@README.md");
    expect(mentionValue("my notes.txt")).toBe("@\"my notes.txt\"");
    expect(mentionValue("my dir/", true)).toBe("@\"my dir/");
  });
});

describe("rankMentions", () => {
  const files = ["README.md", "src/App.tsx", "src/components/Composer.tsx", "src/readme-utils.ts", "docs/guide/read.md", "package.json"];

  it("lists the top level for an empty query, folders first and shortest first", () => {
    expect(rankMentions(files, "").map((entry) => entry.path)).toEqual(["src/", "docs/", "README.md", "package.json"]);
  });

  it("prefers basename matches over deeper path matches", () => {
    const paths = rankMentions(files, "read").map((entry) => entry.path);
    expect(paths[0]).toBe("docs/guide/read.md");
    expect(paths).toContain("README.md");
    expect(paths).toContain("src/readme-utils.ts");
  });

  it("derives folders and lists their direct children first", () => {
    expect(rankMentions(files, "src/").map((entry) => entry.path)).toEqual(["src/components/", "src/App.tsx", "src/readme-utils.ts", "src/components/Composer.tsx"]);
    expect(rankMentions(files, "comp")[0]).toEqual({ path: "src/components/", directory: true });
  });

  it("falls back to fuzzy subsequences", () => {
    expect(rankMentions(files, "cmpsr").map((entry) => entry.path)).toEqual(["src/components/Composer.tsx"]);
    expect(rankMentions(files, "rad").map((entry) => entry.path)).toEqual(["README.md", "src/readme-utils.ts", "docs/guide/read.md"]);
    expect(rankMentions(files, "zzz")).toEqual([]);
  });

  it("caps the list", () => {
    const many = Array.from({ length: 50 }, (_, index) => `file${index}.ts`);
    expect(rankMentions(many, "file")).toHaveLength(20);
  });
});

describe("splitMentions", () => {
  it("marks path-like mentions", () => {
    expect(splitMentions("fix @src/App.tsx, then @README.md.")).toEqual([
      { text: "fix " },
      { text: "@src/App.tsx", mention: true },
      { text: ", then " },
      { text: "@README.md", mention: true },
      { text: "." },
    ]);
  });

  it("marks quoted mentions and leaves handles and emails alone", () => {
    expect(splitMentions("ask @someone about @\"my notes.txt\" or a@b.com")).toEqual([
      { text: "ask @someone about " },
      { text: "@\"my notes.txt\"", mention: true },
      { text: " or a@b.com" },
    ]);
  });
});
