import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Markdown } from "./components/Markdown";

vi.mock("./api", () => ({ api: { revealPath: vi.fn().mockResolvedValue(undefined) } }));

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
