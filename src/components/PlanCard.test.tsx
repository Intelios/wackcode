import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PlanCard } from "./PlanCard";

afterEach(cleanup);

describe("PlanCard", () => {
  const plan = "# Ship it\n\n- Do the thing\n- Test it";

  it("renders the plan as markdown with the action row when it is the current proposal", () => {
    const onAction = vi.fn();
    render(<PlanCard plan={plan} current onAction={onAction} />);
    expect(screen.getByText("Ship it")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Approve & implement" }));
    expect(onAction).toHaveBeenCalledWith("implement");
    fireEvent.click(screen.getByRole("button", { name: "Save PLAN.md" }));
    expect(onAction).toHaveBeenCalledWith("save");
    fireEvent.click(screen.getByRole("button", { name: "Copy" }));
    expect(onAction).toHaveBeenCalledWith("copy");
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    expect(onAction).toHaveBeenCalledWith("discard");
  });

  it("shows an older proposal read-only — only the latest plan gets the decision row", () => {
    render(<PlanCard plan={plan} current={false} onAction={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Approve & implement" })).not.toBeInTheDocument();
    expect(screen.getByText("Ship it")).toBeInTheDocument();
  });

  it("disables actions while a run is in flight", () => {
    render(<PlanCard plan={plan} current busy onAction={vi.fn()} />);
    expect(screen.getByRole("button", { name: "Approve & implement" })).toBeDisabled();
  });
});
