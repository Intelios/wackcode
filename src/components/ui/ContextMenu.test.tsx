import { useState } from "react";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ContextMenuProvider, useContextMenu } from "./ContextMenu";

afterEach(() => { cleanup(); window.getSelection()?.removeAllRanges(); vi.restoreAllMocks(); });

function Target({ action = () => undefined }: { action?: () => void }) {
  const menu = useContextMenu();
  return <button onContextMenu={(event) => menu(event, [
    { label: "Unavailable", disabled: true },
    "separator",
    { label: "Rename", onSelect: action },
    { label: "Delete", danger: true, onSelect: action }
  ], "Chat menu")}>Chat row</button>;
}

function setup(children = <Target />, props: Partial<React.ComponentProps<typeof ContextMenuProvider>> = {}) {
  const copyText = vi.fn().mockResolvedValue(undefined);
  const readText = vi.fn().mockResolvedValue("pasted");
  const openLink = vi.fn().mockResolvedValue(undefined);
  const onError = vi.fn();
  const renderProps = { items: [{ label: "New chat" }], copyText, readText, openLink, onError, scope: "chat1", ...props };
  const result = render(<ContextMenuProvider {...renderProps}>{children}</ContextMenuProvider>);
  return { ...result, ...renderProps };
}

function rightClick(target: HTMLElement) {
  const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 40, clientY: 60 });
  fireEvent(target, event);
  expect(event.defaultPrevented).toBe(true);
}

