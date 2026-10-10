import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { applyBuiltinModelSuggestion, formatTokens, mergeDiscoveredModels, modelIsReady, searchBuiltinModels } from "../model-utils";
import type {
  ApiFormat, BuiltinModelSuggestion, CustomProviderRecord, ModelRecord, ProviderRecord, SaveProviderInput, SubscriptionProviderInfo,
  SubscriptionProviderRecord, ThinkingLevel
} from "../types";
import { Icon, type IconName } from "./Icons";
import { SettingsHero, stagger } from "./SettingsHero";
import { Checkbox } from "./ui/Checkbox";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import { Popover } from "./ui/Popover";
import { Select } from "./ui/Select";

export type ConnectionMethod = "apiKey" | "subscription";

const LEVELS: ThinkingLevel[] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
const API_FORMATS: { value: ApiFormat; label: string }[] = [
  { value: "openai-completions", label: "Chat Completions compatible" },
  { value: "openai-responses", label: "Responses compatible" },
  { value: "anthropic-messages", label: "Messages compatible" }
];
const FORMAT_NAMES: Record<ApiFormat, string> = { "openai-completions": "Chat Completions", "openai-responses": "Responses", "anthropic-messages": "Messages" };
const ANTHROPIC_GUIDANCE = "https://support.claude.com/en/articles/13189465-log-in-to-your-claude-account";
/** A discovery with more new IDs than this starts with nothing ticked, and gets a filter. */
const FOUND_PRESELECT_LIMIT = 10;
/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

const METHODS: { value: ConnectionMethod; icon: IconName; title: string; detail: (names: string) => string }[] = [
  {
    value: "apiKey", icon: "key", title: "API key",
    detail: () => "Any Chat Completions, Responses or Messages compatible endpoint: a provider's API, a gateway, or a server on your Mac."
  },
  {
    value: "subscription", icon: "lock", title: "Sign in with a subscription",
    detail: (names) => `Use a plan you already pay for, through Pi's sign-in${names ? `: ${names}` : ""}.`
  }
];

// ── Shared bits ─────────────────────────────────────────────

interface Draft extends SaveProviderInput {
  id?: string;
}

function blankDraft(): Draft {
  return { name: "", baseUrl: "", apiFormat: "openai-completions", apiKey: "", models: [] };
}

function blankModel(): ModelRecord {
  return { id: "", name: "", contextWindow: null, maxTokens: null, reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: { off: null }, vision: false };
}

function fromProvider(provider: CustomProviderRecord): Draft {
  return {
    id: provider.id,
    name: provider.name,
    baseUrl: provider.baseUrl,
    apiFormat: provider.apiFormat,
    apiKey: "",
    models: provider.models.map((model) => ({ ...model, thinkingLevels: [...model.thinkingLevels], thinkingLevelMap: { ...model.thinkingLevelMap } }))
  };
}

/** JSON with object keys sorted, so a draft edited back to how it was compares equal again. */
function stableKey(value: unknown): string {
  return JSON.stringify(value, (_key, entry: unknown) =>
    entry && typeof entry === "object" && !Array.isArray(entry)
      ? Object.fromEntries(Object.entries(entry).sort(([left], [right]) => left.localeCompare(right)))
      : entry);
}

let nextRowKey = 0;
const newRowKeys = (count: number) => Array.from({ length: count }, () => ++nextRowKey);

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url.trim();
  }
}

const initialOf = (name: string) => (name.trim().charAt(0) || "?").toUpperCase();

function formatLabel(provider: ProviderRecord): string {
  return provider.kind === "subscription" ? "Subscription" : FORMAT_NAMES[provider.apiFormat];
}

/** What a connection's status dot says, in the `package-state` tones. */
function connectionState(provider: ProviderRecord): { tone: "on" | "off" | "idle"; label: string } {
  if (provider.enabled === false) return { tone: "idle", label: "Off" };
  if (provider.kind === "subscription") return provider.connected ? { tone: "on", label: "Signed in" } : { tone: "off", label: "Signed out" };
  return provider.connected ? { tone: "on", label: "Key saved" } : { tone: "off", label: "No API key" };
}

/** On, reachable, and with at least one model whose limits are set: what the model picker offers. */
const usable = (provider: ProviderRecord) => provider.enabled !== false && provider.connected && provider.models.some(modelIsReady);

