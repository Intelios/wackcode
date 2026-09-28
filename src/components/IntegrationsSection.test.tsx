import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { IntegrationsSection } from "./IntegrationsSection";
import { api } from "../api";
vi.mock("../api", () => ({ api: { usageStatus: vi.fn(), setUsageRecording: vi.fn(), revealPath: vi.fn() } }));
const status = { enabled: true, path: "/usage", hasHistory: true, error: null, pending: 0, dropped: 0, lastWritten: 100 };
describe("TokenTrail integration", () => {
  beforeEach(() => { vi.clearAllMocks(); vi.mocked(api.usageStatus).mockResolvedValue(status); });
  it("toggles recording and reveals its folder", async () => {
    vi.mocked(api.revealPath).mockResolvedValue(undefined);
    vi.mocked(api.setUsageRecording).mockResolvedValue(false);
    render(<IntegrationsSection />);
    const toggle = await screen.findByRole("switch", { name: "Record TokenTrail usage" });
    await waitFor(() => expect(toggle).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Usage folder" }));
    expect(api.revealPath).toHaveBeenCalledWith("/usage");
    vi.mocked(api.usageStatus).mockResolvedValue({ ...status, enabled: false });
    fireEvent.click(toggle);
    await screen.findByText("Recording paused");
    expect(api.setUsageRecording).toHaveBeenCalledWith(false);
  });
  it("surfaces write failures and dropped records", async () => {
    vi.mocked(api.usageStatus).mockResolvedValue({ ...status, error: "Disk full", pending: 2, dropped: 1 });
    render(<IntegrationsSection />);
    await screen.findByText("Disk full");
    expect(screen.getByText("2 records waiting to be saved.")).toBeInTheDocument();
    expect(screen.getByText("1 records could not be retained.")).toBeInTheDocument();
  });
});
