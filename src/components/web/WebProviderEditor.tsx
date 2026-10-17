import { useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { CustomSearchSpec, ProviderRecord, SaveSearchProviderInput, SearchProviderRecord, WebConfig, WebTestResult } from "../../types";
import {
  draftComplete, draftFromRecord, keyState, linkableConnections, monogram, needsKey, presetFor
} from "../../web-presets";
import { Icon } from "../Icons";
import { stagger } from "../SettingsHero";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { Select } from "../ui/Select";
import { WebResults } from "./WebResults";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];
const DEFAULT_TEST_QUERY = "latest Rust release";

export interface WebEditorActions {
  onSave: (input: SaveSearchProviderInput) => Promise<WebConfig>;
  onDelete: (providerId: string) => Promise<WebConfig>;
  onTest: (input: { id?: string; draft?: SaveSearchProviderInput; query: string }) => Promise<WebTestResult>;
  /** Opens a Test search result outside the app. */
  onOpenLink: (url: string) => void;
}

interface Props extends WebEditorActions {
  /** Undefined while the provider is new. */
  record?: SearchProviderRecord;
  /** The new provider's starting form. */
  initialDraft: SaveSearchProviderInput;
  providers: readonly ProviderRecord[];
  /** The ids saved before this page opened, to find a new provider's id after its first save. */
  knownIds: readonly string[];
  active: boolean;
  onBack: () => void;
  /** Rust's config after a save or delete, plus the id saved (to keep its page open). */
  onChanged: (config: WebConfig, savedId?: string) => void;
}

/** JSON with object keys sorted, so a form edited back to how it was compares equal again. */
function stableKey(value: unknown): string {
  return JSON.stringify(value, (_key, entry: unknown) =>
    entry && typeof entry === "object" && !Array.isArray(entry)
      ? Object.fromEntries(Object.entries(entry).sort(([left], [right]) => left.localeCompare(right)))
      : entry);
}

type TestState = { status: "idle" } | { status: "running" } | { status: "done"; result: WebTestResult };

/**
 * One search provider's page in Settings › Web, under a back link: the key (its own, or one a
 * DeepSeek connection lends), the URL for SearXNG and Custom, Custom's request and response
 * mapping, prices for the usage estimate, and a Test search that runs the form as it stands.
 * Saved from a bar that appears only while there is something to save.
 */
