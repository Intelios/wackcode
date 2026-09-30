import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { fetchedLabel, firstLine, syncAction } from "../git-mode";
import type { GitNetworkKind } from "../hooks/useGitMode";
import type { GitSyncStatus } from "../types";
import { Icon, type IconName } from "./Icons";
import { Menu, type MenuEntry } from "./ui/Menu";
import { Popover } from "./ui/Popover";
import { Tooltip } from "./ui/Tooltip";

interface GitSyncButtonProps {
  sync?: GitSyncStatus;
  network?: { kind: GitNetworkKind; background: boolean };
  error?: { message: string; background: boolean };
  /** A chat is running in this folder: pulling would move files under it. */
  busyReason?: string;
  onFetch: () => void;
  onPull: () => void;
  onPush: () => void;
  onPublish: (remote: string) => void;
  onCreatePr?: () => void;
  onDismissError: () => void;
  /** Injected in tests; otherwise the clock ticks every 30 s for the "Fetched 4m ago" line only. */
  now?: Date;
}

/** Re-renders the "ago" caption now and then. Display only: it never fetches anything. */
function useNow(injected?: Date): Date {
  const [now, setNow] = useState(() => injected ?? new Date());
  useEffect(() => {
    if (injected) return;
    const timer = setInterval(() => setNow(new Date()), 30_000);
    return () => clearInterval(timer);
  }, [injected]);
  return injected ?? now;
}

const RUNNING: Record<GitNetworkKind, string> = { fetch: "Fetching", pull: "Pulling", push: "Pushing", publish: "Publishing" };

/**
 * GitHub Desktop's third toolbar button: Fetch origin, then Pull ↓n or Push ↑n or Publish
 * branch depending on where the branch stands. The icon launches on click and a comet ring
 * runs round the button while Git talks to the remote.
 */
export function GitSyncButton(props: GitSyncButtonProps) {
  const reduce = useReducedMotion();
  const now = useNow(props.now);
  const menuRef = useRef<HTMLButtonElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [launch, setLaunch] = useState(0);
  const action = syncAction(props.sync);
  const running = props.network;
  const pullBlocked = action.kind === "pull" && props.busyReason;

  let icon: IconName = "fetch";
  let title = "";
  let caption = "";
  if (running) {
    icon = running.kind === "pull" ? "pull" : running.kind === "fetch" ? "fetch" : "push";
    title = `${RUNNING[running.kind]} ${props.sync?.fetchRemote ?? "origin"}…`;
    caption = running.background ? "Checking for new commits" : "Talking to the remote";
  } else if (action.kind === "none") {
    icon = "fetch";
    title = action.label;
    caption = action.reason;
  } else if (action.kind === "fetch") {
    title = `Fetch ${action.remote}`;
    caption = fetchedLabel(action.fetchedAt, now);
  } else if (action.kind === "pull") {
    icon = "pull";
    title = `Pull ${action.remote}`;
    caption = action.ahead > 0 ? `${action.behind} to pull · ${action.ahead} to push` : fetchedLabel(props.sync?.fetchedAt ?? null, now);
  } else if (action.kind === "push") {
    icon = "push";
    title = `Push ${action.remote}`;
    caption = fetchedLabel(props.sync?.fetchedAt ?? null, now);
  } else {
    icon = "push";
    title = "Publish branch";
    caption = `Push ${props.sync?.branch ?? "this branch"} to ${action.remote}`;
  }

  const disabled = Boolean(running) || action.kind === "none" || Boolean(pullBlocked);

  function run() {
    setLaunch((value) => value + 1);
    if (action.kind === "fetch") props.onFetch();
    else if (action.kind === "pull") props.onPull();
    else if (action.kind === "push") props.onPush();
    else if (action.kind === "publish") props.onPublish(action.remote);
  }

  const remote = props.sync?.fetchRemote;
  const items: MenuEntry[] = [
    { label: remote ? `Fetch ${remote}` : "Fetch", icon: <Icon name="fetch" />, disabled: !remote || Boolean(running), onSelect: () => { setLaunch((value) => value + 1); props.onFetch(); } },
    { label: "Pull", icon: <Icon name="pull" />, hint: props.sync?.behind ? `↓${props.sync.behind}` : undefined,
      disabled: !props.sync?.upstream || !props.sync.behind || Boolean(running) || Boolean(props.busyReason), onSelect: props.onPull },
    props.sync?.upstream
      ? { label: "Push", icon: <Icon name="push" />, hint: props.sync.ahead ? `↑${props.sync.ahead}` : undefined,
        disabled: !props.sync.ahead || Boolean(running) || Boolean(props.busyReason), onSelect: props.onPush }
      : { label: "Publish branch", icon: <Icon name="push" />, disabled: !remote || !props.sync?.branch || !props.sync.hasHead || Boolean(running) || Boolean(props.busyReason), onSelect: () => remote && props.onPublish(remote) },
    ...(props.onCreatePr ? ["separator" as const, { label: "Create pull request…", icon: <Icon name="pullRequest" />, disabled: !props.sync?.upstream, onSelect: props.onCreatePr }] : [])
  ];

  const quietError = props.error && props.error.background;
  const main = (
    <button
      type="button"
      className={`git-sync-main ${running ? "running" : ""} ${action.kind}`}
      disabled={disabled}
      onClick={run}
      aria-label={`${title}. ${caption}`}
    >
      <span className="git-sync-icon" key={`${icon}-${launch}`} data-launch={launch > 0 && !reduce ? icon : undefined}>
        <Icon name={icon} className={running && running.kind === "fetch" ? "spinning" : undefined} />
      </span>
      <span className="git-sync-text">
        <AnimatePresence initial={false} mode="popLayout">
          <motion.strong
            key={title}
            initial={reduce ? { opacity: 0 } : { opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            exit={reduce ? { opacity: 0 } : { opacity: 0, y: -10 }}
            transition={{ type: "spring", stiffness: 520, damping: 34 }}
          >
            {title}
            {!running && action.kind === "pull" && <span className="git-sync-count">↓{action.behind}</span>}
            {!running && action.kind === "push" && <span className="git-sync-count">↑{action.ahead}</span>}
          </motion.strong>
        </AnimatePresence>
        <small>{caption}</small>
      </span>
      {quietError && <span className="git-sync-warn" aria-hidden="true" />}
    </button>
  );

  const tooltip = pullBlocked ? props.busyReason : quietError ? firstLine(props.error!.message) : undefined;
  return (
    <div className={`git-sync ${running ? "running" : ""}`}>
      {tooltip ? <Tooltip label={tooltip}><span className="git-sync-wrap">{main}</span></Tooltip> : main}
      <button
        ref={menuRef}
        type="button"
        className="git-sync-more"
        aria-label="More sync actions"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        onClick={() => setMenuOpen((value) => !value)}
      >
        <svg className="select-chevron" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
      </button>
      <Popover anchor={menuRef} open={menuOpen} onClose={() => setMenuOpen(false)} align="end">
        <Menu items={items} onClose={() => setMenuOpen(false)} />
      </Popover>
      {props.error && !props.error.background && (
        <div className="git-sync-error" role="alert">
          <span>{firstLine(props.error.message)}</span>
          <button type="button" className="icon-button" aria-label="Dismiss sync error" onClick={props.onDismissError}><Icon name="close" /></button>
        </div>
      )}
    </div>
  );
}
