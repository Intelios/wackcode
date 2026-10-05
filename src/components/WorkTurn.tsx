import { Component, createRef, useId, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { WorkRow, WorkTurn as Turn } from "../completed-work";
import { formatRunDuration } from "../chat-utils";
import type { useFollowScroll } from "../hooks/useFollowScroll";
import { motionAllowed } from "../hooks/useSmoothText";
import { Icon } from "./Icons";

interface Props {
  turn: Turn;
  scopeKey: string;
  user: ReactNode;
  duration: ReactNode;
  durationMs?: number;
  enabled: boolean;
  open: boolean;
  onToggle: () => void;
  onAutoCollapse: () => void;
  renderRows: (rows: WorkRow[], part?: "work" | "outcome") => ReactNode;
  scroll: Pick<ReturnType<typeof useFollowScroll>, "ref" | "isFollowing" | "keepAnchor" | "reconcile">;
}

interface BoundaryProps extends Props { regionId: string; animate: boolean }
interface Snapshot {
  anchor?: { key: string; offset: number; workRoot?: HTMLElement };
  focus: boolean;
}

const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];
const anchors = (element: HTMLElement) => [...element.querySelectorAll<HTMLElement>("[data-transcript-anchor]")];
const offset = (node: HTMLElement, scroll: HTMLElement) => node.getBoundingClientRect().top - scroll.getBoundingClientRect().top;

/**
 * This small class is deliberately a pre-mutation layout boundary. An automatic fold can
 * remove the very row being read/focused; an effect runs too late to capture its old rect.
 * React focus events also bubble from work-owned portals (screenshot lightboxes), unlike
 * DOM containment. The wrapper must stay unpositioned: user offsetTop belongs to the scroller.
 */
class WorkTurnBoundary extends Component<BoundaryProps, Record<string, never>, Snapshot | null> {
  private button = createRef<HTMLButtonElement>();
  private focusedWork: EventTarget | null = null;
  private committed = this.props;

  private completedNow() {
    const previous = this.committed;
    return previous.scopeKey === this.props.scopeKey && previous.turn.userKey === this.props.turn.userKey
      && !previous.turn.folded && Boolean(this.props.turn.folded);
  }

  getSnapshotBeforeUpdate(): Snapshot | null {
    const previous = this.committed;
    const beforeFold = previous.enabled && Boolean(previous.turn.folded);
    const afterFold = this.props.enabled && Boolean(this.props.turn.folded);
    const beforeOpen = !beforeFold || previous.open;
    const afterOpen = !afterFold || (this.props.open && !this.completedNow());
    if (beforeFold === afterFold && beforeOpen === afterOpen && previous.turn.key === this.props.turn.key) return null;
    const scroller = this.props.scroll.ref.current;
    if (!scroller) return null;
    const focus = !afterOpen && this.focusedWork !== null && this.focusedWork === document.activeElement;
    if (this.props.scroll.isFollowing()) return { focus };

    // User toggles always hold the clicked disclosure. Automatic changes hold the reading
    // row, unless it disappears into the fold, in which case the new disclosure is the anchor.
    const manual = beforeFold && afterFold && previous.open !== this.props.open;
    if (manual && this.button.current) {
      return { focus, anchor: { key: `work:${this.props.turn.userIndex}`, offset: offset(this.button.current, scroller) } };
    }
    const top = scroller.getBoundingClientRect().top;
    const reading = anchors(scroller).find((node) => node.getBoundingClientRect().bottom > top);
    if (!reading) return { focus };
    return { focus, anchor: {
      key: reading.dataset.transcriptAnchor!, offset: offset(reading, scroller),
      workRoot: reading.closest(".work-transcript-region")?.closest<HTMLElement>(".transcript-turn") ?? undefined
    } };
  }

