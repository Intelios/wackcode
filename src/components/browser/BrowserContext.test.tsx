import { useState } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { BrowserContextAttachment, ImageContent, ProviderRecord } from "../../types";
import { BROWSER_VIEWPORTS, encodeContexts, splitContextSection } from "../../browser-context";
import { Composer } from "../Composer";
import type { ComposerDraft } from "../../hooks/useComposerDrafts";
import { MessageEditor } from "../MessageEditor";
import { QueuedRows, queuedDisplay } from "../composer/QueuedRows";

afterEach(cleanup);
const image: ImageContent = { type: "image", mimeType: "image/png", data: "aW1hZ2U=" };
const context: BrowserContextAttachment = { id: "one", kind: "element", capturedAt: 1, title: "Example", url: "http://localhost/", viewport: BROWSER_VIEWPORTS.desktop, image,
  targets: [{ selector: "#save", path: [], dom: "<button>Save</button>", label: "Save", bounds: { x: 1, y: 1, width: 20, height: 20 } }] };
const providers: ProviderRecord[] = [{ id: "p", name: "Mock", kind: "custom", baseUrl: "http://localhost/", apiFormat: "openai-completions", connected: true, hasApiKey: false, createdAt: "", updatedAt: "",
  models: [{ id: "vision", name: "Vision", contextWindow: 8000, maxTokens: 1000, reasoning: false, vision: true, thinkingLevels: ["off"], thinkingLevelMap: { off: null } }] }];
const base = { providers, providerId: "p", modelId: "vision", thinkingLevel: "off" as const, favoriteModels: [], onSetFavorite: vi.fn(), onConfigure: vi.fn(), onStop: vi.fn(), onOpenSettings: vi.fn() };

function Draft({ onSend, initial = { text: "", images: [], files: [], browserContexts: [context] }, queued = false, onDequeue }: {
  onSend: (text: string, images: ImageContent[], files: unknown[]) => Promise<boolean>; initial?: ComposerDraft; queued?: boolean;
  onDequeue?: () => Promise<{ text: string; images: ImageContent[] }[]>;
}) {
  const [value, update] = useState(initial);
  return <Composer {...base} status={queued ? "running" : "idle"} draftState={{ key: "chat", value, update }} onSend={onSend}
    onDequeue={onDequeue} queuedMessages={queued ? [{ id: "queued", text: "queued" }] : []} />;
}

describe("browser evidence lifecycle", () => {
  it("sends a context-only message and restores the whole chip after a send failure", async () => {
    const send = vi.fn().mockResolvedValue(false);
    render(<Draft onSend={send} />);
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(send).toHaveBeenCalledOnce());
    const [text, images] = send.mock.calls[0];
    expect(splitContextSection(text).contexts[0]).toMatchObject({ id: "one", imageIndex: 0 });
    expect(images).toEqual([image]);
    expect(await screen.findByRole("button", { name: "Inspect Save" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Remove Save" }));
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
  });
  it("restores queued images and their context mapping, alongside the current draft", async () => {
    const outgoing = encodeContexts("queued words", [image], [context]);
    const restore = vi.fn().mockResolvedValue([{ ...outgoing, text: outgoing.text }]);
    render(<Draft onSend={vi.fn().mockResolvedValue(true)} initial={{ text: "new words", images: [], files: [] }} queued onDequeue={restore} />);
    fireEvent.click(screen.getByRole("button", { name: "Restore queued messages to the composer" }));
    expect(await screen.findByRole("button", { name: "Inspect Save" })).toBeInTheDocument();
    expect(screen.getByAltText("Attached image 1")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: /message/i })).toHaveValue("new words\n\nqueued words");
  });
  it("reindexes a sent chip after an ordinary image is removed and loads its original by the old index", async () => {
    const metadata = splitContextSection(encodeContexts("", [image, image], [context]).text).contexts;
    const send = vi.fn().mockResolvedValue(true), load = vi.fn().mockResolvedValue("data:image/png;base64,aW1hZ2U=");
    render(<MessageEditor text="Change it" images={[0, 1, 2].map((index) => ({ type: "image", imageId: String(index), thumbnail: "data:image/png;base64,aW1hZ2U=" }))}
      files={[]} browserContexts={metadata} vision loadImage={load} onSend={send} onCancel={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Inspect Save" }));
    await waitFor(() => expect(load).toHaveBeenCalledWith(2));
    fireEvent.click(screen.getByRole("button", { name: "Close browser context" }));
    fireEvent.click(screen.getByRole("button", { name: "Remove image 1" }));
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(send).toHaveBeenCalledOnce());
    expect(splitContextSection(send.mock.calls[0][0]).contexts[0].imageIndex).toBe(1);
    expect(send.mock.calls[0][2]).toEqual([0]);
  });
  it("inspects a queued bundle using its explicit image mapping", () => {
    const outgoing = encodeContexts("Change the button", [{ ...image, data: "ordinary" }], [context]);
    render(<QueuedRows entries={queuedDisplay([{ id: "queued", ...outgoing }])} agentName="WackCode" disabled={false}
      canSteer canRestore onSteer={vi.fn()} onRestore={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Inspect Save" }));
    expect(screen.getByRole("img", { name: "Save" })).toHaveAttribute("src", `data:image/png;base64,${image.data}`);
    expect(screen.getByText("#save")).toBeInTheDocument();
    expect(screen.queryByText(/browser-contexts-v1/)).not.toBeInTheDocument();
  });
});
