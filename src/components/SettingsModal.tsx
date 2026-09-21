import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { mergeDiscoveredModels, modelIsReady } from "../model-utils";
import type { ApiFormat, ModelRecord, ProviderRecord, SaveProviderInput, ThinkingLevel } from "../types";
import { Icon } from "./Icons";

const levels: ThinkingLevel[] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

interface Draft extends SaveProviderInput {
  id?: string;
}

interface Props {
  providers: ProviderRecord[];
  appDataPath: string;
  onClose: () => void;
  onSave: (input: SaveProviderInput) => Promise<ProviderRecord>;
  onDelete: (providerId: string) => Promise<void>;
}

function blankDraft(): Draft {
  return { name: "", baseUrl: "", apiFormat: "openai-completions", apiKey: "", models: [] };
}

function fromProvider(provider: ProviderRecord): Draft {
  return {
    id: provider.id,
    name: provider.name,
    baseUrl: provider.baseUrl,
    apiFormat: provider.apiFormat,
    apiKey: "",
    models: provider.models.map((model) => ({
      ...model,
      thinkingLevels: [...model.thinkingLevels],
      thinkingLevelMap: { ...model.thinkingLevelMap }
    }))
  };
}

export function SettingsModal({ providers, appDataPath, onClose, onSave, onDelete }: Props) {
  const [selectedId, setSelectedId] = useState(providers[0]?.id ?? "new");
  const selected = providers.find((provider) => provider.id === selectedId);
  const [draft, setDraft] = useState<Draft>(() => selected ? fromProvider(selected) : blankDraft());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();

  useEffect(() => {
    const provider = providers.find((item) => item.id === selectedId);
    setDraft(provider ? fromProvider(provider) : blankDraft());
    setError(undefined);
    setNotice(undefined);
  }, [selectedId, providers]);

  const incomplete = useMemo(() => draft.models.filter((model) => !modelIsReady(model)).length, [draft.models]);

  async function save(): Promise<ProviderRecord | undefined> {
    setBusy(true);
    setError(undefined);
    setNotice(undefined);
    try {
      const saved = await onSave({ ...draft, apiKey: draft.apiKey?.trim() || undefined });
      setSelectedId(saved.id);
      setDraft(fromProvider(saved));
      setNotice("Connection saved. The API key is held in macOS Keychain.");
      return saved;
    } catch (reason) {
      setError(String(reason));
      return undefined;
    } finally {
      setBusy(false);
    }
  }

  async function fetchModels() {
    const saved = await save();
    if (!saved) return;
    setBusy(true);
    setError(undefined);
    try {
      const ids = await api.discoverModels(saved.id);
      const models = mergeDiscoveredModels(saved.models, ids);
      setDraft({ ...fromProvider(saved), models });
      setNotice(`Found ${ids.length} model${ids.length === 1 ? "" : "s"}. Confirm limits for new entries, then save.`);
    } catch (reason) {
      setError(`${String(reason)} Manual model entry is still available below.`);
    } finally {
      setBusy(false);
    }
  }

  function updateModel(index: number, patch: Partial<ModelRecord>) {
    setDraft((current) => ({
      ...current,
      models: current.models.map((model, modelIndex) => modelIndex === index ? { ...model, ...patch } : model)
    }));
  }

  function toggleLevel(index: number, level: ThinkingLevel) {
    const model = draft.models[index];
    const present = model.thinkingLevels.includes(level);
    const next = present ? model.thinkingLevels.filter((item) => item !== level) : [...model.thinkingLevels, level];
    const thinkingLevelMap = { ...model.thinkingLevelMap };
    if (present) delete thinkingLevelMap[level];
    else thinkingLevelMap[level] = level === "off" ? null : level;
    if (next.length === 0) thinkingLevelMap.off = null;
    updateModel(index, { thinkingLevels: next.length ? next : ["off"], thinkingLevelMap });
  }

  function updateThinkingMapping(index: number, level: ThinkingLevel, value: string) {
    const model = draft.models[index];
    updateModel(index, {
      thinkingLevelMap: {
        ...model.thinkingLevelMap,
        [level]: value.trim() ? value : level === "off" ? null : level
      }
    });
  }

  async function removeProvider() {
    if (!draft.id || !window.confirm(`Delete the “${draft.name}” connection and its Keychain credential?`)) return;
    setBusy(true);
    setError(undefined);
    try {
      await onDelete(draft.id);
      setSelectedId("new");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="settings-modal" role="dialog" aria-modal="true" aria-label="WackCode settings">
        <aside className="settings-nav">
          <div className="settings-nav-heading">Connections</div>
          {providers.map((provider) => (
            <button key={provider.id} className={`settings-nav-item ${selectedId === provider.id ? "active" : ""}`} onClick={() => setSelectedId(provider.id)}>
              <span className={`credential-dot ${provider.hasApiKey ? "connected" : ""}`} />
              <span>{provider.name}</span>
            </button>
          ))}
          <button className={`settings-nav-item ${selectedId === "new" ? "active" : ""}`} onClick={() => setSelectedId("new")}>
            <Icon name="plus" /> New connection
          </button>
          <div className="settings-privacy">
            <strong>Local by design</strong>
            <span>Settings live in</span>
            <code title={appDataPath}>{appDataPath}</code>
          </div>
        </aside>

        <div className="settings-content">
          <header className="modal-header">
            <div>
              <span className="eyebrow">Provider settings</span>
              <h2>{draft.id ? draft.name || "Connection" : "New connection"}</h2>
            </div>
            <button className="icon-button close-button" onClick={onClose} aria-label="Close settings">×</button>
          </header>

          <div className="settings-scroll">
            <div className="form-grid connection-grid">
              <label>
                <span>Name</span>
                <input value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} placeholder="My inference gateway" />
              </label>
              <label>
                <span>API format</span>
                <select value={draft.apiFormat} onChange={(event) => setDraft({ ...draft, apiFormat: event.target.value as ApiFormat })}>
                  <option value="openai-completions">Chat Completions compatible</option>
                  <option value="openai-responses">Responses compatible</option>
                </select>
              </label>
              <label className="wide-field">
                <span>Base URL</span>
                <input value={draft.baseUrl} onChange={(event) => setDraft({ ...draft, baseUrl: event.target.value })} placeholder="https://api.example.com/v1" spellCheck={false} />
              </label>
              <label className="wide-field">
                <span>API key <small>{draft.id && selected?.hasApiKey ? "Leave blank to keep the saved key" : "Stored in macOS Keychain"}</small></span>
                <div className="input-with-icon">
                  <Icon name="key" />
                  <input type="password" autoComplete="off" value={draft.apiKey ?? ""} onChange={(event) => setDraft({ ...draft, apiKey: event.target.value })} placeholder={selected?.hasApiKey ? "••••••••••••••••" : "Enter API key"} />
                </div>
              </label>
            </div>

            <div className="section-heading-row">
              <div>
                <h3>Models</h3>
                <p>Discovery adds IDs only. WackCode waits for you to confirm limits and reasoning support.</p>
              </div>
              <div className="row-actions">
                <button className="secondary-button" disabled={busy} onClick={fetchModels}><Icon name="refresh" /> Fetch models</button>
                <button className="secondary-button" onClick={() => setDraft({ ...draft, models: [...draft.models, {
                  id: "", name: "", contextWindow: null, maxTokens: null, reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: { off: null }
                }] })}><Icon name="plus" /> Add manually</button>
              </div>
            </div>

            {draft.models.length === 0 ? (
              <div className="model-empty">Fetch from the provider or add a model ID manually.</div>
            ) : (
              <div className="model-list">
                {draft.models.map((model, index) => (
                  <article className={`model-card ${modelIsReady(model) ? "" : "incomplete"}`} key={`${model.id}-${index}`}>
                    <div className="model-card-top">
                      <div className="model-index">{String(index + 1).padStart(2, "0")}</div>
                      <label><span>Model ID</span><input value={model.id} onChange={(event) => updateModel(index, { id: event.target.value })} placeholder="provider/model-id" spellCheck={false} /></label>
                      <label><span>Display name</span><input value={model.name} onChange={(event) => updateModel(index, { name: event.target.value })} placeholder={model.id || "Model name"} /></label>
                      <button className="icon-button" aria-label="Remove model" onClick={() => setDraft({ ...draft, models: draft.models.filter((_, modelIndex) => modelIndex !== index) })}><Icon name="trash" /></button>
                    </div>
                    <div className="model-limits">
                      <label><span>Context tokens</span><input type="number" min="1" value={model.contextWindow ?? ""} onChange={(event) => updateModel(index, { contextWindow: event.target.value ? Number(event.target.value) : null })} placeholder="Required" /></label>
                      <label><span>Max output tokens</span><input type="number" min="1" value={model.maxTokens ?? ""} onChange={(event) => updateModel(index, { maxTokens: event.target.value ? Number(event.target.value) : null })} placeholder="Required" /></label>
                      <label className="reasoning-toggle"><span>Reasoning</span><button className={`toggle ${model.reasoning ? "on" : ""}`} onClick={() => updateModel(index, {
                        reasoning: !model.reasoning,
                        thinkingLevels: !model.reasoning ? ["off", "low", "medium", "high"] : ["off"],
                        thinkingLevelMap: !model.reasoning ? { off: null, low: "low", medium: "medium", high: "high" } : { off: null }
                      })} type="button"><span /></button></label>
                    </div>
                    {model.reasoning && (
                      <>
                        <div className="reasoning-levels">
                          <span>Supported efforts</span>
                          {levels.map((level) => <button key={level} type="button" className={model.thinkingLevels.includes(level) ? "selected" : ""} onClick={() => toggleLevel(index, level)}>{level}</button>)}
                        </div>
                        <div className="reasoning-mappings">
                          <span>Provider values</span>
                          {model.thinkingLevels.map((level) => (
                            <label key={level}>
                              <span>{level}</span>
                              <input
                                value={model.thinkingLevelMap[level] ?? ""}
                                onChange={(event) => updateThinkingMapping(index, level, event.target.value)}
                                placeholder={level === "off" ? "omit" : level}
                                spellCheck={false}
                              />
                            </label>
                          ))}
                          <small>Blank “off” omits reasoning; other blanks use the effort name.</small>
                        </div>
                      </>
                    )}
                    {!modelIsReady(model) && <div className="model-warning">Confirm context and output limits before this model can be used.</div>}
                  </article>
                ))}
              </div>
            )}
          </div>

          <footer className="modal-footer">
            <div className="form-status">
              {error && <span className="error-text">{error}</span>}
              {!error && notice && <span className="success-text">{notice}</span>}
              {!error && !notice && incomplete > 0 && <span>{incomplete} model{incomplete === 1 ? " needs" : "s need"} limits</span>}
            </div>
            {draft.id && <button className="danger-button" disabled={busy} onClick={removeProvider}>Delete</button>}
            <button className="primary-button" disabled={busy || !draft.name.trim() || !draft.baseUrl.trim()} onClick={save}>{busy ? "Working…" : "Save connection"}</button>
          </footer>
        </div>
      </section>
    </div>
  );
}