  componentDidUpdate(_previous: BoundaryProps, _state: Record<string, never>, snapshot: Snapshot | null) {
    const completed = this.completedNow();
    // Record the rendered open state, not the stale prop we are about to clear. Otherwise
    // that cleanup looks like a manual click and replaces an answer's reading anchor.
    this.committed = completed ? { ...this.props, open: false } : this.props;
    // Completion overrides a past manual reveal even if an existing branch is resumed.
    // Settings toggles or navigation to another finished turn still restore manual choices.
    if (completed) this.props.onAutoCollapse();
    if (!snapshot) return;
    if (snapshot.focus) this.button.current?.focus({ preventScroll: true });
    const scroller = this.props.scroll.ref.current;
    if (scroller && snapshot.anchor) {
      const disclosure = snapshot.anchor.workRoot?.querySelector<HTMLButtonElement>(".work-disclosure-button");
      // Every sibling captured the old layout. After all DOM mutations, resolve work owned
      // by ANY closing turn to that turn's disclosure; never hold its exiting animation body.
      // Otherwise simultaneous steering folds let the last sibling overwrite the right anchor.
      if (disclosure?.isConnected && disclosure.getAttribute("aria-expanded") === "false") {
        this.props.scroll.keepAnchor(disclosure, 0);
      } else {
        const anchor = anchors(scroller).find((node) => node.dataset.transcriptAnchor === snapshot.anchor!.key);
        if (anchor) this.props.scroll.keepAnchor(anchor, snapshot.anchor.offset);
      }
    }
    this.props.scroll.reconcile();
  }

  render() {
    const { turn, user, duration, durationMs, enabled, onToggle, renderRows, regionId, animate, scroll } = this.props;
    const open = this.props.open && !this.completedNow();
    const folded = enabled && Boolean(turn.folded);
    const partition = enabled ? turn.partition : undefined;
    const visible = !folded || open;
    const lastWork = partition?.work[partition.work.length - 1];
    const firstOutcome = partition?.outcome[0];
    const splitMessage = lastWork?.type === "message" && firstOutcome?.type === "message" && lastWork.index === firstOutcome.index;
    const workRows = () => renderRows(partition?.work ?? turn.body, partition ? "work" : undefined);
    const label = durationMs === undefined ? (open ? "Hide work" : "View work") : `Worked for ${formatRunDuration(durationMs)}`;
    // Match the existing duration chip's typography without hiding the total in the button name.
    const content = durationMs === undefined ? label : <>Worked for <strong>{formatRunDuration(durationMs)}</strong></>;

    return (
      <div className={`transcript-turn${partition && !folded ? " live-partition" : ""}${splitMessage ? " split-message" : ""}`}>
        {user}
        {folded ? (
          <div className={`run-duration work-disclosure${open ? " open" : ""}`}>
            <button ref={this.button} type="button" className="run-duration-chip work-disclosure-button"
              data-transcript-anchor={`work:${turn.userIndex}`}
              aria-label={`${label}. ${open ? "Hide" : "Show"} work transcript`}
              aria-expanded={open} aria-controls={regionId} onClick={onToggle}>
              <Icon name="clock" />{content}<Icon name="chevron" className="work-chevron" />
            </button>
          </div>
        ) : duration}
        <div id={regionId} className="work-transcript-region" role={folded ? "region" : undefined}
          aria-label={folded ? "Work transcript" : undefined} aria-hidden={!visible || undefined} inert={!visible || undefined}
          onFocusCapture={(event) => { this.focusedWork = event.target; }}
          onBlurCapture={(event) => { if (this.focusedWork === event.target) this.focusedWork = null; }}>
          {!animate ? (visible && <div className={folded ? "work-transcript-body" : "turn-content"}>{workRows()}</div>) : (
            <AnimatePresence initial={false} onExitComplete={scroll.reconcile}>
              {visible && <motion.div key="body" className={folded ? "work-transcript-body" : "turn-content"}
                initial={{ height: 0, opacity: 0 }} animate={{ height: "auto", opacity: 1 }}
                exit={{ height: 0, opacity: 0 }} transition={{ height: { duration: 0.22, ease: EASE }, opacity: { duration: 0.15, ease: EASE } }}>
                {workRows()}
              </motion.div>}
            </AnimatePresence>
          )}
        </div>
        {partition && renderRows(partition.outcome, "outcome")}
      </div>
    );
  }
}

export function WorkTurn(props: Props) {
  const regionId = useId();
  const reduce = useReducedMotion();
  return <WorkTurnBoundary {...props} regionId={regionId} animate={reduce !== true && motionAllowed()} />;
}

