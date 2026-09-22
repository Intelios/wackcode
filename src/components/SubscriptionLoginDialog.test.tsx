import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SubscriptionLoginDialog } from "./SubscriptionLoginDialog";

afterEach(cleanup);

describe("SubscriptionLoginDialog", () => {
  it("allows the blank GitHub domain that selects github.com", async () => {
    const onRespond = vi.fn().mockResolvedValue(undefined);
    render(<SubscriptionLoginDialog
      login={{ loginId: "login", providerId: "github-copilot", prompt: {
        type: "prompt", loginId: "login", providerId: "github-copilot", promptId: "domain",
        prompt: { type: "text", message: "GitHub Enterprise URL/domain (blank for github.com)" }
      } }}
      onOpenUrl={vi.fn()} onCopyCode={vi.fn()} onRespond={onRespond} onCancel={vi.fn()}
    />);
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() => expect(onRespond).toHaveBeenCalledWith("domain", ""));
  });

  it("shows a device code and supports opening and copying it", async () => {
    const onOpenUrl = vi.fn().mockResolvedValue(undefined);
    const onCopyCode = vi.fn().mockResolvedValue(undefined);
    render(<SubscriptionLoginDialog
      login={{ loginId: "login", providerId: "openai-codex", authUrl: "https://auth.openai.com/codex/device", deviceCode: {
        userCode: "ABCD-EFGH", verificationUri: "https://auth.openai.com/codex/device"
      } }}
      onOpenUrl={onOpenUrl} onCopyCode={onCopyCode} onRespond={vi.fn()} onCancel={vi.fn()}
    />);
    expect(screen.getByText("ABCD-EFGH")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open sign-in page" }));
    fireEvent.click(screen.getByRole("button", { name: "Copy code" }));
    await waitFor(() => expect(onOpenUrl).toHaveBeenCalledWith("https://auth.openai.com/codex/device"));
    expect(onCopyCode).toHaveBeenCalledWith("ABCD-EFGH");
  });
});
