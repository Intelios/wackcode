import { useId, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { ProjectCommandKind, ProjectRecord, RunInfo } from "../types";
import { runIsActive } from "../run-state";
import { Icon } from "./Icons";
import { Popover } from "./ui/Popover";
import { Tooltip } from "./ui/Tooltip";

interface RunButtonProps {
  kind?: ProjectCommandKind;
  project: ProjectRecord;
  workspacePath: string;
  run?: RunInfo;
  onSave: (command: string) => Promise<void>;
  onRun: () => Promise<void>;
  onStop: () => Promise<void>;
  onShowOutput: () => void;
}

/** The project owns configuration; the selected checkout owns execution. */
export function RunButton({ kind = "run", project, workspacePath, run, onSave, onRun, onStop, onShowOutput }: RunButtonProps) {
  const reduce = useReducedMotion();
  const id = useId();
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const title = kind === "build" ? "Build" : "Run";
  const configuredCommand = kind === "build" ? project.buildCommand : project.runCommand;
  const [command, setCommand] = useState(configuredCommand ?? "");
  const [pending, setPending] = useState<"save" | "start" | "stop">();
  const [error, setError] = useState<string>();
  const active = runIsActive(run);
  const disabled = Boolean(pending) || run?.status === "stopping";
  const stopping = run?.status === "stopping" || pending === "stop";
  const label = pending === "start" ? "Starting…" : stopping ? "Stopping…" : active ? `Stop ${kind}` : title;

  function edit() {
    setCommand(configuredCommand ?? "");
    setError(undefined);
    setOpen(true);
  }

  async function action(kind: "save" | "start" | "stop", callback: () => Promise<void>, close = false) {
    setPending(kind);
    setError(undefined);
    try {
      await callback();
      if (close) setOpen(false);
    } catch (reason) {
      setError(String(reason));
      setOpen(true);
    } finally { setPending(undefined); }
  }

  async function save(andRun: boolean) {
    await action(andRun ? "start" : "save", async () => {
      await onSave(command);
      if (andRun) await onRun();
    }, true);
  }

  return (
    <div className={"project-run-control " + (active ? "running" : "")}>
      <Tooltip label={active ? "Stop " + run?.command : configuredCommand ?? `Set a project ${kind} command`} side="bottom">
        <button type="button" className="panel-button project-run-main" disabled={disabled}
          aria-label={stopping ? `Stopping ${kind}…` : pending === "start" ? `Starting ${kind}…` : undefined}
          onClick={() => active ? void action("stop", onStop) : configuredCommand ? void action("start", onRun) : edit()}>
          <AnimatePresence mode="wait" initial={false}>
            <motion.span key={active ? "stop" : "play"} className="project-run-icon"
              initial={reduce ? false : { opacity: 0, scale: 0.65 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.65 }}
              transition={{ duration: reduce ? 0 : 0.14 }}>
              <Icon name={active ? "stop" : kind === "build" ? "hammer" : "play"} />
            </motion.span>
          </AnimatePresence>
          <span>{label}</span>
        </button>
      </Tooltip>
      <button ref={anchor} type="button" className="panel-button project-run-more"
        aria-label={`Configure ${kind} command`} aria-haspopup="dialog" aria-expanded={open}
        onClick={() => open ? setOpen(false) : edit()}>
        <Icon name="chevron" />
      </button>
      <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} side="bottom" align="end">
        <form className="run-command-editor" role="dialog" aria-label={`Project ${kind} command`}
          onSubmit={(event) => { event.preventDefault(); if (!disabled) void save(false); }}>
          <div className="run-editor-heading"><span>{title} command</span><strong>{project.name}</strong></div>
          <label htmlFor={id}>Command</label>
          <input id={id} autoFocus className="run-command-input" value={command} placeholder={kind === "build" ? "pnpm build" : "pnpm dev"}
            spellCheck={false} autoCapitalize="off" autoCorrect="off"
            onChange={(event) => setCommand(event.target.value)} />
          <p className="run-command-folder" title={workspacePath}><Icon name="folder" />{workspacePath}</p>
          <p className="run-command-help">{active
            ? `Edits apply to the next ${kind}. Chats in this folder share the current ${kind}.`
            : "Saved for this project. Runs in the current chat’s folder."}</p>
          {error && <p className="run-command-error" role="alert">{error}</p>}
          <div className="run-editor-actions">
            {run && <button type="button" className="text-button" onClick={() => { setOpen(false); onShowOutput(); }}>Show output</button>}
            <span className="run-editor-spacer" />
            <button type="submit" className="secondary-button compact" disabled={disabled}>Save</button>
            {!active && <button type="button" className="primary-button compact" disabled={disabled || !command.trim()} onClick={() => void save(true)}>Save &amp; {title}</button>}
          </div>
        </form>
      </Popover>
    </div>
  );
}
