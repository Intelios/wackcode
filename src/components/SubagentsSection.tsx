import { useMemo, useState } from "react";
import { autoTitleModelIssue, modelIsReady, pickThinkingLevel, subagentModelIssue } from "../model-utils";
import {
  MAX_SUBAGENT_CONCURRENCY,
  READ_ONLY_SUBAGENT_TOOLS,
  SUBAGENT_TOOLS,
  type AutoTitleConfig,
  type ProviderRecord,
  type SubagentConfig,
  type SubagentModel,
  type SubagentRecord,
  type ThinkingLevel
} from "../types";
import { DuckMark } from "./DuckMark";
import { Icon, type IconName } from "./Icons";
import { ModelPicker, ReasoningToggle, type ModelFavoritesProps } from "./ModelPicker";
import { RobotMark } from "./RobotMark";
import { SettingsHero, stagger } from "./SettingsHero";
import { ConfirmDialog } from "./ui/ConfirmDialog";

interface Props extends ModelFavoritesProps {
  config: SubagentConfig;
  providers: ProviderRecord[];
  /** Saves the whole configuration; rejects with a user-facing message. */
  onChange: (config: SubagentConfig) => Promise<void>;
  /** False while Web Fetch is switched off in Packages, which keeps web_fetch from every agent. */
  webFetchEnabled?: boolean;
  /** Automatic titles, shown as an agent WackCode runs itself — it is never offered to the model. */
  autoTitle: AutoTitleConfig;
  onSetAutoTitle: (config: AutoTitleConfig) => Promise<void>;
  /** Opens Settings → Providers on a blank connection, for when nothing usable is connected. */
  onOpenProviders: () => void;
  /** The agent's name in the app's own copy (Settings › Appearance). */
  agentName?: string;
}

/** Shown under an agent's tools when it lists web_fetch but Web Fetch is switched off. */
function WebFetchOffHint({ tools, webFetchEnabled }: { tools: string[]; webFetchEnabled: boolean }) {
  if (webFetchEnabled || !tools.includes("web_fetch")) return null;
  return <small className="subagent-hint">Web Fetch is off in Settings › Packages, so no sub-agent gets web_fetch.</small>;
}

const TRIGGER_OPTIONS: { value: SubagentConfig["trigger"]; label: string; hint: string; icon: IconName }[] = [
  { value: "on_request", label: "Only when I ask", hint: "Predictable cost", icon: "comment" },
  { value: "auto", label: "Whenever useful", hint: "The agent decides", icon: "spark" }
];

const CONCURRENCY_OPTIONS = Array.from({ length: MAX_SUBAGENT_CONCURRENCY }, (_, index) => index + 1);

/**
 * The hero's little stage: the duck hands work to three robots. As many as run at the same time
 * (up to three) are awake, scanning, their progress bars filling in turn; the rest doze. With every
 * agent off they all doze and the lines rest dashed. Pure decoration; the pill says the same in
 * words. The loop lives in styles.css, which stills it (and the robots) under reduced motion.
 */
function SubagentsStage({ working }: { working: number }) {
  const lanes = [16, 44, 72];
  return (
    <svg className={`settings-stage agents-stage ${working > 0 ? "live" : ""}`} viewBox="0 0 160 110" aria-hidden="true">
      {lanes.map((y, index) => {
        const awake = index < working;
        return (
          <g key={y} className={`agents-stage-lane ${awake ? "on" : ""}`} style={{ "--k": index } as React.CSSProperties}>
            <path className="agents-stage-line" d={`M58 55C80 55 76 ${y + 12} 94 ${y + 12}`} />
            <g transform={`translate(94 ${y}) scale(1.35)`}>
              <RobotMark status={awake ? "running" : "queued"} />
            </g>
            <rect className="agents-stage-track" x="127" y={y + 10} width="22" height="4" rx="2" />
            <rect className="agents-stage-fill" x="127" y={y + 10} width="22" height="4" rx="2" />
          </g>
        );
      })}
      <rect className="agents-stage-tile" x="14" y="33" width="44" height="44" rx="12" />
      <DuckMark x="24" y="43" width="24" height="24" />
    </svg>
  );
}

