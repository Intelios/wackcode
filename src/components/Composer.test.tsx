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

describe("Composer comet and frozen", () => {
  const base = {
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

  it("adds the comet class when comet is set", () => {
    const { container } = render(<Composer {...base} comet />);
    expect(container.querySelector(".composer")).toHaveClass("comet");
    const plain = render(<Composer {...base} />);
    expect(plain.container.querySelector(".composer")).not.toHaveClass("comet");
  });

  it("shows the frozen text read-only instead of the draft", () => {
    render(<Composer {...base} frozen="Off it goes" />);
    const area = screen.getByRole("textbox");
    expect(area).toHaveValue("Off it goes");
    expect(area).toHaveAttribute("readonly");
    fireEvent.change(area, { target: { value: "Edited" } });
    expect(area).toHaveValue("Off it goes");
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
    setup({
      commands: [command, { id: "app:name", name: "name", description: "Rename", source: "app", sourceLabel: "WackCode" }],
      onCommand
    });
    const area = screen.getByRole("textbox");
    fireEvent.change(area, { target: { value: "/name ", selectionStart: 6 } });
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Enter a name after /name.");
    expect(area).toHaveValue("/name ");
  });

  it("shows /init but refuses image attachments for it", async () => {
    const onCommand = vi.fn().mockResolvedValue(true);
    setup({
      commands: [{ id: "app:init", name: "init", description: "Create AGENTS.md", source: "app", sourceLabel: "WackCode" }],
      onCommand
    });
    const area = screen.getByRole("textbox");
    fireEvent.change(area, { target: { value: "/ini", selectionStart: 4 } });
    expect(screen.getByRole("option", { name: /init/ })).toBeInTheDocument();
    attach(png());
    await screen.findByAltText("Attached image 1");
    fireEvent.change(area, { target: { value: "/init", selectionStart: 5 } });
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    expect(screen.getByRole("status")).toHaveTextContent("Remove images before running this command.");
    expect(onCommand).not.toHaveBeenCalled();
  });
});

describe("Composer queueing while the agent is working", () => {
  function setup(extra: Partial<React.ComponentProps<typeof Composer>> = {}) {
    const onSend = vi.fn().mockResolvedValue(true);
    render(<Composer status="running" providers={providers} providerId="p" modelId="sees" thinkingLevel="off"
      onConfigure={vi.fn()} onSend={onSend} onStop={vi.fn()} onOpenSettings={vi.fn()} {...extra} />);
    return { onSend, area: screen.getByRole("textbox") };
  }

  it("steers with Enter instead of refusing to send", async () => {
    const { onSend, area } = setup();
    fireEvent.change(area, { target: { value: "Try the other approach" } });
    fireEvent.keyDown(area, { key: "Enter" });
    await waitFor(() => expect(onSend).toHaveBeenCalledWith("Try the other approach", [], "steer"));
    expect(area).toHaveValue("");
  });

  it("queues a follow-up on ⌥Enter for after the run", async () => {
    const { onSend, area } = setup();
    fireEvent.change(area, { target: { value: "Then run the tests" } });
    fireEvent.keyDown(area, { key: "Enter", altKey: true });
    await waitFor(() => expect(onSend).toHaveBeenCalledWith("Then run the tests", [], "follow_up"));
  });

  it("keeps the draft while the run is stopping", () => {
    const { onSend, area } = setup({ status: "stopping" });
    fireEvent.change(area, { target: { value: "Hold this" } });
    fireEvent.keyDown(area, { key: "Enter" });
    expect(onSend).not.toHaveBeenCalled();
    expect(area).toHaveValue("Hold this");
  });

  it("restores the draft when the queue request fails", async () => {
    const onSend = vi.fn().mockResolvedValue(false);
    const { area } = setup({ onSend });
    fireEvent.change(area, { target: { value: "Try again later" } });
    fireEvent.keyDown(area, { key: "Enter" });
    await waitFor(() => expect(onSend).toHaveBeenCalled());
    expect(area).toHaveValue("Try again later");
  });

  it("queues app commands as refused but unknown slash text as a message", async () => {
    const compact = { id: "app:compact", name: "compact", description: "Summarize", source: "app" as const, sourceLabel: "WackCode" };
    const { onSend, area } = setup({ commands: [compact] });
    // Escape closes the command picker, so Enter queues instead of completing the command.
    fireEvent.change(area, { target: { value: "/compact" } });
    fireEvent.keyDown(area, { key: "Escape" });
    fireEvent.keyDown(area, { key: "Enter" });
    expect(await screen.findByRole("status")).toHaveTextContent("Wait for Pi to finish before running /compact.");
    expect(onSend).not.toHaveBeenCalled();

    fireEvent.change(area, { target: { value: "/not-a-command" } });
    fireEvent.keyDown(area, { key: "Enter" });
    await waitFor(() => expect(onSend).toHaveBeenCalledWith("/not-a-command", [], "steer"));
  });

  it("lets /goal pause reach a live run but holds a bare /goal back", async () => {
    const goal = { id: "app:goal", name: "goal", description: "Iterate", source: "app" as const, sourceLabel: "WackCode" };
    const onCommand = vi.fn().mockResolvedValue(true);
    const { onSend, area } = setup({ commands: [goal], onCommand });
    fireEvent.change(area, { target: { value: "/goal pause" } });
    fireEvent.keyDown(area, { key: "Escape" });
    fireEvent.keyDown(area, { key: "Enter" });
    await waitFor(() => expect(onCommand).toHaveBeenCalledWith("goal", "pause", []));
    expect(area).toHaveValue("");
    expect(onSend).not.toHaveBeenCalled();

    // A new goal can't start mid-run — it is refused like every other app command.
    fireEvent.change(area, { target: { value: "/goal ship it" } });
    fireEvent.keyDown(area, { key: "Enter" });
    expect(await screen.findByRole("status")).toHaveTextContent("Wait for Pi to finish before running /goal.");
    expect(onSend).not.toHaveBeenCalled();
    expect(onCommand).toHaveBeenCalledTimes(1);
  });

  it("speaks the chosen agent name in the busy notice and placeholders", async () => {
    const compact = { id: "app:compact", name: "compact", description: "Summarize", source: "app" as const, sourceLabel: "WackCode" };
    const { area } = setup({ commands: [compact], agentName: "Nova" });
    expect(area).toHaveAttribute("placeholder", "Nova is working — ⏎ steers the run, ⌥⏎ queues for after…");
    fireEvent.change(area, { target: { value: "/compact" } });
    fireEvent.keyDown(area, { key: "Escape" });
    fireEvent.keyDown(area, { key: "Enter" });
    expect(await screen.findByRole("status")).toHaveTextContent("Wait for Nova to finish before running /compact.");
  });

  it("lists queued messages and restores them into the draft", async () => {
    const onDequeue = vi.fn().mockResolvedValue(["Steered note", "Later note"]);
    const { area } = setup({
      queuedMessages: { steer: ["Steered note"], followUp: ["Later note"] },
      onDequeue
    });
    const list = screen.getByRole("list", { name: "Queued messages" });
    expect(list).toHaveTextContent("Steered note");
    expect(list).toHaveTextContent("Later note");
    expect(screen.getAllByRole("listitem")[0]).toHaveTextContent("Steering");
    fireEvent.click(screen.getAllByRole("button", { name: "Restore queued messages to the composer" })[0]);
    await waitFor(() => expect(onDequeue).toHaveBeenCalled());
    expect(area).toHaveValue("Steered note\n\nLater note");
  });
});

describe("Composer @ file mentions", () => {
  const files = ["README.md", "src/App.tsx", "src/components/Composer.tsx", "my notes.txt"];

  function setup(extra: Partial<React.ComponentProps<typeof Composer>> = {}) {
    const onSend = vi.fn().mockResolvedValue(true);
    const onRequestMentions = vi.fn();
    render(<Composer status="idle" providers={providers} providerId="p" modelId="sees" thinkingLevel="off"
      onConfigure={vi.fn()} onSend={onSend} onStop={vi.fn()} onOpenSettings={vi.fn()}
      mentionFiles={files} onRequestMentions={onRequestMentions} {...extra} />);
    return { onSend, onRequestMentions, area: screen.getByRole("textbox") };
  }

  it("suggests matching files and inserts the chosen one", () => {
    const { onRequestMentions, onSend, area } = setup();
    fireEvent.change(area, { target: { value: "look at @rea", selectionStart: 12 } });
    expect(onRequestMentions).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("listbox", { name: "Files" })).toBeInTheDocument();
    expect(screen.getAllByRole("option")[0]).toHaveTextContent("README.md");
    fireEvent.keyDown(area, { key: "Enter" });
    expect(area).toHaveValue("look at @README.md ");
    expect(onSend).not.toHaveBeenCalled();
    expect(screen.queryByRole("listbox", { name: "Files" })).not.toBeInTheDocument();
  });

  it("keeps the picker open inside a chosen folder", () => {
    const { area } = setup();
    fireEvent.change(area, { target: { value: "@comp", selectionStart: 5 } });
    const folder = screen.getAllByRole("option")[0];
    expect(folder).toHaveTextContent("components/src/");
    fireEvent.click(folder);
    expect(area).toHaveValue("@src/components/");
    fireEvent.select(area, { target: { selectionStart: 16 } });
    expect(screen.getAllByRole("option")[0]).toHaveTextContent("Composer.tsx");
  });

  it("quotes paths with spaces", () => {
    const { area } = setup();
    fireEvent.change(area, { target: { value: "@notes", selectionStart: 6 } });
    fireEvent.keyDown(area, { key: "Tab" });
    expect(area).toHaveValue("@\"my notes.txt\" ");
  });

  it("closes on Escape so Enter sends", () => {
    const { onSend, area } = setup();
    fireEvent.change(area, { target: { value: "see @src", selectionStart: 8 } });
    fireEvent.keyDown(area, { key: "Escape" });
    expect(screen.queryByRole("listbox", { name: "Files" })).not.toBeInTheDocument();
    fireEvent.keyDown(area, { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("see @src", []);
  });

  it("shows loading and errors, and ignores emails", () => {
    const onRequestMentions = vi.fn();
    const { area } = setup({ mentionFiles: undefined, mentionsLoading: true, onRequestMentions });
    fireEvent.change(area, { target: { value: "mail a@b", selectionStart: 8 } });
    expect(screen.queryByRole("listbox", { name: "Files" })).not.toBeInTheDocument();
    fireEvent.change(area, { target: { value: "@", selectionStart: 1 } });
    expect(screen.getByRole("listbox", { name: "Files" })).toHaveTextContent("Loading files…");
    cleanup();
    const failed = setup({ mentionFiles: undefined, mentionsError: "Pick a project to mention its files." });
    fireEvent.change(failed.area, { target: { value: "@", selectionStart: 1 } });
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(failed.onRequestMentions).toHaveBeenCalledTimes(2);
  });
});
