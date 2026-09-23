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
});
