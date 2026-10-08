/**
 * Chat mode's tool calls as little chips in the agent's reply ("Looked up example.com"). A
 * chip opens a soft sheet with the same detail the Code transcript shows (`ToolDetail`), and a
 * settled run of several calls folds into one "Did N things" chip. Screenshots show inline.
 * Labels and folding are pure (`chat-activity.ts`); this file only renders them.
 */
import { useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { NormalizedBlock } from "../../types";
import { Icon } from "../Icons";
import { OrbitSpinner } from "../OrbitSpinner";
import { ToolDetail, ToolImages } from "../ToolRow";
import { activitiesOf, groupActivities, type Activity } from "./chat-activity";
import type { ScratchpadFile } from "./chat-scratchpad";

const POP = { type: "spring", stiffness: 520, damping: 28 } as const;
const SHEET = { duration: 0.24, ease: [0.33, 1, 0.68, 1] } as const;

function Chip({ activity, open, onToggle }: { activity: Activity; open: boolean; onToggle: () => void }) {
  const reduce = useReducedMotion() ?? false;
  return (
    <motion.button
      type="button"
      className={`chat-chip ${activity.status}${open ? " open" : ""}`}
      aria-expanded={open}
      onClick={onToggle}
      initial={reduce ? false : { opacity: 0, scale: 0.8, y: 4 }}
      animate={{ opacity: 1, scale: 1, y: 0 }}
      transition={POP}
    >
      {activity.status === "live" ? <OrbitSpinner active className="chat-chip-icon" /> : <Icon name={activity.icon} className="chat-chip-icon" />}
      <span className="chat-chip-label">{activity.label}</span>
      {activity.status === "failed" && <span className="chat-chip-failed">didn't work</span>}
    </motion.button>
  );
}

function Sheet({ activity }: { activity: Activity }) {
  const reduce = useReducedMotion() ?? false;
  return (
    <motion.div
      className="chat-sheet"
      initial={reduce ? false : { opacity: 0, height: 0 }}
      animate={{ opacity: 1, height: "auto" }}
      exit={reduce ? { opacity: 0 } : { opacity: 0, height: 0 }}
      transition={SHEET}
    >
      <div className="chat-sheet-inner">
        <ToolDetail call={activity.call} result={activity.result} />
      </div>
    </motion.div>
  );
}

/** One run of consecutive tool calls inside a reply. */
export function ActivityChips({ calls, results, live }: { calls: NormalizedBlock[]; results: Map<string, NormalizedBlock>; live: boolean }) {
  const [open, setOpen] = useState<string>();
  const [unfolded, setUnfolded] = useState<ReadonlySet<string>>(() => new Set());
  const activities = activitiesOf(calls, results, live);
  const groups = groupActivities(activities);
  const shown: Activity[] = [];
  const chips = groups.map((group) => {
    if (group.type === "single" || unfolded.has(group.key)) {
      const items = group.type === "single" ? [group.activity] : group.activities;
      shown.push(...items);
      return items.map((activity) => (
        <Chip key={activity.key} activity={activity} open={open === activity.key} onToggle={() => setOpen((current) => current === activity.key ? undefined : activity.key)} />
      ));
    }
    return (
      <motion.button key={group.key} type="button" className="chat-chip fold" aria-expanded={false}
        onClick={() => setUnfolded((current) => new Set(current).add(group.key))}
        initial={false} animate={{ opacity: 1, scale: 1 }} transition={POP}>
        <span className="chat-chip-stack" aria-hidden="true">
          {group.activities.slice(0, 3).map((activity) => <Icon key={activity.key} name={activity.icon} />)}
        </span>
        <span className="chat-chip-label">{group.label}</span>
      </motion.button>
    );
  });
  const openActivity = shown.find((activity) => activity.key === open);
  const screenshots = activities.filter((activity) => activity.images && activity.result);
  return (
    <div className="chat-activity">
      <div className="chat-chips">{chips}</div>
      <AnimatePresence initial={false}>
        {openActivity && <Sheet key={openActivity.key} activity={openActivity} />}
      </AnimatePresence>
      {screenshots.map((activity) => <ToolImages key={activity.key} call={activity.call} result={activity.result!} />)}
    </div>
  );
}

/** The files a reply wrote into the scratchpad; each reveals itself in Finder. */
export function ScratchpadChips({ files, onReveal }: { files: ScratchpadFile[]; onReveal: (path: string) => void }) {
  if (files.length === 0) return null;
  return (
    <div className="chat-files" role="group" aria-label="Files in the scratchpad">
      {files.map((file) => (
        <button key={file.path} type="button" className="chat-file" title={file.folder ? `${file.folder}/${file.name}` : file.name}
          aria-label={`Show ${file.name} in Finder`} onClick={() => onReveal(file.path)}>
          <span className="chat-file-page" aria-hidden="true"><Icon name="file" /></span>
          <span className="chat-file-name">{file.name}</span>
          <Icon name="external" className="chat-file-go" />
        </button>
      ))}
    </div>
  );
}
