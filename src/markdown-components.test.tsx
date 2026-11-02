import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Markdown } from "./components/Markdown";

vi.mock("./api", () => ({ api: { revealPath: vi.fn().mockResolvedValue(undefined) } }));

// Counting each source ReactMarkdown is asked to parse is how the segment memoization is
// proven: a growing tail must never send a settled segment through the parser again.
const { markdownRenders } = vi.hoisted(() => ({ markdownRenders: vi.fn() }));
vi.mock("react-markdown", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react-markdown")>();
  const Counted = (options: Parameters<typeof actual.default>[0]) => {
    markdownRenders(options.children);
    return actual.default(options);
  };
  return { ...actual, default: Counted };
});

afterEach(cleanup);

// Fresh ink goes through `Markdown`, whose `freshInk` flag carries the trimEnd'ed source
// length — the same path ThinkingRow's live body takes.
describe("markdownComponents fresh ink", () => {
  it("word-splits only the source's final block", () => {
    render(<Markdown freshInk>{"Settled words.\n\nFresh words arriving"}</Markdown>);
    expect(screen.getByText("Settled words.").querySelectorAll(".ink-fresh")).toHaveLength(0);
    const spans = [...document.querySelectorAll(".ink-fresh")];
    expect(spans.map((span) => span.textContent).join(" ")).toBe("Fresh words arriving");
  });

  it("blinks one ink caret at the write head, and only there", () => {
    render(<Markdown freshInk>{"Settled words.\n\nFresh words arriving"}</Markdown>);
    expect(screen.getByText("Settled words.").querySelectorAll(".ink-caret")).toHaveLength(0);
    const carets = [...document.querySelectorAll(".ink-caret")];
    expect(carets).toHaveLength(1);
    expect(carets[0].parentElement?.tagName).toBe("P");
  });

  it("renders no ink spans without the flag", () => {
    render(<Markdown streaming>{"Settled words.\n\nFresh words arriving"}</Markdown>);
    expect(document.querySelectorAll(".ink-fresh")).toHaveLength(0);
  });

  it("leaves inline elements inside the final block untouched", () => {
    render(<Markdown freshInk>{"Code stays `plain` while words arrive"}</Markdown>);
    const code = screen.getByText("plain");
    expect(code).toHaveProperty("tagName", "CODE");
    expect(code.querySelectorAll(".ink-fresh")).toHaveLength(0);
    expect([...document.querySelectorAll(".ink-fresh")].map((span) => span.textContent))
      .toEqual(["Code", "stays", "while", "words", "arrive"]);
  });

  it("splits the final list item, not the earlier ones", () => {
    render(<Markdown freshInk>{"- first item\n- second item"}</Markdown>);
    expect(screen.getByText("first item").querySelectorAll(".ink-fresh")).toHaveLength(0);
    expect([...document.querySelectorAll(".ink-fresh")].map((span) => span.textContent))
      .toEqual(["second", "item"]);
  });

  it("keeps a final heading plain: ink belongs to prose, not headings", () => {
    render(<Markdown freshInk>{"Intro.\n\n## Heading"}</Markdown>);
    expect(document.querySelectorAll(".ink-fresh")).toHaveLength(0);
  });

  it("still finds the write head through trailing newlines", () => {
    render(<Markdown freshInk>{"Last words.\n"}</Markdown>);
    expect([...document.querySelectorAll(".ink-fresh")].map((span) => span.textContent))
      .toEqual(["Last", "words."]);
  });
});

// While `streaming`, Markdown splits the source at stable block boundaries and re-parses only
// the open tail (src/markdown-segments.ts).
describe("streaming segmentation", () => {
  it("re-parses only the open tail as the stream grows", () => {
    markdownRenders.mockClear();
    const view = render(<Markdown streaming>{"Settled.\n\nTail words"}</Markdown>);
    expect(screen.getByText("Settled.")).toBeInTheDocument();
    expect(screen.getByText("Tail words")).toBeInTheDocument();
    markdownRenders.mockClear();
    view.rerender(<Markdown streaming>{"Settled.\n\nTail words keep growing"}</Markdown>);
    const sources = markdownRenders.mock.calls.map((call) => call[0]);
    expect(sources).toEqual(["\nTail words keep growing"]);
    expect(screen.getByText("Settled.")).toBeInTheDocument();
    expect(screen.getByText("Tail words keep growing")).toBeInTheDocument();
  });

  it("parses a newly settled segment exactly once", () => {
    const view = render(<Markdown streaming>{"One.\n\nTwo"}</Markdown>);
    markdownRenders.mockClear();
    view.rerender(<Markdown streaming>{"One.\n\nTwo.\n\nThree"}</Markdown>);
    const sources = markdownRenders.mock.calls.map((call) => call[0]);
    expect(sources).toEqual(["\nTwo.\n", "\nThree"]);
  });

  it("keeps fresh ink on the final block only", () => {
    render(<Markdown streaming freshInk>{"Settled words.\n\nFresh words arriving"}</Markdown>);
    expect(screen.getByText("Settled words.").querySelectorAll(".ink-fresh")).toHaveLength(0);
    expect([...document.querySelectorAll(".ink-fresh")].map((span) => span.textContent).join(" "))
      .toBe("Fresh words arriving");
    expect(document.querySelectorAll(".ink-caret")).toHaveLength(1);
  });

  it("highlights a closed fence in a settled segment while the open tail stays plain", () => {
    const view = render(<Markdown streaming>{"```ts\nconst done = 1\n```\n\n```ts\nconst open = 2"}</Markdown>);
    const codes = [...view.container.querySelectorAll("pre code")];
    expect(codes).toHaveLength(2);
    expect(codes[0]).toHaveClass("hljs");
    expect(codes[0].textContent).toBe("const done = 1\n");
    expect(codes[1]).not.toHaveClass("hljs");
    expect(codes[1].textContent).toBe("const open = 2\n");
  });

  it("does not segment settled prose", () => {
    markdownRenders.mockClear();
    render(<Markdown>{"One.\n\nTwo."}</Markdown>);
    expect(markdownRenders.mock.calls.map((call) => call[0])).toEqual(["One.\n\nTwo."]);
  });
});
