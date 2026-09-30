import { useRef, useState } from "react";
import { Icon } from "./Icons";
import { Menu, type MenuEntry } from "./ui/Menu";
import { Popover } from "./ui/Popover";
import { Tooltip } from "./ui/Tooltip";

interface GitEditorButtonProps {
  /** The editors installed on this Mac (VS Code, Zed, …), in the order Rust found them. */
  editors: string[];
  /** The one the main click opens; undefined when none is installed. */
  editor?: string;
  onOpen: (editor: string) => void;
}

/** Git mode's "Open in editor", beside the sync button: the main click opens the chosen
 *  editor on the project folder, and the chevron offers every installed one, remembering
 *  the pick. */
export function GitEditorButton({ editors, editor, onOpen }: GitEditorButtonProps) {
  const menuRef = useRef<HTMLButtonElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const items: MenuEntry[] = editors.map((name) => ({
    label: name,
    icon: <Icon name="code" />,
    selected: name === editor,
    onSelect: () => onOpen(name)
  }));

  const main = (
    <button
      type="button"
      className="git-editor-main"
      disabled={!editor}
      aria-label={editor ? `Open in ${editor}` : "Open in editor"}
      onClick={() => editor && onOpen(editor)}
    >
      <Icon name="code" />
      <span className="git-editor-text">
        <strong>Open in editor</strong>
        <small>{editor ?? "No editor found"}</small>
      </span>
    </button>
  );

  return (
    <div className="git-editor">
      {editor ? main : (
        <Tooltip label="Install an editor like Visual Studio Code or Zed first">
          <span className="git-editor-wrap">{main}</span>
        </Tooltip>
      )}
      <button
        ref={menuRef}
        type="button"
        className="git-editor-more"
        aria-label="Choose an editor"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        disabled={editors.length === 0}
        onClick={() => setMenuOpen((value) => !value)}
      >
        <svg className="select-chevron" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
      </button>
      <Popover anchor={menuRef} open={menuOpen && items.length > 0} onClose={() => setMenuOpen(false)} align="start">
        <Menu items={items} onClose={() => setMenuOpen(false)} />
      </Popover>
    </div>
  );
}
