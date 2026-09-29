import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ModeToggle } from "./ModeToggle";

afterEach(cleanup);

describe("ModeToggle", () => {
  it("reports the mode it is asked to switch to", () => {
    const onChange = vi.fn();
    render(<ModeToggle mode="build" onChange={onChange} />);
    expect(screen.getByRole("radio", { name: "Build" })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("radio", { name: "Plan" }));
    expect(onChange).toHaveBeenCalledWith("plan");
  });

  it("toggles Plan ↔ Ultra Plan when Plan is clicked again", () => {
    const onChange = vi.fn();
    const { rerender } = render(<ModeToggle mode="plan" onChange={onChange} />);
    fireEvent.click(screen.getByRole("radio", { name: "Plan" }));
    expect(onChange).toHaveBeenLastCalledWith("ultraplan");

    rerender(<ModeToggle mode="ultraplan" onChange={onChange} />);
    const ultra = screen.getByRole("radio", { name: "Ultra Plan" });
    expect(ultra).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("radio", { name: "Build" })).toHaveAttribute("aria-checked", "false");
    fireEvent.click(ultra);
    expect(onChange).toHaveBeenLastCalledWith("plan");
    fireEvent.click(screen.getByRole("radio", { name: "Build" }));
    expect(onChange).toHaveBeenLastCalledWith("build");
  });

  it("does not fire while disabled (a run owns the mode)", () => {
    const onChange = vi.fn();
    render(<ModeToggle mode="plan" disabled onChange={onChange} />);
    fireEvent.click(screen.getByRole("radio", { name: "Build" }));
    expect(onChange).not.toHaveBeenCalled();
  });

  it("shows a hammer icon on Build, beside its label like Plan's brain", () => {
    render(<ModeToggle mode="build" onChange={() => {}} />);
    const build = screen.getByRole("radio", { name: "Build" });
    expect(build.querySelector("svg.mode-icon")).toBeTruthy();
    expect(build.querySelector(".mode-label")).toHaveTextContent("Build");
  });

  it("plays a one-shot nudge when Build or Plan becomes active, but not on mount", () => {
    const onChange = vi.fn();
    const { rerender } = render(<ModeToggle mode="build" onChange={onChange} />);
    expect(screen.getByRole("radio", { name: "Build" })).not.toHaveClass("mode-in");

    rerender(<ModeToggle mode="plan" onChange={onChange} />);
    const plan = screen.getByRole("radio", { name: "Plan" });
    expect(plan).toHaveClass("mode-in");
    expect(plan.querySelector(".mode-glow")).toBeTruthy();

    rerender(<ModeToggle mode="build" onChange={onChange} />);
    expect(screen.getByRole("radio", { name: "Build" })).toHaveClass("mode-in");
    expect(plan).not.toHaveClass("mode-in");
  });

  it("leaves Ultra Plan's swap to its fire: no nudge on the flame", () => {
    const onChange = vi.fn();
    const { rerender } = render(<ModeToggle mode="plan" onChange={onChange} />);
    rerender(<ModeToggle mode="ultraplan" onChange={onChange} />);
    expect(screen.getByRole("radio", { name: "Ultra Plan" })).not.toHaveClass("mode-in");

    // Back to Plan is a switch like any other and does get the nudge.
    rerender(<ModeToggle mode="plan" onChange={onChange} />);
    expect(screen.getByRole("radio", { name: "Plan" })).toHaveClass("mode-in");
  });

  it("holds the nudge still while disabled (a run owns the mode)", () => {
    const onChange = vi.fn();
    const { rerender } = render(<ModeToggle mode="build" onChange={onChange} />);
    rerender(<ModeToggle mode="plan" disabled onChange={onChange} />);
    expect(screen.getByRole("radio", { name: "Plan" })).not.toHaveClass("mode-in");
  });
});
