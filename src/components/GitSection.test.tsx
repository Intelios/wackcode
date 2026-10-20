import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { WACKCODE_COAUTHOR_EMAIL } from "../promptDefaults";
import { GitSection } from "./GitSection";

afterEach(cleanup);

describe("Settings › Git", () => {
  it("previews the exact trailer and switches attribution off and on", async () => {
    const onChange = vi.fn().mockResolvedValue(undefined);
    const { rerender } = render(<GitSection config={{ commitAttribution: true }} agentName="Jarvis" onChange={onChange} />);
    expect(screen.getByRole("heading", { name: "Jarvis as co-author" })).toBeInTheDocument();
    // The trailer names the GitHub App, never the agent's custom name.
    expect(screen.getByText(/Co-authored-by:/)).toHaveTextContent(`Co-authored-by: the chat’s model (WackCode) <${WACKCODE_COAUTHOR_EMAIL}>`);
    const toggle = screen.getByRole("switch", { name: "Credit commits" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    fireEvent.click(toggle);
    await waitFor(() => expect(onChange).toHaveBeenCalledWith({ commitAttribution: false }));

    rerender(<GitSection config={{ commitAttribution: false }} agentName="Jarvis" onChange={onChange} />);
    expect(toggle).toHaveAttribute("aria-checked", "false");
    expect(screen.getByText("Switched off: the agent's commits carry no co-author line.")).toBeInTheDocument();
    fireEvent.click(toggle);
    await waitFor(() => expect(onChange).toHaveBeenLastCalledWith({ commitAttribution: true }));
  });

  it("shows a save error and keeps the switch usable", async () => {
    const onChange = vi.fn().mockRejectedValue("Could not save Settings.");
    render(<GitSection config={{ commitAttribution: true }} agentName="WackCode" onChange={onChange} />);
    fireEvent.click(screen.getByRole("switch", { name: "Credit commits" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save Settings.");
    expect(screen.getByRole("switch", { name: "Credit commits" })).toBeEnabled();
  });
});
