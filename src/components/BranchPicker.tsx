import { useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { GitBranch, GitBranches, GitCheckoutKind } from "../types";
import { Icon } from "./Icons";
import { Popover } from "./ui/Popover";

interface BranchPickerProps {
  /** The checked-out branch; null on a detached HEAD. */
  branch: string | null;
  /** `pill` sits in the new-chat project bar; `meta` in the chat header's workspace line. */
  variant: "pill" | "meta";
  side?: "top" | "bottom";
  onLoad: () => Promise<GitBranches>;
  /** Rejects with a user-facing sentence, shown in the popover. */
  onCheckout: (name: string, kind: GitCheckoutKind) => Promise<void>;
}

type Row = { key: string; name: string; kind: GitCheckoutKind; branch?: GitBranch };

/** The branch label, and a searchable popover to switch the checkout to another branch,
 *  track a remote one, or create one at HEAD. Nothing is fetched: it lists the refs Git has. */
export function BranchPicker({ branch, variant, side = "bottom", onLoad, onCheckout }: BranchPickerProps) {
  const reduce = useReducedMotion();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [list, setList] = useState<GitBranches>();
  const [loading, setLoading] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [pending, setPending] = useState<string>();
  const [error, setError] = useState("");
  const loadRequest = useRef(0);

  function show() {
    setOpen(true); setQuery(""); setActive(0); setError("");
    const request = ++loadRequest.current;
    setLoading(true);
    void onLoad()
      .then((result) => { if (loadRequest.current === request) setList(result); })
      .catch((reason) => { if (loadRequest.current === request) setError(String(reason)); })
      .finally(() => { if (loadRequest.current === request) setLoading(false); });
    requestAnimationFrame(() => inputRef.current?.focus());
  }

  function close() {
    if (pending) return;
    setOpen(false);
  }

  const current = list ? list.current : branch;
  const needle = query.trim().toLowerCase();
  const matches = (list?.branches ?? []).filter((item) => item.name.toLowerCase().includes(needle));
  const local = matches.filter((item) => !item.remote);
  const remote = matches.filter((item) => item.remote);
  const canCreate = list !== undefined && query.trim() !== "" && !list.branches.some((item) => !item.remote && item.name === query.trim());
  const rows: Row[] = [
    ...local.map((item) => ({ key: `l:${item.name}`, name: item.name, kind: "local" as const, branch: item })),
    ...remote.map((item) => ({ key: `r:${item.name}`, name: item.name, kind: "remote" as const, branch: item })),
    ...(canCreate ? [{ key: "create", name: query.trim(), kind: "create" as const }] : [])
  ];
  const enabled = rows.filter((row) => !row.branch?.worktree);
  const activeKey = enabled[Math.min(active, enabled.length - 1)]?.key;

  async function choose(row: Row) {
    if (pending || row.branch?.worktree) return;
    if (row.kind === "local" && row.name === current) { setOpen(false); return; }
    setPending(row.key); setError("");
    try {
      await onCheckout(row.name, row.kind);
      setOpen(false);
      setList(undefined);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setPending(undefined);
    }
  }

  function onKeyDown(event: React.KeyboardEvent) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const step = event.key === "ArrowDown" ? 1 : -1;
      setActive((index) => Math.max(0, Math.min(enabled.length - 1, Math.min(index, enabled.length - 1) + step)));
    } else if (event.key === "Enter") {
      event.preventDefault();
      const row = enabled.find((item) => item.key === activeKey);
      if (row) void choose(row);
    }
  }

  function renderRow(row: Row) {
    const busy = pending === row.key;
    const isCurrent = row.kind === "local" && row.name === current;
    return (
      <button
        type="button"
        key={row.key}
        className={`picker-item branch-item ${isCurrent ? "selected" : ""} ${row.key === activeKey ? "active" : ""}`}
        disabled={Boolean(row.branch?.worktree) || (Boolean(pending) && !busy)}
        title={row.branch?.worktree ? `Checked out in ${row.branch.worktree}` : undefined}
        onMouseEnter={() => { const index = enabled.findIndex((item) => item.key === row.key); if (index >= 0) setActive(index); }}
        onClick={() => void choose(row)}
      >
        {row.kind === "create"
          ? <><span>Create <code>{row.name}</code></span><Icon name="plus" /></>
          : <><span><code>{row.name}</code></span>
            {row.branch?.worktree && <em className="branch-hint">worktree</em>}
            {busy ? <Icon name="refresh" className="spinning" /> : isCurrent ? <Icon name="check" /> : null}</>}
      </button>
    );
  }

  const label = branch ?? "Detached HEAD";
  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className={`branch-trigger ${variant === "pill" ? "project-bar-branch" : "branch-meta"}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`Branch: ${label}. Switch branch`}
        title={`${label} — switch branch`}
        onClick={() => (open ? close() : show())}
      >
        <Icon name="branch" />
        <span className="branch-trigger-name">
          <AnimatePresence initial={false} mode="popLayout">
            <motion.span
              key={label}
              initial={reduce ? { opacity: 0 } : { opacity: 0, y: 9 }}
              animate={{ opacity: 1, y: 0 }}
              exit={reduce ? { opacity: 0 } : { opacity: 0, y: -9 }}
              transition={{ type: "spring", stiffness: 520, damping: 34 }}
            >{label}</motion.span>
          </AnimatePresence>
        </span>
        <svg className="select-chevron" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
      </button>
      <Popover anchor={triggerRef} open={open} onClose={close} side={side} align="start" className="branch-pop">
        <div className="branch-picker" role="dialog" aria-label="Switch branch">
          <div className="branch-search">
            <input
              ref={inputRef}
              value={query}
              placeholder="Find or create a branch…"
              aria-label="Find or create a branch"
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              onChange={(event) => { setQuery(event.target.value); setActive(0); }}
              onKeyDown={onKeyDown}
            />
          </div>
          <div className="branch-list">
            {local.length > 0 && <div className="picker-heading">Branches</div>}
            {rows.filter((row) => row.kind === "local").map(renderRow)}
            {remote.length > 0 && <div className="picker-heading">Remote</div>}
            {rows.filter((row) => row.kind === "remote").map(renderRow)}
            {canCreate && (local.length > 0 || remote.length > 0) && <div className="branch-divider" />}
            {rows.filter((row) => row.kind === "create").map(renderRow)}
            {!list && loading && <div className="branch-empty">Loading branches…</div>}
            {list && rows.length === 0 && <div className="branch-empty">{needle ? "No matching branches" : "No branches yet"}</div>}
          </div>
          {error && <div className="branch-error" role="alert">{error}</div>}
        </div>
      </Popover>
    </>
  );
}
