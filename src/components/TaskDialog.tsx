import { useMemo, useState } from "react";
import { modelIsReady } from "../model-utils";
import type { ProjectRecord, ProviderRecord, ThinkingLevel } from "../types";
import { Icon } from "./Icons";

interface Props {
  project: ProjectRecord;
  providers: ProviderRecord[];
  onClose: () => void;
  onCreate: (input: {
    projectId: string;
    name: string;
    useWorktree: boolean;
    providerId: string;
    modelId: string;
    thinkingLevel: ThinkingLevel;
  }) => Promise<void>;
}

export function TaskDialog({ project, providers, onClose, onCreate }: Props) {
  const usableProviders = providers.filter((provider) => provider.hasApiKey && provider.models.some(modelIsReady));
  const [name, setName] = useState("");
  const [providerId, setProviderId] = useState(usableProviders[0]?.id ?? "");
  const provider = providers.find((item) => item.id === providerId);
  const models = useMemo(() => provider?.models.filter(modelIsReady) ?? [], [provider]);
  const [modelId, setModelId] = useState(models[0]?.id ?? "");
  const model = models.find((item) => item.id === modelId) ?? models[0];
  const thinkingLevels = model?.thinkingLevels.length ? model.thinkingLevels : (["off"] as ThinkingLevel[]);
  const [thinkingLevel, setThinkingLevel] = useState<ThinkingLevel>(thinkingLevels.includes("medium") ? "medium" : thinkingLevels[0]);
  const [useWorktree, setUseWorktree] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  function chooseProvider(nextProviderId: string) {
    setProviderId(nextProviderId);
    const next = providers.find((item) => item.id === nextProviderId)?.models.find(modelIsReady);
    setModelId(next?.id ?? "");
    const nextLevels = next?.thinkingLevels.length ? next.thinkingLevels : (["off"] as ThinkingLevel[]);
    setThinkingLevel(nextLevels.includes("medium") ? "medium" : nextLevels[0]);
  }

  function chooseModel(nextModelId: string) {
    setModelId(nextModelId);
    const next = models.find((item) => item.id === nextModelId);
    const nextLevels = next?.thinkingLevels.length ? next.thinkingLevels : (["off"] as ThinkingLevel[]);
    setThinkingLevel(nextLevels.includes("medium") ? "medium" : nextLevels[0]);
  }

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await onCreate({ projectId: project.id, name, useWorktree, providerId, modelId, thinkingLevel });
      onClose();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <form className="task-dialog" onSubmit={submit}>
        <header className="modal-header">
          <div><span className="eyebrow">{project.name}</span><h2>Start a new task</h2></div>
          <button type="button" className="icon-button close-button" onClick={onClose} aria-label="Close">×</button>
        </header>
        <div className="task-dialog-body">
          <label><span>Task name</span><input autoFocus value={name} onChange={(event) => setName(event.target.value)} placeholder="Describe what you’re working on" /></label>
          {usableProviders.length === 0 ? (
            <div className="dialog-callout"><Icon name="key" /><div><strong>No ready connection</strong><span>Add an API key and confirm at least one model’s limits in Settings.</span></div></div>
          ) : (
            <div className="form-grid">
              <label><span>Connection</span><select value={providerId} onChange={(event) => chooseProvider(event.target.value)}>{usableProviders.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}</select></label>
              <label><span>Model</span><select value={modelId} onChange={(event) => chooseModel(event.target.value)}>{models.map((item) => <option key={item.id} value={item.id}>{item.name || item.id}</option>)}</select></label>
              <label className="wide-field"><span>Reasoning effort</span><div className="segmented-control">{thinkingLevels.map((level) => <button type="button" key={level} className={thinkingLevel === level ? "active" : ""} onClick={() => setThinkingLevel(level)}>{level}</button>)}</div></label>
            </div>
          )}
          <label className={`worktree-option ${project.gitHasHead ? "" : "disabled"}`}>
            <input type="checkbox" checked={useWorktree} disabled={!project.gitHasHead} onChange={(event) => setUseWorktree(event.target.checked)} />
            <span className="checkbox-mark" />
            <div><strong><Icon name="branch" /> Use a worktree</strong><span>{project.gitHasHead ? "Start from the repository’s current HEAD in an isolated folder. Uncommitted changes are not copied." : "Available for Git repositories after their first commit."}</span></div>
          </label>
          {error && <div className="error-banner">{error}</div>}
        </div>
        <footer className="modal-footer">
          <button type="button" className="secondary-button" onClick={onClose}>Cancel</button>
          <button className="primary-button" disabled={busy || !name.trim() || !providerId || !modelId}>{busy ? "Creating…" : "Create task"}</button>
        </footer>
      </form>
    </div>
  );
}
