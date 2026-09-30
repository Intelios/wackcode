import { useEffect, useState } from "react";
import { api } from "../api";
import { formatRelativeTime } from "../chat-utils";
import type { UsageStatus } from "../types";
import { DuckMark } from "./DuckMark";
import type { IconName } from "./Icons";
import { Icon } from "./Icons";
import { SettingsHero, stagger } from "./SettingsHero";

/** One entry in the integrations grid. Cards render against live status keyed by `kind`,
 *  mirroring BUILTIN_EXTENSIONS in PackagesSection: add a row (and its card below) per app. */
interface Integration {
  kind: "tokentrail";
  name: string;
  description: string;
  icon: IconName;
  badge: string;
  toggleLabel: string;
}

const INTEGRATIONS: readonly Integration[] = [
  {
    kind: "tokentrail",
    name: "TokenTrail",
    description:
      "Reads the usage-only ledger WackCode writes locally — token counts, model, timing and outcome for every request — and turns it into cost estimates.",
    icon: "flame",
    badge: "local",
    toggleLabel: "Record TokenTrail usage"
  }
];

/* The ledger contract in one glance, condensed from docs/wackcode-usage-v1.md. */
const RECORDED = [
  "Token counts and timing",
  "Model, provider and purpose",
  "Project and workspace paths",
  "Outcome — completed, failed, cancelled"
];
const NEVER_RECORDED = [
  "Prompts, responses and chat titles",
  "Credentials and endpoint URLs",
  "Tool arguments"
];

/** The flame from Icons.tsx, drawn larger on the stage. */
const FLAME = "M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.07-2.14-.22-4.05 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.15.43-2.29 1-3a2.5 2.5 0 0 0 2.5 2.5z";

/**
 * The hero's little stage: the duck writes records into a ledger one after another, and
 * TokenTrail's flame reads them from the other side. Rests dashed and dimmed while recording is
 * paused. Pure decoration; the pill says the same in words. The loop lives in styles.css, which
 * stills it under reduced motion.
 */
function LedgerStage({ live }: { live: boolean }) {
  const rows = [28, 22, 30, 18, 26];
  return (
    <svg className={`settings-stage ledger-stage ${live ? "live" : ""}`} viewBox="0 0 160 110" aria-hidden="true">
      <path className="ledger-stage-flow" d="M44 55h12" />
      <path className="ledger-stage-flow read" d="M108 55h10" />
      <rect className="ledger-stage-tile" x="8" y="37" width="36" height="36" rx="10" />
      <DuckMark x="16" y="45" width="20" height="20" />
      <rect className="ledger-stage-page" x="56" y="12" width="52" height="86" rx="8" />
      <rect className="ledger-stage-head" x="64" y="21" width="24" height="4" rx="2" />
      {rows.map((width, index) => (
        <g key={index} className="ledger-stage-row" style={{ "--k": index } as React.CSSProperties}>
          <circle cx="66" cy={37 + index * 12} r="1.8" />
          <rect x="71" y={35 + index * 12} width={width} height="4" rx="2" />
        </g>
      ))}
      <g transform="translate(116 29) scale(1.6)">
        <path className="ledger-stage-flame" d={FLAME} />
      </g>
    </svg>
  );
}

/** Settings › Integrations: a card per app that consumes what WackCode produces locally.
 *  Status is polled so a write error or a draining queue shows up without a refresh. */
