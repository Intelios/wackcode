import { useCallback, useEffect, useState } from "react";
import type { ComputerUseConfig, ComputerUseStatus } from "../types";
import { Icon } from "./Icons";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import { Select } from "./ui/Select";

type Pane = "accessibility" | "screenRecording";

/** What Settings › Computer use and its setup dialog ask the host for (wired in SettingsPage). */
export interface ComputerUseActions {
  onStatus: () => Promise<ComputerUseStatus>;
  onRequestPermission: (pane: Pane) => Promise<void>;
  onOpenSettings: (pane: Pane) => Promise<void>;
  onResetPermissions: () => Promise<void>;
  onRelaunch: () => Promise<void>;
  onListApps: () => Promise<{ name: string; bundleId: string }[]>;
}

/** Live permission status, re-read every `intervalMs` so granting in System Settings shows up. */
export function useComputerUseStatus(onStatus: ComputerUseActions["onStatus"], intervalMs = 1500) {
  const [status, setStatus] = useState<ComputerUseStatus>();
  const refresh = useCallback(() => {
    void onStatus().then(setStatus).catch(() => undefined);
  }, [onStatus]);
  useEffect(() => {
    refresh();
    const timer = window.setInterval(refresh, intervalMs);
    return () => window.clearInterval(timer);
  }, [refresh, intervalMs]);
  return { status, refresh };
}

const STEPS: { pane: Pane; title: string; detail: string }[] = [
  { pane: "accessibility", title: "Accessibility", detail: "Read an app's buttons, fields and menus, and press, type and choose in them." },
  { pane: "screenRecording", title: "Screen Recording", detail: "Capture the window of an app you allow, so a model with vision can see it." }
];

interface StepsProps {
  status?: ComputerUseStatus;
  actions: ComputerUseActions;
  onError: (message: string) => void;
  onRelaunch: () => void;
}

/** The two macOS permissions, each with its live state and the way to grant it. */
export function ComputerUsePermissionSteps({ status, actions, onError, onRelaunch }: StepsProps) {
  const [asked, setAsked] = useState<Record<Pane, boolean>>({ accessibility: false, screenRecording: false });
  return (
    <ol className="computer-permissions">
      {STEPS.map((step, index) => {
        const granted = step.pane === "accessibility" ? status?.accessibility : status?.screenRecording;
        return (
          <li key={step.pane} className={`computer-permission ${granted ? "granted" : ""}`}>
            <span className="computer-permission-mark" aria-hidden="true">
              {granted ? <Icon name="check" /> : index + 1}
            </span>
            <div className="computer-permission-text">
              <strong>{step.title}</strong>
              <span>{step.detail}</span>
              {!granted && asked[step.pane] && step.pane === "screenRecording" && (
                <small>Already switched on? macOS applies Screen Recording only after WackCode reopens.</small>
              )}
            </div>
            <div className="computer-permission-actions">
              {granted ? (
                <span className="computer-permission-state" role="status">Allowed</span>
              ) : (
                <>
                  {asked[step.pane] && step.pane === "screenRecording" && (
                    <button type="button" className="secondary-button compact" onClick={onRelaunch}>Quit &amp; Reopen</button>
                  )}
                  <button
                    type="button"
                    className="primary-button compact"
                    disabled={!status?.supported}
                    onClick={() => {
                      setAsked((current) => ({ ...current, [step.pane]: true }));
                      const request = asked[step.pane] ? actions.onOpenSettings(step.pane) : actions.onRequestPermission(step.pane);
                      void request.catch((reason) => onError(String(reason)));
                    }}
                  >
                    {asked[step.pane] ? "Open System Settings" : "Allow…"}
                  </button>
                </>
              )}
            </div>
          </li>
        );
      })}
    </ol>
  );
}

const ALWAYS_BLOCKED = [
  "WackCode itself — the agent can never see or click its own windows",
  "Terminals and script runners (Terminal, iTerm, Ghostty, Warp, Script Editor, Shortcuts…)",
  "Password managers and Keychain Access",
  "System Settings, sign-in, security and permission prompts",
  "The Dock, menu bar, Control Center, Spotlight and notifications"
];

interface Props {
  config: ComputerUseConfig;
  actions: ComputerUseActions;
  onChange: (config: ComputerUseConfig) => Promise<void>;
}

