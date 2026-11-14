import { useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import type { TurnFileChangesSummary } from "../turn-changes";
import { Icon } from "./Icons";
import { CoalescedToolDetail } from "./ToolDetail";
import { useTranscriptMotion } from "./ToolRow";

const MAX_SHOWN_FILES = 3;
const DIFF_SHEET = { duration: 0.22, ease: [0.33, 1, 0.68, 1] } as const;

export interface TurnChangesCardProps {
  summary: TurnFileChangesSummary;
  onOpenChanges?: () => void;
  index?: number;
}

/**
 * Per-turn edited/created file summary anchored beneath a settled turn. Surfaces up to 3 files
 * with folder collision hints, net +/− totals, inline diff inspection, and a jump to the
 * workspace Changes drawer when available.
 */
export function TurnChangesCard({ summary, onOpenChanges, index = 0 }: TurnChangesCardProps) {
  const animate = useTranscriptMotion();
  const [expandedList, setExpandedList] = useState(false);
  const [openFiles, setOpenFiles] = useState<Set<string>>(() => new Set());

  const visibleFiles = expandedList ? summary.files : summary.files.slice(0, MAX_SHOWN_FILES);
  const hiddenCount = summary.files.length - visibleFiles.length;

  const toggleFile = (path: string) => {
    setOpenFiles((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  return (
    <motion.div
      className="turn-changes-card"
      role="region"
      aria-label={summary.heading}
      initial={animate ? { opacity: 0, y: 6 } : false}
      animate={{ opacity: 1, y: 0 }}
      transition={{ type: "spring", stiffness: 420, damping: 34, delay: animate ? index * 0.05 : 0 }}
    >
      <div className="turn-changes-head">
        <span className="turn-changes-mark" aria-hidden="true"><Icon name="git" /></span>
        <span className="turn-changes-title">{summary.heading}</span>
        {(summary.additions > 0 || summary.deletions > 0) && (
          <span className="tool-row-stats">
            {summary.additions > 0 && <em className="add">+{summary.additions}</em>}
            {summary.deletions > 0 && <em className="del">−{summary.deletions}</em>}
          </span>
        )}
        {onOpenChanges && (
          <button type="button" className="text-button turn-changes-open" onClick={onOpenChanges}>
            Changes
          </button>
        )}
      </div>

      <div className="turn-changes-files">
        {visibleFiles.map((file) => {
          const isOpen = openFiles.has(file.path);
          return (
            <div key={file.path} className={`turn-changes-file${isOpen ? " open" : ""}`}>
              <button
                type="button"
                className="turn-changes-file-head"
                aria-expanded={isOpen}
                aria-label={`Inspect changes to ${file.displayPath}`}
                onClick={() => toggleFile(file.path)}
              >
                <Icon name={file.kind === "create" ? "filePlus" : "file"} className="turn-changes-file-icon" />
                <code className={`turn-changes-file-path${file.dir || file.folderHint ? " has-dir" : ""}`} title={file.displayPath}>
                  {file.folderHint ? (
                    <>
                      <span className="tool-row-subject-dir">{file.folderHint}/</span>
                      <span className="tool-row-subject-name">{file.name}</span>
                    </>
                  ) : (
                    <>
                      {file.dir && <span className="tool-row-subject-dir">{file.dir}</span>}
                      <span className="tool-row-subject-name">{file.name}</span>
                    </>
                  )}
                </code>
                {(file.additions > 0 || file.deletions > 0) && (
                  <span className="tool-row-stats">
                    {file.additions > 0 && <em className="add">+{file.additions}</em>}
                    {file.deletions > 0 && <em className="del">−{file.deletions}</em>}
                  </span>
                )}
                <Icon name="chevron" className="tool-chevron" />
              </button>
              {animate ? (
                <AnimatePresence initial={false}>
                  {isOpen && (
                    <motion.div
                      key="diff"
                      className="turn-changes-file-diff"
                      initial={{ opacity: 0, height: 0 }}
                      animate={{ opacity: 1, height: "auto" }}
                      exit={{ opacity: 0, height: 0 }}
                      transition={DIFF_SHEET}
                    >
                      <CoalescedToolDetail entries={file.calls} head={false} />
                    </motion.div>
                  )}
                </AnimatePresence>
              ) : isOpen && (
                <div className="turn-changes-file-diff">
                  <CoalescedToolDetail entries={file.calls} head={false} />
                </div>
              )}
            </div>
          );
        })}
      </div>

      {summary.files.length > MAX_SHOWN_FILES && (
        <button
          type="button"
          className="turn-changes-more"
          aria-expanded={expandedList}
          onClick={() => setExpandedList((v) => !v)}
        >
          {expandedList ? "Show less" : `Show ${hiddenCount} more`}
        </button>
      )}
    </motion.div>
  );
}
