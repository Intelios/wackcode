import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NormalizedMessage } from "../types";
import { replyImageReference, replyImages } from "../reply-images";
import { Markdown } from "./Markdown";
import { ReplyImageScope } from "./ReplyImage";

vi.mock("../api", () => ({ api: { revealPath: vi.fn().mockResolvedValue(undefined) } }));
afterEach(cleanup);

const preview = "data:image/png;base64,dGh1bWI=";
const messages: NormalizedMessage[] = [
  { id: "u", entryId: "upload", role: "user", blocks: [{ type: "image", thumbnail: preview }] },
  { id: "r", entryId: "result", role: "tool", blocks: [{ type: "tool-result", toolCallId: "screenshot", images: [{ imageId: "one", thumbnail: preview }] }] }
];

describe("reply image Markdown", () => {
  it("draws selected images in prose order, with an optional caption, and loads the right saved source on demand", async () => {
    const loadMessage = vi.fn().mockResolvedValue("data:image/png;base64,ZnVsbA==");
    const loadTool = vi.fn().mockResolvedValue("data:image/png;base64,ZnVsbA==");
    const { container } = render(<ReplyImageScope.Provider value={{ images: replyImages(messages), loadMessage, loadTool }}>
      <Markdown replyImages>{`Before.\n\n![Original](${replyImageReference("upload", 0)} "Before the edit")\n\nAfter.\n\n![Result](${replyImageReference("result", 0)})`}</Markdown>
    </ReplyImageScope.Provider>);
    expect([...container.querySelectorAll("p")].map((paragraph) => paragraph.textContent)).toEqual(["Before.", "Before the edit", "After.", ""]);
    expect(screen.getByAltText("Original")).toHaveAttribute("src", preview);
    expect(loadMessage).not.toHaveBeenCalled();
    expect(loadTool).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Open image: Original" }));
    expect(loadMessage).toHaveBeenCalledWith("upload", 0);
    await act(async () => {});
    expect(within(screen.getByRole("dialog", { name: "Original" })).getByAltText("Original")).toHaveAttribute("src", "data:image/png;base64,ZnVsbA==");
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open image: Result" }));
    expect(loadTool).toHaveBeenCalledWith("screenshot", 0);
    await act(async () => {});
  });

  it("stays blocked without the prose opt-in and never renders arbitrary image URLs", () => {
    const source = `![Stored](${replyImageReference("upload", 0)})`;
    const { rerender, container } = render(<ReplyImageScope.Provider value={{ images: replyImages(messages) }}><Markdown>{source}</Markdown></ReplyImageScope.Provider>);
    expect(container.querySelector("img")).toBeNull();
    rerender(<ReplyImageScope.Provider value={{ images: replyImages(messages) }}><Markdown replyImages>{[
      "![Remote](https://example.com/pixel.png)", "![Local](/tmp/image.png)", "![Inline](data:image/png;base64,AA==)",
      "![File](file:///tmp/image.png)", "![Malformed](wackcode-image://entry/upload/00)"
    ].join("\n\n")}</Markdown></ReplyImageScope.Provider>);
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelectorAll(".blocked-image")).toHaveLength(4);
    expect(screen.getByRole("img", { name: "Image unavailable: Malformed" })).toBeInTheDocument();
  });

  it("shows pending and unavailable states without loading any source", () => {
    const preparing: NormalizedMessage[] = [{ ...messages[0], blocks: [{ type: "image" }] }];
    const { rerender } = render(<ReplyImageScope.Provider value={{ images: replyImages(preparing) }}>
      <Markdown replyImages>{`![Original](${replyImageReference("upload", 0)})`}</Markdown>
    </ReplyImageScope.Provider>);
    expect(screen.getByRole("img", { name: "Preparing image: Original" })).toBeInTheDocument();
    rerender(<ReplyImageScope.Provider value={{ images: replyImages(messages) }}>
      <Markdown replyImages>{`![Missing](${replyImageReference("other-branch", 0)})`}</Markdown>
    </ReplyImageScope.Provider>);
    expect(screen.getByRole("img", { name: "Image unavailable: Missing" })).toBeInTheDocument();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("handles image load failure and removes an image link's outer navigation", () => {
    const { container } = render(<ReplyImageScope.Provider value={{ images: replyImages(messages) }}>
      <Markdown replyImages>{`[*![Original](${replyImageReference("upload", 0)})*](https://example.com/)`}</Markdown>
    </ReplyImageScope.Provider>);
    expect(container.querySelector("a")).toBeNull();
    fireEvent.error(screen.getByAltText("Original"));
    expect(screen.getByRole("img", { name: "Image unavailable: Original" })).toBeInTheDocument();
    expect(container.querySelector("img")).toBeNull();
  });

  it("keeps custom image references unusable as links", () => {
    render(<Markdown replyImages>{`[Stored image](${replyImageReference("upload", 0)})`}</Markdown>);
    expect(screen.getByText("Stored image")).toHaveAttribute("href", "");
  });

  it("does not reload an open original when an unrelated snapshot rebuilds the registry", async () => {
    const loadMessage = vi.fn().mockResolvedValue(undefined);
    const surface = () => <ReplyImageScope.Provider value={{ images: replyImages(messages), loadMessage }}>
      <Markdown replyImages>{`![Original](${replyImageReference("upload", 0)})`}</Markdown>
    </ReplyImageScope.Provider>;
    const view = render(surface());
    fireEvent.click(screen.getByRole("button", { name: "Open image: Original" }));
    await act(async () => {});
    expect(loadMessage).toHaveBeenCalledOnce();
    view.rerender(surface());
    await act(async () => {});
    expect(loadMessage).toHaveBeenCalledOnce();
    expect(within(screen.getByRole("dialog")).getByAltText("Original")).toHaveAttribute("src", preview);
  });

  it("reloads an open original when an edit replaces the image at the same saved reference", async () => {
    const firstFull = "data:image/png;base64,Zmlyc3Q=";
    const nextFull = "data:image/png;base64,bmV4dA==";
    const loadMessage = vi.fn().mockResolvedValueOnce(firstFull).mockResolvedValueOnce(nextFull);
    const surface = (thumbnail: string) => <ReplyImageScope.Provider value={{
      images: replyImages([{ ...messages[0], blocks: [{ type: "image", thumbnail }] }]), loadMessage
    }}>
      <Markdown replyImages>{`![Original](${replyImageReference("upload", 0)})`}</Markdown>
    </ReplyImageScope.Provider>;
    const view = render(surface(preview));
    fireEvent.click(screen.getByRole("button", { name: "Open image: Original" }));
    await act(async () => {});
    expect(within(screen.getByRole("dialog")).getByAltText("Original")).toHaveAttribute("src", firstFull);
    view.rerender(surface("data:image/png;base64,bmV3LXRodW1i"));
    await act(async () => {});
    expect(loadMessage).toHaveBeenCalledTimes(2);
    expect(loadMessage).toHaveBeenLastCalledWith("upload", 0);
    expect(within(screen.getByRole("dialog")).getByAltText("Original")).toHaveAttribute("src", nextFull);
  });

  it("shows a completed streaming reference once and keeps it when the answer settles", () => {
    const source = `Before.\n\n![Original](${replyImageReference("upload", 0)})`;
    const surface = (text: string, streaming: boolean) => <ReplyImageScope.Provider value={{ images: replyImages(messages) }}>
      <Markdown replyImages streaming={streaming}>{text}</Markdown>
    </ReplyImageScope.Provider>;
    const view = render(surface(source.slice(0, -3), true));
    expect(screen.queryByRole("button", { name: "Open image: Original" })).toBeNull();
    view.rerender(surface(source, true));
    expect(screen.getAllByRole("button", { name: "Open image: Original" })).toHaveLength(1);
    view.rerender(surface(`${source}\n\nAfter.`, false));
    expect(screen.getAllByRole("button", { name: "Open image: Original" })).toHaveLength(1);
    expect(screen.getByText("After.")).toBeInTheDocument();
  });

  it("reloads a changed saved original even when its thumbnail pixels are identical", async () => {
    const firstFull = "data:image/png;base64,Zmlyc3Q=";
    const nextFull = "data:image/png;base64,bmV4dA==";
    const loadMessage = vi.fn().mockResolvedValueOnce(firstFull).mockResolvedValueOnce(nextFull);
    const surface = (imageId: string) => <ReplyImageScope.Provider value={{
      images: replyImages([{ ...messages[0], blocks: [{ type: "image", imageId, thumbnail: preview }] }]), loadMessage
    }}>
      <Markdown replyImages>{`![Original](${replyImageReference("upload", 0)})`}</Markdown>
    </ReplyImageScope.Provider>;
    const view = render(surface("first-image"));
    fireEvent.click(screen.getByRole("button", { name: "Open image: Original" }));
    await act(async () => {});
    expect(within(screen.getByRole("dialog")).getByAltText("Original")).toHaveAttribute("src", firstFull);
    view.rerender(surface("changed-image"));
    await act(async () => {});
    expect(loadMessage).toHaveBeenCalledTimes(2);
    expect(within(screen.getByRole("dialog")).getByAltText("Original")).toHaveAttribute("src", nextFull);
  });
});
