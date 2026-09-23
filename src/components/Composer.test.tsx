import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ImageContent, ProviderRecord } from "../types";
import { Composer } from "./Composer";

afterEach(cleanup);

const providers: ProviderRecord[] = [{
  id: "p",
  name: "Gateway",
  baseUrl: "https://example.test/v1",
  apiFormat: "openai-completions",
  createdAt: "2026-01-01",
  updatedAt: "2026-01-01",
  kind: "custom",
  connected: true,
  hasApiKey: true,
  models: [
    { id: "sees", name: "Sees", contextWindow: 8000, maxTokens: 1000, reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: { off: null }, vision: true },
    { id: "blind", name: "Blind", contextWindow: 8000, maxTokens: 1000, reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: { off: null }, vision: false }
  ]
}];

function png(name = "shot.png"): File {
  return new File([new Uint8Array([0x89, 0x50, 0x4e, 0x47])], name, { type: "image/png" });
}

function renderComposer(modelId: string, onSend: (message: string, images: ImageContent[]) => Promise<boolean> = vi.fn().mockResolvedValue(true)) {
  const props = {
    status: "idle" as const,
    providers,
    providerId: "p",
    thinkingLevel: "off" as const,
    onConfigure: vi.fn(),
    onSend,
    onStop: vi.fn(),
    onOpenSettings: vi.fn()
  };
  const view = render(<Composer {...props} modelId={modelId} />);
  return { ...view, onSend, rerenderWith: (next: string) => view.rerender(<Composer {...props} modelId={next} />) };
}

function attach(...files: File[]) {
  fireEvent.change(screen.getByTestId("attach-input"), { target: { files } });
}

describe("Composer image attachments", () => {
  it("explains instead of attaching when the model has no vision", async () => {
    renderComposer("blind");
    const button = screen.getByRole("button", { name: "Attach images" });
    expect(button).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(button);
    expect(screen.getByRole("status")).toHaveTextContent("Blind doesn't accept images");
    fireEvent.paste(screen.getByRole("textbox"), { clipboardData: { files: [png()], types: ["Files"] } });
    expect(screen.queryByAltText("Attached image 1")).not.toBeInTheDocument();
  });

  it("sends picked images with the message and clears the tray", async () => {
    const { onSend } = renderComposer("sees");
    attach(png());
    expect(await screen.findByAltText("Attached image 1")).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "What is this?" } });
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(onSend).toHaveBeenCalledWith("What is this?", [{ type: "image", mimeType: "image/png", data: "iVBORw==" }]));
    expect(screen.queryByAltText("Attached image 1")).not.toBeInTheDocument();
  });

  it("attaches a pasted screenshot but leaves text pastes alone", async () => {
    renderComposer("sees");
    const box = screen.getByRole("textbox");
    fireEvent.paste(box, { clipboardData: { files: [png()], types: ["text/plain", "Files"] } });
    expect(screen.queryByAltText("Attached image 1")).not.toBeInTheDocument();
    fireEvent.paste(box, { clipboardData: { files: [png()], types: ["Files"] } });
    expect(await screen.findByAltText("Attached image 1")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Remove image 1" }));
    expect(screen.queryByAltText("Attached image 1")).not.toBeInTheDocument();
  });

  it("restores the message and images when the send fails", async () => {
    const onSend = vi.fn().mockResolvedValue(false);
    renderComposer("sees", onSend);
    attach(png());
    await screen.findByAltText("Attached image 1");
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "Look" } });
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(onSend).toHaveBeenCalled());
    expect(await screen.findByAltText("Attached image 1")).toBeInTheDocument();
    expect(screen.getByRole("textbox")).toHaveValue("Look");
  });

  it("blocks sending when the model is switched to one without vision", async () => {
    const { rerenderWith } = renderComposer("sees");
    attach(png());
    await screen.findByAltText("Attached image 1");
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "Look" } });
    rerenderWith("blind");
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent("remove the images");
  });
});

