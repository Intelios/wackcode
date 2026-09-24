import { useEffect, useState } from "react";

interface Props {
  source: string;
  /** "install" fetches and enables; "enable" only grants trust to something already on disk. */
  mode: "install" | "enable";
  busy?: boolean;
  error?: string;
  /** Settings › Skills: only the package's skills are switched on. */
  skillsOnly?: boolean;
  /** Resource kinds the package's manifest declares, once fetched; "unknown" if that failed. */
  declares?: string[] | "unknown";
  onCancel: () => void;
  onConfirm: () => void;
}

const OTHER_KINDS: Record<string, string> = { extensions: "extensions (code)", prompts: "prompt templates", themes: "themes" };

function alsoContains(declares: string[] | "unknown" | undefined): string | undefined {
  if (declares === undefined) return undefined;
  if (declares === "unknown") return "Anything else it contains, such as extensions, stays off until you turn it on in Settings › Packages.";
  const others = declares.filter((kind) => kind !== "skills").map((kind) => OTHER_KINDS[kind] ?? kind);
  if (others.length === 0) return undefined;
  const list = others.length === 1 ? others[0] : `${others.slice(0, -1).join(", ")} and ${others[others.length - 1]}`;
  return `It also contains ${list}, which stay off until you turn them on in Settings › Packages.`;
}

/**
 * Shown before the first install of a package. Installing grants arbitrary local code execution
 * inside the process that can read provider credentials, so the consequences are stated plainly and
 * the confirm button stays locked until the user acknowledges them.
 */
export function TrustDialog({ source, mode, busy, error, skillsOnly = false, declares, onCancel, onConfirm }: Props) {
  const [acknowledged, setAcknowledged] = useState(false);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape" && !busy) onCancel();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [busy, onCancel]);

  return (
    <div className="modal-backdrop" onPointerDown={() => { if (!busy) onCancel(); }}>
      <div
        className="confirm-dialog trust-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="trust-dialog-title"
        onPointerDown={(event) => event.stopPropagation()}
      >
        <h3 id="trust-dialog-title">
          {skillsOnly ? "Install this package's skills?" : mode === "install" ? "Install this package?" : "Enable this package?"}
        </h3>
        <p className="trust-source">{source}</p>
        {skillsOnly ? (
          <>
            <p className="trust-scope">Only its skills are switched on. {alsoContains(declares)}</p>
            <div className="trust-warning">
              Installing runs the package&rsquo;s npm install scripts, and its skills can tell the agent to run the
              programs it ships. Both run as ordinary local code with your macOS account&rsquo;s permissions: they can
              read and write any file you can, run any command, and make network requests. Install it only if you
              trust its author.
            </div>
          </>
        ) : (
          <div className="trust-warning">
            Pi packages run as ordinary local code with your macOS account&rsquo;s permissions. This package
            will be able to read and write any file you can, run any command, make network requests, and
            read WackCode&rsquo;s API keys and subscription credentials. Install it only if you trust its author.
          </div>
        )}
        <label className="trust-ack">
          <input
            type="checkbox"
            checked={acknowledged}
            onChange={(event) => setAcknowledged(event.target.checked)}
          />
          <span>I understand and trust this package</span>
        </label>
        {error && <div className="error-banner">{error}</div>}
        <div className="confirm-actions">
          <button type="button" className="secondary-button" onClick={onCancel} disabled={busy}>Cancel</button>
          <button type="button" className="danger-button" onClick={onConfirm} disabled={busy || !acknowledged}>
            {busy ? "Working…" : mode === "install" ? (skillsOnly ? "Trust and install skills" : "Trust and install") : "Trust and enable"}
          </button>
        </div>
      </div>
    </div>
  );
}
