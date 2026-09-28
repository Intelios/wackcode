import { useEffect, useState } from "react";
import { api } from "../api";
import type { UsageStatus } from "../types";
import { Icon } from "./Icons";

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
  return <section className="settings-section integrations-section">
    <div className="integration-heading"><h3>TokenTrail</h3>
      <button type="button" role="switch" aria-label="Record TokenTrail usage" aria-checked={status?.enabled ?? false}
        className={`toggle ${status?.enabled ? "on" : ""}`} disabled={!status || saving} onClick={() => void toggle()}><span /></button>
    </div>
    <p role="status">{status ? status.enabled ? status.error ? "Recording needs attention" : status.lastWritten ? "Recording usage locally" : "Ready to record usage" : "Recording paused" : "Loading recording status"}</p>
    {status?.pending ? <p>{status.pending} records waiting to be saved.</p> : null}
    {status?.dropped ? <p role="alert">{status.dropped} records could not be retained.</p> : null}
    {(error || status?.error) && <p role="alert">{error || status?.error}</p>}
    <button type="button" className="secondary-button" disabled={!status?.hasHistory} onClick={() => {
      if (status) void api.revealPath(status.path).catch((reason) => setError(String(reason)));
    }}><Icon name="folder" /> Usage folder</button>
  </section>;
}
