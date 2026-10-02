import { useCallback, useEffect, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { ComputerUseConfig, ComputerUseStatus } from "../types";
import { DuckMark } from "./DuckMark";
import { Icon, type IconName } from "./Icons";
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

const STEPS: { pane: Pane; icon: IconName; title: string; detail: string }[] = [
  { pane: "accessibility", icon: "accessibility", title: "Accessibility", detail: "Read an app's buttons, fields and menus, and press, type and choose in them." },
  { pane: "screenRecording", icon: "viewfinder", title: "Screen Recording", detail: "Capture the window of an app you allow, so a model with vision can see it." }
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
  const [requesting, setRequesting] = useState<Pane>();
  return (
    <ol className="computer-permissions">
      {STEPS.map((step, index) => {
        const granted = step.pane === "accessibility" ? status?.accessibility : status?.screenRecording;
        return (
          <li key={step.pane} className={`computer-permission ${granted ? "granted" : ""}`}>
            <span className="computer-permission-mark" aria-hidden="true">
              <Icon name={granted ? "check" : step.icon} />
              {!granted && <b>{index + 1}</b>}
            </span>
            <div className="computer-permission-text">
              <strong>{step.title}</strong>
              <span>{step.detail}</span>
              {!granted && asked[step.pane] && (
                <small>
                  {step.pane === "accessibility" && "If WackCode isn't listed, use + in System Settings to add the WackCode.app you're running. "}
                  Already switched on? Quit and reopen WackCode so macOS can apply the new approval.
                </small>
              )}
            </div>
            <div className="computer-permission-actions">
              {granted ? (
                <span className="computer-permission-state" role="status"><i aria-hidden="true" />Allowed</span>
              ) : (
                <>
                  {asked[step.pane] && (
                    <button type="button" className="secondary-button compact" onClick={onRelaunch}>Quit &amp; Reopen</button>
                  )}
                  <button
                    type="button"
                    className="primary-button compact"
                    disabled={!status?.supported || !!requesting}
                    onClick={() => {
                      setRequesting(step.pane);
                      const request = asked[step.pane] ? actions.onOpenSettings(step.pane) : actions.onRequestPermission(step.pane);
                      void request
                        .then(() => setAsked((current) => ({ ...current, [step.pane]: true })))
                        .catch((reason) => onError(String(reason)))
                        .finally(() => setRequesting(undefined));
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

const ALWAYS_BLOCKED: { icon: IconName | "duck"; title: string; detail: string }[] = [
  { icon: "duck", title: "WackCode itself", detail: "The agent can never see or click its own windows." },
  { icon: "terminal", title: "Terminals and script runners", detail: "Terminal, iTerm, Ghostty, Warp, Script Editor, Shortcuts…" },
  { icon: "key", title: "Password managers", detail: "And Keychain Access." },
  { icon: "settings", title: "System Settings", detail: "Sign-in, security and permission prompts." },
  { icon: "dock", title: "The Dock and menu bar", detail: "Control Center, Spotlight and notifications." }
];

/**
 * The hero's little stage: an app window with a pointer that types into a field and presses a
 * button while computer use is ready, and rests, dimmed, until it is. Pure decoration; the pill
 * beside it says the same in words. The loop lives in styles.css, which stills it under reduced
 * motion.
 */
function ComputerStage({ live }: { live: boolean }) {
  return (
    <svg className={`computer-stage ${live ? "live" : ""}`} viewBox="0 0 160 110" aria-hidden="true">
      <rect className="computer-stage-window" x="10" y="8" width="140" height="94" rx="9" />
      <path className="computer-stage-bar" d="M10 26h140" />
      <circle className="computer-stage-dot" cx="21" cy="17" r="2.2" />
      <circle className="computer-stage-dot" cx="29" cy="17" r="2.2" />
      <circle className="computer-stage-dot" cx="37" cy="17" r="2.2" />
      <rect className="computer-stage-field" x="24" y="38" width="112" height="16" rx="5" />
      <rect className="computer-stage-typed" x="30" y="44" width="62" height="4" rx="2" />
      <rect className="computer-stage-button" x="94" y="68" width="42" height="17" rx="6" />
      <path className="computer-stage-lines" d="M24 68h48M24 77h32" />
      <g className="computer-stage-pointer">
        <path d="M0 0v15l4.2-3.6 3 6.9 3-1.3-3-6.7 5.6-.4z" />
      </g>
    </svg>
  );
}

/** ⌃⌥⌘. as four physical keys that press in turn, like the chord they are. */
function Keycaps({ available }: { available: boolean }) {
  return (
    <div className={`computer-keys ${available ? "" : "unavailable"}`} role="img" aria-label="Control Option Command Period">
      {["⌃", "⌥", "⌘", "."].map((key, index) => (
        <kbd key={key} style={{ "--k": index } as React.CSSProperties} aria-hidden="true">{key}</kbd>
      ))}
    </div>
  );
}

interface Props {
  config: ComputerUseConfig;
  actions: ComputerUseActions;
  /** The agent's name in the app's own copy (Settings › Appearance). */
  agentName?: string;
  onChange: (config: ComputerUseConfig) => Promise<void>;
}

/** Settings › Computer use: permissions, the stop shortcut, and the apps it never uses. */
export function ComputerUseSection({ config, actions, agentName = "WackCode", onChange }: Props) {
  const { status, refresh } = useComputerUseStatus(actions.onStatus);
  const reduceMotion = useReducedMotion();
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

  const missing = status ? Number(!status.accessibility) + Number(!status.screenRecording) : undefined;
  const ready = status?.supported && missing === 0;
  const pill = !status ? "Checking…"
    : !status.supported ? "Needs macOS 14"
    : missing === 0 ? "Ready"
    : missing === 1 ? "1 permission to go"
    : "2 permissions to go";
  const step = (index: number) => ({ "--i": index } as React.CSSProperties);

  return (
    <div className="settings-scroll computer-settings">
      <div className="computer-page">
        <section className={`computer-hero ${ready ? "ready" : ""}`} style={step(0)} aria-label="Computer use status">
          <ComputerStage live={!!ready} />
          <div className="computer-hero-text">
            <span className={`computer-pill ${ready ? "ready" : ""}`}><i aria-hidden="true" />{pill}</span>
            <h3>{agentName} can use the apps you build</h3>
            <p>macOS asks you to allow {agentName} twice. After that, the agent still asks in the chat the first time it wants each app.</p>
          </div>
          <button type="button" className="secondary-button compact computer-recheck" onClick={refresh}><Icon name="refresh" /> Recheck</button>
        </section>

        <section className="computer-block" style={step(1)}>
          <h3 className="computer-block-title">Permissions</h3>
          {status && !status.supported && <div className="error-banner" role="alert">Computer use needs macOS 14 or later.</div>}
          <ComputerUsePermissionSteps status={status} actions={actions} onError={setError} onRelaunch={() => setConfirm("relaunch")} />
          <div className="computer-trouble">
            {status?.devBuild && (
              <p>
                This is a development build. macOS credits these permissions to whatever launched it (your terminal, for example), not to WackCode.
              </p>
            )}
            <p>
              Switched on in System Settings but still not allowed here? Click Allow… above to replace an approval from an older WackCode build.
              Only that permission is renewed. If it's still unavailable after allowing it, quit and reopen WackCode.
            </p>
            <p>
              To start over with both permissions, <button type="button" className="text-button" onClick={() => setConfirm("reset")}>Reset WackCode's permissions</button> and allow them again.
            </p>
          </div>
        </section>

        <section className="computer-block" style={step(2)}>
          <h3 className="computer-block-title">Stop shortcut</h3>
          <div className="computer-stop">
            <Keycaps available={status?.hotkeyAvailable !== false} />
            <div className="computer-stop-text">
              <p>Stops computer use in every chat. It's only claimed while the agent is using an app.</p>
              <ul aria-label="Other ways to stop">
                <li><span className="computer-stop-duck"><DuckMark /></span> The menu bar icon</li>
                <li><Icon name="stop" /> A chat's Stop button</li>
              </ul>
            </div>
          </div>
          {status && !status.hotkeyAvailable && (
            <div className="error-banner" role="alert">Another app is using ⌃⌥⌘., so the shortcut isn't available. Use the menu bar icon or Stop instead.</div>
          )}
        </section>

        <section className="computer-block" style={step(3)}>
          <h3 className="computer-block-title">Apps it never uses</h3>
          <p className="computer-block-sub">Refused before anything is asked.</p>
          <ul className="computer-blocked" aria-label="Always blocked">
            {ALWAYS_BLOCKED.map((entry) => (
              <li key={entry.title}>
                <span className="computer-blocked-icon" aria-hidden="true">{entry.icon === "duck" ? <DuckMark /> : <Icon name={entry.icon} />}</span>
                <div>
                  <strong>{entry.title}</strong>
                  <span>{entry.detail}</span>
                </div>
                <Icon name="lock" />
              </li>
            ))}
          </ul>

          <h4 className="computer-list-title">Your list</h4>
          {config.neverAllow.length === 0 ? (
            <p className="computer-never-empty">Nothing added yet. Apps you add here are never asked about.</p>
          ) : (
            <ul className="computer-never" aria-label="Apps you never allow">
              <AnimatePresence initial={false}>
                {config.neverAllow.map((bundleId) => {
                  const name = nameOf(bundleId);
                  return (
                    <motion.li
                      key={bundleId}
                      layout={!reduceMotion}
                      initial={reduceMotion ? false : { opacity: 0, scale: .85 }}
                      animate={{ opacity: 1, scale: 1 }}
                      exit={reduceMotion ? { opacity: 0 } : { opacity: 0, scale: .85 }}
                      transition={{ type: "spring", stiffness: 520, damping: 32 }}
                    >
                      <span className="computer-never-mono" aria-hidden="true">{(name ?? bundleId).charAt(0).toUpperCase()}</span>
                      <span>{name ?? bundleId}</span>
                      {name && <code>{bundleId}</code>}
                      <button
                        type="button"
                        className="ghost-button"
                        aria-label={`Allow asking for ${name ?? bundleId} again`}
                        disabled={busy}
                        onClick={() => void save(config.neverAllow.filter((entry) => entry !== bundleId))}
                      >
                        <Icon name="close" />
                      </button>
                    </motion.li>
                  );
                })}
              </AnimatePresence>
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
        </section>
        {error && <div className="error-banner" role="alert">{error}</div>}
      </div>

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
