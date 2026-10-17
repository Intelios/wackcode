import { useEffect, useState } from "react";
import { LayoutGroup, motion, useReducedMotion } from "motion/react";
import type { ProviderRecord, SaveSearchProviderInput, SearchProviderKind, SearchProviderRecord, WebConfig, WebTestResult, WebUsageSummary } from "../../types";
import {
  SEARCH_PRESETS, draftFor, draftFromRecord, estimatedCost, formatCost, keyReady, keyState, monogram, monthLabel, plural, presetFor, usageFor, webPill
} from "../../web-presets";
import { Icon } from "../Icons";
import { SettingsHero, stagger } from "../SettingsHero";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { WebProviderEditor } from "./WebProviderEditor";
import { WebStage } from "./WebStage";

export interface WebActions {
  onSave: (input: SaveSearchProviderInput) => Promise<WebConfig>;
  onDelete: (providerId: string) => Promise<WebConfig>;
  onSetActive: (providerId: string | null) => Promise<WebConfig>;
  onSetConfig: (input: { enabled: boolean; routeFetch: boolean }) => Promise<WebConfig>;
  onTest: (input: { id?: string; draft?: SaveSearchProviderInput; query: string }) => Promise<WebTestResult>;
  onUsage: () => Promise<WebUsageSummary>;
  onResetUsage: (providerId?: string) => Promise<WebUsageSummary>;
  onOpenLink: (url: string) => void;
}

interface Props extends WebActions {
  config: WebConfig;
  providers: readonly ProviderRecord[];
  agentName: string;
  /** web_fetch's switch: it rides the Tools denylist, so it applies live. */
  webFetchEnabled: boolean;
  onToggleWebFetch: (enabled: boolean) => Promise<void>;
  /** Every config Rust returns, so `data.web` stays current. */
  onChanged: (config: WebConfig) => void;
}

type View = { kind: "overview" } | { kind: "provider"; id: string } | { kind: "new"; draft: SaveSearchProviderInput };

function Switch({ on, label, disabled, title, onToggle }: { on: boolean; label: string; disabled?: boolean; title?: string; onToggle: () => void }) {
  return (
    <button type="button" role="switch" aria-checked={on} aria-label={label} title={title} className={`toggle ${on ? "on" : ""}`} disabled={disabled} onClick={onToggle}>
      <span />
    </button>
  );
}

/**
 * Settings › Web. The web_search tool's providers (one active, the rest on standby), whether
 * web_fetch reads pages through the active one, web_fetch's own switch, and this month's usage
 * meter. Every request goes through Rust (`web.rs`), which holds the keys; this page only edits
 * settings and shows counts. Each provider opens in its own editor under a back link.
 */
