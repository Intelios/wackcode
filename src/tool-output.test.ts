import { describe, expect, it } from "vitest";
import { argRows, askedQuestions, grepMatcher, parseGrep, readRange, splitCommandStatus, splitMatches, splitNotice } from "./tool-output";

describe("splitNotice", () => {
  it("lifts Pi's cut-short note off a result and drops the advice meant for the model", () => {
    expect(splitNotice("a\nb\n\n[Showing lines 1-2 of 9. Use offset=3 to continue.]")).toEqual({ body: "a\nb", notice: "Showing lines 1-2 of 9" });
    expect(splitNotice("x:1: hit\n\n[100 matches limit reached. Use limit=200 for more, or refine pattern]")).toEqual({ body: "x:1: hit", notice: "100 matches limit reached" });
    expect(splitNotice("out\n\n[Showing lines 61-200 of 200. Full output: /tmp/pi.log]").notice).toBe("Showing lines 61-200 of 200. Full output: /tmp/pi.log");
  });

  it("leaves a file that merely ends in a bracketed line whole", () => {
    const toml = "name = \"x\"\n\n[dependencies]";
    expect(splitNotice(toml)).toEqual({ body: toml });
  });
});

describe("splitCommandStatus", () => {
  it("separates how a failed command ended from what it printed", () => {
    expect(splitCommandStatus("boom\n\nCommand exited with code 2")).toEqual({ output: "boom", status: "exit 2" });
    expect(splitCommandStatus("Command timed out after 30 seconds")).toEqual({ output: "", status: "timed out after 30s" });
    expect(splitCommandStatus("partial\n\nCommand aborted")).toEqual({ output: "partial", status: "stopped" });
  });

  it("keeps any other failure text whole", () => {
    expect(splitCommandStatus("spawn bash ENOENT")).toEqual({ output: "spawn bash ENOENT" });
  });
});

describe("readRange", () => {
  it("numbers from the offset and words the range from Pi's note", () => {
    expect(readRange({}, "a\nb\nc")).toEqual({ first: 1, label: "3 lines" });
    expect(readRange({}, "only\n")).toEqual({ first: 1, label: "1 line" });
    expect(readRange({ offset: 40, limit: 3 }, "a\nb\nc")).toEqual({ first: 40, label: "lines 40–42" });
    expect(readRange({ offset: 40, limit: 3 }, "a\nb\nc", "10 more lines in file")).toEqual({ first: 40, label: "lines 40–42 of 52" });
    expect(readRange({}, "a\nb", "Showing lines 1-2 of 900")).toEqual({ first: 1, label: "lines 1–2 of 900" });
  });
});

describe("parseGrep", () => {
  it("groups matches and their context by file", () => {
    expect(parseGrep(["src/a.ts-3- before", "src/a.ts:4: const hit = 1;", "src/b.ts:10: hit()", ""])).toEqual([
      { path: "src/a.ts", rows: [{ line: 3, text: "before", hit: false }, { line: 4, text: "const hit = 1;", hit: true }] },
      { path: "src/b.ts", rows: [{ line: 10, text: "hit()", hit: true }] }
    ]);
  });

  it("reads a path with digits between dashes of its own, and an empty line of text", () => {
    const files = parseGrep(["posts/2024-10-11-notes.md-6- before", "posts/2024-10-11-notes.md:7: at 12:30:45", "posts/2024-10-11-notes.md-8- "]);
    expect(files).toHaveLength(1);
    expect(files?.[0].path).toBe("posts/2024-10-11-notes.md");
    expect(files?.[0].rows).toEqual([{ line: 6, text: "before", hit: false }, { line: 7, text: "at 12:30:45", hit: true }, { line: 8, text: "", hit: false }]);
  });

  it("gives up on anything that isn't grep output", () => {
    expect(parseGrep(["src/a.ts:4: hit", "ripgrep: something else"])).toBeUndefined();
    expect(parseGrep(["", ""])).toBeUndefined();
  });
});

describe("grep matches", () => {
  it("lights each hit, literally or as a pattern, ignoring case when asked", () => {
    expect(splitMatches("a.b axb", grepMatcher({ pattern: "a.b", literal: true }))).toEqual([{ text: "a.b", hit: true }, { text: " axb", hit: false }]);
    expect(splitMatches("Foo foo", grepMatcher({ pattern: "foo", ignoreCase: true })).filter((part) => part.hit)).toHaveLength(2);
  });

  it("lights nothing for a pattern JavaScript can't compile, or one that matches emptily", () => {
    expect(grepMatcher({ pattern: "(?i)foo(" })).toBeUndefined();
    expect(splitMatches("abc", grepMatcher({ pattern: "x*" }))).toEqual([{ text: "abc", hit: false }]);
  });
});

describe("argRows", () => {
  it("labels each argument and keeps nested data as JSON", () => {
    expect(argRows({ url: "https://example.com", max: 3, raw: false, skip: undefined, opts: { a: 1 } })).toEqual([
      { key: "url", value: "https://example.com", kind: "text" },
      { key: "max", value: "3", kind: "scalar" },
      { key: "raw", value: "false", kind: "scalar" },
      { key: "opts", value: "{\n  \"a\": 1\n}", kind: "data" }
    ]);
  });
});

describe("askedQuestions", () => {
  const args = { questions: [{ id: "q", header: "Next", question: "Which?", options: [{ label: "One", description: "First." }, { label: "Two", description: "Second." }] }] };

  it("marks what was picked, and carries a written answer", () => {
    const asked = askedQuestions(args, { details: { cancelled: false, answers: [{ questionId: "q", selected: ["Two"], custom: "or three" }] } });
    expect(asked?.outcome).toBe("answered");
    expect(asked?.questions[0].options.map((option) => option.chosen)).toEqual([false, true]);
    expect(asked?.questions[0].custom).toBe("or three");
    expect(asked?.questions[0].multi).toBe(false);
    expect(askedQuestions({ questions: [{ ...args.questions[0], multiSelect: true }] })?.questions[0].multi).toBe(true);
  });

  it("tells waiting, dismissed and wrapped-up apart", () => {
    expect(askedQuestions(args)?.outcome).toBe("waiting");
    expect(askedQuestions(args, { details: { cancelled: true, reason: "cancelled" } })?.outcome).toBe("dismissed");
    expect(askedQuestions(args, { details: { cancelled: true, reason: "wrap_up" } })?.outcome).toBe("wrapped-up");
  });

  it("leaves a refused call, or a result without its answers, to the plain sheet", () => {
    expect(askedQuestions(args, { isError: true })).toBeUndefined();
    expect(askedQuestions(args, { details: undefined })).toBeUndefined();
    expect(askedQuestions({ questions: "not yet" })).toBeUndefined();
  });
});