/** A custom agent being created or edited, before it is saved. */
interface AgentDraft {
  /** Empty for a new agent: the host assigns one. */
  id: string;
  name: string;
  description: string;
  prompt: string;
  tools: string[];
  readOnly: boolean;
  model: SubagentModel | null;
}

function draftFrom(agent: SubagentRecord): AgentDraft {
  return {
    id: agent.id,
    name: agent.name,
    description: agent.description,
    prompt: agent.prompt,
    tools: [...agent.tools],
    readOnly: agent.readOnly,
    model: agent.model
  };
}

function uniqueName(base: string, agents: SubagentRecord[]): string {
  const taken = new Set(agents.map((agent) => agent.name));
  const stem = base.slice(0, 26) || "agent";
  if (!taken.has(stem)) return stem;
  for (let suffix = 2; ; suffix += 1) {
    const candidate = `${stem}-${suffix}`;
    if (!taken.has(candidate)) return candidate;
  }
}

function modelSummary(model: SubagentModel | null, providers: ProviderRecord[]): string {
  if (!model) return "Chat's model";
  const provider = providers.find((item) => item.id === model.providerId);
  const record = provider?.models.find((item) => item.id === model.modelId);
  const name = record?.name || model.modelId;
  const level = model.thinkingLevel === "off" ? "" : ` · ${model.thinkingLevel}`;
  return `${provider?.name ?? "Missing connection"} · ${name}${level}`;
}

interface ModelFieldProps extends ModelFavoritesProps {
  agentName: string;
  model: SubagentModel | null;
  providers: ProviderRecord[];
  disabled: boolean;
  onChange: (model: SubagentModel | null) => void;
}

/** "Chat's model", or a model of the agent's own from any connected provider. */
function ModelField({ agentName, model, providers, favoriteModels, favoriteSaving, onSetFavorite, disabled, onChange }: ModelFieldProps) {
  const usable = useMemo(() => providers.filter((provider) => provider.enabled !== false && provider.connected && provider.models.some(modelIsReady)), [providers]);

  function choose(patch: { providerId?: string; modelId?: string; thinkingLevel?: ThinkingLevel }) {
    const providerId = patch.providerId ?? model?.providerId ?? usable[0]?.id;
    const provider = usable.find((item) => item.id === providerId);
    const modelId = patch.modelId ?? (patch.providerId || !model ? provider?.models.find(modelIsReady)?.id : model.modelId);
    const record = provider?.models.find((item) => item.id === modelId);
    if (!providerId || !modelId) return;
    onChange({ providerId, modelId, thinkingLevel: pickThinkingLevel(record, patch.thinkingLevel, model?.thinkingLevel) });
  }

  return (
    <div className="subagent-model-field">
      <span className="subagent-field-label">Model</span>
      <div className="subagent-model-row">
        <div className="mode-toggle" role="radiogroup" aria-label={`${agentName} model`}>
          <button type="button" role="radio" aria-checked={!model} className={`mode-option ${model ? "" : "active"}`} disabled={disabled} onClick={() => onChange(null)}>
            Chat's model
          </button>
          <button
            type="button"
            role="radio"
            aria-checked={Boolean(model)}
            className={`mode-option ${model ? "active" : ""}`}
            disabled={disabled || usable.length === 0}
            onClick={() => { if (!model) choose({}); }}
          >
            Specific model
          </button>
        </div>
        {model && (
          <>
            <ModelPicker providers={usable} favoriteModels={favoriteModels} favoriteSaving={favoriteSaving} onSetFavorite={onSetFavorite} providerId={model.providerId} modelId={model.modelId} disabled={disabled} popoverSide="bottom" onConfigure={choose} />
            <ReasoningToggle
              providers={usable}
              providerId={model.providerId}
              modelId={model.modelId}
              thinkingLevel={model.thinkingLevel}
              disabled={disabled}
              popoverSide="bottom"
              onConfigure={choose}
            />
          </>
        )}
      </div>
      <small className="subagent-hint">
        {model ? "Used whenever this agent runs, whatever model the chat is on." : "Runs on whatever model and reasoning level the chat uses."}
      </small>
    </div>
  );
}