export function WebSection({
  config, providers, agentName, webFetchEnabled, onToggleWebFetch, onChanged,
  onSave, onDelete, onSetActive, onSetConfig, onTest, onUsage, onResetUsage, onOpenLink
}: Props) {
  const reduce = useReducedMotion();
  const [chosenView, setView] = useState<View>({ kind: "overview" });
  // A provider deleted (or never saved) while its page was open lands back on the overview.
  const view: View = chosenView.kind === "provider" && !config.providers.some((entry) => entry.id === chosenView.id) ? { kind: "overview" } : chosenView;
  const [usage, setUsage] = useState<WebUsageSummary>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [confirmReset, setConfirmReset] = useState(false);

  useEffect(() => {
    if (view.kind !== "overview") return;
    let active = true;
    // Counts only change when the agent searches; reading them when the overview shows is enough.
    onUsage().then((summary) => { if (active) setUsage(summary); }).catch(() => undefined);
    return () => { active = false; };
  }, [onUsage, view.kind]);

  async function run(action: () => Promise<WebConfig | void>) {
    setBusy(true);
    setError(undefined);
    try {
      const next = await action();
      if (next) onChanged(next);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const active = config.providers.find((record) => record.id === config.activeProviderId);
  const pill = webPill(config, usage, providers);
  const canRoute = Boolean(active && presetFor(active.kind).supportsFetch);

  if (view.kind !== "overview") {
    const record = view.kind === "provider" ? config.providers.find((entry) => entry.id === view.id) : undefined;
    return (
      <WebProviderEditor
        key={view.kind === "provider" ? view.id : `new-${view.draft.kind}`}
        record={record}
        initialDraft={view.kind === "new" ? view.draft : draftFromRecord(record as SearchProviderRecord)}
        providers={providers}
        knownIds={config.providers.map((entry) => entry.id)}
        active={Boolean(record && record.id === config.activeProviderId)}
        onBack={() => setView({ kind: "overview" })}
        onChanged={(next, savedId) => {
          onChanged(next);
          if (savedId && view.kind === "new") setView({ kind: "provider", id: savedId });
        }}
        onSave={onSave}
        onDelete={onDelete}
        onTest={onTest}
        onOpenLink={onOpenLink}
      />
    );
  }

  function startNew(kind: SearchProviderKind) {
    setView({ kind: "new", draft: draftFor(kind, providers, config.providers.map((entry) => entry.name)) });
  }

  const totals = config.providers.map((record) => ({ record, counters: usageFor(usage, record.id) }));
  const busiest = Math.max(1, ...totals.map(({ counters }) => counters.searches + counters.fetches));
  const anyUsage = totals.some(({ counters }) => counters.searches + counters.fetches > 0);
  const totalCost = totals.reduce<number | undefined>((sum, { record, counters }) => {
    const cost = estimatedCost(record, counters);
    return cost === undefined ? sum : (sum ?? 0) + cost;
  }, undefined);

  return (
    <div className="settings-scroll web-settings">
      <div className="settings-page">
        <SettingsHero
          label="Web overview"
          stage={<WebStage live={pill.live} />}
          live={pill.live}
          attention={pill.attention}
          pill={pill.text}
          title={`Let ${agentName} search the web`}
          action={(
            <span className="settings-hero-switch">
              <span aria-hidden="true">Web search</span>
              <Switch
                on={config.enabled}
                label="Web search"
                disabled={busy || config.providers.length === 0}
                title={config.providers.length === 0 ? "Add a search provider first" : undefined}
                onToggle={() => void run(() => onSetConfig({ enabled: !config.enabled, routeFetch: config.routeFetch }))}
              />
            </span>
          )}
        >
          <p>
            A search tool through a provider you pick. Searches usually cost a little per request, so it&rsquo;s off until you switch it on.
            Your queries go to that provider; keys stay on this Mac.
          </p>
        </SettingsHero>
        {error && <div className="error-banner" role="alert">{error}</div>}

        {config.providers.length > 0 && (
          <section className="settings-block" style={stagger(1)} aria-labelledby="web-providers-title">
            <h3 className="settings-block-title" id="web-providers-title">Search providers</h3>
            <p className="settings-block-sub">Every search goes to the active one; the others wait on standby, ready to swap in.</p>
            <LayoutGroup id="web-providers">
              <ul className="web-provider-list" aria-label="Search providers">
                {config.providers.map((record) => {
                  const isActive = record.id === config.activeProviderId;
                  const state = keyState(record, providers);
                  const counters = usageFor(usage, record.id);
                  const preset = presetFor(record.kind);
                  return (
                    <li key={record.id} className={`web-provider ${isActive ? "active" : ""} ${state.tone === "off" ? "needs-key" : ""}`}>
                      {isActive && (
                        <motion.span
                          layoutId="web-provider-active"
                          className="web-provider-glow"
                          aria-hidden="true"
                          transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 420, damping: 34 }}
                        />
                      )}
                      <button type="button" className="web-provider-main" onClick={() => setView({ kind: "provider", id: record.id })}>
                        <span className={`web-mark kind-${record.kind}`} aria-hidden="true">{monogram(record.name)}</span>
                        <span className="web-provider-text">
                          <strong>{record.name}</strong>
                          <span>
                            {preset.label}{record.baseUrl ? ` · ${hostOf(record.baseUrl)}` : ""}
                            {preset.supportsFetch && <em className="web-fetch-chip">reads pages</em>}
                          </span>
                        </span>
                        <span className="web-provider-meta">
                          <span className={`package-state ${state.tone}`}>{state.label}</span>
                          <span className="web-provider-count">{counters.searches ? `${plural(counters.searches, "search")} this month` : "No searches yet"}</span>
                        </span>
                        <Icon name="chevron" />
                      </button>
                      {isActive ? (
                        <span className="web-active-chip"><Icon name="check" /> Active</span>
                      ) : (
                        <button
                          type="button"
                          className="secondary-button compact web-make-active"
                          disabled={busy}
                          aria-label={`Make ${record.name} the active provider`}
                          onClick={() => void run(() => onSetActive(record.id))}
                        >
                          Make active
                        </button>
                      )}
                    </li>
                  );
                })}
              </ul>
            </LayoutGroup>
            {config.enabled && !active && <p className="web-note attention">No provider is active, so the agent can&rsquo;t search. Pick one above.</p>}
            {config.enabled && active && !keyReady(active, providers) && (
              <p className="web-note attention">{active.name} has no usable key, so the agent can&rsquo;t search. Open it to add one.</p>
            )}
          </section>
        )}

        <section className="settings-block" style={stagger(config.providers.length ? 2 : 1)} aria-labelledby="web-add-title">
          <h3 className="settings-block-title" id="web-add-title">{config.providers.length ? "Add a provider" : "Pick a search provider"}</h3>
          <p className="settings-block-sub">
            {config.providers.length
              ? "Another API, a self-hosted instance, or anything that answers in JSON."
              : `${agentName} can search once you add one. Each needs its own account, except SearXNG, which you run yourself.`}
          </p>
          <div className="web-presets">
            {SEARCH_PRESETS.map((preset, index) => (
              <button
                key={preset.kind}
                type="button"
                className="provider-method web-preset"
                style={{ "--k": index } as React.CSSProperties}
                onClick={() => startNew(preset.kind)}
              >
                <span className={`web-mark kind-${preset.kind}`} aria-hidden="true">{preset.kind === "custom" ? <Icon name="code" /> : monogram(preset.label)}</span>
                <span className="provider-method-text">
                  <strong>{preset.label}</strong>
                  <span>{preset.tagline}</span>
                </span>
                <Icon name="chevron" />
              </button>
            ))}
          </div>
        </section>

        <section className="settings-block" style={stagger(config.providers.length ? 3 : 2)} aria-labelledby="web-reading-title">
          <h3 className="settings-block-title" id="web-reading-title">Reading pages</h3>
          <p className="settings-block-sub">How {agentName} opens a page once it has a link.</p>
          <ul className="web-switches">
            <li>
              <span className="web-switch-icon" aria-hidden="true"><Icon name="globe" /></span>
              <div>
                <strong>Web Fetch</strong>
                <p>Reads a public page by URL from this Mac and turns it into Markdown. It never reaches localhost or your local network.</p>
              </div>
              <Switch on={webFetchEnabled} label="Web Fetch" disabled={busy} onToggle={() => void run(() => onToggleWebFetch(!webFetchEnabled))} />
            </li>
            <li className={!canRoute ? "unavailable" : ""}>
              <span className="web-switch-icon" aria-hidden="true"><Icon name="external" /></span>
              <div>
                <strong>Read pages through {active && canRoute ? active.name : "the provider"}</strong>
                <p>
                  {!active ? "Pick an active provider first."
                    : !canRoute ? `${active.name} can't read pages; only DeepSeek and Tavily can.`
                    : `Every page goes to ${active.name}'s reader instead, which copes better with script-heavy sites and bot walls. Each page URL is sent to ${active.name}, and each read is billed.`}
                </p>
              </div>
              <Switch
                on={config.routeFetch && canRoute}
                label={`Read pages through ${active?.name ?? "the provider"}`}
                disabled={busy || !canRoute || !webFetchEnabled || !config.enabled}
                title={!webFetchEnabled ? "Switch on Web Fetch first" : !config.enabled ? "Switch on web search first" : undefined}
                onToggle={() => void run(() => onSetConfig({ enabled: config.enabled, routeFetch: !config.routeFetch }))}
              />
            </li>
          </ul>
        </section>

        {config.providers.length > 0 && (
          <section className="settings-block web-usage" style={stagger(4)} aria-labelledby="web-usage-title">
            <div className="models-block-head">
              <div>
                <h3 className="settings-block-title" id="web-usage-title">{usage ? monthLabel(usage.month) : "This month"}, on this Mac</h3>
                <p className="settings-block-sub">
                  Counted here as {agentName} searches. The estimate uses your prices and counts only answers with results, which is how DeepSeek bills.
                </p>
              </div>
              {totalCost !== undefined && <span className="web-usage-total" aria-label="Estimated cost this month">≈ {formatCost(totalCost)}</span>}
            </div>
            {anyUsage ? (
              <ul className="web-usage-list">
                {totals.map(({ record, counters }) => {
                  const cost = estimatedCost(record, counters);
                  const share = (counters.searches + counters.fetches) / busiest;
                  return (
                    <li key={record.id}>
                      <span className="web-usage-name">{record.name}</span>
                      <span className="web-usage-track" aria-hidden="true">
                        <motion.span
                          className="web-usage-bar"
                          initial={reduce ? false : { scaleX: 0 }}
                          animate={{ scaleX: share }}
                          transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 160, damping: 24 }}
                        />
                      </span>
                      <span className="web-usage-figures">
                        {plural(counters.searches, "search")}
                        {counters.fetches > 0 && <> · {plural(counters.fetches, "page")}</>}
                        {counters.failed > 0 && <em> · {counters.failed} failed</em>}
                        {cost !== undefined && <strong> · ≈ {formatCost(cost)}</strong>}
                      </span>
                    </li>
                  );
                })}
              </ul>
            ) : (
              <p className="models-empty">Nothing yet this month.</p>
            )}
            {anyUsage && (
              <div className="provider-remove">
                <span>Counts reset on the first of each month. These are estimates; your provider&rsquo;s dashboard has the real bill.</span>
                <button type="button" className="secondary-button compact" disabled={busy} onClick={() => setConfirmReset(true)}>Reset counts</button>
              </div>
            )}
          </section>
        )}
      </div>

      {confirmReset && (
        <ConfirmDialog
          title="Reset the usage counts?"
          body="This clears every provider's counts on this Mac. It changes nothing at the providers."
          confirmLabel="Reset"
          danger
          onConfirm={async () => { setUsage(await onResetUsage()); setConfirmReset(false); }}
          onCancel={() => setConfirmReset(false)}
        />
      )}
    </div>
  );
}

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}
