import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import { VisualCallCard, VisualCard } from "./VisualCard";

vi.mock("../api", () => ({
  api: { stageVisual: vi.fn().mockResolvedValue("wcvisual://localhost/abc123"), revealPath: vi.fn().mockResolvedValue(undefined) }
}));

afterEach(cleanup);
beforeEach(() => {
  vi.mocked(api.stageVisual).mockClear();
  vi.mocked(api.stageVisual).mockResolvedValue("wcvisual://localhost/abc123");
});

/** The card's frame, once staging has answered. */
async function frame() {
  const doc = await waitFor(() => {
    const found = document.querySelector("iframe.visual-frame-doc");
    expect(found).not.toBeNull();
    return found as HTMLIFrameElement;
  });
  return doc;
}

/** A `postMessage` as if it came from the frame itself. */
function fromFrame(target: HTMLIFrameElement, data: unknown) {
  act(() => {
    window.dispatchEvent(Object.assign(new Event("message"), { data, source: target.contentWindow }));
  });
}

describe("VisualCard", () => {
  it("stages the visual's HTML with the app's theme and draws it sandboxed", async () => {
    render(<VisualCard title="Latency by region" html="<p>42</p>" current busy={false} />);
    expect(screen.getByRole("figure", { name: "Visual: Latency by region" })).toBeInTheDocument();
    const doc = await frame();
    expect(api.stageVisual).toHaveBeenCalledTimes(1);
    const [html, theme] = vi.mocked(api.stageVisual).mock.calls[0];
    expect(html).toBe("<p>42</p>");
    expect(theme.accent).toBeTruthy();
    expect(doc.src).toBe("wcvisual://localhost/abc123");
    // No same-origin: the frame is an opaque origin that cannot reach the app.
    expect(doc.getAttribute("sandbox")).toBe("allow-scripts");
    expect(doc.getAttribute("referrerpolicy")).toBe("no-referrer");
    expect(doc.getAttribute("title")).toBe("Latency by region");
  });

  it("sizes the frame from its own report, clamped to the card's cap", async () => {
    render(<VisualCard title="Chart" html="<p>42</p>" current busy={false} />);
    const doc = await frame();
    const well = doc.parentElement as HTMLElement;
    fromFrame(doc, { type: "wackcode-visual:size", height: 240 });
    await waitFor(() => expect(well.style.height).toBe("240px"));
    fromFrame(doc, { type: "wackcode-visual:size", height: 5000 });
    await waitFor(() => expect(well.style.height).toBe("640px"));
    fromFrame(doc, { type: "wackcode-visual:size", height: 4 });
    await waitFor(() => expect(well.style.height).toBe("96px"));
  });

  it("ignores messages that did not come from its own frame", async () => {
    render(<VisualCard title="Chart" html="<p>42</p>" current busy={false} />);
    const doc = await frame();
    const well = doc.parentElement as HTMLElement;
    const before = well.style.height;
    act(() => {
      window.dispatchEvent(Object.assign(new Event("message"), { data: { type: "wackcode-visual:size", height: 300 }, source: window }));
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(well.style.height).toBe(before);
  });

  it("forwards a link the user clicked, and only http(s)", async () => {
    const onOpenLink = vi.fn();
    render(<VisualCard title="Chart" html="<p>42</p>" current busy={false} onOpenLink={onOpenLink} />);
    const doc = await frame();
    fromFrame(doc, { type: "wackcode-visual:link", href: "https://example.com/docs" });
    expect(onOpenLink).toHaveBeenCalledWith("https://example.com/docs");
    fromFrame(doc, { type: "wackcode-visual:link", href: "file:///etc/passwd" });
    expect(onOpenLink).toHaveBeenCalledTimes(1);
  });

  it("offers the copy and expand actions, and says the HTML is the document", async () => {
    render(<VisualCard title="Chart" html="<p>42</p>" current busy={false} />);
    expect(screen.getByRole("button", { name: "Expand visual" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy visual HTML" })).toBeInTheDocument();
  });

  it("opens an expanded frame at window size and closes it on Escape", async () => {
    render(<VisualCard title="Chart" html="<p>42</p>" current busy={false} />);
    await frame();
    fireEvent.click(screen.getByRole("button", { name: "Expand visual" }));
    const dialog = await screen.findByRole("dialog", { name: "Visual: Chart" });
    expect(dialog).toBeInTheDocument();
    // A second, freshly staged frame rather than the inline one moved: the card keeps its state.
    await waitFor(() => expect(api.stageVisual).toHaveBeenCalledTimes(2));
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Visual: Chart" })).toBeNull());
  });

  it("shows the error state when staging fails, and stages again on Try again", async () => {
    vi.mocked(api.stageVisual).mockRejectedValueOnce(new Error("nope"));
    render(<VisualCard title="Chart" html="<p>42</p>" current busy={false} />);
    expect(await screen.findByText("This visual couldn’t be shown.")).toBeInTheDocument();
    vi.mocked(api.stageVisual).mockResolvedValue("wcvisual://localhost/def456");
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    const doc = await frame();
    expect(doc.src).toBe("wcvisual://localhost/def456");
    expect(api.stageVisual).toHaveBeenCalledTimes(2);
  });
});

describe("VisualCard sending", () => {
  /** Delivers a frame's send and waits for the answer the card posts back. */
  async function send(doc: HTMLIFrameElement, text: string, id = 1): Promise<Record<string, unknown> | undefined> {
    const answers: Array<Record<string, unknown>> = [];
    const postMessage = vi.fn((message: Record<string, unknown>) => { answers.push(message); });
    Object.defineProperty(doc, "contentWindow", { value: { postMessage }, configurable: true });
    fromFrame(doc, { type: "wackcode-visual:send", id, text });
    await waitFor(() => expect(postMessage).toHaveBeenCalled());
    return answers.find((answer) => answer.type === "wackcode-visual:send-result");
  }

  it("sends the widget's words when this visual is the latest answer's", async () => {
    const onSend = vi.fn().mockResolvedValue({ ok: true });
    render(<VisualCard title="Chart" html="<p>42</p>" current busy={false} onSend={onSend} />);
    const doc = await frame();
    expect(await send(doc, "  Pick option B  ")).toMatchObject({ ok: true, id: 1 });
    expect(onSend).toHaveBeenCalledWith("Pick option B", "Chart");
  });

  it("refuses an older visual's message without sending anything", async () => {
    const onSend = vi.fn();
    render(<VisualCard title="Chart" html="<p>42</p>" current={false} busy={false} onSend={onSend} />);
    const doc = await frame();
    expect(await send(doc, "Pick option B")).toMatchObject({ ok: false, reason: "stale" });
    expect(onSend).not.toHaveBeenCalled();
    expect(await screen.findByText(/From an earlier reply/)).toBeInTheDocument();
  });

  it("refuses while a reply is running, and reports the host's refusal", async () => {
    const busySend = vi.fn();
    const { unmount } = render(<VisualCard title="Chart" html="<p>42</p>" current busy onSend={busySend} />);
    const busyFrame = await frame();
    expect(await send(busyFrame, "Go")).toMatchObject({ ok: false, reason: "busy" });
    expect(busySend).not.toHaveBeenCalled();
    unmount();

    cleanup();
    vi.mocked(api.stageVisual).mockClear();
    const onSend = vi.fn().mockResolvedValue({ ok: false, reason: "busy" });
    render(<VisualCard title="Chart" html="<p>42</p>" current busy={false} onSend={onSend} />);
    const doc = await frame();
    expect(await send(doc, "Go")).toMatchObject({ ok: false, reason: "busy" });
    expect(await screen.findByText(/Wait for the reply to finish/)).toBeInTheDocument();
  });

  it("refuses empty text, and a second message within a second", async () => {
    const onSend = vi.fn().mockResolvedValue({ ok: true });
    render(<VisualCard title="Chart" html="<p>42</p>" current busy={false} onSend={onSend} />);
    const doc = await frame();
    Object.defineProperty(doc, "contentWindow", { value: { postMessage: vi.fn() }, configurable: true });
    fromFrame(doc, { type: "wackcode-visual:send", id: 1, text: "   " });
    await waitFor(() => expect(screen.getByText(/That message was empty/)).toBeInTheDocument());
    expect(onSend).not.toHaveBeenCalled();
    expect(await send(doc, "One", 2)).toMatchObject({ ok: true });
    expect(await send(doc, "Two", 3)).toMatchObject({ ok: false, reason: "throttled" });
    expect(onSend).toHaveBeenCalledTimes(1);
  });
});

describe("VisualCallCard", () => {
  it("draws a call's title and body from its arguments", async () => {
    render(<VisualCallCard block={{ type: "tool-call", toolName: "show_visual", toolCallId: "call-1", arguments: { title: "Chart", html: "<p>42</p>" } }} />);
    expect(screen.getByRole("figure", { name: "Visual: Chart" })).toBeInTheDocument();
    await waitFor(() => expect(document.querySelector("iframe.visual-frame-doc")).not.toBeNull());
  });

  it("renders nothing for arguments that do not parse", () => {
    const { container } = render(<VisualCallCard block={{ type: "tool-call", toolName: "show_visual", toolCallId: "call-1", arguments: { title: "" } }} />);
    expect(container).toBeEmptyDOMElement();
  });
});
