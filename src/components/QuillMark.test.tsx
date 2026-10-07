import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { QuillMark } from "./QuillMark";

describe("QuillMark", () => {
  it("renders a decorative SVG with the quill body and its page ink", () => {
    const { container } = render(<QuillMark live />);
    const svg = container.querySelector("svg.quill-mark");
    expect(svg).toHaveAttribute("aria-hidden", "true");
    expect(container.querySelector(".quill-body")).not.toBeNull();
    expect(container.querySelector(".quill-ink")).not.toBeNull();
  });

  it("animates only while live", () => {
    const { container } = render(<QuillMark live={false} />);
    expect(container.querySelector("svg")).not.toHaveClass("live");
    expect(render(<QuillMark live />).container.querySelector("svg")).toHaveClass("live");
  });

  it("forwards className so callers set size and colour", () => {
    const { container } = render(<QuillMark live className="tool-row-icon" />);
    const svg = container.querySelector("svg");
    expect(svg).toHaveClass("quill-mark");
    expect(svg).toHaveClass("tool-row-icon");
  });

  it("clips three independently revealed ink lines to this instance's paper", () => {
    const { container } = render(<><QuillMark live /><QuillMark live /></>);
    const marks = container.querySelectorAll(".quill-mark");
    const ids = [...marks].map((mark) => mark.querySelector("clipPath")!.id);
    expect(new Set(ids).size).toBe(2);
    marks.forEach((mark, index) => {
      expect(mark.querySelector("g[clip-path]")).toHaveAttribute("clip-path", `url(#${ids[index]})`);
      expect(mark.querySelectorAll(".quill-ink")).toHaveLength(3);
      mark.querySelectorAll(".quill-ink").forEach((ink) => expect(ink).toHaveAttribute("pathLength", "1"));
      expect(mark.querySelector(".quill-page")).not.toBeNull();
      expect(mark.querySelector(".quill-body path")).toHaveAttribute("fill-rule", "evenodd");
    });
  });
});
