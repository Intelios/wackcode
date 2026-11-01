/**
 * Suggested-task cards: the follow-ups the agent filed with `suggest_task` while it worked —
 * a bug, a rough edge, a missing bit of polish it noticed but did not touch — drawn as a tray
 * under the turn's final answer (`WorkTurn`'s `after` slot, from `layoutWorkTurns`).
 *
 * The card is a task, not a widget: unlike a visual reply it stays actionable on older turns,
 * because "I'll come back to this" has to survive a few more messages. Nothing is started by
 * accident either — "Start here" is disabled while a run is in progress, and a new chat only
 * exists where there is a project to put it in.
 *
 * State comes from the transcript and the chat's saved metadata, never from this component:
 * `SuggestionScope` carries which suggestions were started here (read out of this chat's own
 * later messages), which opened a chat of their own (`TaskRecord.originSuggestion`), and which
 * the user dismissed (`TaskRecord.dismissedSuggestions`). So a rewind re-opens a card, and a
 * deleted chat's card becomes actionable again.
 */
import { createContext, useCallback, useContext, useEffect, useId, useRef, useState } from "react";
import { motion, useReducedMotion } from "motion/react";
import type { TaskMode } from "../types";
import {
  SUGGESTION_KIND_LABELS,
  type SuggestionCard,
  type SuggestionLink,
} from "../suggestions";
import { Icon } from "./Icons";
import { UltraMark } from "./UltraMark";
import { Markdown } from "./Markdown";
import { MenuButton } from "./ui/MenuButton";
import { Tooltip } from "./ui/Tooltip";
import type { MenuEntry } from "./ui/Menu";

/** What a card can ask the app to do. `here` sends into this chat; `new` starts another one. */
export type SuggestionAction =
  | { type: "start"; card: SuggestionCard; target: "here" | "new"; mode: TaskMode }
  | { type: "dismiss"; card: SuggestionCard; dismissed: boolean }
  | { type: "copy"; card: SuggestionCard };

/** How every card in one transcript reaches the app and the chat's own state. */
export interface SuggestionScope {
  /** This chat's suggestions → the chat each one opened, by suggestion id. */
  links: ReadonlyMap<string, SuggestionLink>;
  /** Suggestion ids started in this chat itself. */
  started: ReadonlySet<string>;
  /** Suggestion ids the user dismissed in this chat. */
  dismissed: ReadonlySet<string>;
  /** A run is in progress or stopping: nothing may start here until it settles. */
  busy: boolean;
  /** The chat belongs to a project, so a new chat would see the same files. */
  canStartNewChat: boolean;
  /** The chat's model left its connection: nothing can run until another is picked. */
  modelMissing?: boolean;
  onAction?: (action: SuggestionAction) => void;
  /** Opens the chat a card points at. */
  onOpenChat?: (taskId: string) => void;
}

export const SuggestionScopeContext = createContext<SuggestionScope>({
  links: new Map(),
  started: new Set(),
  dismissed: new Set(),
  busy: false,
  canStartNewChat: false,
});

const KIND_ICONS = { bug: "alert", polish: "spark", improvement: "plus" } as const;

/** A brief longer than this gets a Show more: the card is a summary, not the whole ticket. */
const CLAMP_AT = 260;
/**
 * How long a pressed Start keeps its card's buttons down. A new chat is created before the
 * origin's card can turn into a link, so without this a double click would open two of them.
 */
const START_GUARD_MS = 1_500;

interface TrayProps {
  suggestions: SuggestionCard[];
}