/** Settings › Computer use: permissions, the stop shortcut, and the apps it never uses. */
export function ComputerUseSection({ config, actions, onChange }: Props) {
  const { status, refresh } = useComputerUseStatus(actions.onStatus);
  const [error, setError] = useState<string>();
  const [confirm, setConfirm] = useState<"reset" | "relaunch">();
  const [apps, setApps] = useState<{ name: string; bundleId: string }[]>([]);
  const [manual, setManual] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    void actions.onListApps().then((list) => { if (live) setApps(list); }).catch(() => undefined);
    return () => { live = false; };
  }, [actions]);

  async function save(neverAllow: string[]) {
    setBusy(true);
    setError(undefined);
    try { await onChange({ ...config, neverAllow }); }
    catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  }

  function add(bundleId: string) {
    const id = bundleId.trim();
    if (!id || config.neverAllow.some((entry) => entry.toLowerCase() === id.toLowerCase())) return;
    void save([...config.neverAllow, id]);
    setManual("");
  }

  const nameOf = (bundleId: string) => apps.find((app) => app.bundleId.toLowerCase() === bundleId.toLowerCase())?.name;
  const candidates = apps.filter((app) => !config.neverAllow.some((entry) => entry.toLowerCase() === app.bundleId.toLowerCase()));

  return (
    <div className="settings-scroll computer-settings">
      <div className="section-heading-row">
        <div>
          <h3>Permissions</h3>
          <p>
            macOS asks you to allow WackCode twice. After that, the agent still asks in the chat the first time it wants each app.
          </p>
        </div>
        <div className="row-actions">
          <button type="button" className="secondary-button compact" onClick={refresh}><Icon name="refresh" /> Recheck</button>
        </div>
      </div>
      {status && !status.supported && <div className="error-banner" role="alert">Computer use needs macOS 14 or later.</div>}
      <ComputerUsePermissionSteps status={status} actions={actions} onError={setError} onRelaunch={() => setConfirm("relaunch")} />
      {status?.devBuild && (
        <p className="computer-note">
          This is a development build. macOS credits these permissions to whatever launched it (your terminal, for example), not to WackCode.
        </p>
      )}
      <p className="computer-note">
        Switched on in System Settings but still not allowed here? macOS forgets approvals when WackCode is rebuilt or updated.{" "}
        <button type="button" className="text-button" onClick={() => setConfirm("reset")}>Reset WackCode's permissions</button> and allow them again.
      </p>

      <div className="section-heading-row">
        <div>
          <h3>Stop shortcut</h3>
          <p>
            <kbd>⌃⌥⌘.</kbd> stops computer use in every chat. It's only claimed while the agent is using an app; the menu bar icon and a chat's Stop button work too.
          </p>
        </div>
      </div>
      {status && !status.hotkeyAvailable && (
        <div className="error-banner" role="alert">Another app is using ⌃⌥⌘., so the shortcut isn't available. Use the menu bar icon or Stop instead.</div>
      )}

      <div className="section-heading-row">
        <div>
          <h3>Apps it never uses</h3>
          <p>Refused before anything is asked. These are always blocked:</p>
        </div>
      </div>
      <ul className="computer-blocked">
        {ALWAYS_BLOCKED.map((entry) => <li key={entry}><Icon name="close" /> {entry}</li>)}
      </ul>
      {config.neverAllow.length > 0 && (
        <ul className="computer-never" aria-label="Apps you never allow">
          {config.neverAllow.map((bundleId) => (
            <li key={bundleId}>
              <span>{nameOf(bundleId) ?? bundleId}</span>
              {nameOf(bundleId) && <code>{bundleId}</code>}
              <button
                type="button"
                className="ghost-button"
                aria-label={`Allow asking for ${nameOf(bundleId) ?? bundleId} again`}
                disabled={busy}
                onClick={() => void save(config.neverAllow.filter((entry) => entry !== bundleId))}
              >
                <Icon name="close" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="computer-never-add">
        {candidates.length > 0 && (
          <Select
            className="settings-select"
            value=""
            placeholder="Add a running app…"
            disabled={busy}
            options={candidates.map((app) => ({ value: app.bundleId, label: app.name, hint: app.bundleId }))}
            onChange={add}
            aria-label="Add a running app to never allow"
          />
        )}
        <input
          value={manual}
          placeholder="or a bundle id, e.g. com.example.App"
          aria-label="Bundle id to never allow"
          disabled={busy}
          onChange={(event) => setManual(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter") add(manual); }}
        />
        <button type="button" className="secondary-button compact" disabled={busy || !manual.trim()} onClick={() => add(manual)}>Add</button>
      </div>
      {error && <div className="error-banner" role="alert">{error}</div>}

      {confirm === "reset" && (
        <ConfirmDialog
          title="Reset WackCode's permissions?"
          body="This removes WackCode from the Accessibility and Screen Recording lists in System Settings, for this and every other build. Allow them again afterwards."
          confirmLabel="Reset"
          danger
          onConfirm={async () => { await actions.onResetPermissions(); refresh(); }}
          onCancel={() => setConfirm(undefined)}
        />
      )}
      {confirm === "relaunch" && (
        <ConfirmDialog
          title="Quit and reopen WackCode?"
          body="Running chats are stopped. Your chats and settings are kept."
          confirmLabel="Quit & Reopen"
          onConfirm={actions.onRelaunch}
          onCancel={() => setConfirm(undefined)}
        />
      )}
    </div>
  );
}
