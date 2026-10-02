import { useEffect, useState } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { api } from "../api";
import { aboutSummary, buildLabel } from "../about-utils";
import type { AppInfo } from "../types";
import { DuckMark } from "./DuckMark";
import { Icon } from "./Icons";
import { SettingsHero, stagger } from "./SettingsHero";
import { CopyButton, CopyText } from "./ui/CopyButton";

/** Where "View on GitHub" goes; opened through `revealPath`, like every external link. */
const GITHUB_URL = "https://github.com/Intelios/wackcode";

/**
 * The hero's little stage: the duck stands on its tile presenting the spec card beside it,
 * whose lines fill in one after another. Pure decoration; the pill says the version in words.
 * The loop lives in styles.css, which stills it under reduced motion.
 */
function AboutStage({ live }: { live: boolean }) {
  const lines = [30, 22, 26];
  return (
    <svg className={`settings-stage about-stage ${live ? "live" : ""}`} viewBox="0 0 160 110" aria-hidden="true">
      <rect className="about-stage-card" x="58" y="12" width="56" height="86" rx="10" />
      <circle className="about-stage-dot" cx="70" cy="27" r="1.8" />
      {lines.map((width, index) => (
        <rect
          key={index}
          className="about-stage-line"
          style={{ "--k": index } as React.CSSProperties}
          x="78"
          y={25 + index * 13}
          width={width}
          height="4"
          rx="2"
        />
      ))}
      <rect className="about-stage-tile" x="6" y="60" width="44" height="44" rx="12" />
      <DuckMark x="12" y="64" width="32" height="32" />
    </svg>
  );
}

/** One line of the Versions card: a plain label, and the fact itself in the mono voice of paths. */
function FactRow({ label, value }: { label: string; value: string }) {
  return (
    <li className="about-row">
      <span className="about-row-label">{label}</span>
      <code className="about-row-value" title={value}>{value}</code>
    </li>
  );
}

/**
 * Settings › About: this copy of the app — its version, the agent and runtime bundled with it,
 * the Mac it's on, and how much library it's keeping. Talks to Rust directly, like
 * IntegrationsSection; `api`'s functions are stable. Polls so the live-worker count stays
 * current while the page is open.
 */
export function AboutSection() {
  const [info, setInfo] = useState<AppInfo>();
  const [error, setError] = useState<string>();
  const [revealError, setRevealError] = useState<string>();
  useEffect(() => {
    let active = true;
    const refresh = () => {
      void api.appInfo().then((value) => { if (active) { setInfo(value); setError(undefined); } })
        .catch((reason) => { if (active) setError(String(reason)); });
    };
    refresh();
    const timer = setInterval(refresh, 5000);
    return () => { active = false; clearInterval(timer); };
  }, []);

  function reveal(path: string) {
    setRevealError(undefined);
    // `select` reveals the binary where it sits instead of launching a second WackCode.
    void api.revealPath(path, true).catch((reason) => setRevealError(String(reason)));
  }
  function openGitHub() {
    setRevealError(undefined);
    void api.revealPath(GITHUB_URL).catch((reason) => setRevealError(String(reason)));
  }

  const stats = info ? [
    { value: info.projectCount, label: "Projects" },
    { value: info.chatCount, label: "Chats" },
    { value: info.archivedCount, label: "Archived" },
    { value: info.activeWorkers, label: "Live workers" }
  ] : [];

  return (
    <div className="settings-scroll about-settings">
      <div className="settings-page">
        {/* CopyText doesn't reach Settings (App provides it around the transcript), so the page
            brings the clipboard writer it needs for its one copy button. */}
        <CopyText.Provider value={writeText}>
          <SettingsHero
            label="About overview"
            stage={<AboutStage live={Boolean(info)} />}
            live={Boolean(info)}
            attention={Boolean(error)}
            pill={!info ? (error ? "Unavailable" : "Reading…") : info.appVersion}
            title="WackCode"
            action={info ? <CopyButton text={aboutSummary(info)} label="Copy details" /> : undefined}
          >
            <p>This copy of WackCode — its version, the agent and runtime bundled with it, and the Mac it's running on.</p>
            <p>Copy details puts the whole picture on the clipboard, ready to paste into a report.</p>
          </SettingsHero>
          {error && <p className="package-error" role="alert">{error}</p>}
          {revealError && <p className="package-error" role="alert">{revealError}</p>}
          {info && (
            <>
              <section className="settings-block" style={stagger(1)} aria-labelledby="about-versions-title">
                <h3 className="settings-block-title" id="about-versions-title">What this copy ships with</h3>
                <p className="settings-block-sub">The engine rides along inside the app, so nothing here is picked up from your Mac.</p>
                <ul className="about-rows">
                  <FactRow label="WackCode" value={info.appVersion} />
                  <FactRow label="Pi agent (bundled)" value={info.piVersion} />
                  <FactRow label="Node runtime (bundled)" value={info.nodeVersion} />
                  <FactRow label="macOS" value={info.osVersion} />
                  <FactRow label="Chip" value={info.chip} />
                </ul>
              </section>

              <section className="settings-block" style={stagger(2)} aria-labelledby="about-build-title">
                <h3 className="settings-block-title" id="about-build-title">Build</h3>
                <p className="settings-block-sub">
                  <span className={`about-build-badge ${info.build === "development" ? "dev" : ""}`}>
                    <i aria-hidden="true" />{buildLabel(info.build)}
                  </span>
                </p>
                <div className="about-path">
                  <div>
                    <span>Running from</span>
                    <code title={info.appPath}>{info.appPath}</code>
                  </div>
                  <button type="button" className="secondary-button compact" onClick={() => reveal(info.appPath)}>
                    <Icon name="folder" /> Reveal
                  </button>
                </div>
              </section>

              <section className="settings-block" style={stagger(3)} aria-labelledby="about-stats-title">
                <h3 className="settings-block-title" id="about-stats-title">On this Mac</h3>
                <p className="settings-block-sub">Everything WackCode keeps lives in its application-data folder on this Mac.</p>
                <ul className="about-stats">
                  {stats.map((stat) => (
                    <li key={stat.label}>
                      <strong>{stat.value}</strong>
                      <span>{stat.label}</span>
                    </li>
                  ))}
                </ul>
              </section>

              <section className="settings-block" style={stagger(4)} aria-labelledby="about-project-title">
                <h3 className="settings-block-title" id="about-project-title">The project</h3>
                <div className="about-path">
                  <div>
                    <span>Source, issues and releases</span>
                    <code>{GITHUB_URL.replace("https://", "")}</code>
                  </div>
                  <button type="button" className="secondary-button compact" onClick={openGitHub}>
                    <Icon name="external" /> View on GitHub
                  </button>
                </div>
              </section>
            </>
          )}
        </CopyText.Provider>
      </div>
    </div>
  );
}