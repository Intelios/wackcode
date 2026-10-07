import { cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useReducedMotion } from "motion/react";
import { RollingNumber } from "./RollingNumber";

vi.mock("motion/react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("motion/react")>();
  return { ...actual, useReducedMotion: vi.fn(() => false) };
});

afterEach(cleanup);
beforeEach(() => { vi.mocked(useReducedMotion).mockReturnValue(false); });

describe("RollingNumber", () => {
  it("renders one column per digit with the signed value as its accessible label", () => {
    const { container } = render(<RollingNumber prefix="+" value={12} />);
    const root = container.querySelector(".rolling-number")!;
    expect(root).toHaveAttribute("aria-label", "+12");
    expect(container.querySelectorAll(".rolling-digit")).toHaveLength(2);
    expect(root.textContent).toBe("+12");
  });

  it("grows a leading column rolling when 9 becomes 10", async () => {
    const { container, rerender } = render(<RollingNumber prefix="+" value={9} />);
    rerender(<RollingNumber prefix="+" value={10} />);
    const root = container.querySelector(".rolling-number")!;
    expect(root).toHaveAttribute("aria-label", "+10");
    expect(container.querySelectorAll(".rolling-digit")).toHaveLength(2);
    // The old 9 exits while the new digits settle in, so the drum holds "+910" briefly.
    await waitFor(() => expect(root.textContent).toBe("+10"));
  });

  it("collapses a column when 10 becomes 9", async () => {
    const { container, rerender } = render(<RollingNumber prefix="−" value={10} />);
    rerender(<RollingNumber prefix="−" value={9} />);
    const root = container.querySelector(".rolling-number")!;
    expect(root).toHaveAttribute("aria-label", "−9");
    await waitFor(() => expect(container.querySelectorAll(".rolling-digit")).toHaveLength(1));
    // The units column is still rolling from 1 to 9 — assert the settled drum.
    await waitFor(() => expect(root.textContent).toBe("−9"));
  });

  it("rolls the old digit out as the new one slides in", () => {
    const { rerender } = render(<RollingNumber prefix="+" value={4} />);
    rerender(<RollingNumber prefix="+" value={5} />);
    // While the exit animation runs both digits exist in the drum.
    const digits = document.querySelectorAll(".rolling-digit span");
    expect(digits.length).toBeGreaterThan(1);
  });

  it("does not animate on first mount", () => {
    const { container } = render(<RollingNumber prefix="+" value={7} />);
    const digit = container.querySelector<HTMLElement>(".rolling-digit span")!;
    expect(digit.style.transform ?? "").not.toMatch(/translate/);
  });

  it("renders plain text under reduced motion", async () => {
    vi.mocked(useReducedMotion).mockReturnValue(true);
    const { container, rerender } = render(<RollingNumber prefix="+" value={12} />);
    rerender(<RollingNumber prefix="+" value={13} />);
    expect(container.querySelectorAll(".rolling-digit")).toHaveLength(0);
    expect(container.textContent).toBe("+13");
  });
});