describe("Composer seed", () => {
  it("replaces the draft only when a new seed arrives", () => {
    const props = {
      status: "idle" as const,
      providers,
      providerId: "p",
      modelId: "sees",
      thinkingLevel: "off" as const,
      onConfigure: vi.fn(),
      onSend: vi.fn().mockResolvedValue(true),
      onStop: vi.fn(),
      onOpenSettings: vi.fn()
    };
    const view = render(<Composer {...props} seed={{ text: "Rewound request", nonce: 1 }} />);
    const area = screen.getByRole("textbox");
    expect(area).toHaveValue("Rewound request");
    fireEvent.change(area, { target: { value: "Rewound request, tweaked" } });
    view.rerender(<Composer {...props} seed={{ text: "Rewound request", nonce: 1 }} />);
    expect(area).toHaveValue("Rewound request, tweaked");
    view.rerender(<Composer {...props} seed={{ text: "Another", nonce: 2 }} />);
    expect(area).toHaveValue("Another");
  });
});

describe("Composer slash commands", () => {
  const command = { id: "extension:hello", name: "hello", description: "Say hello", source: "extension" as const, sourceLabel: "Fixture" };
  function setup(extra: Partial<React.ComponentProps<typeof Composer>> = {}) {
    const onCommand = vi.fn().mockResolvedValue(true);
    const onLiteral = vi.fn().mockResolvedValue(true);
    const onRequestCommands = vi.fn();
    render(<Composer status="idle" providers={providers} providerId="p" modelId="sees" thinkingLevel="off"
      onConfigure={vi.fn()} onSend={vi.fn().mockResolvedValue(true)} onStop={vi.fn()} onOpenSettings={vi.fn()}
      commands={[command]} onCommand={onCommand} onLiteral={onLiteral} onRequestCommands={onRequestCommands} {...extra} />);
    return { onCommand, onLiteral, onRequestCommands };
  }

  it("filters, selects, and runs a command with arguments", async () => {
    const { onCommand, onRequestCommands } = setup();
    const area = screen.getByRole("textbox");
    fireEvent.change(area, { target: { value: "/he", selectionStart: 3 } });
    expect(onRequestCommands).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("listbox", { name: "Slash commands" })).toBeInTheDocument();
    fireEvent.keyDown(area, { key: "Tab" });
    expect(area).toHaveValue("/hello ");
    fireEvent.change(area, { target: { value: "/he world", selectionStart: 3 } });
    fireEvent.click(screen.getByRole("option", { name: /hello/ }));
    expect(area).toHaveValue("/hello world");
    fireEvent.change(area, { target: { value: "/hello world", selectionStart: 12 } });
    fireEvent.keyDown(area, { key: "Enter" });
    await waitFor(() => expect(onCommand).toHaveBeenCalledWith("hello", "world", []));
  });

  it("keeps unknown slash text until Send as message is chosen", async () => {
    const { onLiteral, onCommand } = setup();
    const area = screen.getByRole("textbox");
    fireEvent.change(area, { target: { value: "/unknown", selectionStart: 8 } });
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    expect(area).toHaveValue("/unknown");
    expect(onCommand).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Send as message" }));
    await waitFor(() => expect(onLiteral).toHaveBeenCalledWith("/unknown", []));
  });

  it("carries a new draft into the initialized chat and clears it after a send", () => {
    const props = {
      status: "idle" as const, providers, providerId: "p", modelId: "sees", thinkingLevel: "off" as const,
      onConfigure: vi.fn(), onSend: vi.fn().mockResolvedValue(true), onStop: vi.fn(), onOpenSettings: vi.fn()
    };
    const view = render(<Composer {...props} transfer={{ text: "/hello draft", images: [], nonce: 1 }} />);
    expect(screen.getByRole("textbox")).toHaveValue("/hello draft");
    view.rerender(<Composer {...props} transfer={{ text: "", images: [], nonce: 2 }} />);
    expect(screen.getByRole("textbox")).toHaveValue("");
  });

  it("keeps a command and its attachments after validation fails", async () => {
    const onCommand = vi.fn().mockRejectedValue(new Error("Enter a name after /name."));
    setup({ onCommand });
    const area = screen.getByRole("textbox");
    fireEvent.change(area, { target: { value: "/name ", selectionStart: 6 } });
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Enter a name after /name.");
    expect(area).toHaveValue("/name ");
  });
});
