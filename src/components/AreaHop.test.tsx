import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AreaHop } from "./AreaHop";

let reducedMotion = false;
vi.mock("motion/react", async (importOriginal) => {
  const mod = await importOriginal<typeof import("motion/react")>();
  return { ...mod, useReducedMotion: () => reducedMotion };
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  reducedMotion = false;
});

describe("AreaHop", () => {
  it("stays home on first mount", () => {
    const { container } = render(<AreaHop area="code" />);
    expect(container.querySelector(".area-hop")).toBeNull();
  });

  it("hops right on Code → Chat and heads left back home", () => {
    vi.spyOn(Math, "random").mockReturnValue(0.99);
    const view = render(<AreaHop area="code" />);
    view.rerender(<AreaHop area="chat" />);
    const hop = view.container.querySelector(".area-hop");
    expect(hop).not.toBeNull();
    expect(hop).toHaveAttribute("data-direction", "right");
    expect(hop).not.toHaveClass("special");
    // The lights dim for the spotlit crossing.
    expect(view.container.querySelector(".area-hop-scrim")).not.toBeNull();
    expect(view.container.querySelector(".area-hop-glow")).not.toBeNull();
    expect(view.container.querySelector(".area-hop .duck-mark")).not.toBeNull();
    view.rerender(<AreaHop area="code" />);
    expect(view.container.querySelector(".area-hop")).toHaveAttribute("data-direction", "left");
  });

  it("replaces a mid-flight duck instead of stacking a flock", () => {
    vi.spyOn(Math, "random").mockReturnValue(0.99);
    const view = render(<AreaHop area="code" />);
    view.rerender(<AreaHop area="chat" />);
    view.rerender(<AreaHop area="chat" />);
    const first = view.container.querySelectorAll(".area-hop");
    view.rerender(<AreaHop area="code" />);
    expect(first).toHaveLength(1);
    expect(view.container.querySelectorAll(".area-hop")).toHaveLength(1);
  });

  it("somersaults on a lucky roll", () => {
    vi.spyOn(Math, "random").mockReturnValue(0.05);
    const view = render(<AreaHop area="code" />);
    view.rerender(<AreaHop area="chat" />);
    expect(view.container.querySelector(".area-hop.special")).not.toBeNull();
  });

  it("never mounts under reduced motion", () => {
    reducedMotion = true;
    const view = render(<AreaHop area="code" />);
    view.rerender(<AreaHop area="chat" />);
    expect(view.container.querySelector(".area-hop")).toBeNull();
  });
});
