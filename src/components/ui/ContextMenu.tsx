import { createContext, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Icon } from "../Icons";
import { Menu, type MenuEntry } from "./Menu";

type ContextEvent = MouseEvent | React.MouseEvent;
type OpenMenu = (event: ContextEvent, items: MenuEntry[], label?: string, textEditing?: boolean) => void;
const ContextMenus = createContext<OpenMenu>(() => undefined);
export const useContextMenu = () => useContext(ContextMenus);
const ContextClipboard = createContext<Pick<Props, "copyText" | "readText"> | undefined>(undefined);
export const useContextClipboard = () => useContext(ContextClipboard);

interface Props {
  children: ReactNode;
  items: MenuEntry[];
  copyText: (text: string) => Promise<void>;
  readText: () => Promise<string>;
  openLink: (url: string) => Promise<unknown>;
  onError: (message: string) => void;
  /** Navigation invalidates actions captured for the previous surface. */
  scope: string;
}
interface OpenState {
  id: number;
  x: number;
  y: number;
  items: MenuEntry[];
  label: string;
  focus: HTMLElement | null;
}

/** One pointer-anchored menu for the renderer. Native browser previews are separate webviews.
 * Capture selected text before Menu takes focus; editable fields own their menu even inside
 * a chat/file target. Clipboard reads happen only when the user chooses Paste text.
 */
