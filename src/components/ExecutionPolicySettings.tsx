import { useState } from "react";
import type { ExecutionPolicyConfig } from "../types";
import { EXECUTION_POLICY_WARNING } from "../execution-policy";
import { ConfirmDialog } from "./ui/ConfirmDialog";

const OPTIONS: { key: keyof ExecutionPolicyConfig; label: string; detail: string; confirmation: string }[] = [
  { key: "unrestrictedSubagents", label: "Remove sub-agent read-only restrictions", detail: "Includes Scout, Reviewer and custom read-only agents. Adds edit and write tools; their existing shell tool becomes unrestricted. Read-only planning still restricts them.", confirmation: "Includes Scout, Reviewer and custom read-only agents. Read-only planning still restricts them." },
  { key: "unrestrictedPlanning", label: "Remove Plan / Ultra Plan read-only restrictions", detail: "Both planning modes can use all enabled tools, including tools that change connected services. The interview, plan review and approval before implementation still apply.", confirmation: "Plan and Ultra Plan can use all enabled tools, including tools that change connected services. Plan review and approval still apply." },
];

/** Explicit off-to-on confirmation, with no per-tool approval gates. */
export function ExecutionPolicySettings({ config, onChange }: {
  config: ExecutionPolicyConfig;
  onChange?: (config: ExecutionPolicyConfig) => Promise<void>;
}) {
  const [confirming, setConfirming] = useState<typeof OPTIONS[number]>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  async function save(key: keyof ExecutionPolicyConfig, enabled: boolean) {
    if (!onChange) return;
    setBusy(true);
    setError(undefined);
    try { await onChange({ ...config, [key]: enabled }); }
    finally { setBusy(false); }
  }

  return (
    <><section className="settings-block execution-policy-settings" aria-labelledby="execution-policy-title">
      <h3 className="settings-block-title" id="execution-policy-title">Read-only restrictions</h3>
      <p className="settings-block-sub">Overrides are off by default and not recommended. They apply to all chats from their next turn and stay enabled until switched off. Changing a switch does not stop an active run.</p>
      {error && <div className="error-banner" role="alert">{error}</div>}
      {OPTIONS.map((option) => (
        <div className={`execution-policy-row ${config[option.key] ? "unrestricted" : ""}`} key={option.key}>
          <div className="execution-policy-copy">
            <strong>{option.label}</strong>
            <p>{option.detail}</p>
            {config[option.key] && <p className="execution-policy-warning">{EXECUTION_POLICY_WARNING}</p>}
          </div>
          <button type="button" role="switch" aria-label={option.label} aria-checked={config[option.key]}
            className={`toggle ${config[option.key] ? "on" : ""}`} disabled={busy || !onChange || !!confirming}
            onClick={() => {
              if (!config[option.key]) setConfirming(option);
              else void save(option.key, false).catch((reason) => setError(String(reason)));
            }}><span /></button>
        </div>
      ))}
    </section>
      {confirming && <ConfirmDialog
        title={confirming.key === "unrestrictedPlanning" ? "Remove planning restrictions?" : "Remove sub-agent restrictions?"}
        body={`${EXECUTION_POLICY_WARNING} ${confirming.confirmation} Applies to all chats from their next turn and stays enabled until you switch it off.`}
        danger confirmLabel="Remove restrictions"
        onConfirm={() => save(confirming.key, true)} onCancel={() => setConfirming(undefined)}
      />}
    </>
  );
}
