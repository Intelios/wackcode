import { AnimatePresence, LayoutGroup, motion, useReducedMotion } from "motion/react";
import type { ChangeEntry } from "../changes-utils";
import type { GitDiffSection } from "../types";
import { splitPath } from "../changes-utils";
import { useContextClipboard, useContextMenu } from "./ui/ContextMenu";
import { Icon } from "./Icons";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

const GROUPS = [
  { layer: "staged" as const, label: "Staged" },
  { layer: "working" as const, label: "Working tree" }
];

interface ChangesFileListProps {
  entries: ChangeEntry[];
  selected?: { path: string; layer: GitDiffSection["layer"] };
  /** `${layer}:${path}` → pending comment count, shown as a small bubble on the row. */
  commentCounts: Map<string, number>;
  disabled: boolean;
  onSelect: (entry: ChangeEntry) => void;
  /** Opens the commit sheet scoped to this file. */
  onCommitFile: (file: string) => void;
  /** Discards the entry's changes (behind a confirmation in the parent). */
  onAction: (entry: ChangeEntry) => void;
}

export function ChangesFileList({ entries, selected, commentCounts, disabled, onSelect, onCommitFile, onAction }: ChangesFileListProps) {
  const contextMenu = useContextMenu();
  const clipboard = useContextClipboard();
  const reduce = useReducedMotion();
  const transition = reduce ? { duration: 0 } : { duration: 0.18, ease: EASE };

  return (
    <div className="changed-files">
      <LayoutGroup>
        {GROUPS.map(({ layer, label }) => {
          const group = entries.filter((entry) => entry.section.layer === layer);
          if (group.length === 0) return null;
          return (
            <section key={layer} className="change-group">
              <div className="change-group-head">
                <h4>{label} <span>{group.length}</span></h4>
              </div>
              <ul>
                <AnimatePresence initial={false}>
                {group.map((entry) => {
                  const { file, section } = entry;
                  const { dir, base } = splitPath(file.path);
                  const key = `${layer}:${file.path}`;
                  const comments = commentCounts.get(key) ?? 0;
                  const active = selected?.path === file.path && selected.layer === layer;
                  const conflict = file.status === "conflict";
                  return (
                    <motion.li
                      key={key}
                      onContextMenu={(event) => contextMenu(event, [
                        { label: "Show diff", icon: <Icon name="code" />, onSelect: () => onSelect(entry) },
                        { label: "Commit this file…", icon: <Icon name="commit" />, disabled: disabled || conflict, onSelect: () => onCommitFile(file.path) },
                        { label: "Discard changes…", icon: <Icon name="trash" />, danger: true, disabled: disabled || conflict, onSelect: () => onAction(entry) },
                        "separator",
                        { label: "Copy path", icon: <Icon name="copy" />, disabled: !clipboard, onSelect: () => clipboard?.copyText(file.path) }
                      ], "File menu")}
                      layoutId={file.sections.length === 1 ? file.path : key}
                      initial={reduce ? false : { opacity: 0, x: 8 }}
                      animate={{ opacity: 1, x: 0 }}
                      exit={{ opacity: 0, x: -8 }}
                      transition={transition}
                      className={active ? "active" : ""}
                    >
                      <button type="button" className="change-file" onClick={() => onSelect(entry)} title={file.path}>
                        <span className={"status-letter " + file.status}>{file.status[0]?.toUpperCase()}</span>
                        <span className="change-path">
                          {dir && <span className="change-dir">{dir}</span>}
                          <span className="change-base">{base}</span>
                        </span>
                        {comments > 0 && <span className="change-comments" title={`${comments} pending ${comments === 1 ? "comment" : "comments"}`}><Icon name="comment" />{comments}</span>}
                        {(section.additions > 0 || section.deletions > 0) && (
                          <span className="change-stats">
                            {section.additions > 0 && <em className="stat-add">+{section.additions}</em>}
                            {section.deletions > 0 && <em className="stat-del">−{section.deletions}</em>}
                          </span>
                        )}
                      </button>
                      <span className="change-file-actions">
                        <button type="button" disabled={disabled || conflict} title={conflict ? "Resolve this conflict first" : "Commit this file"} onClick={() => onCommitFile(file.path)}>Commit</button>
                        <button type="button" className="danger" disabled={disabled || conflict} title={conflict ? "Resolve this conflict first" : "Discard file changes"} onClick={() => onAction(entry)}>Discard</button>
                      </span>
                    </motion.li>
                  );
                })}
                </AnimatePresence>
              </ul>
            </section>
          );
        })}
      </LayoutGroup>
    </div>
  );
}