describe("app context menus", () => {
  it("opens the target's menu, skips disabled entries and restores focus before an action", () => {
    const action = vi.fn();
    setup(<Target action={action} />);
    const row = screen.getByRole("button", { name: "Chat row" });
    row.focus(); rightClick(row);
    expect(screen.getByRole("menu", { name: "Chat menu" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "New chat" })).toBeNull();
    expect(screen.getByRole("menuitem", { name: "Unavailable" })).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: "Rename" })).toHaveFocus();
    fireEvent.keyDown(document.activeElement!, { key: "ArrowUp" });
    expect(screen.getByRole("menuitem", { name: "Delete" })).toHaveFocus();
    fireEvent.keyDown(document.activeElement!, { key: "Enter" });
    expect(action).toHaveBeenCalledOnce();
    expect(row).toHaveFocus();
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("supports keyboard invocation and Escape without leaking keys to app shortcuts", () => {
    setup();
    const row = screen.getByRole("button", { name: "Chat row" });
    row.focus();
    fireEvent.keyDown(row, { key: "F10", shiftKey: true });
    expect(screen.getByRole("menu", { name: "Chat menu" })).toBeInTheDocument();
    const shortcut = vi.fn();
    window.addEventListener("keydown", shortcut);
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(shortcut).not.toHaveBeenCalled();
    window.removeEventListener("keydown", shortcut);
    expect(row).toHaveFocus();
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("dismisses on outside click, navigation, scrolling and window blur", () => {
    const { rerender, ...props } = setup();
    const row = screen.getByRole("button", { name: "Chat row" });
    rightClick(row); fireEvent.pointerDown(document.body);
    expect(screen.queryByRole("menu")).toBeNull();
    rightClick(row); fireEvent.scroll(window);
    expect(screen.queryByRole("menu")).toBeNull();
    rightClick(row); fireEvent.blur(window);
    expect(screen.queryByRole("menu")).toBeNull();
    rightClick(row);
    rerender(<ContextMenuProvider {...props} scope="chat2"><Target /></ContextMenuProvider>);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("clamps the menu to window edges and replaces the WebKit fallback on blank space", () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ width: 220, height: 170, top: 0, left: 0, right: 220, bottom: 170, x: 0, y: 0, toJSON: () => ({}) });
    setup(<div>Empty space</div>);
    fireEvent.contextMenu(screen.getByText("Empty space"), { clientX: window.innerWidth - 1, clientY: window.innerHeight - 1 });
    const popup = screen.getByRole("menu").parentElement!;
    expect(popup.style.left).toBe(`${window.innerWidth - 228}px`);
    expect(popup.style.top).toBe(`${window.innerHeight - 178}px`);
    expect(screen.getByRole("menuitem", { name: "New chat" })).toBeInTheDocument();
  });

  it("captures a selection and exposes only user-triggered HTTP(S) link actions", async () => {
    const { copyText, openLink } = setup(<a href="https://example.com/docs">Read these docs</a>);
    const link = screen.getByRole("link");
    const range = document.createRange(); range.selectNodeContents(link);
    window.getSelection()?.addRange(range);
    rightClick(link);
    window.getSelection()?.removeAllRanges();
    fireEvent.click(screen.getByRole("menuitem", { name: /Copy selection/ }));
    expect(copyText).toHaveBeenCalledWith("Read these docs");
    rightClick(link);
    fireEvent.click(screen.getByRole("menuitem", { name: "Copy link" }));
    expect(copyText).toHaveBeenCalledWith("https://example.com/docs");
    expect(openLink).not.toHaveBeenCalled();
    rightClick(link);
    fireEvent.click(screen.getByRole("menuitem", { name: "Open link in browser" }));
    expect(openLink).toHaveBeenCalledWith("https://example.com/docs");
  });

  it("gives a nested controlled text field its own menu and reads clipboard only on paste", async () => {
    function Editor() {
      const open = useContextMenu();
      const [value, setValue] = useState("hello world");
      return <div onContextMenu={(event) => open(event, [{ label: "Delete chat" }])}>
        <textarea aria-label="Draft" value={value} onChange={(event) => setValue(event.target.value)} />
      </div>;
    }
    const command = vi.fn().mockImplementation((_command, _ui, text) => {
      const field = screen.getByRole("textbox") as HTMLTextAreaElement;
      const next = field.value.slice(0, field.selectionStart) + text + field.value.slice(field.selectionEnd);
      fireEvent.input(field, { target: { value: next } });
      return true;
    });
    Object.defineProperty(document, "execCommand", { configurable: true, value: command });
    const { readText, copyText } = setup(<Editor />);
    const field = screen.getByRole("textbox") as HTMLTextAreaElement;
    field.focus(); field.setSelectionRange(6, 11);
    rightClick(field);
    expect(screen.queryByRole("menuitem", { name: "Delete chat" })).toBeNull();
    expect(readText).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("menuitem", { name: /^Copy/ }));
    expect(copyText).toHaveBeenCalledWith("world");
    field.setSelectionRange(6, 11); rightClick(field);
    await act(async () => { fireEvent.click(screen.getByRole("menuitem", { name: /Paste text/ })); });
    expect(command).toHaveBeenCalledWith("insertText", false, "pasted");
    expect(field.value).toBe("hello pasted");
  });

  it("does not overwrite a newer draft when clipboard reading finishes late", async () => {
    let resolve!: (text: string) => void;
    const readText = vi.fn(() => new Promise<string>((done) => { resolve = done; }));
    const command = vi.fn();
    Object.defineProperty(document, "execCommand", { configurable: true, value: command });
    setup(<textarea aria-label="Draft" defaultValue="old" />, { readText });
    const field = screen.getByRole("textbox");
    rightClick(field);
    fireEvent.click(screen.getByRole("menuitem", { name: /Paste text/ }));
    fireEvent.input(field, { target: { value: "new" } });
    await act(async () => resolve("late paste"));
    expect(command).not.toHaveBeenCalled();
    expect(field).toHaveValue("new");
  });

  it("does not paste into another chat that reuses the same empty composer field", async () => {
    let resolve!: (text: string) => void;
    const readText = vi.fn(() => new Promise<string>((done) => { resolve = done; }));
    const command = vi.fn();
    Object.defineProperty(document, "execCommand", { configurable: true, value: command });
    const { rerender, ...props } = setup(<textarea aria-label="Draft" />, { readText });
    const field = screen.getByRole("textbox");
    field.focus(); rightClick(field);
    fireEvent.click(screen.getByRole("menuitem", { name: /Paste text/ }));
    rerender(<ContextMenuProvider {...props} scope="chat2"><textarea aria-label="Draft" /></ContextMenuProvider>);
    await act(async () => resolve("belongs to chat1"));
    expect(command).not.toHaveBeenCalled();
    expect(screen.getByRole("textbox")).toHaveValue("");
  });

  it("disables writes on read-only fields and does not offer to copy passwords", () => {
    setup(<><textarea aria-label="Read only" defaultValue="read me" readOnly /><input aria-label="Secret" type="password" defaultValue="secret" /></>);
    const field = screen.getByRole("textbox");
    (field as HTMLTextAreaElement).select(); rightClick(field);
    expect(screen.getByRole("menuitem", { name: /^Cut/ })).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: /Paste text/ })).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: /^Copy/ })).not.toBeDisabled();
    const secret = screen.getByLabelText("Secret") as HTMLInputElement;
    secret.select(); rightClick(secret);
    expect(screen.getByRole("menuitem", { name: /^Copy/ })).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: /^Cut/ })).toBeDisabled();
  });

  it("reports clipboard failures", async () => {
    const { onError } = setup(<textarea aria-label="Draft" />, { readText: vi.fn().mockRejectedValue(new Error("Clipboard unavailable")) });
    rightClick(screen.getByRole("textbox"));
    await act(async () => { fireEvent.click(screen.getByRole("menuitem", { name: /Paste text/ })); });
    expect(onError).toHaveBeenCalledWith("Error: Clipboard unavailable");
  });
});
