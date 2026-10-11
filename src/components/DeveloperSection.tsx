import { useEffect, useState } from "react";
import { api } from "../api";
import { forceReducedMotionOn, setForceReducedMotion } from "../reducedMotion";
import type { AppInfo, ProviderRecord } from "../types";
import { Icon } from "./Icons";
import { SettingsHero } from "./SettingsHero";
import { ConfirmDialog } from "./ui/ConfirmDialog";

/** The deterministic mock provider `pnpm mock:provider` serves (docs/testing.md). */
const MOCK_BASE_URL = "http://127.0.0.1:43127/v1";
const MOCK_NAME = "Mock (local)";
/** Pi requires a key to send a request; the mock ignores what it is. */
const MOCK_PLACEHOLDER_KEY = "wackcode-mock";

/**
 * Settings › Developer, shown only in dev builds (`devBuild`): the dev identity in one read,
 * the toggles that make automated runs deterministic, and the nuclear reset. Its Rust command
 * (`developer_*`) exists only in debug builds and re-checks the dev bundle id.
 */
interface DeveloperSectionProps {
  /** The app's data folder (bootstrap's `appDataPath`) — the folder Reset moves. */
  appDataPath: string;
  providers: ProviderRecord[];
}

export function DeveloperSection({ appDataPath, providers }: DeveloperSectionProps) {
  const [info, setInfo] = useState<AppInfo>();
  const [error, setError] = useState<string>();
  const [reduce, setReduce] = useState(forceReducedMotionOn());
  const [mockBusy, setMockBusy] = useState(false);
  const [mockNote, setMockNote] = useState<string>();
  const [confirmingReset, setConfirmingReset] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    api.appInfo()
      .then((value) => { if (active) { setInfo(value); setError(undefined); } })
      .catch((reason) => { if (active) setError(String(reason)); });
    return () => { active = false; };
  }, []);

  function toggleReduce(on: boolean) {
    setForceReducedMotion(on);
    setReduce(on);
  }

  function reveal(path: string) {
    setError(undefined);
    void api.revealPath(path, true).catch((reason) => setError(String(reason)));
  }

  async function addMockProvider() {
    setMockBusy(true);
    setMockNote(undefined);
    setError(undefined);
    try {
      const existing = providers.find((provider) => provider.baseUrl === MOCK_BASE_URL);
      if (existing) {
        if (existing.enabled === false) await api.setProviderEnabled(existing.id, true);
        setMockNote(`“${existing.name}” is already here${existing.enabled === false ? " — switched back on" : ""}.`);
        return;
      }
      await api.saveProvider({
        name: MOCK_NAME,
        baseUrl: MOCK_BASE_URL,
        apiFormat: "openai-completions",
        models: [
          { id: "mock-model", name: "Mock model", contextWindow: 128000, maxTokens: 8192, reasoning: false, thinkingLevels: [], thinkingLevelMap: {}, vision: false },
        ],
        apiKey: MOCK_PLACEHOLDER_KEY,
      });
      setMockNote(`Added “${MOCK_NAME}”. Start it with \`pnpm mock:provider\`, then pick it in a chat's model picker.`);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setMockBusy(false);
    }
  }

  return (
    <div className="settings-scroll">
      <div className="settings-page">
        <SettingsHero
          label="Developer"
          stage={null}
          live={Boolean(info)}
          pill={info ? "DEV build" : undefined}
          title="Developer"
        >
          <p>This copy is a development build: its own bundle id, its own data folder, and a few controls the installed app never gets.</p>
          <p>Everything here is for driving and testing the dev app — nothing reaches the installed WackCode.</p>
        </SettingsHero>
        {error && <p className="package-error" role="alert">{error}</p>}

        <section className="settings-block" aria-labelledby="dev-identity-title">
          <h3 className="settings-block-title" id="dev-identity-title">Identity</h3>
          <p className="settings-block-sub">What makes this app the dev one — its own id and data, side by side with the installed WackCode.</p>
          {info ? (
            <ul className="about-rows">
              <li className="about-row"><span className="about-row-label">Bundle id</span><code className="about-row-value" title={info.bundleId}>{info.bundleId}</code></li>
              <li className="about-row"><span className="about-row-label">Version</span><code className="about-row-value">{info.appVersion} ({info.build})</code></li>
              <li className="about-row"><span className="about-row-label">Pi agent (bundled)</span><code className="about-row-value">{info.piVersion}</code></li>
            </ul>
          ) : (
            <p className="settings-block-sub">Reading…</p>
          )}
          {appDataPath && (
            <div className="about-path">
              <div>
                <span>Data folder</span>
                <code title={appDataPath}>{appDataPath}</code>
              </div>
              <button type="button" className="secondary-button compact" onClick={() => reveal(appDataPath)}>
                <Icon name="folder" /> Reveal
              </button>
            </div>
          )}
          {info?.workerPath && (
            <div className="about-path">
              <div>
                <span>Worker</span>
                <code title={info.workerPath}>{info.workerPath}</code>
              </div>
              <button type="button" className="secondary-button compact" onClick={() => reveal(info.workerPath)}>
                <Icon name="folder" /> Reveal
              </button>
            </div>
          )}
        </section>

        <section className="settings-block" aria-labelledby="dev-motion-title">
          <h3 className="settings-block-title" id="dev-motion-title">Force reduced motion</h3>
          <p className="settings-block-sub">Every animation and transition renders its end state, so screenshots and driver waits are deterministic. Applies the moment it's set, and remembered for next launch.</p>
          <button
            type="button"
            role="switch"
            aria-checked={reduce}
            aria-label="Force reduced motion"
            className={`toggle ${reduce ? "on" : ""}`}
            onClick={() => toggleReduce(!reduce)}
          >
            <span />
          </button>
        </section>

        <section className="settings-block" aria-labelledby="dev-mock-title">
          <h3 className="settings-block-title" id="dev-mock-title">Mock provider</h3>
          <p className="settings-block-sub">Adds a connection for the deterministic mock at 127.0.0.1:43127 (<code>pnpm mock:provider</code>). Reuses it if it's already there.</p>
          <button type="button" className="secondary-button compact" disabled={mockBusy} onClick={() => void addMockProvider()}>
            <Icon name="plus" /> {mockBusy ? "Adding…" : "Add the mock provider"}
          </button>
          {mockNote && <p className="settings-block-sub">{mockNote}</p>}
        </section>

        <section className="settings-block" aria-labelledby="dev-reset-title">
          <h3 className="settings-block-title" id="dev-reset-title">Reset dev data</h3>
          <p className="settings-block-sub">Moves this dev app's data folder — settings, chats, connections — to the Trash and relaunches. Only the dev folder; the installed WackCode is never touched.</p>
          <button type="button" className="danger-button compact" disabled={busy} onClick={() => setConfirmingReset(true)}>
            <Icon name="trash" /> Reset dev data…
          </button>
        </section>
      </div>
      {confirmingReset && (
        <ConfirmDialog
          danger
          title="Reset dev data?"
          body="The dev app's whole data folder moves to the Trash and the app relaunches empty — settings, chats and connections included. The installed WackCode keeps its own data."
          confirmLabel="Reset & relaunch"
          onCancel={() => setConfirmingReset(false)}
          onConfirm={async () => {
            setBusy(true);
            // Never resolves: the app relaunches. A failure leaves the app up and reports.
            await api.developerResetData();
          }}
        />
      )}
    </div>
  );
}