export function WebProviderEditor({ record, initialDraft, providers, knownIds, active, onBack, onChanged, onSave, onDelete, onTest, onOpenLink }: Props) {
  const reduce = useReducedMotion();
  const seed = record ? draftFromRecord(record) : initialDraft;
  const [draft, setDraft] = useState<SaveSearchProviderInput>(seed);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [query, setQuery] = useState(DEFAULT_TEST_QUERY);
  const [test, setTest] = useState<TestState>({ status: "idle" });

  const preset = presetFor(draft.kind);
  const isNew = !record;
  const dirty = isNew || stableKey(draft) !== stableKey(seed);
  const complete = draftComplete(draft, record);
  const keyed = needsKey(draft.kind, draft.custom);
  const links = linkableConnections(draft.kind, providers);
  // A link to a connection that can no longer lend stays selectable, so the user sees it.
  const linkedId = draft.keySource.type === "connection" ? draft.keySource.providerId : undefined;
  const linkOptions = links.map((provider) => ({ value: provider.id, label: provider.name }));
  if (linkedId && !links.some((provider) => provider.id === linkedId)) {
    linkOptions.push({ value: linkedId, label: providers.find((provider) => provider.id === linkedId)?.name ?? "Removed connection" });
  }
  const canLink = Boolean(preset.linkOrigin) && linkOptions.length > 0;
  const state = record ? keyState(record, providers) : undefined;

  function edit(patch: Partial<SaveSearchProviderInput>) {
    setDraft((current) => ({ ...current, ...patch }));
    setNotice(undefined);
    setError(undefined);
  }

  function editCustom(patch: Partial<CustomSearchSpec>) {
    setDraft((current) => {
      const custom = { ...(current.custom as CustomSearchSpec), ...patch };
      // An auth header means a key: start asking for one; without it there's nothing to send.
      const keySource = needsKey(current.kind, custom)
        ? current.keySource.type === "none" ? { type: "own" as const } : current.keySource
        : { type: "none" as const };
      return { ...current, custom, keySource };
    });
    setNotice(undefined);
    setError(undefined);
  }

  function priceInput(value: string): number | null {
    const number = Number(value);
    return value.trim() === "" || !Number.isFinite(number) ? null : number;
  }

  async function save() {
    setBusy(true);
    setError(undefined);
    try {
      const input = { ...draft, apiKey: draft.apiKey?.trim() || undefined };
      const config = await onSave(input);
      const saved = record?.id ?? config.providers.find((entry) => !knownIds.includes(entry.id))?.id;
      setDraft((current) => ({ ...current, apiKey: "" }));
      setNotice(draft.keySource.type === "own" && draft.apiKey?.trim() ? "Saved. The key is stored on this Mac." : "Saved.");
      onChanged(config, saved);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function runTest() {
    setTest({ status: "running" });
    try {
      const result = await onTest(dirty ? { id: record?.id, draft: { ...draft, id: record?.id }, query } : { id: record?.id, query });
      setTest({ status: "done", result });
    } catch (reason) {
      setTest({ status: "done", result: { ok: false, error: String(reason), results: [], elapsedMs: 0 } });
    }
  }

  const showBar = dirty || Boolean(error) || Boolean(notice);

  return (
    <>
      <div className="settings-scroll web-settings">
        <div className="settings-page">
          <button type="button" className="ghost-button skill-back provider-back" onClick={onBack}>
            <Icon name="back" /> All search providers
          </button>

          <section className="settings-block provider-head web-head" style={stagger(0)} aria-label={`${draft.name.trim() || preset.label} search provider`}>
            <span className={`web-mark large kind-${draft.kind}`} aria-hidden="true">{monogram(draft.name || preset.label)}</span>
            <div className="provider-head-text">
              <h3 className="settings-block-title">{draft.name.trim() || preset.label}</h3>
              <span className="provider-head-meta">{preset.label}{preset.supportsFetch ? " · search and page reading" : " · search"}</span>
              <span className="provider-head-status">
                {state ? <span className={`package-state ${state.tone}`}>{state.label}</span> : <span className="package-state off">Not saved yet</span>}
                {active && <span className="web-active-chip"><Icon name="check" /> Active</span>}
              </span>
            </div>
          </section>

          <section className="settings-block" style={stagger(1)} aria-labelledby="web-provider-fields">
            <h3 className="settings-block-title" id="web-provider-fields">Provider</h3>
            <p className="settings-block-sub">{preset.tagline}</p>
            <div className="form-grid connection-grid">
              <label>
                <span>Name</span>
                <input value={draft.name} onChange={(event) => edit({ name: event.target.value })} placeholder={preset.label} />
              </label>
              {preset.needsUrl && (
                <label>
                  <span>{draft.kind === "searxng" ? "Instance URL" : "Search URL"} {draft.kind === "searxng" && <small>/search is added for you</small>}</span>
                  <input
                    value={draft.baseUrl ?? ""}
                    onChange={(event) => edit({ baseUrl: event.target.value })}
                    placeholder={preset.urlPlaceholder}
                    spellCheck={false}
                  />
                </label>
              )}
              {keyed && (
                <div className="web-key-field wide-field">
                  <div className="web-key-head">
                    <span className="web-field-label">API key</span>
                    {canLink && (
                      <div className="segmented" role="radiogroup" aria-label="Where the key comes from">
                        <button
                          type="button" role="radio" aria-checked={draft.keySource.type === "connection"}
                          className={`segmented-option ${draft.keySource.type === "connection" ? "selected" : ""}`}
                          onClick={() => edit({ keySource: { type: "connection", providerId: linkedId ?? linkOptions[0].value } })}
                        >
                          From a connection
                        </button>
                        <button
                          type="button" role="radio" aria-checked={draft.keySource.type === "own"}
                          className={`segmented-option ${draft.keySource.type === "own" ? "selected" : ""}`}
                          onClick={() => edit({ keySource: { type: "own" } })}
                        >
                          Its own key
                        </button>
                      </div>
                    )}
                  </div>
                  {draft.keySource.type === "connection" ? (
                    <>
                      <Select
                        className="settings-select"
                        matchWidth
                        value={draft.keySource.providerId}
                        onChange={(providerId) => edit({ keySource: { type: "connection", providerId } })}
                        options={linkOptions}
                        aria-label="Connection to borrow the key from"
                      />
                      <small className="web-field-hint">
                        Read from Settings › Providers each time, never copied. Only connections on {preset.linkOrigin} can lend their key.
                      </small>
                    </>
                  ) : (
                    <>
                      <div className="input-with-icon">
                        <Icon name="key" />
                        <input
                          type="password"
                          autoComplete="off"
                          aria-label="API key"
                          value={draft.apiKey ?? ""}
                          onChange={(event) => edit({ apiKey: event.target.value })}
                          placeholder={record?.keySource.type === "own" && record.hasKey ? "••••••••••••••••" : "Paste the API key"}
                        />
                      </div>
                      <small className="web-field-hint">
                        {record?.keySource.type === "own" && record.hasKey ? "Leave blank to keep the saved key. " : ""}
                        {preset.keyHint ?? "Sent only to this provider, over https."} Stored on this Mac.
                      </small>
                    </>
                  )}
                </div>
              )}
            </div>
          </section>

          {draft.kind === "custom" && draft.custom && (
            <CustomFields spec={draft.custom} onChange={editCustom} />
          )}

          <section className="settings-block" style={stagger(3)} aria-labelledby="web-provider-prices">
            <h3 className="settings-block-title" id="web-provider-prices">Prices</h3>
            <p className="settings-block-sub">
              What this provider charges per 1,000 successful requests, for the estimate on the Web page. Leave blank if it&rsquo;s free or you&rsquo;d rather not track it.
            </p>
            <div className="form-grid web-price-grid">
              <label>
                <span>Searches <small>US$ per 1,000</small></span>
                <input
                  type="number" min={0} step="0.01" inputMode="decimal"
                  value={draft.priceSearchPer1k ?? ""}
                  onChange={(event) => edit({ priceSearchPer1k: priceInput(event.target.value) })}
                  placeholder="0.00"
                />
              </label>
              {preset.supportsFetch && (
                <label>
                  <span>Page reads <small>US$ per 1,000</small></span>
                  <input
                    type="number" min={0} step="0.01" inputMode="decimal"
                    value={draft.priceFetchPer1k ?? ""}
                    onChange={(event) => edit({ priceFetchPer1k: priceInput(event.target.value) })}
                    placeholder="0.00"
                  />
                </label>
              )}
            </div>
          </section>

          <section className="settings-block web-test" style={stagger(4)} aria-labelledby="web-provider-test">
            <h3 className="settings-block-title" id="web-provider-test">Test search</h3>
            <p className="settings-block-sub">
              Runs one real search for three results{dirty ? ", with the form as it is now" : ""}. A saved provider&rsquo;s test counts in its usage.
            </p>
            <form className="web-test-row" onSubmit={(event) => { event.preventDefault(); void runTest(); }}>
              <div className="input-with-icon">
                <Icon name="search" />
                <input value={query} onChange={(event) => setQuery(event.target.value)} aria-label="Test query" placeholder={DEFAULT_TEST_QUERY} />
              </div>
              <button type="submit" className="secondary-button" disabled={!complete || !query.trim() || test.status === "running"}>
                {test.status === "running" ? <><span className="web-test-spinner" aria-hidden="true" /> Searching…</> : "Test search"}
              </button>
            </form>
            <AnimatePresence initial={false} mode="wait">
              {test.status === "done" && (
                <motion.div
                  key={test.result.ok ? `ok-${test.result.elapsedMs}` : "failed"}
                  className="web-test-result"
                  initial={reduce ? { opacity: 0 } : { opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.2, ease: EASE }}
                >
                  {test.result.ok ? (
                    <>
                      <p className="web-test-status success-text" role="status">
                        <Icon name="check" />
                        {test.result.results.length
                          ? `${test.result.results.length} result${test.result.results.length === 1 ? "" : "s"} in ${(test.result.elapsedMs / 1000).toFixed(1)} s`
                          : "It answered, but found nothing. Try another query."}
                      </p>
                      <WebResults
                        label="Test search results"
                        results={test.result.results.map((hit) => ({ title: hit.title, url: hit.url, host: hostOf(hit.url), snippet: hit.snippet, date: hit.date }))}
                        onOpen={(url) => onOpenLink(url)}
                      />
                    </>
                  ) : (
                    <p className="web-test-status error-text" role="alert"><Icon name="alert" /> {test.result.error ?? "The search failed."}</p>
                  )}
                </motion.div>
              )}
            </AnimatePresence>
          </section>

          {record && (
            <section className="settings-block" style={stagger(5)} aria-label="Remove provider">
              <div className="provider-remove web-remove">
                <span>Deleting removes the provider{record.keySource.type === "own" ? ", its saved key" : ""} and its usage counts.</span>
                <button type="button" className="danger-button compact" disabled={busy} onClick={() => setConfirmDelete(true)}>Delete provider</button>
              </div>
            </section>
          )}
        </div>
      </div>

      <AnimatePresence>
        {showBar && (
          <motion.footer
            key="bar"
            className={`settings-footer provider-savebar ${error ? "failed" : dirty ? "" : "saved"}`}
            initial={reduce ? { opacity: 0 } : { opacity: 0, y: 18 }}
            animate={{ opacity: 1, y: 0 }}
            exit={reduce ? { opacity: 0 } : { opacity: 0, y: 18 }}
            transition={{ duration: 0.22, ease: EASE }}
          >
            <div className="form-status">
              {error ? <span className="error-text">{error}</span>
                : dirty ? <span className="provider-savebar-note"><i aria-hidden="true" />{isNew ? "Not saved yet" : "Unsaved changes"}</span>
                : notice ? <span className="success-text" role="status">{notice}</span> : null}
            </div>
            {dirty && (
              <>
                <button type="button" className="secondary-button" disabled={busy} onClick={isNew ? onBack : () => { setDraft(seed); setError(undefined); }}>
                  {isNew ? "Cancel" : "Discard"}
                </button>
                <button type="button" className="primary-button" disabled={busy || !complete} onClick={() => void save()}>
                  {busy ? "Saving…" : isNew ? "Add provider" : "Save provider"}
                </button>
              </>
            )}
          </motion.footer>
        )}
      </AnimatePresence>

      {confirmDelete && record && (
        <ConfirmDialog
          title={`Delete “${record.name}”?`}
          body={`${active ? "It's the active provider, so web search stops until you pick another. " : ""}This removes the provider${record.keySource.type === "own" ? ", its saved key" : ""} and its usage counts.`}
          confirmLabel="Delete"
          danger
          onConfirm={async () => { onChanged(await onDelete(record.id)); onBack(); }}
          onCancel={() => setConfirmDelete(false)}
        />
      )}
    </>
  );
}

function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return url;
  }
}

