import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useReducedMotion } from "motion/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { UltraLabel } from "./UltraLabel";
import { UltraMark } from "./UltraMark";

vi.mock("motion/react", () => ({ useReducedMotion: vi.fn(() => false) }));

afterEach(() => {
  cleanup();
  vi.mocked(useReducedMotion).mockReturnValue(false);
});

const noise = () => document.querySelector(".ultra-label-noise");
const decoding = () => document.querySelector(".ultra-label")?.hasAttribute("data-decoding");

describe("UltraLabel", () => {
  it("decodes on arrival without losing its name or width", () => {
    render(<button type="button"><UltraLabel text="Ultra Plan" /></button>);
    expect(decoding()).toBe(true);
    expect(noise()).toHaveAttribute("aria-hidden", "true");
    expect(noise()?.textContent).toHaveLength("Ultra Plan".length);
    expect(screen.getByRole("button", { name: "Ultra Plan" })).toBeInTheDocument();
  });

  it("replays when its button is hovered, but not while the button is disabled", () => {
    render(<button type="button"><UltraLabel text="Ultra" decodeOnMount={false} /></button>);
    expect(decoding()).toBe(false);
    fireEvent.pointerEnter(screen.getByRole("button", { name: "Ultra" }));
    expect(decoding()).toBe(true);

    cleanup();
    render(<button type="button" disabled><UltraLabel text="Ultra" /></button>);
    fireEvent.pointerEnter(screen.getByRole("button", { name: "Ultra" }));
    expect(decoding()).toBe(false);
  });

  it("never decodes under reduced motion", () => {
    vi.mocked(useReducedMotion).mockReturnValue(true);
    render(<button type="button"><UltraLabel text="Ultra" /></button>);
    fireEvent.pointerEnter(screen.getByRole("button", { name: "Ultra" }));
    expect(decoding()).toBe(false);
    expect(noise()?.textContent).toBe("");
  });
});

describe("UltraMark", () => {
  it("is decorative and rests on the settled wildcard unless animated", () => {
    const { container, rerender } = render(<UltraMark />);
    const mark = container.querySelector("svg.ultra-mark");
    expect(mark).toHaveAttribute("aria-hidden", "true");
    expect(mark?.querySelectorAll("rect")).toHaveLength(9);
    expect(mark).not.toHaveClass("live");

    rerender(<UltraMark animated />);
    expect(container.querySelector("svg.ultra-mark")).toHaveClass("live");
  });

  it("holds still under reduced motion even when asked to animate", () => {
    vi.mocked(useReducedMotion).mockReturnValue(true);
    const { container } = render(<UltraMark animated />);
    expect(container.querySelector("svg.ultra-mark")).not.toHaveClass("live");
  });
});