interface EditorProps extends ModelFavoritesProps {
  draft: AgentDraft;
  providers: ProviderRecord[];
  busy: boolean;
  isNew: boolean;
  onChange: (draft: AgentDraft) => void;
  onSave: () => void;
  onCancel: () => void;
  onDelete?: () => void;
  webFetchEnabled: boolean;
}

/** Name, description, instructions, tools and model of a custom agent. */
function AgentEditor({ draft, providers, favoriteModels, favoriteSaving, onSetFavorite, busy, isNew, onChange, onSave, onCancel, onDelete, webFetchEnabled }: EditorProps) {
  const toggleTool = (tool: string) =>
    onChange({ ...draft, tools: draft.tools.includes(tool) ? draft.tools.filter((item) => item !== tool) : [...draft.tools, tool] });
  const setReadOnly = (readOnly: boolean) =>
    onChange({ ...draft, readOnly, tools: readOnly ? draft.tools.filter((tool) => READ_ONLY_SUBAGENT_TOOLS.includes(tool)) : draft.tools });

  return (
    <div className="subagent-editor">
      <div>
        <h3 className="settings-block-title">{isNew ? "New agent" : "Edit agent"}</h3>
        <p className="settings-block-sub subagent-editor-sub">Saved agents are offered to the model from a chat&rsquo;s next turn.</p>
      </div>
      <div className="form-grid">
        <label>
          <span>Name <small>Lowercase letters, digits and hyphens</small></span>
          <input value={draft.name} onChange={(event) => onChange({ ...draft, name: event.target.value.toLowerCase() })} placeholder="docs-checker" spellCheck={false} maxLength={32} />
        </label>
        <label>
          <span>Description <small>Tells the agent when to use it</small></span>
          <input value={draft.description} onChange={(event) => onChange({ ...draft, description: event.target.value })} placeholder="Checks the docs match the code" maxLength={400} />
        </label>
        <label className="wide-field">
          <span>Instructions <small>Added to the sub-agent's system prompt</small></span>
          <textarea
            className="subagent-prompt-input"
            value={draft.prompt}
            onChange={(event) => onChange({ ...draft, prompt: event.target.value })}
            placeholder="You are a sub-agent that…"
            rows={8}
          />
        </label>
      </div>
      <div className="subagent-tools-field">
        <span className="subagent-field-label">Tools</span>
        <div className="subagent-tool-chips" role="group" aria-label="Tools">
          {SUBAGENT_TOOLS.map((tool) => {
            const locked = draft.readOnly && !READ_ONLY_SUBAGENT_TOOLS.includes(tool);
            return (
              <button
                key={tool}
                type="button"
                aria-pressed={draft.tools.includes(tool)}
                className={draft.tools.includes(tool) ? "selected" : ""}
                disabled={busy || locked}
                title={locked ? "Allow editing files to give this agent edit and write" : undefined}
                onClick={() => toggleTool(tool)}
              >
                {tool}
              </button>
            );
          })}
        </div>
        <WebFetchOffHint tools={draft.tools} webFetchEnabled={webFetchEnabled} />
        <label className="subagent-inline-switch">
          <button
            type="button"
            role="switch"
            aria-checked={!draft.readOnly}
            aria-label="Can edit files"
            className={`toggle ${draft.readOnly ? "" : "on"}`}
            disabled={busy}
            onClick={() => setReadOnly(!draft.readOnly)}
          >
            <span />
          </button>
          <span>Can edit files <small>{draft.readOnly ? "Read-only: allowed in Plan mode, and runs in parallel" : "Can run in parallel on separate files; can't run in Plan mode"}</small></span>
        </label>
      </div>
      <ModelField agentName={draft.name || "New agent"} model={draft.model} providers={providers} favoriteModels={favoriteModels} favoriteSaving={favoriteSaving} onSetFavorite={onSetFavorite} disabled={busy} onChange={(model) => onChange({ ...draft, model })} />
      <div className="subagent-editor-actions">
        {onDelete && <button type="button" className="danger-button" disabled={busy} onClick={onDelete}>Delete</button>}
        <span className="subagent-editor-spacer" />
        <button type="button" className="secondary-button" disabled={busy} onClick={onCancel}>Cancel</button>
        <button
          type="button"
          className="primary-button"
          disabled={busy || !draft.name.trim() || !draft.description.trim() || !draft.prompt.trim()}
          onClick={onSave}
        >
          {isNew ? "Add agent" : "Save"}
        </button>
      </div>
    </div>
  );
}