export function IntegrationsSection() {
  const [status, setStatus] = useState<UsageStatus>();
  const [error, setError] = useState<string>();
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    let live = true;
    const refresh = () => { void api.usageStatus().then((value) => { if (live) { setStatus(value); setError(undefined); } }).catch((reason) => { if (live) setError(String(reason)); }); };
    refresh();
    const timer = setInterval(refresh, 5000);
    return () => { live = false; clearInterval(timer); };
  }, []);
  async function toggle() {
    if (!status) return;
    setSaving(true);
    setError(undefined);
    try { await api.setUsageRecording(!status.enabled); setStatus(await api.usageStatus()); }
    catch (reason) { setError(String(reason)); }
    finally { setSaving(false); }
  }
  const recording = Boolean(status?.enabled && !status.error);
  const pill = !status ? "Checking…" : status.error ? "Needs attention" : status.enabled ? "Recording" : "Paused";
  return (
    <div className="settings-scroll integrations-settings">
      <div className="settings-page">
        <SettingsHero
          label="Integrations overview"
          stage={<LedgerStage live={recording} />}
          live={recording}
          attention={Boolean(status?.error)}
          pill={pill}
          title="Apps that read what WackCode keeps"
        >
          <p>Integrations work alongside WackCode by reading files it writes on this Mac. Nothing is sent anywhere.</p>
        </SettingsHero>
        {INTEGRATIONS.map((integration, index) => (
          <TokenTrailCard
            key={integration.kind}
            index={index + 1}
            integration={integration}
            status={status}
            saving={saving}
            error={error}
            onToggle={() => void toggle()}
            onReveal={() => { if (status) void api.revealPath(status.path).catch((reason) => setError(String(reason))); }}
          />
        ))}

        <section className="settings-block" style={stagger(INTEGRATIONS.length + 1)} aria-labelledby="integration-scope-title">
          <h3 className="settings-block-title" id="integration-scope-title">What TokenTrail can see</h3>
          <p className="settings-block-sub">Every record in the ledger has the same shape; TokenTrail only ever reads it.</p>
          <div className="integration-scope">
            <div className="integration-scope-group">
              <h4>In each record</h4>
              <ul>
                {RECORDED.map((item) => <li key={item}><Icon name="check" />{item}</li>)}
              </ul>
            </div>
            <div className="integration-scope-group">
              <h4>Never recorded</h4>
              <ul>
                {NEVER_RECORDED.map((item) => <li key={item}><Icon name="close" />{item}</li>)}
              </ul>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
}

interface CardProps {
  /** Its place in the page's entrance. */
  index: number;
  integration: Integration;
  status?: UsageStatus;
  saving: boolean;
  /** The last failure, from either polling the status or flipping the switch. */
  error?: string;
  onToggle: () => void;
  onReveal: () => void;
}

/** TokenTrail's card: the recording switch, live ledger health, and the folder it reads. */
function TokenTrailCard({ index, integration, status, saving, error, onToggle, onReveal }: CardProps) {
  const failure = error ?? status?.error ?? undefined;
  const state = !status ? "" : status.error ? "attention" : status.enabled ? "on" : "off";
  const stateLabel = !status ? "Loading recording status"
    : status.error ? "Recording needs attention"
    : status.enabled ? (status.lastWritten ? "Recording usage locally" : "Ready to record usage")
    : "Recording paused";
  return (
    <section className={`settings-block integration-block ${state}`} style={stagger(index)} aria-label={integration.name}>
      <header className="integration-head">
        <span className="integration-mark" aria-hidden="true"><Icon name={integration.icon} /></span>
        <div className="integration-title">
          <h3 className="settings-block-title">{integration.name} <span className="package-badge">{integration.badge}</span></h3>
          <span className="integration-status">
            <span className={`package-state ${state}`} role="status">{stateLabel}</span>
            {status?.lastWritten && status.enabled && !status.error && (
              <span className="package-meta" title={new Date(status.lastWritten).toLocaleString()}>
                last record {formatRelativeTime(new Date(status.lastWritten).toISOString())}
              </span>
            )}
          </span>
        </div>
        <button
          type="button"
          role="switch"
          aria-label={integration.toggleLabel}
          aria-checked={status?.enabled ?? false}
          className={`toggle ${status?.enabled ? "on" : ""}`}
          disabled={!status || saving}
          onClick={onToggle}
        ><span /></button>
      </header>
      <p className="integration-desc">{integration.description}</p>
      {status?.pending ? <p className="integration-note">{status.pending} records waiting to be saved.</p> : null}
      {status?.dropped ? <p className="package-error" role="alert">{status.dropped} records could not be retained.</p> : null}
      {failure && <p className="package-error" role="alert">{failure}</p>}
      <div className="integration-path">
        <div>
          <span>Ledger · kept indefinitely, read-only for {integration.name}</span>
          {status?.path && <code title={status.path}>{status.path}</code>}
        </div>
        <button type="button" className="secondary-button compact" disabled={!status?.hasHistory} onClick={onReveal}>
          <Icon name="folder" /> Usage folder
        </button>
      </div>
    </section>
  );
}
