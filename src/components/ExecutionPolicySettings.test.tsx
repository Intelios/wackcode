import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_EXECUTION_POLICY, EXECUTION_POLICY_WARNING } from "../execution-policy";
import { ExecutionPolicySettings } from "./ExecutionPolicySettings";

afterEach(cleanup);

describe("read-only override settings", () => {
  it.each([
    ["unrestrictedSubagents", "Remove sub-agent read-only restrictions"],
    ["unrestrictedPlanning", "Remove Plan / Ultra Plan read-only restrictions"],
  ] as const)("confirms every enable of %s and saves only after confirmation", async (key, label) => {
    const onChange = vi.fn().mockResolvedValue(undefined);
    const { rerender } = render(<ExecutionPolicySettings config={DEFAULT_EXECUTION_POLICY} onChange={onChange} />);
    const toggle = screen.getByRole("switch", { name: label });
    fireEvent.click(toggle);
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByRole("alertdialog")).toHaveTextContent(EXECUTION_POLICY_WARNING);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(toggle).toHaveAttribute("aria-checked", "false");
    fireEvent.click(toggle);
    fireEvent.click(screen.getByRole("button", { name: "Remove restrictions" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith({ ...DEFAULT_EXECUTION_POLICY, [key]: true }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    rerender(<ExecutionPolicySettings config={{ ...DEFAULT_EXECUTION_POLICY, [key]: true }} onChange={onChange} />);
    expect(screen.getByText(EXECUTION_POLICY_WARNING)).toBeInTheDocument();
    fireEvent.click(toggle);
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(DEFAULT_EXECUTION_POLICY));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    rerender(<ExecutionPolicySettings config={DEFAULT_EXECUTION_POLICY} onChange={onChange} />);
    fireEvent.click(toggle);
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
  });

  it("keeps the switch off and displays an enable save error inside the dialog", async () => {
    const onChange = vi.fn().mockRejectedValue("Could not save Settings.");
    render(<ExecutionPolicySettings config={DEFAULT_EXECUTION_POLICY} onChange={onChange} />);
    const toggle = screen.getByRole("switch", { name: "Remove Plan / Ultra Plan read-only restrictions" });
    fireEvent.click(toggle);
    const dialog = screen.getByRole("alertdialog");
    expect(dialog).toHaveTextContent("change connected services");
    fireEvent.click(within(dialog).getByRole("button", { name: "Remove restrictions" }));
    await waitFor(() => expect(dialog).toHaveTextContent("Could not save Settings."));
    expect(toggle).toHaveAttribute("aria-checked", "false");
  });

  it("keeps an enabled switch on when disabling fails", async () => {
    render(<ExecutionPolicySettings config={{ ...DEFAULT_EXECUTION_POLICY, unrestrictedSubagents: true }} onChange={vi.fn().mockRejectedValue("Could not save Settings.")} />);
    const toggle = screen.getByRole("switch", { name: "Remove sub-agent read-only restrictions" });
    fireEvent.click(toggle);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save Settings.");
    expect(toggle).toHaveAttribute("aria-checked", "true");
  });
});
