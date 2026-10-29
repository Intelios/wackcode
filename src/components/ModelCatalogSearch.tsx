import { useId, useMemo, useRef, useState } from "react";
import { matchingCatalogProfile, searchModelCatalog, sortedCatalogProfiles } from "../model-catalog";
import { formatTokens } from "../model-utils";
import type { ApiFormat, ModelCatalogEntry, ModelCatalogProfile } from "../types";
import { Popover } from "./ui/Popover";
import { Select } from "./ui/Select";

interface Props {
  modelLabel: string;
  modelId?: string;
  baseUrl: string;
  apiFormat: ApiFormat;
  catalog: ModelCatalogEntry[];
  loading: boolean;
  error?: string;
  autoFocus?: boolean;
  onSelect: (entry: ModelCatalogEntry, profile: ModelCatalogProfile) => void;
}

/** Grouped search always previews one source's facts before changing a connection draft. */
export function ModelCatalogSearch({ modelLabel, modelId, baseUrl, apiFormat, catalog, loading, error, autoFocus = false, onSelect }: Props) {
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const [pending, setPending] = useState<{ entry: ModelCatalogEntry; profileKey: string; matched: boolean }>();
  const [applied, setApplied] = useState<string>();
  const anchor = useRef<HTMLDivElement>(null);
  const listId = useId();
  const matches = useMemo(() => searchModelCatalog(catalog, query), [catalog, query]);
  const visible = open && matches.length > 0;
  const profiles = pending ? sortedCatalogProfiles(pending.entry) : [];
  const profile = profiles.find((item) => item.key === pending?.profileKey);

  function choose(entry: ModelCatalogEntry) {
    const matched = matchingCatalogProfile(entry, baseUrl, apiFormat, modelId);
    setPending({ entry, profileKey: (matched ?? sortedCatalogProfiles(entry)[0]).key, matched: Boolean(matched) });
    setOpen(false);
    setActive(-1);
    setQuery("");
    setApplied(undefined);
  }

  return (
    <div className="model-catalog-search" ref={anchor}>
      <label>
        <span>Find model specifications</span>
        <input
          role="combobox"
          aria-label={`Find model specifications for ${modelLabel}`}
          aria-autocomplete="list"
          aria-expanded={visible}
          aria-controls={visible ? listId : undefined}
          aria-activedescendant={visible && active >= 0 && active < matches.length ? `${listId}-${active}` : undefined}
          value={query}
          autoFocus={autoFocus}
          onChange={(event) => { setQuery(event.target.value); setOpen(true); setActive(-1); setPending(undefined); }}
          onFocus={() => { if (query.trim()) setOpen(true); }}
          onKeyDown={(event) => {
            if (event.key === "Escape") { setOpen(false); setActive(-1); }
            else if (event.key === "Tab") setOpen(false);
            else if (event.key === "ArrowDown" && matches.length) {
              event.preventDefault(); setOpen(true); setActive((value) => (value + 1) % matches.length);
            } else if (event.key === "ArrowUp" && matches.length) {
              event.preventDefault(); setOpen(true); setActive((value) => value < 0 ? matches.length - 1 : (value - 1 + matches.length) % matches.length);
            } else if (event.key === "Enter" && visible) {
              event.preventDefault(); choose(matches[active >= 0 && active < matches.length ? active : 0]);
            }
          }}
          placeholder="Search by model name or ID"
          autoComplete="off"
          spellCheck={false}
        />
      </label>
      {loading && <small role="status">Loading offline model catalog…</small>}
      {error && !catalog.length && <small role="status">Could not load model catalog: {error} Manual entry is still available.</small>}
      {!loading && catalog.length > 0 && query.trim() && matches.length === 0 && <small role="status">No model catalog matches. You can enter settings manually.</small>}
      {applied && <small>Filled from {applied}. Review these settings before saving.</small>}
      <Popover anchor={anchor} open={visible} onClose={() => { setOpen(false); setActive(-1); }} matchWidth className="model-catalog-popover">
        <div id={listId} role="listbox" aria-label="Model specifications" className="model-catalog-options">
          {matches.map((entry, index) => (
            <button id={`${listId}-${index}`} key={entry.id} type="button" role="option" aria-selected={index === active}
              className={index === active ? "active" : ""} onMouseDown={(event) => event.preventDefault()}
              onMouseEnter={() => setActive(index)} onClick={() => choose(entry)}>
              <strong>{entry.name}</strong>
              <span>{entry.id} · {entry.profiles.length} {entry.profiles.length === 1 ? "source" : "sources"}</span>
            </button>
          ))}
        </div>
      </Popover>
      {pending && profile && (
        <section className="model-catalog-preview" aria-label={`Specifications for ${pending.entry.name}`}>
          <strong>{pending.entry.name}</strong>
          <p>{pending.matched ? "Source matched to your connection." : "Choose the source whose specifications fit your endpoint."}</p>
          <label>
            <span>Metadata source</span>
            <Select className="settings-select" matchWidth value={profile.key} aria-label={`Metadata source for ${pending.entry.name}`}
              options={profiles.map((item) => ({ value: item.key, label: item.providerName, hint: item.id }))}
              onChange={(profileKey) => setPending({ ...pending, profileKey, matched: false })} />
          </label>
          <dl className="model-catalog-facts">
            <div><dt>Model ID</dt><dd>{profile.id}</dd></div>
            <div><dt>Context</dt><dd>{profile.contextWindow ? `${formatTokens(profile.contextWindow)} tokens` : "Set manually"}</dd></div>
            <div><dt>Max output</dt><dd>{profile.maxTokens ? `${formatTokens(profile.maxTokens)} tokens` : "Set manually"}</dd></div>
            <div><dt>Reasoning</dt><dd>{!profile.reasoning ? "No" : profile.thinkingLevels?.join(" · ") ?? "Set efforts manually"}</dd></div>
            <div><dt>Vision</dt><dd>{profile.vision ? "Yes" : "No"}</dd></div>
          </dl>
          {profile.reasoning && !profile.thinkingLevels && <small>The source does not list exact efforts. Set supported efforts manually after applying.</small>}
          {modelId?.trim() && modelId !== profile.id && <small>Your model ID “{modelId}” will be kept.</small>}
          <div className="row-actions">
            <button type="button" className="secondary-button compact" onClick={() => {
              onSelect(pending.entry, profile); setApplied(profile.providerName); setPending(undefined);
            }}>Use these specifications</button>
            <button type="button" className="ghost-button compact" onClick={() => setPending(undefined)}>Cancel</button>
          </div>
        </section>
      )}
    </div>
  );
}
