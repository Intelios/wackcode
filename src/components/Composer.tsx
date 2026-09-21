import { useLayoutEffect, useRef, useState } from "react";
import type { ProviderRecord, SessionSnapshot, TaskRecord, ThinkingLevel } from "../types";
import { formatTokens } from "../chat-utils";
import { Icon } from "./Icons";
import { ContextPanel } from "./ContextPanel";
import { ModelPicker } from "./ModelPicker";
import { Tooltip } from "./ui/Tooltip";

interface ComposerProps {
  task: TaskRecord;
  providers: ProviderRecord[];
  stats?: SessionSnapshot["stats"];
  onConfigure: (patch: { providerId?: string; modelId?: string; thinkingLevel?: ThinkingLevel }) => void;
  onSend: (message: string) => Promise<boolean>;
  onStop: () => void;
  onOpenSettings: () => void;
}

export function Composer({ task, providers, stats, onConfigure, onSend, onStop, onOpenSettings }: ComposerProps) {
  const [draft, setDraft] = useState("");
  const areaRef = useRef<HTMLTextAreaElement>(null);
  const busy = task.status === "running" || task.status === "stopping";

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
    <div className="composer-wrap">
      <div className="composer">
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
          placeholder={providers.length === 0 ? "Connect a provider to start…" : busy ? "Pi is working — queue your next message…" : "Ask Pi to inspect, change, or run something…"}
          disabled={providers.length === 0}
        />
        <div className="composer-toolbar">
          <div className="composer-left">
            {providers.length === 0 ? (
              <button type="button" className="model-pill callout" onClick={onOpenSettings}>
                <Icon name="key" /> Connect a provider
              </button>
            ) : (
              <ModelPicker
                providers={providers}
                providerId={task.providerId}
                modelId={task.modelId}
                thinkingLevel={task.thinkingLevel}
                disabled={busy}
                onConfigure={onConfigure}
              />
            )}
          </div>
          <div className="composer-right">
            {statsLabel && stats && <ContextPanel stats={stats} label={statsLabel} />}
            {busy ? (
              <Tooltip label={task.status === "stopping" ? "Stopping…" : "Stop"}>
                <button type="button" className="send-button stop" onClick={onStop} disabled={task.status === "stopping"} aria-label="Stop">
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