interface AutoTitleCardProps extends ModelFavoritesProps {
  config: AutoTitleConfig;
  providers: ProviderRecord[];
  busy: boolean;
  open: boolean;
  onToggleOpen: () => void;
  onSave: (config: AutoTitleConfig) => Promise<void>;
  onOpenProviders: () => void;
}

/**
 * Automatic titles as an agent card: WackCode runs it on a new chat's first prompt rather
 * than the model calling it, so it needs its own model and has no tools or instructions.
 */
function AutoTitleCard({ config, providers, favoriteModels, favoriteSaving, onSetFavorite, busy, open, onToggleOpen, onSave, onOpenProviders }: AutoTitleCardProps) {
  const available = providers.filter((provider) => provider.enabled !== false && provider.connected && provider.models.some(modelIsReady));
  const provider = providers.find((item) => item.id === config.providerId);
  const record = provider?.models.find((item) => item.id === config.modelId);
  const issue = autoTitleModelIssue(config, providers);
  const ready = Boolean(provider?.enabled !== false && provider?.connected && record && modelIsReady(record));
  const summary = config.providerId && config.modelId
    ? `${provider?.name ?? "Missing connection"} · ${record?.name || config.modelId}`
    : "No model chosen";

  function choose(patch: { providerId?: string; modelId?: string }) {
    const providerId = patch.providerId ?? config.providerId;
    const modelId = patch.modelId ?? (patch.providerId ? available.find((item) => item.id === providerId)?.models.find(modelIsReady)?.id : config.modelId);
    if (!providerId || !modelId) return;
    void onSave({ ...config, providerId, modelId });
  }

  return (
    <article className={`subagent-setting ${open ? "open" : ""} ${config.enabled ? "" : "off"}`}>
      <div className="subagent-setting-head">
        <button type="button" className={`package-disclosure ${open ? "open" : ""}`} aria-expanded={open} onClick={onToggleOpen}>
          <Icon name="chevron" />
          <span className="subagent-setting-name">auto-titles</span>
          <span className="subagent-badge">Built-in</span>
          <span className="subagent-badge">Not callable</span>
        </button>
        <span className={`subagent-model-summary ${issue ? "warning" : ""}`} title={issue}>
          {summary}
        </span>
        <button
          type="button"
          role="switch"
          aria-checked={config.enabled}
          aria-label="Automatic titles"
          className={`toggle ${config.enabled ? "on" : ""}`}
          disabled={busy || (!ready && !config.enabled)}
          title={!ready && !config.enabled ? "Choose a title model first" : undefined}
          onClick={() => void onSave({ ...config, enabled: !config.enabled })}
        >
          <span />
        </button>
      </div>
      <p className="subagent-setting-description">
        Names each new chat from its first message — one extra request on its own model, while the chat responds.
      </p>
      {issue && <p className="subagent-issue">{issue} Automatic titles can't run until you pick another model.</p>}
      {open && (
        <div className="subagent-setting-body">
          <div className="subagent-model-field">
            <span className="subagent-field-label">Model</span>
            <div className="subagent-model-row">
              {available.length > 0 ? (
                <ModelPicker providers={available} favoriteModels={favoriteModels} favoriteSaving={favoriteSaving} onSetFavorite={onSetFavorite} providerId={config.providerId ?? ""} modelId={config.modelId ?? ""} disabled={busy} popoverSide="bottom" onConfigure={choose} />
              ) : (
                <button type="button" className="secondary-button" onClick={onOpenProviders}>Add a connection</button>
              )}
            </div>
            <small className="subagent-hint">
              {available.length > 0
                ? "It always needs a model of its own — a small, inexpensive one is enough."
                : "Automatic titles needs a connected provider and model."}
            </small>
          </div>
          <div>
            <span className="subagent-field-label">How it runs</span>
            <small className="subagent-hint">
              On a new chat's first prompt only that prompt's text goes to the title model, in one extra request.
              Each chat gets one attempt, even if it fails — until a title lands the free title from the opening
              prompt stays, and a name you typed always wins.
            </small>
          </div>
        </div>
      )}
    </article>
  );
}

/**
 * Settings → Sub-agents. Only shown while the built-in is switched on (from Settings →
 * Packages). Switches and model choices save at once; a custom agent's text is edited as a
 * draft and saved explicitly.
 */
export function SubagentsSection({ config, providers, favoriteModels, favoriteSaving, onSetFavorite, onChange, webFetchEnabled = true, autoTitle, onSetAutoTitle, onOpenProviders, agentName = "WackCode" }: Props) {
  const favorites = { favoriteModels, favoriteSaving, onSetFavorite };
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [expanded, setExpanded] = useState<string>();
  /** The custom agent being edited (by id), or "new" for one not yet saved. */
  const [editing, setEditing] = useState<{ key: string; draft: AgentDraft }>();
  const [deleting, setDeleting] = useState<SubagentRecord>();

  async function commit(next: SubagentConfig): Promise<boolean> {
    setBusy(true);
    setError(undefined);
    try {
      await onChange(next);
      return true;
    } catch (reason) {
      setError(String(reason));
      return false;
    } finally {
      setBusy(false);
    }
  }

  const updateAgent = (id: string, patch: Partial<SubagentRecord>) =>
    void commit({ ...config, agents: config.agents.map((agent) => agent.id === id ? { ...agent, ...patch } : agent) });

  async function commitTitle(next: AutoTitleConfig): Promise<void> {
    setBusy(true);
    setError(undefined);
    try {
      await onSetAutoTitle(next);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  function startNew() {
    setEditing({
      key: "new",
      draft: {
        id: "",
        name: uniqueName("agent", config.agents),
        description: "",
        prompt: "",
        tools: ["read", "grep", "find", "ls", "bash"],
        readOnly: true,
        model: null
      }
    });
    setExpanded(undefined);
  }

  function duplicate(agent: SubagentRecord) {
    setEditing({ key: "new", draft: { ...draftFrom(agent), id: "", name: uniqueName(`${agent.name}-custom`, config.agents) } });
    setExpanded(undefined);
  }

  async function saveDraft() {
    if (!editing) return;
    const { draft } = editing;
    const record: SubagentRecord = { ...draft, builtin: false, enabled: true, name: draft.name.trim() };
    const agents = editing.key === "new"
      ? [...config.agents, record]
      : config.agents.map((agent) => agent.id === editing.key ? { ...record, enabled: agent.enabled } : agent);
    if (await commit({ ...config, agents })) {
      setEditing(undefined);
      if (editing.key !== "new") setExpanded(editing.key);
    }
  }

  const builtins = config.agents.filter((agent) => agent.builtin);
  const customs = config.agents.filter((agent) => !agent.builtin);

  function renderAgent(agent: SubagentRecord) {
    const open = expanded === agent.id;
    const issue = subagentModelIssue(agent.model, providers);
    if (editing?.key === agent.id) {
      return (
        <article className="subagent-setting open" key={agent.id}>
          <AgentEditor
            {...favorites}
            draft={editing.draft}
            providers={providers}
            busy={busy}
            isNew={false}
            webFetchEnabled={webFetchEnabled}
            onChange={(draft) => setEditing({ key: agent.id, draft })}
            onSave={() => void saveDraft()}
            onCancel={() => setEditing(undefined)}
            onDelete={() => setDeleting(agent)}
          />
        </article>
      );
    }
    return (
      <article className={`subagent-setting ${open ? "open" : ""} ${agent.enabled ? "" : "off"}`} key={agent.id}>
        <div className="subagent-setting-head">
          <button type="button" className={`package-disclosure ${open ? "open" : ""}`} aria-expanded={open} onClick={() => setExpanded(open ? undefined : agent.id)}>
            <Icon name="chevron" />
            <span className="subagent-setting-name">{agent.name}</span>
            <span className="subagent-badge">{agent.builtin ? "Built-in" : "Custom"}</span>
            <span className={`subagent-badge ${agent.readOnly ? "" : "edits"}`}>{agent.readOnly ? "Read-only" : "Edits files"}</span>
          </button>
          <span className={`subagent-model-summary ${issue ? "warning" : ""}`} title={issue}>
            {modelSummary(agent.model, providers)}
          </span>
          <button
            type="button"
            role="switch"
            aria-checked={agent.enabled}
            aria-label={`Use ${agent.name}`}
            className={`toggle ${agent.enabled ? "on" : ""}`}
            disabled={busy}
            onClick={() => updateAgent(agent.id, { enabled: !agent.enabled })}
          >
            <span />
          </button>
        </div>
        <p className="subagent-setting-description">{agent.description}</p>
        {issue && <p className="subagent-issue">{issue} It will refuse to run until you pick another model.</p>}
        {open && (
          <div className="subagent-setting-body">
            <ModelField {...favorites} agentName={agent.name} model={agent.model} providers={providers} disabled={busy} onChange={(model) => updateAgent(agent.id, { model })} />
            <div className="subagent-tools-field">
              <span className="subagent-field-label">Tools</span>
              <div className="subagent-tool-list">{agent.tools.join(", ") || "None"}</div>
              <WebFetchOffHint tools={agent.tools} webFetchEnabled={webFetchEnabled} />
            </div>
            <div className="subagent-prompt-field">
              <span className="subagent-field-label">Instructions</span>
              <pre className="subagent-prompt-view">{agent.prompt}</pre>
            </div>
            <div className="subagent-editor-actions">
              {agent.builtin ? (
                <>
                  <small className="subagent-hint">Built-in agents are updated with the app. Duplicate one to change its instructions or tools.</small>
                  <span className="subagent-editor-spacer" />
                  <button type="button" className="secondary-button" disabled={busy} onClick={() => duplicate(agent)}><Icon name="copy" /> Duplicate as custom</button>
                </>
              ) : (
                <>
                  <button type="button" className="danger-button" disabled={busy} onClick={() => setDeleting(agent)}>Delete</button>
                  <span className="subagent-editor-spacer" />
                  <button type="button" className="secondary-button" disabled={busy} onClick={() => duplicate(agent)}><Icon name="copy" /> Duplicate</button>
                  <button type="button" className="secondary-button" disabled={busy} onClick={() => setEditing({ key: agent.id, draft: draftFrom(agent) })}><Icon name="pencil" /> Edit</button>
                </>
              )}
            </div>
          </div>
        )}
      </article>
    );
  }

  const enabledAgents = config.agents.filter((agent) => agent.enabled).length;
  const pill = enabledAgents === config.agents.length ? "All on"
    : enabledAgents === 0 ? "All off"
    : `${enabledAgents} of ${config.agents.length} on`;

  return (
    <div className="settings-scroll subagents-settings">
      <div className="settings-page">
        <SettingsHero
          label="Sub-agents overview"
          stage={<SubagentsStage working={Math.min(enabledAgents, config.maxConcurrency, 3)} />}
          live={enabledAgents > 0}
          pill={pill}
          title={`Helpers ${agentName} can hand work to`}
          action={(
            <button type="button" className="secondary-button compact" disabled={busy || editing?.key === "new"} onClick={startNew}>
              <Icon name="plus" /> New agent
            </button>
          )}
        >
          <p>
            A sub-agent works on one self-contained task in its own context window and hands back its answer. Each one is extra
            model usage, billed like any other request.
          </p>
        </SettingsHero>
        {error && <div className="error-banner" role="alert">{error}</div>}

        <section className="settings-block" style={stagger(1)} aria-labelledby="subagents-usage-title">
          <h3 className="settings-block-title" id="subagents-usage-title">How they&rsquo;re used</h3>
          <p className="settings-block-sub">Changes apply to running chats on their next turn.</p>
          <span className="subagent-field-label">When to use them</span>
          <div className="subagent-trigger" role="radiogroup" aria-label="When to use sub-agents">
            {TRIGGER_OPTIONS.map((option) => {
              const selected = config.trigger === option.value;
              return (
                <button
                  key={option.value}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  className={`subagent-trigger-option ${selected ? "selected" : ""}`}
                  disabled={busy}
                  onClick={() => { if (!selected) void commit({ ...config, trigger: option.value }); }}
                >
                  <span className="subagent-trigger-mark" aria-hidden="true"><Icon name={option.icon} /></span>
                  <span className="subagent-trigger-text">
                    <strong>{option.label}</strong>
                    <small>{option.hint}</small>
                  </span>
                </button>
              );
            })}
          </div>
          <small className="subagent-hint subagent-usage-hint">
            {config.trigger === "auto"
              ? "The agent delegates on its own when a task clearly benefits, such as broad exploration or an independent review."
              : "The agent only delegates when you ask for sub-agents, or name one."}
          </small>
          <span className="subagent-field-label">Run at the same time</span>
          <div className="subagent-concurrency" role="radiogroup" aria-label="Sub-agents running at the same time">
            {CONCURRENCY_OPTIONS.map((count) => (
              <button
                key={count}
                type="button"
                role="radio"
                aria-checked={config.maxConcurrency === count}
                className={config.maxConcurrency === count ? "selected" : count < config.maxConcurrency ? "within" : ""}
                disabled={busy}
                onClick={() => { if (config.maxConcurrency !== count) void commit({ ...config, maxConcurrency: count }); }}
              >
                {count}
              </button>
            ))}
          </div>
          <small className="subagent-hint subagent-usage-hint">Per request, for all agents. Give parallel editing agents separate files.</small>
        </section>

        {editing?.key === "new" && (
          <article className="subagent-setting open new">
            <AgentEditor
              {...favorites}
              draft={editing.draft}
              providers={providers}
              busy={busy}
              isNew
              webFetchEnabled={webFetchEnabled}
              onChange={(draft) => setEditing({ key: "new", draft })}
              onSave={() => void saveDraft()}
              onCancel={() => { setEditing(undefined); setError(undefined); }}
            />
          </article>
        )}

        <section className="settings-block" style={stagger(2)} aria-labelledby="subagents-agents-title">
          <h3 className="settings-block-title" id="subagents-agents-title">Agents</h3>
          <p className="settings-block-sub">Switched-off agents are not offered to the model. Each one uses the chat&rsquo;s model unless you give it its own.</p>
          <section className="subagent-group" aria-label="Built-in agents">
            <h4>Built-in</h4>
            {builtins.map(renderAgent)}
          </section>
          {customs.length > 0 && (
            <section className="subagent-group" aria-label="Custom agents">
              <h4>Custom</h4>
              {customs.map(renderAgent)}
            </section>
          )}
        </section>

        <section className="settings-block" style={stagger(3)} aria-labelledby="subagents-auto-title">
          <h3 className="settings-block-title" id="subagents-auto-title">Automatic</h3>
          <p className="settings-block-sub">WackCode runs this one itself; the model can never call it.</p>
          <AutoTitleCard
            {...favorites}
            config={autoTitle}
            providers={providers}
            busy={busy}
            open={expanded === "auto-titles"}
            onToggleOpen={() => setExpanded(expanded === "auto-titles" ? undefined : "auto-titles")}
            onSave={commitTitle}
            onOpenProviders={onOpenProviders}
          />
        </section>
      </div>

      {deleting && (
        <ConfirmDialog
          title={`Delete ${deleting.name}?`}
          body="The agent will no longer be offered to the model. Chats that already used it keep their results."
          confirmLabel="Delete"
          danger
          onConfirm={async () => {
            await onChange({ ...config, agents: config.agents.filter((agent) => agent.id !== deleting.id) });
            if (editing?.key === deleting.id) setEditing(undefined);
            setDeleting(undefined);
          }}
          onCancel={() => setDeleting(undefined)}
        />
      )}
    </div>
  );
}
