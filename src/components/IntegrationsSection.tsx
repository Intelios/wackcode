import { useEffect, useState } from "react";
import { api } from "../api";
import { formatRelativeTime } from "../chat-utils";
import type { UsageStatus } from "../types";
import type { IconName } from "./Icons";
import { Icon } from "./Icons";

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
  return (
    <div className="settings-scroll integrations-settings">
      <div className="section-heading-row">
        <div>
          <h3>Integrations</h3>
          <p>Apps that work alongside WackCode by reading what it writes locally — nothing is sent anywhere.</p>
        </div>
      </div>
      <div className="package-grid">
        {INTEGRATIONS.map((integration) => (
          <TokenTrailCard
            key={integration.kind}
            integration={integration}
            status={status}
            saving={saving}
            error={error}
            onToggle={() => void toggle()}
            onReveal={() => { if (status) void api.revealPath(status.path).catch((reason) => setError(String(reason))); }}
          />
        ))}
      </div>

      <div className="section-heading-row">
        <div>
          <h3>What TokenTrail can see</h3>
          <p>Every record in the ledger has the same shape; TokenTrail only ever reads it.</p>
        </div>
      </div>
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
    </div>
  );
}

interface CardProps {
  integration: Integration;
  status?: UsageStatus;
  saving: boolean;
  /** The last failure, from either polling the status or flipping the switch. */
  error?: string;
  onToggle: () => void;
  onReveal: () => void;
}

/** TokenTrail's card: the recording switch, live ledger health, and the folder it reads. */
function TokenTrailCard({ integration, status, saving, error, onToggle, onReveal }: CardProps) {
  const failure = error ?? status?.error ?? undefined;
  const state = !status ? "" : status.error ? "attention" : status.enabled ? "on" : "off";
  const stateLabel = !status ? "Loading recording status"
    : status.error ? "Recording needs attention"
    : status.enabled ? (status.lastWritten ? "Recording usage locally" : "Ready to record usage")
    : "Recording paused";
  return (
    <article className="package-card integration-card">
      <div className="package-card-head">
        <span className="package-icon kind-integration" aria-hidden="true"><Icon name={integration.icon} /></span>
        <span className="package-name builtin-name">{integration.name}</span>
        <span className="package-badge">{integration.badge}</span>
        <button
          type="button"
          role="switch"
          aria-label={integration.toggleLabel}
          aria-checked={status?.enabled ?? false}
          className={`toggle ${status?.enabled ? "on" : ""}`}
          disabled={!status || saving}
          onClick={onToggle}
        ><span /></button>
      </div>
      <div className="package-card-status">
        <span className={`package-state ${state}`} role="status">{stateLabel}</span>
        {status?.lastWritten && status.enabled && !status.error && (
          <span className="package-meta" title={new Date(status.lastWritten).toLocaleString()}>
            last record {formatRelativeTime(new Date(status.lastWritten).toISOString())}
          </span>
        )}
      </div>
      <p className="builtin-desc">{integration.description}</p>
      {status?.path && <p className="package-source" title={status.path}>{status.path}</p>}
      {status?.pending ? <p className="integration-note">{status.pending} records waiting to be saved.</p> : null}
      {status?.dropped ? <p className="package-error" role="alert">{status.dropped} records could not be retained.</p> : null}
      {failure && <p className="package-error" role="alert">{failure}</p>}
      <div className="package-card-foot">
        <span className="package-foot-note">kept indefinitely · read-only for TokenTrail</span>
        <button type="button" className="secondary-button" disabled={!status?.hasHistory} onClick={onReveal}>
          <Icon name="folder" /> Usage folder
        </button>
      </div>
    </article>
  );
}