/** One turn's suggested tasks. The caller draws it only once the turn's run has settled. */
export function SuggestionTray({ suggestions }: TrayProps) {
  const scope = useContext(SuggestionScopeContext);
  const reduce = useReducedMotion();
  const titleId = useId();
  if (suggestions.length === 0) return null;
  const open = suggestions.filter((card) => !scope.dismissed.has(card.id)).length;
  return (
    <motion.section
      className="suggestion-tray"
      aria-labelledby={titleId}
      initial={reduce ? false : { opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={reduce ? { duration: 0 } : { duration: 0.34, ease: [0.33, 1, 0.68, 1] }}
    >
      <header className="suggestion-tray-head">
        <span className="suggestion-tray-mark" aria-hidden="true"><Icon name="bulb" /></span>
        <div className="suggestion-tray-title">
          <h3 id={titleId}>Spotted along the way</h3>
          <span>{open === 1 ? "One follow-up the agent left for later." : `${open} follow-ups the agent left for later.`}</span>
        </div>
      </header>
      <ul className="suggestion-list">
        {suggestions.map((card, index) => (
          <SuggestionItem key={card.id} card={card} index={index} scope={scope} reduce={reduce === true} />
        ))}
      </ul>
    </motion.section>
  );
}

interface ItemProps {
  card: SuggestionCard;
  index: number;
  scope: SuggestionScope;
  reduce: boolean;
}

function SuggestionItem({ card, index, scope, reduce }: ItemProps) {
  const [expanded, setExpanded] = useState(false);
  const [starting, setStarting] = useState(false);
  const startTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const startedHere = scope.started.has(card.id);
  // One press starts one task: the card turns into its started state once the App has created
  // the chat, and the timer covers the gap (and gives the buttons back if that failed).
  const start = useCallback((target: "here" | "new", mode: TaskMode) => {
    setStarting(true);
    clearTimeout(startTimer.current);
    startTimer.current = setTimeout(() => setStarting(false), START_GUARD_MS);
    scope.onAction?.({ type: "start", card, target, mode });
  }, [card, scope]);
  useEffect(() => () => clearTimeout(startTimer.current), []);
  const link = scope.links.get(card.id);
  const dismissed = scope.dismissed.has(card.id);
  const act = (action: SuggestionAction) => scope.onAction?.(action);

  if (dismissed) {
    return (
      <li className="suggestion-item dismissed">
        <span className="suggestion-dismissed-title"><Icon name="close" aria-hidden="true" />{card.title}</span>
        <button type="button" className="text-button" onClick={() => act({ type: "dismiss", card, dismissed: false })}>
          Restore
        </button>
      </li>
    );
  }

  /** Opened in a chat of its own (`TaskRecord.originSuggestion`), or started right here. */
  const started = startedHere || Boolean(link);
  const menu: MenuEntry[] = started ? [] : [
    { label: "Plan it in a new chat", icon: <Icon name="brain" />, disabled: starting, onSelect: () => start("new", "plan") },
    { label: "Plan it here", icon: <Icon name="brain" />, disabled: starting || scope.busy || scope.modelMissing, onSelect: () => start("here", "plan") },
    { label: "Ultra Plan it in a new chat", icon: <UltraMark />, disabled: starting, onSelect: () => start("new", "ultraplan") },
    { label: "Ultra Plan it here", icon: <UltraMark />, disabled: starting || scope.busy || scope.modelMissing, onSelect: () => start("here", "ultraplan") },
    "separator",
    { label: "Copy as prompt", icon: <Icon name="copy" />, onSelect: () => act({ type: "copy", card }) },
  ];

  return (
    <motion.li
      className={`suggestion-item kind-${card.kind}${started ? " started" : ""}`}
      initial={reduce ? false : { opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={reduce ? { duration: 0 } : { duration: 0.3, delay: Math.min(index, 4) * 0.05, ease: [0.33, 1, 0.68, 1] }}
    >
      <div className="suggestion-head">
        <span className="suggestion-kind"><Icon name={KIND_ICONS[card.kind]} aria-hidden="true" />{SUGGESTION_KIND_LABELS[card.kind]}</span>
        <strong className="suggestion-title">{card.title}</strong>
        {!started && (
          <Tooltip label="Dismiss">
            <button type="button" className="icon-button suggestion-dismiss" aria-label={`Dismiss “${card.title}”`} onClick={() => act({ type: "dismiss", card, dismissed: true })}>
              <Icon name="close" />
            </button>
          </Tooltip>
        )}
      </div>
      <div className={`suggestion-brief${expanded ? "" : " clamped"}`}>
        <Markdown>{card.description}</Markdown>
      </div>
      {card.description.length > CLAMP_AT && (
        <button type="button" className="text-button suggestion-more" onClick={() => setExpanded((value) => !value)}>
          {expanded ? "Show less" : "Show more"}
        </button>
      )}
      {link ? (
        <div className="suggestion-state">
          <span><Icon name="check" aria-hidden="true" />Opened in</span>
          <button type="button" className="text-button" onClick={() => scope.onOpenChat?.(link.taskId)}>“{link.name}”</button>
        </div>
      ) : startedHere ? (
        <div className="suggestion-state">
          <span><Icon name="check" aria-hidden="true" />Started in this chat</span>
        </div>
      ) : (
        <div className="suggestion-actions">
          {scope.canStartNewChat && (
            <button type="button" className="primary-button" disabled={starting} onClick={() => start("new", "build")}>
              <Icon name="plus" /> Start in new chat
            </button>
          )}
          <Tooltip label={scope.modelMissing ? "Pick a model for this chat first" : scope.busy ? "Wait for this reply to finish" : "Send it in this chat"}>
            <button type="button" className={scope.canStartNewChat ? "secondary-button" : "primary-button"}
              disabled={starting || scope.busy || scope.modelMissing}
              onClick={() => start("here", "build")}>
              <Icon name="send" /> Start here
            </button>
          </Tooltip>
          {menu.length > 0 && <MenuButton items={menu} label={`More ways to start “${card.title}”`} />}
        </div>
      )}
    </motion.li>
  );
}

/** Under a user's message: these words started a suggested task, not a fresh request. */
export function SuggestionNote({ origin }: { origin?: { chatId: string; chatName: string } }) {
  return (
    <p className="suggestion-sent">
      <Icon name="bulb" aria-hidden="true" />
      <span>Suggested task</span>
      {origin && <span className="suggestion-sent-origin">from “{origin.chatName}”</span>}
    </p>
  );
}
