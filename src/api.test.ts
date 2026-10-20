import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { api } from "./api";
import type { ComputerUseConfig } from "./types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), Channel: class {} }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: vi.fn() }));

beforeEach(() => vi.clearAllMocks());

const grant = { name: "TextEdit", bundleId: "com.apple.TextEdit", path: "/System/Applications/TextEdit.app" };

describe("computer-use bridge", () => {
  it("never submits saved app approvals through the settings command", async () => {
    const config: ComputerUseConfig = { enabled: true, showAgentCursor: true, neverAllow: [], alwaysAllow: [grant] };
    await api.setComputerUseConfig(config);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("set_computer_use_config", {
      input: { enabled: true, showAgentCursor: true, neverAllow: [] },
    });
  });

  it("answers a host-owned card by request id, not an app identity supplied by the renderer", async () => {
    await api.computerUseRespondAccess("chat", "request", "always");
    expect(invoke).toHaveBeenCalledExactlyOnceWith("computer_use_respond_access", {
      taskId: "chat", requestId: "request", decision: "always",
    });
  });

  it("revokes an exact app identity, including a path-only app", async () => {
    await api.computerUseRevokeApp(grant);
    expect(invoke).toHaveBeenLastCalledWith("computer_use_revoke_app", { bundleId: grant.bundleId, path: grant.path });
    await api.computerUseRevokeApp({ name: "My App", path: "/tmp/My App.app" });
    expect(invoke).toHaveBeenLastCalledWith("computer_use_revoke_app", { bundleId: null, path: "/tmp/My App.app" });
  });
});
