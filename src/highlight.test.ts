import { Children, isValidElement, type ReactNode } from "react";
import { describe, expect, it } from "vitest";
import { highlightBlock, highlightDiffLines, languageForPath } from "./highlight";

/** Flatten a node tree to its text content — tokenization must never lose characters. */
function textOf(nodes: ReactNode[]): string {
  let text = "";
  for (const node of nodes) {
    if (typeof node === "string" || typeof node === "number") text += String(node);
    else if (isValidElement(node)) text += textOf(Children.toArray((node.props as { children?: ReactNode }).children));
    else if (Array.isArray(node)) text += textOf(node);
  }
  return text;
}

/** Every class name anywhere in a node tree. */
function classSet(nodes: ReactNode[]): Set<string> {
  const found = new Set<string>();
  for (const node of nodes) {
    if (isValidElement(node)) {
      const className = (node.props as { className?: string }).className;
      if (className) for (const name of className.split(" ")) if (name) found.add(name);
      for (const name of classSet(Children.toArray((node.props as { children?: ReactNode }).children))) found.add(name);
    } else if (Array.isArray(node)) {
      for (const name of classSet(node)) found.add(name);
    }
  }
  return found;
}

describe("languageForPath", () => {
  it("maps extensions and Makefile to highlighting languages", () => {
    expect(languageForPath("src/app.tsx")).toBe("typescript");
    expect(languageForPath("/a/b/main.py")).toBe("python");
    expect(languageForPath("lib.rs")).toBe("rust");
    expect(languageForPath("Makefile")).toBe("makefile");
    expect(languageForPath("notes.TXT")).toBeUndefined();
  });

  it("says nothing about dotfiles, extensionless or missing paths", () => {
    expect(languageForPath(".zshrc")).toBeUndefined();
    expect(languageForPath("README")).toBeUndefined();
    expect(languageForPath("")).toBeUndefined();
    expect(languageForPath(undefined)).toBeUndefined();
  });
});

describe("highlightBlock", () => {
  it("tokenizes a known language, keeping every character", () => {
    const code = "const rate = 0.5; // half";
    const nodes = highlightBlock(code, "ts");
    expect(textOf(nodes)).toBe(code);
    expect(classSet(nodes)).toContain("hljs-keyword");
    expect(classSet(nodes)).toContain("hljs-comment");
  });

  it("highlights an unknown tag via auto-detection and survives garbage", () => {
    expect(classSet(highlightBlock("def greet(name):\n    return name\n", "notalanguage!"))).toContain("hljs-keyword");
    expect(textOf(highlightBlock("\x00 ?? {{", "notalanguage!"))).toBe("\x00 ?? {{");
  });

  it("leaves untagged-as-plain tags and no tag as plain text", () => {
    expect(highlightBlock("$ npm run build", "console")).toEqual(["$ npm run build"]);
    expect(highlightBlock("just prose", undefined)).toEqual(["just prose"]);
  });

  it("returns the identical memoized array for repeated content", () => {
    const first = highlightBlock("const x = 1;", "typescript");
    expect(highlightBlock("const x = 1;", "typescript")).toBe(first);
  });
});

describe("highlightDiffLines", () => {
  it("keeps the diff marker plain and tokenizes the code after it", () => {
    const nodes = highlightDiffLines(
      ['-"gone"', "+const x = 1;", " unchanged()", "@@ -1 +1 @@"],
      ["code", "code", "code", "meta"],
      "typescript"
    );
    expect(nodes).toHaveLength(4);
    expect(textOf(nodes)).toBe('-"gone"+const x = 1; unchanged()@@ -1 +1 @@');
    expect(classSet([nodes[0]])).toContain("hljs-string");
    expect(classSet([nodes[1]])).toContain("hljs-keyword");
    expect(classSet([nodes[2]])).toContain("hljs-title");
    expect(classSet([nodes[3]])).toEqual(new Set());
  });

  it("carries tokens across line breaks (block comments) in diffs", () => {
    const nodes = highlightDiffLines(["+/* header", "+   still the comment */"], ["code", "code"], "css");
    expect(nodes).toHaveLength(2);
    expect(classSet([nodes[0]])).toContain("hljs-comment");
    expect(classSet([nodes[1]])).toContain("hljs-comment");
  });

  it("falls back to plain text without a language, and pads empty lines", () => {
    const nodes = highlightDiffLines(["-old", "+new", ""], ["code", "code", "code"], undefined);
    expect(nodes).toEqual(["-old", "+new", " "]);
  });
});
