import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { PonderingDuck } from "./PonderingDuck";

describe("PonderingDuck", () => {
  it("renders a decorative SVG with the duck and three thought bubbles", () => {
    const { container } = render(<PonderingDuck live />);
    const svg = container.querySelector("svg.ponder-duck");
    expect(svg).toHaveAttribute("aria-hidden", "true");
    expect(svg).toContainHTML("duck-mark");
    expect(container.querySelectorAll(".ponder-bubble")).toHaveLength(3);
  });

  it("animates only while live", () => {
    const { container } = render(<PonderingDuck live={false} />);
    expect(container.querySelector("svg")).not.toHaveClass("live");
    expect(render(<PonderingDuck live />).container.querySelector("svg")).toHaveClass("live");
  });

  it("forwards className so callers set size and colour", () => {
    const { container } = render(<PonderingDuck live className="thinking-icon" />);
    const svg = container.querySelector("svg");
    expect(svg).toHaveClass("ponder-duck");
    expect(svg).toHaveClass("thinking-icon");
  });
});
