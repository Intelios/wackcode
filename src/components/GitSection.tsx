import { useState } from "react";
import { WACKCODE_COAUTHOR_EMAIL } from "../promptDefaults";
import type { GitConfig } from "../types";
import { DuckMark } from "./DuckMark";
import { Icon, type IconName } from "./Icons";
import { SettingsHero, stagger } from "./SettingsHero";

const NOTES: { icon: IconName; title: string; detail: string }[] = [
  { icon: "commit", title: "Only the agent's commits", detail: "Commits you make from the Changes panel stay yours alone. Chat mode never commits." },
  { icon: "file", title: "Projects can opt out", detail: "An AGENTS.md or CLAUDE.md that asks for no co-author line wins for that project." },
  { icon: "lock", title: "No access to your account", detail: "WackCode Agent is a GitHub App with no permissions that never signs in. The line is plain text." }
];

/**
 * The hero's little stage: a commit card settles onto the branch's newest commit, and the duck
 * slides in beside you as co-author while its trailer line writes itself. While attribution is
 * off the card is yours alone. The loop lives in styles.css, which stills it under reduced motion.
 */
function GitStage({ live }: { live: boolean }) {
  return (
    <svg className={`settings-stage git-stage ${live ? "live" : ""}`} viewBox="0 0 160 110" aria-hidden="true">
      <path className="git-stage-branch" d="M10 94H150" />
      {[24, 52, 80].map((cx) => <circle key={cx} className="git-stage-commit" cx={cx} cy="94" r="4" />)}
      <path className="git-stage-stem" d="M118 76V88" />
      <circle className="git-stage-commit head" cx="118" cy="94" r="5.5" />
      <g className="git-stage-card">
        <rect className="git-stage-sheet" x="30" y="10" width="112" height="66" rx="9" />
        <rect className="git-stage-title" x="42" y="22" width="56" height="5" rx="2.5" />
        <rect className="git-stage-line" x="42" y="32" width="38" height="4" rx="2" />
        <g className="git-stage-you">
          <circle className="git-stage-avatar" cx="52" cy="56" r="10" />
          <circle className="git-stage-person" cx="52" cy="53" r="3.4" />
          <path className="git-stage-person" d="M45.6 62.4a6.4 6.4 0 0 1 12.8 0z" />
        </g>
        <g className="git-stage-duck">
          <circle className="git-stage-avatar duck" cx="67" cy="56" r="10" />
          <DuckMark className="git-stage-duck-mark" x="60" y="49" width="14" height="14" />
        </g>
        <rect className="git-stage-trailer" x="84" y="54" width="46" height="4" rx="2" />
      </g>
    </svg>
  );
}

interface Props {
  config: GitConfig;
  /** The agent's name in the app's own copy (Settings › Appearance). */
  agentName: string;
  onChange: (config: GitConfig) => Promise<void>;
}

/**
 * Settings › Git: whether coding chats end their commits with a Co-authored-by trailer for the
 * chat's model. The trailer itself is built in the worker's coding guide; "(WackCode)" names the
 * GitHub App, so it never follows the agent's custom name.
 */
export function GitSection({ config, agentName, onChange }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const on = config.commitAttribution;

  async function toggle() {
    setBusy(true);
    setError(undefined);
    try {
      await onChange({ ...config, commitAttribution: !on });
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="settings-scroll git-settings">
      <div className="settings-page">
        <SettingsHero
          label="Git overview"
          stage={<GitStage live={on} />}
          live={on}
          pill={on ? "Co-authoring" : "Off"}
          title={`${agentName} as co-author`}
          action={(
            <span className="settings-hero-switch">
              <span aria-hidden="true">Credit commits</span>
              <button
                type="button"
                role="switch"
                aria-checked={on}
                aria-label="Credit commits"
                className={`toggle ${on ? "on" : ""}`}
                disabled={busy}
                onClick={() => void toggle()}
              >
                <span />
              </button>
            </span>
          )}
        >
          <p>
            When {agentName} commits for you, it ends the message with a Co-authored-by line naming the model it ran on. GitHub
            then shows WackCode Agent beside you on the commit, so you can see how much it helped.
          </p>
          <p>Switching applies from a chat&rsquo;s next message.</p>
        </SettingsHero>
        {error && <div className="error-banner" role="alert">{error}</div>}

        <section className={`settings-block git-trailer-block ${on ? "" : "off"}`} style={stagger(1)} aria-labelledby="git-trailer-title">
          <h3 className="settings-block-title" id="git-trailer-title">The line it adds</h3>
          <p className="settings-block-sub">
            {on
              ? "The last line of each commit message the agent writes, after a blank line. GitHub matches the address, so the name can say anything."
              : "Switched off: the agent's commits carry no co-author line."}
          </p>
          <pre className="git-trailer-preview">
            <span className="git-trailer-subject">Fix the flaky upload test</span>
            {"\n\n"}
            <span className="git-trailer-line">
              Co-authored-by: <span className="git-trailer-model">the chat&rsquo;s model</span> (WackCode) &lt;{WACKCODE_COAUTHOR_EMAIL}&gt;
            </span>
          </pre>
        </section>

        <section className="settings-block" style={stagger(2)} aria-labelledby="git-notes-title">
          <h3 className="settings-block-title" id="git-notes-title">Good to know</h3>
          <ul className="git-notes">
            {NOTES.map((note) => (
              <li key={note.title}>
                <span className="git-note-mark" aria-hidden="true"><Icon name={note.icon} /></span>
                <strong>{note.title}</strong>
                <span>{note.detail}</span>
              </li>
            ))}
          </ul>
        </section>
      </div>
    </div>
  );
}