export function ContextMenuProvider({ children, items, copyText, readText, openLink, onError, scope }: Props) {
  const clipboard = useMemo(() => ({ copyText, readText }), [copyText, readText]);
  const [menu, setMenu] = useState<OpenState>();
  const sequence = useRef(0);
  const panel = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ left: 0, top: 0 });
  const scopeRef = useRef(scope);
  scopeRef.current = scope;
  const menuRef = useRef(menu);
  menuRef.current = menu;
  const close = useCallback((restore = true) => {
    const focus = menuRef.current?.focus;
    if (restore && focus?.isConnected) focus.focus({ preventScroll: true });
    setMenu(undefined);
  }, []);

  const open: OpenMenu = useCallback((event, contextualItems, label = "App menu", textEditing = true) => {
    event.preventDefault();
    event.stopPropagation();
    const target = event.target instanceof Element ? event.target : null;
    if (!target || panel.current?.contains(target)) return;
    const field = target.closest("textarea, input");
    let entries: MenuEntry[] = [];
    if (textEditing && (field instanceof HTMLTextAreaElement || (field instanceof HTMLInputElement && field.selectionStart !== null))) {
      const start = field.selectionStart ?? 0;
      const end = field.selectionEnd ?? start;
      const value = field.value;
      const originalScope = scopeRef.current;
      const secret = field instanceof HTMLInputElement && field.type === "password";
      const editable = !field.disabled && !field.readOnly;
      const selected = value.slice(start, end);
      const replace = (text: string) => {
        // A delayed clipboard read must not overwrite edits made after opening this menu.
        if (!field.isConnected || scopeRef.current !== originalScope || field.value !== value || field.disabled || field.readOnly
          || document.activeElement !== field || field.selectionStart !== start || field.selectionEnd !== end) return;
        field.focus({ preventScroll: true });
        field.setSelectionRange(start, end);
        // WebKit's editing command preserves the native undo stack and dispatches input,
        // so controlled React fields and the composer's draft both update normally.
        if (!document.execCommand("insertText", false, text)) throw new Error("Couldn’t edit the text. Try the keyboard shortcut.");
      };
      entries = [
        { label: "Cut", hint: "⌘X", disabled: !editable || !selected || secret, onSelect: async () => { await copyText(selected); replace(""); } },
        { label: "Copy", icon: <Icon name="copy" />, hint: "⌘C", disabled: !selected || secret || field.disabled, onSelect: () => copyText(selected) },
        { label: "Paste text", hint: "⌘V", disabled: !editable, onSelect: async () => replace(await readText()) },
        "separator",
        { label: "Select all", hint: "⌘A", disabled: !value || field.disabled, onSelect: () => { field.focus(); field.select(); } }
      ];
      label = "Text menu";
    } else {
      const selection = window.getSelection();
      // Ignore a stale selection elsewhere in the window.
      const selected = textEditing && selection?.rangeCount && selection.getRangeAt(0).intersectsNode(target) ? selection.toString() : "";
      if (selected) entries.push({ label: "Copy selection", icon: <Icon name="copy" />, hint: "⌘C", onSelect: () => copyText(selected) });
      const link = target.closest<HTMLAnchorElement>("a[href]");
      if (textEditing && link && /^https?:\/\//i.test(link.href)) {
        entries.push(
          { label: "Open link in browser", icon: <Icon name="external" />, onSelect: () => { void openLink(link.href).catch((reason) => onError(String(reason))); } },
          { label: "Copy link", icon: <Icon name="copy" />, onSelect: () => copyText(link.href) }
        );
      }
      if (entries.length && contextualItems.length) entries.push("separator");
      entries.push(...contextualItems);
    }
    if (!entries.length) { close(false); return; }
    const keyboard = event.clientX === 0 && event.clientY === 0;
    const rect = target.getBoundingClientRect();
    const focus = target.closest<HTMLElement>("button, input, textarea, [tabindex]") ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    setMenu({ id: ++sequence.current, x: keyboard ? rect.left : event.clientX, y: keyboard ? rect.bottom : event.clientY, items: entries, label, focus });
  }, [copyText, readText, openLink, onError, close]);

  useEffect(() => {
    const context = (event: MouseEvent) => { if (!event.defaultPrevented) open(event, document.querySelector('[aria-modal="true"]') ? [] : items); };
    const keyboard = (event: KeyboardEvent) => {
      if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
      event.preventDefault();
      event.stopPropagation();
      (event.target as Element)?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    };
    document.addEventListener("contextmenu", context);
    document.addEventListener("keydown", keyboard, true);
    return () => { document.removeEventListener("contextmenu", context); document.removeEventListener("keydown", keyboard, true); };
  }, [open, items]);

  useEffect(() => { close(false); }, [scope, close]);
  useLayoutEffect(() => {
    if (!menu || !panel.current) return;
    const { width, height } = panel.current.getBoundingClientRect();
    setPosition({ left: Math.max(8, Math.min(menu.x, window.innerWidth - width - 8)), top: Math.max(8, Math.min(menu.y, window.innerHeight - height - 8)) });
  }, [menu]);

  useEffect(() => {
    if (!menu) return;
    const pointer = (event: PointerEvent) => { if (!panel.current?.contains(event.target as Node)) close(false); };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape" || event.key === "Tab") {
        event.preventDefault(); event.stopPropagation(); close();
      }
    };
    const dismiss = () => close(false);
    const scroll = (event: Event) => { if (!(event.target instanceof Node) || !panel.current?.contains(event.target)) dismiss(); };
    document.addEventListener("pointerdown", pointer, true);
    document.addEventListener("keydown", key, true);
    window.addEventListener("resize", dismiss);
    window.addEventListener("blur", dismiss);
    window.addEventListener("scroll", scroll, true);
    return () => {
      document.removeEventListener("pointerdown", pointer, true); document.removeEventListener("keydown", key, true);
      window.removeEventListener("resize", dismiss); window.removeEventListener("blur", dismiss); window.removeEventListener("scroll", scroll, true);
    };
  }, [menu, close]);

  return <ContextMenus.Provider value={open}>
    <ContextClipboard.Provider value={clipboard}>{children}</ContextClipboard.Provider>
    {menu && createPortal(
      <div ref={panel} className="popover context-menu" style={position} onContextMenu={(event) => event.preventDefault()}>
        <Menu key={menu.id} label={menu.label} items={menu.items.map((item) => item === "separator" ? item : {
          ...item, onSelect: () => { try { Promise.resolve(item.onSelect?.()).catch((reason) => onError(String(reason))); } catch (reason) { onError(String(reason)); } }
        })} onClose={close} />
      </div>, document.body
    )}
  </ContextMenus.Provider>;
}
