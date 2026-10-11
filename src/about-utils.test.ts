import { describe, expect, it } from "vitest";
import { aboutSummary, buildLabel } from "./about-utils";
import type { AppInfo } from "./types";

const INFO: AppInfo = {
  appVersion: "1.0.5",
  build: "development",
  devBuild: true,
  bundleId: "com.wackcode.desktop.dev",
  appPath: "/Applications/WackCode.app/Contents/MacOS/wackcode",
  workerPath: "/repo/worker/dist/index.js",
  piVersion: "0.99.2",
  nodeVersion: "24.18.0",
  osVersion: "macOS 15.3",
  chip: "Apple Silicon (arm64)",
  projectCount: 3,
  chatCount: 12,
  archivedCount: 2,
  activeWorkers: 1
};

describe("about summary", () => {
  it("copies identity, bundled versions, the machine and the counts, and nothing else", () => {
    expect(aboutSummary(INFO)).toBe(
      [
        "WackCode 1.0.5 (Development build)",
        "Pi 0.99.2 · Node 24.18.0 (bundled)",
        "macOS 15.3 · Apple Silicon (arm64)",
        "3 projects · 12 chats (2 archived) · 1 live worker"
      ].join("\n")
    );
  });

  it("never includes the app's path or any other detail the page keeps out of the block", () => {
    const summary = aboutSummary(INFO);
    expect(summary).not.toContain(INFO.appPath);
    expect(summary).not.toContain("/Users");
  });

  it("labels the build flavour the way the badge does", () => {
    expect(buildLabel("development")).toBe("Development build");
    expect(buildLabel("installed")).toBe("Installed build");
  });

  it("pluralises counts", () => {
    expect(aboutSummary({ ...INFO, projectCount: 1, chatCount: 1, activeWorkers: 0 })).toContain(
      "1 project · 1 chat (2 archived) · 0 live workers"
    );
  });
});