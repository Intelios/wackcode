import { useState } from "react";
import { modelIsReady } from "../model-utils";
import type { AutoTitleConfig, ProviderRecord } from "../types";
import { Icon } from "./Icons";
import { ModelPicker } from "./ModelPicker";

interface Props {
  config: AutoTitleConfig;
  providers: ProviderRecord[];
  onChange: (config: AutoTitleConfig) => Promise<void>;
  onOpenProviders: () => void;
}

export function AutoTitlesSection({ config, providers, onChange, onOpenProviders }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const available = providers.filter((provider) => provider.connected && provider.models.some(modelIsReady));
  const selectedProvider = available.find((provider) => provider.id === config.providerId);
  const selectedModel = selectedProvider?.models.find((model) => model.id === config.modelId && modelIsReady(model));
  const ready = Boolean(selectedModel);

  async function save(next: AutoTitleConfig) {
    setBusy(true);
    setError(undefined);
    try {
      await onChange(next);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  function choose(patch: { providerId?: string; modelId?: string }) {
    const providerId = patch.providerId ?? config.providerId;
    const provider = available.find((item) => item.id === providerId);
    const modelId = patch.modelId ?? (patch.providerId ? provider?.models.find(modelIsReady)?.id : config.modelId);
    if (!providerId || !modelId) return;
    void save({ ...config, providerId, modelId });
  }

  return (
    <div className="settings-scroll auto-title-settings">
      <div className="auto-title-content">
        <div className="auto-title-intro">
          <div className="auto-title-intro-icon"><Icon name="spark" /></div>
          <div>
            <span className="auto-title-kicker">ONE REQUEST · ONE TIME</span>
            <h3>Find a chat by what it’s about.</h3>
            <p>The model you choose turns the first message into a short, useful title. The chat continues while the title is made.</p>
          </div>
        </div>

        <div className="auto-title-example" aria-label="Example automatic title">
          <div className="auto-title-example-message">
            <span>FIRST MESSAGE</span>
            <p>Help me debug the failing checkout tests</p>
          </div>
          <Icon name="chevron" />
          <div className="auto-title-example-result">
            <span>CHAT TITLE</span>
            <strong>Debug Failing Checkout Tests</strong>
          </div>
        </div>

        <div className="auto-title-setup">
          <div className="auto-title-setup-row">
            <span className="auto-title-step">01</span>
            <div className="auto-title-setup-copy">
              <h4>Choose a title model</h4>
              <p>A small, inexpensive model is enough. This choice is separate from the model used in your chats.</p>
            </div>
            <div className="auto-title-model-choice">
              {available.length > 0 ? (
                <>
                  <ModelPicker providers={available} providerId={config.providerId ?? ""} modelId={config.modelId ?? ""} disabled={busy} popoverSide="bottom" onConfigure={choose} />
                  {selectedProvider && <span>{selectedProvider.name}</span>}
                </>
              ) : (
                <button type="button" className="secondary-button" onClick={onOpenProviders}>Add a connection</button>
              )}
            </div>
          </div>
          <div className="auto-title-setup-row">
            <span className="auto-title-step">02</span>
            <div className="auto-title-setup-copy">
              <h4>Turn on automatic titles</h4>
              <p>Each new chat uses one extra model request. Existing chats stay as they are.</p>
            </div>
            <button
              type="button"
              role="switch"
              aria-label="Automatic titles"
              aria-checked={config.enabled}
              className={`toggle ${config.enabled ? "on" : ""}`}
              disabled={busy || (!ready && !config.enabled)}
              onClick={() => void save({ ...config, enabled: !config.enabled })}
            ><span /></button>
          </div>
        </div>

        {!ready && <p className="auto-title-setup-hint">{available.length > 0 ? "Choose a connected model to turn this on." : "Add a connected provider and model to get started."}</p>}
        {error && <p className="auto-title-error" role="alert">{error}</p>}

        <div className="auto-title-details">
          <div><Icon name="key" /><span>Only the first message’s text goes to the title model.</span></div>
          <div><Icon name="check" /><span>One attempt per chat, even if it fails. A manual name always wins.</span></div>
        </div>
      </div>
    </div>
  );
}
