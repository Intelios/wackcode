import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { api } from "../api";
import type { AppInfo } from "../types";
import { AboutSection } from "./AboutSection";

vi.mock("../api", () => ({ api: {
  appInfo: vi.fn(),
  revealPath: vi.fn().mockResolvedValue(undefined)
} }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: vi.fn().mockResolvedValue(undefined) }));

const INFO: AppInfo = {
  appVersion: "1.0.5",
  build: "development",
  appPath: "/Applications/WackCode.app/Contents/MacOS/wackcode",
  piVersion: "0.99.2",
  nodeVersion: "24.18.0",
  osVersion: "macOS 15.3",
  chip: "Apple Silicon (arm64)",
  projectCount: 3,
  chatCount: 12,
  archivedCount: 2,
  activeWorkers: 1
};

afterEach(() => {
  cleanup();
  vi.mocked(api.appInfo).mockReset();
  vi.mocked(api.revealPath).mockClear();
  vi.mocked(writeText).mockClear();
});

describe("Settings › About", () => {
  it("shows the version as the hero pill and the facts as rows", async () => {
    vi.mocked(api.appInfo).mockResolvedValue(INFO);
    render(<AboutSection />);
    const hero = await screen.findByRole("region", { name: "About overview" });
    expect(within(hero).getByText("1.0.5")).toBeInTheDocument();
    expect(screen.getByTitle("1.0.5")).toBeInTheDocument();
    expect(screen.getByTitle("0.99.2")).toBeInTheDocument();
    expect(screen.getByTitle("24.18.0")).toBeInTheDocument();
    expect(screen.getByTitle("macOS 15.3")).toBeInTheDocument();
    expect(screen.getByTitle("Apple Silicon (arm64)")).toBeInTheDocument();
  });

  it("labels a dev build the way the copy block does, with the running app's path beside it", async () => {
    vi.mocked(api.appInfo).mockResolvedValue(INFO);
    render(<AboutSection />);
    expect(await screen.findByText("Development build")).toBeInTheDocument();
    expect(screen.getByTitle(INFO.appPath)).toBeInTheDocument();
  });

  it("labels an installed build", async () => {
    vi.mocked(api.appInfo).mockResolvedValue({ ...INFO, build: "installed" });
    render(<AboutSection />);
    expect(await screen.findByText("Installed build")).toBeInTheDocument();
  });

  it("counts the library on this Mac", async () => {
    vi.mocked(api.appInfo).mockResolvedValue(INFO);
    render(<AboutSection />);
    const stats = await screen.findByRole("region", { name: "On this Mac" });
    for (const [value, label] of [["3", "Projects"], ["12", "Chats"], ["2", "Archived"], ["1", "Live workers"]] as const) {
      expect(within(stats).getByText(value)).toBeInTheDocument();
      expect(within(stats).getByText(label)).toBeInTheDocument();
    }
  });

  it("copies the whole picture for a report", async () => {
    vi.mocked(api.appInfo).mockResolvedValue(INFO);
    render(<AboutSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Copy details" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(
      [
        "WackCode 1.0.5 (Development build)",
        "Pi 0.99.2 · Node 24.18.0 (bundled)",
        "macOS 15.3 · Apple Silicon (arm64)",
        "3 projects · 12 chats (2 archived) · 1 live worker"
      ].join("\n")
    ));
  });

  it("reveals the running app selected in its folder, never launching it", async () => {
    vi.mocked(api.appInfo).mockResolvedValue(INFO);
    render(<AboutSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Reveal" }));
    await waitFor(() => expect(api.revealPath).toHaveBeenCalledWith(INFO.appPath, true));
  });

  it("opens the GitHub page through the external-link path", async () => {
    vi.mocked(api.appInfo).mockResolvedValue(INFO);
    render(<AboutSection />);
    const button = await screen.findByRole("button", { name: "View on GitHub" });
    expect(screen.getByText("github.com/Intelios/wackcode")).toBeInTheDocument();
    fireEvent.click(button);
    await waitFor(() => expect(api.revealPath).toHaveBeenCalledWith("https://github.com/Intelios/wackcode"));
  });

  it("says so in words when the info cannot be read", async () => {
    vi.mocked(api.appInfo).mockRejectedValue("WackCode could not find its own app folder");
    render(<AboutSection />);
    expect(await screen.findByRole("alert")).toHaveTextContent("WackCode could not find its own app folder");
  });
});