const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? "" : "s"}`;

function EnableSwitch({ provider, busy, onToggle }: { provider: ProviderRecord; busy: boolean; onToggle: () => void }) {
  const on = provider.enabled !== false;
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={`Use ${provider.name}`}
      className={`toggle ${on ? "on" : ""}`}
      disabled={busy}
      onClick={onToggle}
    >
      <span />
    </button>
  );
}

function BackLink({ onBack }: { onBack: () => void }) {
  return (
    <button type="button" className="ghost-button skill-back provider-back" onClick={onBack}>
      <Icon name="back" /> All connections
    </button>
  );
}

/**
 * The overview hero's little stage: a key slides into a socket and turns, the socket's light comes
 * on and the models it unlocks line up beside it. Rests dashed and dimmed, the key out, while no
 * connection is ready. Pure decoration; the pill says the same in words. The loop lives in
 * styles.css, which stills it under reduced motion.
 */
function ProvidersStage({ live }: { live: boolean }) {
  const models = [50, 36, 44];
  return (
    <svg className={`settings-stage providers-stage ${live ? "live" : ""}`} viewBox="0 0 160 110" aria-hidden="true">
      <rect className="providers-stage-plate" x="18" y="22" width="46" height="66" rx="11" />
      <path className="providers-stage-hole" d="M41 41.5a5.5 5.5 0 0 1 2.8 10.2l1.4 8.3h-8.4l1.4-8.3A5.5 5.5 0 0 1 41 41.5z" />
      {/* Paint the key over the lock face: its inserted tip lands inside the hole at x=42. */}
      <g className="providers-stage-key">
        <path className="providers-stage-shaft" d="M104 50H64M70 50v6M77 50v4" />
        <g className="providers-stage-bow">
          <circle cx="116" cy="50" r="11" />
          <circle cx="116" cy="50" r="4" />
        </g>
      </g>
      <circle className="providers-stage-led" cx="41" cy="76" r="2.6" />
      {models.map((width, index) => (
        <rect key={index} className="providers-stage-model" x="78" y={70 + index * 9} width={width} height="4" rx="2" style={{ "--k": index } as React.CSSProperties} />
      ))}
    </svg>
  );
}

// ── The section ─────────────────────────────────────────────

interface Props {
  providers: ProviderRecord[];
  /** Undefined shows the overview; "new" a new connection. */
  selectedId?: string;
  /** Changes whenever the user navigates, so an editor starts fresh; a new connection keeps it when saved. */
  editorKey: number;
  newMethod: ConnectionMethod;
  agentName: string;
  builtinModels: BuiltinModelSuggestion[];
  catalogLoading: boolean;
  catalogError?: string;
  subscriptionProviders: SubscriptionProviderInfo[];
  subscriptionError?: string;
  onSelect: (id?: string) => void;
  /** A new connection was saved: show it, keeping the editor as it is. */
  onCreated: (id: string) => void;
  onStartNew: (method: ConnectionMethod) => void;
  onSave: (input: SaveProviderInput) => Promise<ProviderRecord>;
  onDelete: (providerId: string) => Promise<void>;
  onSetProviderEnabled: (providerId: string, enabled: boolean) => Promise<void>;
  onConnectSubscription: (providerId: string) => Promise<void>;
  onSignOutSubscription: (providerId: string) => Promise<void>;
  /** The model IDs the provider lists, fetched with the saved key; rejects with a user-facing message. */
  onDiscover: (providerId: string) => Promise<string[]>;
  onOpenAuthUrl: (url: string) => Promise<void>;
}

/**
 * Settings › Providers. An overview of every connection, with the ways to add one; then a page
 * per connection. An API-key connection is edited as a draft and saved from a bar that appears
 * only while there is something to save; its on/off switch applies at once. Subscription
 * connections are signed in to through Pi and list the models the account offers.
 */
export function ProvidersSection(props: Props) {
  const { providers, selectedId, editorKey, newMethod, onSelect, onStartNew } = props;
  const provider = selectedId && selectedId !== "new" ? providers.find((item) => item.id === selectedId) : undefined;

  if (!selectedId || (selectedId !== "new" && !provider)) {
    return (
      <ProvidersOverview
        providers={providers}
        agentName={props.agentName}
        subscriptionProviders={props.subscriptionProviders}
        onSelect={onSelect}
        onStartNew={onStartNew}
        onSetProviderEnabled={props.onSetProviderEnabled}
      />
    );
  }

  if (provider?.kind === "subscription") {
    return (
      <SubscriptionDetail
        key={provider.id}
        provider={provider}
        guidance={props.subscriptionProviders.find((item) => item.id === provider.id)?.guidance}
        onBack={() => onSelect(undefined)}
        onConnect={props.onConnectSubscription}
        onSignOut={props.onSignOutSubscription}
        onDelete={async (id) => { await props.onDelete(id); onSelect(undefined); }}
        onSetProviderEnabled={props.onSetProviderEnabled}
        onOpenAuthUrl={props.onOpenAuthUrl}
      />
    );
  }

  const methodPicker = (
    <section className="settings-block" style={stagger(0)} aria-labelledby="provider-new-title">
      <h3 className="settings-block-title" id="provider-new-title">New connection</h3>
      <p className="settings-block-sub">How do you reach the model?</p>
      <MethodTiles
        mode="radio"
        selected={newMethod}
        names={props.subscriptionProviders.map((item) => item.name)}
        onPick={onStartNew}
      />
    </section>
  );

  if (!provider && newMethod === "subscription") {
    return (
      <div className="settings-scroll providers-settings">
        <div className="settings-page">
          <BackLink onBack={() => onSelect(undefined)} />
          {methodPicker}
          <SubscriptionCatalog
            providers={props.subscriptionProviders}
            error={props.subscriptionError}
            onConnect={props.onConnectSubscription}
            onOpenAuthUrl={props.onOpenAuthUrl}
          />
        </div>
      </div>
    );
  }

  return (
    <ConnectionEditor
      key={editorKey}
      provider={provider?.kind === "custom" ? provider : undefined}
      lead={provider ? undefined : methodPicker}
      builtinModels={props.builtinModels}
      catalogLoading={props.catalogLoading}
      catalogError={props.catalogError}
      onBack={() => onSelect(undefined)}
      onCreated={props.onCreated}
      onSave={props.onSave}
      onDelete={async (id) => { await props.onDelete(id); onSelect(undefined); }}
      onSetProviderEnabled={props.onSetProviderEnabled}
      onDiscover={props.onDiscover}
    />
  );
}

interface MethodTilesProps {
  /** "radio" on the new-connection page, where one is chosen; "start" on the overview, where either begins one. */
  mode: "radio" | "start";
  selected?: ConnectionMethod;
  /** The subscription providers Pi can sign in to, named on their tile. */
  names: string[];
  onPick: (method: ConnectionMethod) => void;
}

function MethodTiles({ mode, selected, names, onPick }: MethodTilesProps) {
  return (
    <div className="provider-methods" role={mode === "radio" ? "radiogroup" : undefined} aria-label={mode === "radio" ? "Connection method" : undefined}>
      {METHODS.map((method) => {
        const chosen = mode === "radio" && selected === method.value;
        return (
          <button
            key={method.value}
            type="button"
            role={mode === "radio" ? "radio" : undefined}
            aria-checked={mode === "radio" ? chosen : undefined}
            className={`provider-method ${chosen ? "selected" : ""}`}
            onClick={() => onPick(method.value)}
          >
            <span className="provider-method-mark" aria-hidden="true"><Icon name={method.icon} /></span>
            <span className="provider-method-text">
              <strong>{method.title}</strong>
              <span>{method.detail(names.join(", "))}</span>
            </span>
            {mode === "start" && <Icon name="chevron" />}
          </button>
        );
      })}
    </div>
  );
}

// ── Overview ────────────────────────────────────────────────

interface OverviewProps {
  providers: ProviderRecord[];
  agentName: string;
  subscriptionProviders: SubscriptionProviderInfo[];
  onSelect: (id?: string) => void;
  onStartNew: (method: ConnectionMethod) => void;
  onSetProviderEnabled: (providerId: string, enabled: boolean) => Promise<void>;
}

function ProvidersOverview({ providers, agentName, subscriptionProviders, onSelect, onStartNew, onSetProviderEnabled }: OverviewProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const ready = providers.filter(usable).length;
  const pill = ready > 0 ? `${ready} ready` : providers.length ? "None ready" : "No connections";

  async function toggle(provider: ProviderRecord) {
    setBusy(true);
    setError(undefined);
    try {
      await onSetProviderEnabled(provider.id, provider.enabled === false);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="settings-scroll providers-settings">
      <div className="settings-page">
        <SettingsHero
          label="Providers overview"
          stage={<ProvidersStage live={ready > 0} />}
          live={ready > 0}
          pill={pill}
          title={`Models ${agentName} can talk to`}
        >
          <p>
            Each connection is an API you have a key for, or a subscription you sign in to. Its models join the model
            picker.
          </p>
        </SettingsHero>
        {error && <div className="error-banner" role="alert">{error}</div>}

        {providers.length > 0 && (
          <section className="settings-block" style={stagger(1)} aria-labelledby="providers-list-title">
            <h3 className="settings-block-title" id="providers-list-title">Connections</h3>
            <p className="settings-block-sub">Switched off, a connection keeps its key and models but leaves the model picker.</p>
            <ul className="provider-list">
              {providers.map((provider) => {
                const state = connectionState(provider);
                const needLimits = provider.models.filter((model) => !modelIsReady(model)).length;
                const where = provider.kind === "custom" ? hostOf(provider.baseUrl) : "";
                return (
                  <li className={`provider-row ${provider.enabled === false ? "off" : ""}`} key={provider.id}>
                    <button type="button" className="provider-row-main" onClick={() => onSelect(provider.id)}>
                      <span className="provider-mark" aria-hidden="true">{initialOf(provider.name)}</span>
                      <span className="provider-row-text">
                        <strong>{provider.name}</strong>
                        <span>{formatLabel(provider)}{where ? ` · ${where}` : ""}</span>
                      </span>
                      <span className="provider-row-meta">
                        <span className={`package-state ${state.tone}`}>{state.label}</span>
                        <span className="provider-row-count">
                          {plural(provider.models.length, "model")}
                          {needLimits > 0 && <em> · {needLimits} need{needLimits === 1 ? "s" : ""} limits</em>}
                        </span>
                      </span>
                      <Icon name="chevron" />
                    </button>
                    <EnableSwitch provider={provider} busy={busy} onToggle={() => void toggle(provider)} />
                  </li>
                );
              })}
            </ul>
          </section>
        )}

        <section className="settings-block" style={stagger(providers.length ? 2 : 1)} aria-labelledby="providers-add-title">
          <h3 className="settings-block-title" id="providers-add-title">{providers.length ? "Add a connection" : "Connect your first model"}</h3>
          <p className="settings-block-sub">
            {providers.length
              ? "Another provider, a gateway or a local server, or a subscription you pay for."
              : `${agentName} needs at least one model to chat with. Pick how you reach it.`}
          </p>
          <MethodTiles mode="start" names={subscriptionProviders.map((item) => item.name)} onPick={onStartNew} />
        </section>
      </div>
    </div>
  );
}

// ── An API-key connection ───────────────────────────────────

interface EditorProps {
  /** Undefined while the connection is new. */
  provider?: CustomProviderRecord;
  /** Shown first on the page: the method choice while new. */
  lead?: ReactNode;
  builtinModels: BuiltinModelSuggestion[];
  catalogLoading: boolean;
  catalogError?: string;
  onBack: () => void;
  onCreated: (id: string) => void;
  onSave: (input: SaveProviderInput) => Promise<ProviderRecord>;
  onDelete: (providerId: string) => Promise<void>;
  onSetProviderEnabled: (providerId: string, enabled: boolean) => Promise<void>;
  onDiscover: (providerId: string) => Promise<string[]>;
}

interface Found {
  /** IDs the provider listed that the connection doesn't have yet. */
  ids: string[];
  /** How many it listed in all. */
  total: number;
  selected: ReadonlySet<string>;
}

function ConnectionEditor({
  provider, lead, builtinModels, catalogLoading, catalogError, onBack, onCreated, onSave, onDelete, onSetProviderEnabled, onDiscover
}: EditorProps) {
  const reduce = useReducedMotion();
  // Reseed from the saved record when its editable fields change. Keyed by content, so the
  // enabled switch (which rewrites `providers` live) keeps unsaved edits.
  const seed = provider ? fromProvider(provider) : blankDraft();
  const seedKey = stableKey(seed);
  const [draft, setDraft] = useState<Draft>(seed);
  const [rowKeys, setRowKeys] = useState<number[]>(() => newRowKeys(seed.models.length));
  const [open, setOpen] = useState<ReadonlySet<number>>(new Set());
  const [adding, setAdding] = useState(false);
  const [found, setFound] = useState<Found>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => {
    setDraft(seed);
    // Same count: the rows are the same models, so they keep their keys and stay open.
    setRowKeys((keys) => (keys.length === seed.models.length ? keys : newRowKeys(seed.models.length)));
  }, [seedKey]);

  // A save's confirmation lingers in the bar for a moment, then the bar tucks away.
  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(undefined), 3200);
    return () => clearTimeout(timer);
  }, [notice]);

  const isNew = !draft.id;
  const dirty = isNew || stableKey(draft) !== seedKey;
  const complete = Boolean(draft.name.trim() && draft.baseUrl.trim());
  const incomplete = useMemo(() => draft.models.filter((model) => !modelIsReady(model)).length, [draft.models]);

  function edit(patch: Partial<Draft>) {
    setNotice(undefined);
    setDraft((current) => ({ ...current, ...patch }));
  }

  function updateModel(index: number, patch: Partial<ModelRecord>) {
    setNotice(undefined);
    setDraft((current) => ({ ...current, models: current.models.map((model, at) => (at === index ? { ...model, ...patch } : model)) }));
  }

  function addModels(models: ModelRecord[], openThem: boolean) {
    const keys = newRowKeys(models.length);
    setDraft((current) => ({ ...current, models: [...current.models, ...models] }));
    setRowKeys((current) => [...current, ...keys]);
    if (openThem) setOpen((current) => new Set([...current, ...keys]));
    setNotice(undefined);
  }

  function removeModel(index: number) {
    const key = rowKeys[index];
    setDraft((current) => ({ ...current, models: current.models.filter((_, at) => at !== index) }));
    setRowKeys((current) => current.filter((_, at) => at !== index));
    setOpen((current) => { const next = new Set(current); next.delete(key); return next; });
  }

  function toggleRow(key: number) {
    setOpen((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  async function save(): Promise<ProviderRecord | undefined> {
    setBusy(true);
    setError(undefined);
    setNotice(undefined);
    try {
      const withKey = Boolean(draft.apiKey?.trim());
      const saved = await onSave({ ...draft, apiKey: draft.apiKey?.trim() || undefined });
      setDraft(saved.kind === "custom" ? fromProvider(saved) : blankDraft());
      setNotice(withKey ? "Saved. The API key is stored on this Mac." : "Saved.");
      if (isNew) onCreated(saved.id);
      return saved;
    } catch (reason) {
      setError(String(reason));
      return undefined;
    } finally {
      setBusy(false);
    }
  }

  function discard() {
    if (isNew) {
      onBack();
      return;
    }
    setDraft(seed);
    setRowKeys(newRowKeys(seed.models.length));
    setOpen(new Set());
    setFound(undefined);
    setAdding(false);
    setError(undefined);
  }

  /** Discovery reads the saved key, so unsaved changes are saved first, and the button says so. */
  async function fetchModels() {
    let id = draft.id;
    let known = draft.models;
    if (!id || dirty) {
      const saved = await save();
      if (!saved) return;
      id = saved.id;
      known = saved.models;
    }
    setBusy(true);
    setError(undefined);
    try {
      const ids = await onDiscover(id);
      const fresh = ids.filter((modelId) => !known.some((model) => model.id === modelId));
      setAdding(false);
      if (fresh.length === 0) {
        setFound(undefined);
        setNotice(ids.length ? `The provider lists ${plural(ids.length, "model")}, all already added.` : "The provider listed no models. You can still add them by hand.");
      } else {
        setFound({ ids: fresh, total: ids.length, selected: new Set(fresh.length <= FOUND_PRESELECT_LIMIT ? fresh : []) });
      }
    } catch (reason) {
      setError(`${String(reason)} You can still add models by hand.`);
    } finally {
      setBusy(false);
    }
  }

  async function toggleEnabled() {
    if (!provider) return;
    setBusy(true);
    setError(undefined);
    try {
      await onSetProviderEnabled(provider.id, provider.enabled === false);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const fetchLabel = isNew ? "Create & fetch models" : dirty ? "Save & fetch models" : "Fetch models";
  const state = provider ? connectionState(provider) : undefined;
  const showBar = dirty || Boolean(error) || Boolean(notice);

  return (
    <>
      <div className="settings-scroll providers-settings">
        <div className="settings-page">
          <BackLink onBack={onBack} />
          {lead}
          {provider && state && (
            <section className="settings-block provider-head" style={stagger(0)} aria-label={`${provider.name} connection`}>
              <span className="provider-mark large" aria-hidden="true">{initialOf(draft.name || provider.name)}</span>
              <div className="provider-head-text">
                <h3 className="settings-block-title">{draft.name.trim() || provider.name}</h3>
                <span className="provider-head-meta">{formatLabel(provider)} · {hostOf(draft.baseUrl) || "No URL"}</span>
                <span className="provider-head-status">
                  <span className={`package-state ${state.tone}`}>{state.label}</span>
                  <span>{plural(draft.models.length, "model")}{incomplete > 0 ? ` · ${incomplete} need${incomplete === 1 ? "s" : ""} limits` : ""}</span>
                </span>
              </div>
              <span className="provider-head-switch">
                <span aria-hidden="true">Use this connection</span>
                <EnableSwitch provider={provider} busy={busy} onToggle={() => void toggleEnabled()} />
              </span>
            </section>
          )}

          <section className="settings-block" style={stagger(1)} aria-labelledby="connection-fields-title">
            <h3 className="settings-block-title" id="connection-fields-title">Connection</h3>
            <p className="settings-block-sub">Where the API lives, and the key that reaches it. The key is stored on this Mac.</p>
            <div className="form-grid connection-grid">
              <label>
                <span>Name</span>
                <input value={draft.name} onChange={(event) => edit({ name: event.target.value })} placeholder="My inference gateway" />
              </label>
              <label>
                <span>API format</span>
                <Select
                  className="settings-select"
                  matchWidth
                  value={draft.apiFormat}
                  onChange={(value) => edit({ apiFormat: value as ApiFormat })}
                  options={API_FORMATS}
                  aria-label="API format"
                />
              </label>
              <label>
                {/* The Messages API posts to `{baseUrl}/v1/messages`, so its base URL leaves off
                    the `/v1` that the OpenAI formats keep (OpenCode Go: `…/zen/go`); saving trims
                    one a pasted endpoint carried. */}
                <span>Base URL {draft.apiFormat === "anthropic-messages" && <small>/v1/messages is added for you; a pasted /v1 is trimmed</small>}</span>
                <input
                  value={draft.baseUrl}
                  onChange={(event) => edit({ baseUrl: event.target.value })}
                  placeholder={draft.apiFormat === "anthropic-messages" ? "https://api.anthropic.com" : "https://api.example.com/v1"}
                  spellCheck={false}
                />
              </label>
              <label>
                <span>API key <small>{provider?.hasApiKey ? "Leave blank to keep the saved key" : "Stored on this Mac"}</small></span>
                <div className="input-with-icon">
                  <Icon name="key" />
                  <input
                    type="password"
                    autoComplete="off"
                    value={draft.apiKey ?? ""}
                    onChange={(event) => edit({ apiKey: event.target.value })}
                    placeholder={provider?.hasApiKey ? "••••••••••••••••" : "Enter API key"}
                  />
                </div>
              </label>
            </div>
            {provider && (
              <div className="provider-remove">
                <span>Deleting removes the connection and its saved API key.</span>
                <button type="button" className="danger-button compact" disabled={busy} onClick={() => setConfirmDelete(true)}>Delete connection</button>
              </div>
            )}
          </section>

          <section className="settings-block" style={stagger(2)} aria-labelledby="connection-models-title">
            <div className="models-block-head">
              <div>
                <h3 className="settings-block-title" id="connection-models-title">Models</h3>
                <p className="settings-block-sub">
                  {draft.models.length
                    ? "Open one to edit it. Pi's catalogue can fill in a model's limits and reasoning."
                    : "Fetch the list from the provider, or add a model from Pi's catalogue."}
                </p>
              </div>
              <div className="models-block-actions">
                <button type="button" className="secondary-button compact" disabled={busy || !complete} onClick={() => void fetchModels()}>
                  <Icon name="refresh" /> {fetchLabel}
                </button>
                <button type="button" className="secondary-button compact" disabled={busy || adding} onClick={() => { setFound(undefined); setAdding(true); }}>
                  <Icon name="plus" /> Add model
                </button>
              </div>
            </div>

            <AnimatePresence initial={false}>
              {found && (
                <FoundModels
                  key="found"
                  found={found}
                  onChange={(selected) => setFound({ ...found, selected })}
                  onCancel={() => setFound(undefined)}
                  onAdd={() => {
                    addModels(mergeDiscoveredModels([], found.ids.filter((id) => found.selected.has(id))), false);
                    setFound(undefined);
                  }}
                />
              )}
              {adding && (
                <AddModel
                  key="add"
                  catalog={builtinModels}
                  loading={catalogLoading}
                  error={catalogError}
                  onAdd={(model) => { addModels([model], true); setAdding(false); }}
                  onCancel={() => setAdding(false)}
                />
              )}
            </AnimatePresence>

            {draft.models.length === 0 ? (
              !adding && !found && <p className="models-empty">No models yet.</p>
            ) : (
              <ul className="model-rows">
                {draft.models.map((model, index) => {
                  const key = rowKeys[index];
                  return (
                    <ModelRow
                      key={key}
                      model={model}
                      connectionFormat={draft.apiFormat}
                      open={open.has(key)}
                      catalog={builtinModels}
                      catalogLoading={catalogLoading}
                      catalogError={catalogError}
                      reduce={Boolean(reduce)}
                      onToggle={() => toggleRow(key)}
                      onChange={(patch) => updateModel(index, patch)}
                      onRemove={() => removeModel(index)}
                    />
                  );
                })}
              </ul>
            )}
          </section>
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
                : dirty ? (
                  <span className="provider-savebar-note">
                    <i aria-hidden="true" />
                    {isNew ? "Not saved yet" : "Unsaved changes"}
                    {incomplete > 0 ? ` · ${incomplete} model${incomplete === 1 ? " needs" : "s need"} limits` : ""}
                  </span>
                ) : notice ? <span className="success-text" role="status">{notice}</span> : null}
            </div>
            {dirty && (
              <>
                <button type="button" className="secondary-button" disabled={busy} onClick={discard}>{isNew ? "Cancel" : "Discard"}</button>
                <button type="button" className="primary-button" disabled={busy || !complete} onClick={() => void save()}>
                  {busy ? "Saving…" : isNew ? "Create connection" : "Save connection"}
                </button>
              </>
            )}
          </motion.footer>
        )}
      </AnimatePresence>

      {confirmDelete && provider && (
        <ConfirmDialog
          title={`Delete “${provider.name}”?`}
          body="This removes the connection and its saved API key."
          confirmLabel="Delete"
          danger
          onConfirm={() => onDelete(provider.id)}
          onCancel={() => setConfirmDelete(false)}
        />
      )}
    </>
  );
}

interface RowProps {
  model: ModelRecord;
  connectionFormat: ApiFormat;
  open: boolean;
  catalog: BuiltinModelSuggestion[];
  catalogLoading: boolean;
  catalogError?: string;
  reduce: boolean;
  onToggle: () => void;
  onChange: (patch: Partial<ModelRecord>) => void;
  onRemove: () => void;
}

/** One model: a line saying what it is and what it can do, which opens into its settings. */
function ModelRow({ model, connectionFormat, open, catalog, catalogLoading, catalogError, reduce, onToggle, onChange, onRemove }: RowProps) {
  const ready = modelIsReady(model);
  const label = model.name || model.id || "New model";
  return (
    <li className={`model-row ${open ? "open" : ""} ${ready ? "" : "incomplete"}`}>
      <div className="model-row-head">
        <button type="button" className="model-row-main" aria-expanded={open} onClick={onToggle}>
          <Icon name="chevron" />
          <span className="model-row-name">{label}</span>
          {model.name && model.id && model.name !== model.id && <code>{model.id}</code>}
          <span className="model-row-facts">
            {ready ? (
              <span className="model-row-limits">{formatTokens(model.contextWindow!)} context · {formatTokens(model.maxTokens!)} out</span>
            ) : (
              <span className="model-row-warning">Needs limits</span>
            )}
            {model.reasoning && <span className="subagent-badge">Reasoning</span>}
            {model.vision && <span className="subagent-badge">Vision</span>}
            {model.apiFormat && model.apiFormat !== connectionFormat && <span className="subagent-badge">{FORMAT_NAMES[model.apiFormat]}</span>}
          </span>
        </button>
        <button type="button" className="command-icon-button" aria-label={`Remove ${label}`} onClick={onRemove}>
          <Icon name="trash" />
        </button>
      </div>
      {open && (
        <motion.div
          className="model-editor"
          initial={reduce ? false : { opacity: 0, y: -4 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.2, ease: EASE }}
        >
          <ModelEditor model={model} connectionFormat={connectionFormat} catalog={catalog} catalogLoading={catalogLoading} catalogError={catalogError} onChange={onChange} />
        </motion.div>
      )}
    </li>
  );
}

interface ModelEditorProps {
  model: ModelRecord;
  connectionFormat: ApiFormat;
  catalog: BuiltinModelSuggestion[];
  catalogLoading: boolean;
  catalogError?: string;
  onChange: (patch: Partial<ModelRecord>) => void;
}

function ModelEditor({ model, connectionFormat, catalog, catalogLoading, catalogError, onChange }: ModelEditorProps) {
  const [advanced, setAdvanced] = useState(false);
  const label = model.name || model.id || "this model";

  function toggleLevel(level: ThinkingLevel) {
    const present = model.thinkingLevels.includes(level);
    const next = present ? model.thinkingLevels.filter((item) => item !== level) : [...model.thinkingLevels, level];
    const thinkingLevelMap = { ...model.thinkingLevelMap };
    if (present) delete thinkingLevelMap[level];
    else thinkingLevelMap[level] = level === "off" ? null : level;
    if (next.length === 0) thinkingLevelMap.off = null;
    onChange({ thinkingLevels: next.length ? next : ["off"], thinkingLevelMap });
  }

  function mapLevel(level: ThinkingLevel, value: string) {
    onChange({ thinkingLevelMap: { ...model.thinkingLevelMap, [level]: value.trim() ? value : level === "off" ? null : level } });
  }

  return (
    <>
      <div className="model-editor-grid">
        <label><span>Model ID</span><input value={model.id} onChange={(event) => onChange({ id: event.target.value })} placeholder="provider/model-id" spellCheck={false} /></label>
        <label><span>Display name</span><input value={model.name} onChange={(event) => onChange({ name: event.target.value })} placeholder={model.id || "Model name"} /></label>
        {/* Some gateways serve a few models over another API at the same URL; blank follows the connection. */}
        <label>
          <span>API format</span>
          <Select
            className="settings-select"
            matchWidth
            value={model.apiFormat ?? ""}
            onChange={(value) => onChange({ apiFormat: value ? (value as ApiFormat) : undefined })}
            options={[
              { value: "", label: "Same as connection", hint: FORMAT_NAMES[connectionFormat] },
              ...API_FORMATS.map(({ value }) => ({ value, label: FORMAT_NAMES[value] }))
            ]}
            aria-label={`API format for ${label}`}
          />
        </label>
      </div>
      <ModelSuggestionSearch
        modelLabel={model.name || model.id || "this model"}
        catalog={catalog}
        loading={catalogLoading}
        error={catalogError}
        onSelect={(suggestion) => onChange(applyBuiltinModelSuggestion(model, suggestion))}
      />
      <div className="model-limits">
        <label><span>Context tokens</span><input type="number" min="1" value={model.contextWindow ?? ""} onChange={(event) => onChange({ contextWindow: event.target.value ? Number(event.target.value) : null })} placeholder="Required" /></label>
        <label><span>Max output tokens</span><input type="number" min="1" value={model.maxTokens ?? ""} onChange={(event) => onChange({ maxTokens: event.target.value ? Number(event.target.value) : null })} placeholder="Required" /></label>
        <label className="capability-toggle">
          <span>Reasoning</span>
          <button
            type="button"
            role="switch"
            aria-checked={model.reasoning}
            aria-label={`Reasoning for ${label}`}
            className={`toggle ${model.reasoning ? "on" : ""}`}
            onClick={() => onChange({
              reasoning: !model.reasoning,
              thinkingLevels: !model.reasoning ? ["off", "low", "medium", "high"] : ["off"],
              thinkingLevelMap: !model.reasoning ? { off: null, low: "low", medium: "medium", high: "high" } : { off: null }
            })}
          ><span /></button>
        </label>
        <label className="capability-toggle">
          <span>Vision</span>
          <button
            type="button"
            role="switch"
            aria-checked={model.vision}
            aria-label={`Vision for ${label}`}
            className={`toggle ${model.vision ? "on" : ""}`}
            onClick={() => onChange({ vision: !model.vision })}
          ><span /></button>
        </label>
      </div>
      {model.reasoning && (
        <div className="model-reasoning">
          <div className="reasoning-levels" role="group" aria-label="Supported efforts">
            <span>Supported efforts</span>
            {LEVELS.map((level) => (
              <button key={level} type="button" aria-pressed={model.thinkingLevels.includes(level)} className={model.thinkingLevels.includes(level) ? "selected" : ""} onClick={() => toggleLevel(level)}>
                {level}
              </button>
            ))}
          </div>
          <button type="button" className="tool-card-more model-advanced" aria-expanded={advanced} onClick={() => setAdvanced((value) => !value)}>
            <Icon name="chevron" /> Values sent to the provider
          </button>
          {advanced && (
            <div className="reasoning-mappings">
              {model.thinkingLevels.map((level) => (
                <label key={level}>
                  <span>{level}</span>
                  <input value={model.thinkingLevelMap[level] ?? ""} onChange={(event) => mapLevel(level, event.target.value)} placeholder={level === "off" ? "omit" : level} spellCheck={false} />
                </label>
              ))}
              <small>Blank &ldquo;off&rdquo; omits reasoning; other blanks send the effort&rsquo;s name.</small>
            </div>
          )}
        </div>
      )}
      {!modelIsReady(model) && <p className="model-warning">Set the context and output limits before this model can be used.</p>}
    </>
  );
}

interface AddModelProps {
  catalog: BuiltinModelSuggestion[];
  loading: boolean;
  error?: string;
  onAdd: (model: ModelRecord) => void;
  onCancel: () => void;
}

/** Search-first: a pick from Pi's catalogue arrives with its limits; typing it in is the fallback. */
function AddModel({ catalog, loading, error, onAdd, onCancel }: AddModelProps) {
  const reduce = useReducedMotion();
  return (
    <motion.div
      className="model-panel"
      initial={reduce ? { opacity: 0 } : { opacity: 0, y: -4 }}
      animate={{ opacity: 1, y: 0 }}
      exit={reduce ? { opacity: 0 } : { opacity: 0, y: -4 }}
      transition={{ duration: 0.18, ease: EASE }}
    >
      <div className="model-panel-head">
        <strong>Add a model</strong>
        <button type="button" className="ghost-button" aria-label="Close" onClick={onCancel}><Icon name="close" /></button>
      </div>
      <ModelSuggestionSearch
        modelLabel="a new model"
        catalog={catalog}
        loading={loading}
        error={error}
        autoFocus
        onSelect={(suggestion) => onAdd(applyBuiltinModelSuggestion(blankModel(), suggestion))}
      />
      <button type="button" className="text-button model-panel-manual" onClick={() => onAdd(blankModel())}>Enter it manually</button>
    </motion.div>
  );
}

interface FoundProps {
  found: Found;
  onChange: (selected: ReadonlySet<string>) => void;
  onAdd: () => void;
  onCancel: () => void;
}

/** What discovery found that the connection lacks, to pick from rather than take wholesale. */
function FoundModels({ found, onChange, onAdd, onCancel }: FoundProps) {
  const reduce = useReducedMotion();
  const [filter, setFilter] = useState("");
  const shown = filter.trim() ? found.ids.filter((id) => id.toLowerCase().includes(filter.trim().toLowerCase())) : found.ids;
  const count = found.selected.size;
  const all = shown.length > 0 && shown.every((id) => found.selected.has(id));
  const some = shown.some((id) => found.selected.has(id));
  const set = (ids: string[], on: boolean) => {
    const next = new Set(found.selected);
    for (const id of ids) {
      if (on) next.add(id);
      else next.delete(id);
    }
    onChange(next);
  };
  return (
    <motion.div
      className="model-panel"
      initial={reduce ? { opacity: 0 } : { opacity: 0, y: -4 }}
      animate={{ opacity: 1, y: 0 }}
      exit={reduce ? { opacity: 0 } : { opacity: 0, y: -4 }}
      transition={{ duration: 0.18, ease: EASE }}
    >
      <div className="model-panel-head">
        <strong>
          The provider lists {plural(found.total, "model")}
          {found.total !== found.ids.length ? `, ${found.ids.length} new` : ""}
        </strong>
        <button type="button" className="ghost-button" aria-label="Close" onClick={onCancel}><Icon name="close" /></button>
      </div>
      {found.ids.length > FOUND_PRESELECT_LIMIT && (
        <input className="model-found-filter" value={filter} onChange={(event) => setFilter(event.target.value)} placeholder="Filter models…" aria-label="Filter found models" spellCheck={false} />
      )}
      <div className="model-found-all">
        <Checkbox checked={all} indeterminate={!all && some} label="Select all shown" onChange={(on) => set(shown, on)} />
        <span>{filter.trim() ? `All ${shown.length} shown` : "All"}</span>
      </div>
      <ul className="model-found-list">
        {shown.map((id) => (
          <li key={id} onClick={() => set([id], !found.selected.has(id))}>
            <Checkbox checked={found.selected.has(id)} label={id} onChange={(on) => set([id], on)} />
            <code>{id}</code>
          </li>
        ))}
        {shown.length === 0 && <li className="model-found-none">Nothing matches the filter.</li>}
      </ul>
      <div className="model-panel-actions">
        <small>Discovery lists IDs only; open each one afterwards to set its limits.</small>
        <button type="button" className="secondary-button compact" onClick={onCancel}>Cancel</button>
        <button type="button" className="primary-button compact" disabled={count === 0} onClick={onAdd}>
          Add {count > 0 ? plural(count, "model") : "models"}
        </button>
      </div>
    </motion.div>
  );
}

interface SearchProps {
  modelLabel: string;
  catalog: BuiltinModelSuggestion[];
  loading: boolean;
  error?: string;
  autoFocus?: boolean;
  onSelect: (suggestion: BuiltinModelSuggestion) => void;
}

function ModelSuggestionSearch({ modelLabel, catalog, loading, error, autoFocus = false, onSelect }: SearchProps) {
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const [applied, setApplied] = useState<BuiltinModelSuggestion>();
  const anchor = useRef<HTMLDivElement>(null);
  const listId = useId();
  const matches = useMemo(() => searchBuiltinModels(catalog, query), [catalog, query]);
  const visible = open && matches.length > 0 && !error;

  function choose(suggestion: BuiltinModelSuggestion) {
    onSelect(suggestion);
    setApplied(suggestion);
    setQuery("");
    setOpen(false);
    setActive(-1);
  }

  return (
    <div className="model-catalog-search" ref={anchor}>
      <label>
        <span>Find in Pi catalogue</span>
        <input
          role="combobox"
          aria-label={`Find in Pi catalogue for ${modelLabel}`}
          aria-autocomplete="list"
          aria-expanded={visible}
          aria-controls={visible ? listId : undefined}
          aria-activedescendant={visible && active >= 0 ? `${listId}-${active}` : undefined}
          value={query}
          autoFocus={autoFocus}
          onChange={(event) => { setQuery(event.target.value); setOpen(true); setActive(-1); }}
          onFocus={() => { if (query.trim()) setOpen(true); }}
          onKeyDown={(event) => {
            if (event.key === "Escape") { setOpen(false); setActive(-1); }
            else if (event.key === "Tab") setOpen(false);
            else if (event.key === "ArrowDown" && matches.length) {
              event.preventDefault();
              setOpen(true);
              setActive((value) => (value + 1) % matches.length);
            } else if (event.key === "ArrowUp" && matches.length) {
              event.preventDefault();
              setOpen(true);
              setActive((value) => value < 0 ? matches.length - 1 : (value - 1 + matches.length) % matches.length);
            } else if (event.key === "Enter" && visible) {
              event.preventDefault();
              choose(matches[active < 0 ? 0 : active]);
            }
          }}
          placeholder="Search by model name or ID"
          autoComplete="off"
          spellCheck={false}
        />
      </label>
      {loading && <small role="status">Loading bundled Pi catalogue…</small>}
      {error && <small role="status">Could not load Pi catalogue: {error} Manual entry is still available.</small>}
      {!loading && !error && query.trim() && matches.length === 0 && <small role="status">No Pi catalogue matches. You can enter settings manually.</small>}
      {applied && <small>Filled from Pi&rsquo;s {applied.sourceProvider} catalogue entry. Review these settings before saving.</small>}
      <Popover anchor={anchor} open={visible} onClose={() => { setOpen(false); setActive(-1); }} matchWidth className="model-catalog-popover">
        <div id={listId} role="listbox" aria-label="Pi model suggestions" className="model-catalog-options">
          {matches.map((suggestion, index) => (
            <button
              id={`${listId}-${index}`}
              key={`${suggestion.sourceProvider}:${suggestion.id}`}
              type="button"
              role="option"
              aria-selected={index === active}
              className={index === active ? "active" : ""}
              onMouseDown={(event) => event.preventDefault()}
              onMouseEnter={() => setActive(index)}
              onClick={() => choose(suggestion)}
            >
              <strong>{suggestion.name}</strong>
              <span>{suggestion.sourceProvider} · {suggestion.sourceApi} · {suggestion.id}</span>
            </button>
          ))}
        </div>
      </Popover>
    </div>
  );
}

// ── Subscriptions ───────────────────────────────────────────

interface CatalogProps {
  providers: SubscriptionProviderInfo[];
  error?: string;
  onConnect: (providerId: string) => Promise<void>;
  onOpenAuthUrl: (url: string) => Promise<void>;
}

function SubscriptionCatalog({ providers, error, onConnect, onOpenAuthUrl }: CatalogProps) {
  return (
    <section className="settings-block" style={stagger(1)} aria-labelledby="subscription-catalog-title">
      <h3 className="settings-block-title" id="subscription-catalog-title">Sign in with a subscription</h3>
      <p className="settings-block-sub">WackCode uses Pi&rsquo;s built-in sign-in for these providers. Your account stays separate from any Pi CLI installation.</p>
      {error && <div className="error-banner">Could not load subscription providers: {error}</div>}
      <ul className="subscription-options">
        {providers.map((provider) => (
          <li key={provider.id}>
            <span className="provider-mark" aria-hidden="true">{initialOf(provider.name)}</span>
            <div>
              <strong>{provider.name}</strong>
              <p>{provider.guidance}</p>
            </div>
            <button type="button" className="primary-button compact" onClick={() => void onConnect(provider.id)}>Sign in</button>
          </li>
        ))}
      </ul>
      <p className="subscription-billing-note">
        Anthropic may charge usage credits for third-party app access.{" "}
        <button type="button" className="text-button" onClick={() => void onOpenAuthUrl(ANTHROPIC_GUIDANCE)}>Review Anthropic&rsquo;s guidance</button>
      </p>
    </section>
  );
}

interface SubscriptionDetailProps {
  provider: SubscriptionProviderRecord;
  guidance?: string;
  onBack: () => void;
  onConnect: (providerId: string) => Promise<void>;
  onSignOut: (providerId: string) => Promise<void>;
  onDelete: (providerId: string) => Promise<void>;
  onSetProviderEnabled: (providerId: string, enabled: boolean) => Promise<void>;
  onOpenAuthUrl: (url: string) => Promise<void>;
}

function SubscriptionDetail({ provider, guidance, onBack, onConnect, onSignOut, onDelete, onSetProviderEnabled, onOpenAuthUrl }: SubscriptionDetailProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [confirm, setConfirm] = useState<"signOut" | "delete">();
  const state = connectionState(provider);

  async function action(run: () => Promise<void>) {
    setBusy(true);
    setError(undefined);
    try { await run(); }
    catch (reason) { setError(String(reason)); throw reason; }
    finally { setBusy(false); }
  }

  return (
    <div className="settings-scroll providers-settings">
      <div className="settings-page">
        <BackLink onBack={onBack} />
        <section className="settings-block provider-head" style={stagger(0)} aria-label={`${provider.name} connection`}>
          <span className="provider-mark large" aria-hidden="true">{initialOf(provider.name)}</span>
          <div className="provider-head-text">
            <h3 className="settings-block-title">{provider.name}</h3>
            <span className="provider-head-meta">Subscription · signed in through Pi</span>
            <span className="provider-head-status">
              <span className={`package-state ${state.tone}`}>{state.label}</span>
              <span>{plural(provider.models.length, "model")}</span>
            </span>
          </div>
          <span className="provider-head-switch">
            <span aria-hidden="true">Use this connection</span>
            <EnableSwitch provider={provider} busy={busy} onToggle={() => void action(() => onSetProviderEnabled(provider.id, provider.enabled === false)).catch(() => undefined)} />
          </span>
        </section>
        {error && <div className="error-banner" role="alert">{error}</div>}

        <section className="settings-block" style={stagger(1)} aria-labelledby="subscription-signin-title">
          <h3 className="settings-block-title" id="subscription-signin-title">Sign-in</h3>
          {guidance && <p className="settings-block-sub">{guidance}</p>}
          <p className="settings-block-sub">Pi manages this provider&rsquo;s models, and the list updates each time WackCode opens. Pi refreshes the credential when you send a request. Sign in again if authentication fails.</p>
          {provider.id === "anthropic" && (
            <button type="button" className="text-button subscription-guidance" onClick={() => void onOpenAuthUrl(ANTHROPIC_GUIDANCE).catch((reason) => setError(String(reason)))}>
              Review Anthropic&rsquo;s billing guidance
            </button>
          )}
          <div className="subscription-actions">
            <button type="button" className="primary-button compact" disabled={busy} onClick={() => void onConnect(provider.id)}>{provider.connected ? "Reconnect" : "Sign in"}</button>
            {provider.connected && <button type="button" className="secondary-button compact" disabled={busy} onClick={() => setConfirm("signOut")}>Sign out</button>}
            <span className="subagent-editor-spacer" />
            <button type="button" className="danger-button compact" disabled={busy} onClick={() => setConfirm("delete")}>Delete connection</button>
          </div>
        </section>

        <section className="settings-block" style={stagger(2)} aria-labelledby="subscription-models-title">
          <h3 className="settings-block-title" id="subscription-models-title">{provider.connected ? "Available models" : "Last known models"}</h3>
          {provider.models.length ? (
            <ul className="model-rows static">
              {provider.models.map((model) => (
                <li className="model-row" key={model.id}>
                  <div className="model-row-head">
                    <span className="model-row-main">
                      <span className="model-row-name">{model.name}</span>
                      {model.name !== model.id && <code>{model.id}</code>}
                      <span className="model-row-facts">
                        {modelIsReady(model) && <span className="model-row-limits">{formatTokens(model.contextWindow!)} context · {formatTokens(model.maxTokens!)} out</span>}
                        {model.reasoning && <span className="subagent-badge">Reasoning</span>}
                        {model.vision && <span className="subagent-badge">Vision</span>}
                      </span>
                    </span>
                  </div>
                </li>
              ))}
            </ul>
          ) : (
            <p className="models-empty">{provider.connected ? "No models are available to this account." : "Sign in to load the models available to this account."}</p>
          )}
        </section>
      </div>

      {confirm === "signOut" && (
        <ConfirmDialog
          title={`Sign out of ${provider.name}?`}
          body="Saved chats will remain, but they cannot use this connection until you sign in again."
          confirmLabel="Sign out"
          onConfirm={() => action(() => onSignOut(provider.id))}
          onCancel={() => setConfirm(undefined)}
        />
      )}
      {confirm === "delete" && (
        <ConfirmDialog
          title={`Delete ${provider.name}?`}
          body="This removes its WackCode sign-in. Saved chats must use another connection before deletion."
          confirmLabel="Delete"
          danger
          onConfirm={() => action(() => onDelete(provider.id))}
          onCancel={() => setConfirm(undefined)}
        />
      )}
    </div>
  );
}