/** Custom's request and response mapping. */
function CustomFields({ spec, onChange }: { spec: CustomSearchSpec; onChange: (patch: Partial<CustomSearchSpec>) => void }) {
  const text = (key: keyof CustomSearchSpec) => (spec[key] as string | null | undefined) ?? "";
  return (
    <section className="settings-block web-custom" style={stagger(2)} aria-labelledby="web-custom-title">
      <h3 className="settings-block-title" id="web-custom-title">Request and response</h3>
      <p className="settings-block-sub">
        How to ask this API and where its answer keeps the results. Paths are dotted, with numbers for list items: <code>data.results</code>, <code>items.0.link</code>.
      </p>
      <div className="web-custom-method">
        <span className="web-field-label">Method</span>
        <div className="segmented" role="radiogroup" aria-label="Request method">
          {(["GET", "POST"] as const).map((method) => (
            <button
              key={method} type="button" role="radio" aria-checked={spec.method === method}
              className={`segmented-option ${spec.method === method ? "selected" : ""}`}
              onClick={() => onChange({ method })}
            >
              {method}
            </button>
          ))}
        </div>
      </div>
      <div className="form-grid connection-grid">
        {spec.method === "GET" ? (
          <>
            <label><span>Query parameter</span><input value={text("queryParam")} onChange={(event) => onChange({ queryParam: event.target.value })} placeholder="q" spellCheck={false} /></label>
            <label><span>Count parameter <small>optional</small></span><input value={text("countParam")} onChange={(event) => onChange({ countParam: event.target.value })} placeholder="count" spellCheck={false} /></label>
          </>
        ) : (
          <label className="wide-field">
            <span>JSON body <small>{"{{query}}"} and {"{{count}}"} are filled in</small></span>
            <textarea
              className="web-body-template"
              value={text("bodyTemplate")}
              onChange={(event) => onChange({ bodyTemplate: event.target.value })}
              placeholder={'{ "query": "{{query}}", "limit": {{count}} }'}
              spellCheck={false}
              rows={3}
            />
          </label>
        )}
        <label><span>Auth header <small>optional</small></span><input value={text("authHeader")} onChange={(event) => onChange({ authHeader: event.target.value })} placeholder="Authorization" spellCheck={false} /></label>
        <label><span>Before the key <small>optional</small></span><input value={text("authPrefix")} onChange={(event) => onChange({ authPrefix: event.target.value })} placeholder="Bearer " spellCheck={false} /></label>
        <label><span>Results list</span><input value={text("resultsPath")} onChange={(event) => onChange({ resultsPath: event.target.value })} placeholder="results" spellCheck={false} /></label>
        <label><span>Title, in each result</span><input value={text("titlePath")} onChange={(event) => onChange({ titlePath: event.target.value })} placeholder="title" spellCheck={false} /></label>
        <label><span>URL, in each result</span><input value={text("urlPath")} onChange={(event) => onChange({ urlPath: event.target.value })} placeholder="url" spellCheck={false} /></label>
        <label><span>Snippet <small>optional</small></span><input value={text("snippetPath")} onChange={(event) => onChange({ snippetPath: event.target.value })} placeholder="snippet" spellCheck={false} /></label>
        <label><span>Date <small>optional</small></span><input value={text("datePath")} onChange={(event) => onChange({ datePath: event.target.value })} placeholder="published" spellCheck={false} /></label>
      </div>
    </section>
  );
}
