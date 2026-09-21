import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { ProviderRecord, SessionSnapshot, TaskMode, TaskStatus, ThinkingLevel } from "../types";
import { formatTokens } from "../chat-utils";
import { Icon } from "./Icons";
import { ContextPanel } from "./ContextPanel";
import { ModelPicker, ReasoningToggle } from "./ModelPicker";
import { ModeToggle } from "./ModeToggle";
import { Tooltip } from "./ui/Tooltip";

interface ComposerProps {
  status: TaskStatus;
  providerId?: string;
  modelId?: string;
  thinkingLevel?: ThinkingLevel;
  providers: ProviderRecord[];
  stats?: SessionSnapshot["stats"];
  header?: ReactNode;
  placeholder?: string;
  popoverSide?: "top" | "bottom";
  /** The Build/Plan toggle; omit onModeChange to hide it. */
  mode?: TaskMode;
  onModeChange?: (mode: TaskMode) => void;
  onConfigure: (patch: { providerId?: string; modelId?: string; thinkingLevel?: ThinkingLevel }) => void;
  onSend: (message: string) => Promise<boolean>;
  onStop: () => void;
  onOpenSettings: () => void;
  /** When true the composer is visually dimmed and non-interactive (e.g. a dialog needs attention). */
  disabled?: boolean;
}

export function Composer({ status, providerId, modelId, thinkingLevel, providers, stats, header, placeholder, popoverSide = "top", mode, onModeChange, onConfigure, onSend, onStop, onOpenSettings, disabled }: ComposerProps) {
  const [draft, setDraft] = useState("");
  const areaRef = useRef<HTMLTextAreaElement>(null);
  const busy = status === "running" || status === "stopping";

  useLayoutEffect(() => {
    const area = areaRef.current;
    if (!area) return;
    area.style.height = "auto";
    area.style.height = `${Math.min(area.scrollHeight, 200)}px`;
  }, [draft]);

  async function send() {
    const message = draft.trim();
    if (!message || busy) return;
    setDraft("");
    const ok = await onSend(message);
    if (!ok) setDraft(message);
  }

  const context = stats?.contextUsage;
  const statsLabel = stats
    ? `${formatTokens(stats.tokens.total)} tokens · ${context?.percent == null ? "?" : Math.round(context.percent)}% context${stats.cost ? ` · $${stats.cost.toFixed(3)}` : ""}`
    : undefined;

  return (
    <div className={`composer-wrap ${disabled ? "disabled" : ""}`}>
      {header && <div className="composer-header">{header}</div>}
      <div className="composer" aria-disabled={disabled || undefined}>
        <textarea
          ref={areaRef}
          value={draft}
          rows={1}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void send();
            }
          }}
          placeholder={placeholder ?? (providers.length === 0 ? "Connect a provider to start…" : busy ? "Pi is working — queue your next message…" : mode === "plan" ? "Describe the work — in Plan mode Pi inspects and proposes a plan without changing files…" : "Ask Pi to inspect, change, or run something…")}
          disabled={disabled || providers.length === 0}
        />
        <div className="composer-toolbar">
          <div className="composer-left">
            {providers.length === 0 ? (
              <button type="button" className="model-pill callout" onClick={onOpenSettings}>
                <Icon name="key" /> Connect a provider
              </button>
            ) : (
              <>
                <ModelPicker
                  providers={providers}
                  providerId={providerId}
                  modelId={modelId}
                  disabled={busy}
                  popoverSide={popoverSide}
                  onConfigure={onConfigure}
                />
                <ReasoningToggle
                  providers={providers}
                  providerId={providerId}
                  modelId={modelId}
                  thinkingLevel={thinkingLevel}
                  disabled={busy}
                  popoverSide={popoverSide}
                  onConfigure={onConfigure}
                />
                {onModeChange && <ModeToggle mode={mode ?? "build"} disabled={busy} onChange={onModeChange} />}
              </>
            )}
          </div>
          <div className="composer-right">
            {statsLabel && stats && <ContextPanel stats={stats} label={statsLabel} />}
            {busy ? (
              <Tooltip label={status === "stopping" ? "Stopping…" : "Stop"}>
                <button type="button" className="send-button stop" onClick={onStop} disabled={status === "stopping"} aria-label="Stop">
                  <Icon name="stop" />
                </button>
              </Tooltip>
            ) : (
              <Tooltip label="Send (⏎)">
                <button type="button" className="send-button" onClick={() => void send()} disabled={!draft.trim()} aria-label="Send message">
                  <Icon name="send" />
                </button>
              </Tooltip>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
