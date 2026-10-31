import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import type { ModelRef, ProviderRecord, ReasoningEffort, SubagentModel, ThinkingLevel } from "../types";
import { favoriteModelEntries, modelIsReady, pickThinkingLevel, reasoningLevels } from "../model-utils";
import { Icon } from "./Icons";
import { Popover } from "./ui/Popover";
import { Tooltip } from "./ui/Tooltip";
import { UltraGlyphField } from "./UltraGlyphField";
import { UltraLabel } from "./UltraLabel";
import { UltraMark } from "./UltraMark";

export interface ModelFavoritesProps {
  favoriteModels: readonly ModelRef[];
  favoriteSaving?: boolean;
  /** App saves the shared preference and reports any failure. Never selects the model. */
  onSetFavorite: (reference: ModelRef, favorite: boolean) => Promise<void>;
}

/**
 * Team, on a coding chat's model pill. `team` is the chat's saved crew model (null when off);
 * `teamDefault` is the crew Settings › Sub-agents offers a fresh Team. `teamAvailable` is why
 * the crew can't run right now (sub-agents off, the worker role off, or the crew model
 * unreachable); undefined means it can.
 */
export interface TeamPillProps {
  team: SubagentModel | null;
  teamDefault: SubagentModel | null;
  teamAvailable?: string;
  /** Opens Settings so a missing requirement can be fixed there. */
  onOpenSettings: () => void;
  onTeamChange: (team: SubagentModel | null) => void;
}

export interface ModelPickerProps extends ModelFavoritesProps {
  providers: ProviderRecord[];
  providerId?: string;
  modelId?: string;
  disabled?: boolean;
  popoverSide?: "top" | "bottom";
  /** Smoked glass on the popover panel (the Classic Liquid Glass composer passes this). */
  glass?: boolean;
  onConfigure: (patch: { providerId?: string; modelId?: string; thinkingLevel?: ReasoningEffort }) => void;
  /** Present on coding chats only: adds the Team footer to the popover and, while a chat
   *  carries a crew, a second segment to the pill. */
  team?: TeamPillProps;
}

export interface ReasoningToggleProps {
  providers: ProviderRecord[];
  providerId?: string;
  modelId?: string;
  thinkingLevel?: ReasoningEffort;
  /** Offer Ultra above the model's own levels. Off for a sub-agent's own model. */
  allowUltra?: boolean;
  disabled?: boolean;
  popoverSide?: "top" | "bottom";
  /** Smoked glass on the popover panel (the Classic Liquid Glass composer passes this). */
  glass?: boolean;
  onConfigure: (patch: { thinkingLevel: ReasoningEffort }) => void;
}

const MAX_FX_DOTS = [
  { left: "6%", top: "30%", size: 3, dur: "3.4s", delay: "-0.4s" },
  { left: "13%", top: "64%", size: 2.4, dur: "2.6s", delay: "-1.8s" },
  { left: "21%", top: "42%", size: 3.4, dur: "3.9s", delay: "-2.6s" },
  { left: "30%", top: "58%", size: 2.2, dur: "2.9s", delay: "-0.9s" },
  { left: "38%", top: "28%", size: 3, dur: "3.1s", delay: "-2.1s" },
  { left: "47%", top: "66%", size: 2.6, dur: "2.4s", delay: "-1.2s" },
  { left: "56%", top: "36%", size: 3.4, dur: "3.7s", delay: "-3s" },
  { left: "64%", top: "56%", size: 2.2, dur: "2.7s", delay: "-0.2s" },
  { left: "72%", top: "30%", size: 3, dur: "3.2s", delay: "-1.5s" },
  { left: "81%", top: "62%", size: 2.6, dur: "2.5s", delay: "-2.3s" },
  { left: "90%", top: "40%", size: 3.2, dur: "3.6s", delay: "-0.7s" },
  { left: "96%", top: "55%", size: 2.4, dur: "2.8s", delay: "-1.9s" }
];

export function ModelPicker({
  providers,
  providerId,
  modelId,
  disabled,
  favoriteModels,
  favoriteSaving,
  onSetFavorite,
  popoverSide = "top",
  glass = false,
  onConfigure,
  team
}: ModelPickerProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const pickerId = useId();
  const favoritesButtonRef = useRef<HTMLButtonElement>(null);
  const collectionButtonRef = useRef<HTMLButtonElement | null>(null);
  const backButtonRef = useRef<HTMLButtonElement>(null);
  const crewBackRef = useRef<HTMLButtonElement>(null);
  const modelListRef = useRef<HTMLDivElement>(null);
  const removedFavoriteFocus = useRef<{ key: string; index: number } | null>(null);

  const validProviders = useMemo(() => {
    return providers.filter((item) => item.enabled !== false && item.connected && item.models.some(modelIsReady));
  }, [providers]);
  const favorites = useMemo(() => favoriteModelEntries(validProviders, favoriteModels), [validProviders, favoriteModels]);

  const currentProvider = validProviders.find((item) => item.id === providerId);
  const currentModel = currentProvider?.models.find((item) => item.id === modelId && modelIsReady(item));
  const label = currentModel ? `${currentModel.name || currentModel.id}` : "Choose model";

  const [activeProviderId, setActiveProviderId] = useState<string>(() => {
    return currentProvider?.id ?? validProviders[0]?.id ?? "";
  });

  const [view, setView] = useState<"providers" | "models" | "favorites" | "crew">(() => {
    return currentProvider ? "models" : "providers";
  });

  function handleOpen() {
    if (!open) {
      collectionButtonRef.current = null;
      if (currentProvider) {
        setActiveProviderId(currentProvider.id);
        setView("models");
      } else {
        setActiveProviderId(validProviders[0]?.id ?? "");
        setView("providers");
      }
    }
    setOpen((val) => !val);
  }

  const activeProvider = validProviders.find((p) => p.id === activeProviderId) ?? validProviders[0];
  const entries = useMemo(() => view === "favorites" ? favorites
    : activeProvider?.models.filter(modelIsReady).map((model) => ({ provider: activeProvider, model })) ?? [],
  [view, favorites, activeProvider]);

  // A panel sliding off screen is inert. Move focus to the newly visible navigation control.
  useLayoutEffect(() => {
    if (!open) return;
    const target = view === "providers"
      ? (collectionButtonRef.current?.isConnected ? collectionButtonRef.current : favoritesButtonRef.current)
      : (view === "crew" ? crewBackRef.current : backButtonRef.current);
    target?.focus({ preventScroll: true });
  }, [open, view]);

  useLayoutEffect(() => {
    const removed = removedFavoriteFocus.current;
    if (!open || view !== "favorites") {
      removedFavoriteFocus.current = null;
      return;
    }
    if (!removed || entries.some(({ provider, model }) => JSON.stringify([provider.id, model.id]) === removed.key)) return;
    removedFavoriteFocus.current = null;
    // Don't steal focus if the user moved elsewhere while the save was pending.
    if (document.activeElement !== document.body) return;
    const buttons = modelListRef.current?.querySelectorAll<HTMLButtonElement>(".picker-model-select");
    const target = buttons?.[Math.min(removed.index, buttons.length - 1)] ?? backButtonRef.current;
    target?.focus({ preventScroll: true });
  }, [open, view, entries]);

  // The Team crew: the model the chat's built-in sub-agents run on. A stale one (removed
  // connection, signed out, worker role off) renders but reads as off — the panel explains why.
  const crew = team?.team ?? null;
  const crewModel = crew ? providers.find((item) => item.id === crew.providerId)?.models.find((item) => item.id === crew.modelId) : undefined;
  const crewLabel = crew ? (crewModel?.name || crew.modelId) : undefined;
  const crewUnavailable = crew && team?.teamAvailable;

  function openCrew() {
    setActiveProviderId(validProviders[0]?.id ?? "");
    setView("crew");
    if (!open) setOpen(true);
  }

  function chooseCrew(reference: ModelRef) {
    if (!team) return;
    const provider = validProviders.find((item) => item.id === reference.providerId);
    const model = provider?.models.find((item) => item.id === reference.modelId);
    if (!provider || !model) return;
    team.onTeamChange({
      providerId: provider.id,
      modelId: model.id,
      thinkingLevel: pickThinkingLevel(model, crew?.thinkingLevel, undefined, { ultra: false }),
    });
  }

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className="model-pill"
        disabled={disabled}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? pickerId : undefined}
        onClick={handleOpen}
      >
        <span className="model-pill-name">{label}</span>
        <svg className="select-chevron" viewBox="0 0 12 12" aria-hidden="true">
          <path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>
      {crew && (
        <Tooltip label={crewUnavailable ?? `Team crew: ${crewLabel}. Scout, worker and reviewer run on it.`}>
          <button
            type="button"
            className={`model-pill team-pill${crewUnavailable ? " stale" : ""}`}
            disabled={disabled}
            aria-label={crewUnavailable ? `Team crew: ${crewUnavailable}` : `Team crew: ${crewLabel}`}
            onClick={openCrew}
          >
            <Icon name="agents" className="team-pill-icon" />
            <span className="model-pill-name">{crewLabel}</span>
          </button>
        </Tooltip>
      )}

      <Popover
        anchor={triggerRef}
        open={open}
        onClose={() => setOpen(false)}
        side={popoverSide}
        align="start"
        glass={glass}
        className="model-picker-pop"
      >
        <div id={pickerId} className="picker-viewport" role="dialog" aria-label="Choose model">
          <div className={`picker-panels ${view === "crew" ? "show-crew" : view === "providers" ? "show-providers" : "show-models"}`}>
            {/* Panel 1: Provider selection */}
            <div className="picker-panel picker-panel-providers" inert={view !== "providers"} aria-hidden={view !== "providers"}>
              <div className="picker-heading">Providers</div>
              <div className="picker-list">
                <button
                  ref={favoritesButtonRef}
                  type="button"
                  className="picker-item picker-provider-item picker-favorites-entry"
                  aria-label={`Favourites ${favorites.length}`}
                  tabIndex={view === "providers" ? 0 : -1}
                  onClick={(event) => {
                    collectionButtonRef.current = event.currentTarget;
                    setView("favorites");
                  }}
                >
                  <Icon name="star" />
                  <span className="picker-provider-name">Favourites</span>
                  <span className="picker-provider-count">{favorites.length}</span>
                  <Icon name="chevron" className="picker-chevron" />
                </button>
                <div className="picker-provider-divider" />
                {validProviders.map((item) => {
                  const readyCount = item.models.filter(modelIsReady).length;
                  const isSelected = item.id === (currentProvider?.id ?? activeProviderId);
                  return (
                    <button
                      type="button"
                      key={item.id}
                      className={`picker-item picker-provider-item ${isSelected ? "selected" : ""}`}
                      tabIndex={view === "providers" ? 0 : -1}
                      onClick={(event) => {
                        collectionButtonRef.current = event.currentTarget;
                        setActiveProviderId(item.id);
                        setView("models");
                      }}
                    >
                      <span className="picker-provider-name">{item.name}</span>
                      <span className="picker-provider-count">{readyCount}</span>
                      <Icon name="chevron" className="picker-chevron" />
                    </button>
                  );
                })}
              </div>
            </div>

            {/* Panel 2: An actual provider's models, or the shared favourites collection. */}
            <div className="picker-panel picker-panel-models" inert={view === "providers" || view === "crew"} aria-hidden={view === "providers" || view === "crew"}>
              <div className="picker-header">
                <button
                  ref={backButtonRef}
                  type="button"
                  className="picker-back-btn"
                  tabIndex={view === "providers" ? -1 : 0}
                  onClick={() => setView("providers")}
                  aria-label="Back to providers"
                >
                  <Icon name="back" />
                  <span className="picker-back-title">{view === "favorites" ? "Favourites" : activeProvider?.name ?? "Providers"}</span>
                </button>
              </div>
              <div className="picker-list" ref={modelListRef}>
                {view === "favorites" && entries.length === 0 && (
                  <div className="picker-empty" role="status">
                    <strong>{favoriteModels.length ? "No favourites available" : "No favourites yet"}</strong>
                    <span>{favoriteModels.length
                      ? "Enable or reconnect a provider, or star another model."
                      : "Star a model in any provider to add it here."}</span>
                  </div>
                )}
                {entries.map(({ provider, model }, index) => {
                  const reference = { providerId: provider.id, modelId: model.id };
                  const key = JSON.stringify([provider.id, model.id]);
                  const name = model.name || model.id;
                  const isSelected = provider.id === providerId && model.id === modelId;
                  const isFavorite = favoriteModels.some((item) => item.providerId === provider.id && item.modelId === model.id);
                  const starLabel = isFavorite ? `Remove ${name} (${provider.name}) from favourites` : `Add ${name} (${provider.name}) to favourites`;
                  return (
                    <div key={key} className={`picker-model-row${view === "favorites" ? " favorite" : ""}`}>
                      <button
                        type="button"
                        className={`picker-item picker-model-select${isSelected ? " selected" : ""}`}
                        aria-label={view === "favorites" ? `${name} (${provider.name})` : undefined}
                        tabIndex={view === "providers" ? -1 : 0}
                        aria-pressed={isSelected}
                        onClick={() => {
                          onConfigure(reference);
                          setOpen(false);
                          triggerRef.current?.focus({ preventScroll: true });
                        }}
                      >
                        <span className="picker-model-label">
                          <span>{name}</span>
                          {view === "favorites" && <span className="picker-model-provider">{provider.name}</span>}
                        </span>
                        {isSelected && <Icon name="check" />}
                      </button>
                      <button
                        type="button"
                        className={`icon-button picker-favorite-btn${isFavorite ? " is-favorite" : ""}`}
                        tabIndex={view === "providers" ? -1 : 0}
                        aria-label={starLabel}
                        aria-pressed={isFavorite}
                        title={starLabel}
                        disabled={favoriteSaving}
                        onClick={(event) => {
                          if (view === "favorites" && isFavorite && document.activeElement === event.currentTarget) {
                            removedFavoriteFocus.current = { key, index };
                          }
                          void onSetFavorite(reference, !isFavorite).catch(() => {
                            removedFavoriteFocus.current = null;
                          });
                        }}
                      >
                        <Icon name="star" />
                      </button>
                    </div>
                  );
                })}
              </div>
            </div>

            {/* Panel 3: the Team crew — one list across providers; no favourites stars. */}
            {team && <div className="picker-panel picker-panel-crew" inert={view !== "crew"} aria-hidden={view !== "crew"}>
              <div className="picker-header">
                <button
                  ref={crewBackRef}
                  type="button"
                  className="picker-back-btn"
                  tabIndex={view === "crew" ? 0 : -1}
                  onClick={() => setView("providers")}
                  aria-label="Back to providers"
                >
                  <Icon name="back" />
                  <span className="picker-back-title">Team crew</span>
                </button>
              </div>
              <div className="picker-list">
                {validProviders.flatMap((provider) => provider.models.filter(modelIsReady).map((model) => ({ provider, model }))).length === 0 && (
                  <div className="picker-empty" role="status">
                    <strong>No models available</strong>
                    <span>Enable or reconnect a provider to crew the Team.</span>
                  </div>
                )}
                {validProviders.flatMap((provider) => provider.models.filter(modelIsReady).map((model) => ({ provider, model }))).map(({ provider, model }) => {
                  const name = model.name || model.id;
                  const isSelected = crew?.providerId === provider.id && crew.modelId === model.id;
                  return (
                    <button
                      key={JSON.stringify([provider.id, model.id])}
                      type="button"
                      className={`picker-item picker-model-select${isSelected ? " selected" : ""}`}
                      aria-pressed={isSelected}
                      tabIndex={view === "crew" ? 0 : -1}
                      onClick={() => chooseCrew({ providerId: provider.id, modelId: model.id })}
                    >
                      <span className="picker-model-label">
                        <span>{name}</span>
                        <span className="picker-model-provider">{provider.name}</span>
                      </span>
                      {isSelected && <Icon name="check" />}
                    </button>
                  );
                })}
              </div>
              {crew && (
                <div className="picker-crew-levels">
                  <span className="picker-heading">Crew reasoning</span>
                  <div className="mode-toggle picker-crew-toggle" role="radiogroup" aria-label="Crew reasoning">
                    {/* `ultra: false` keeps the crew's reasoning inside ThinkingLevel. */}
                    {(reasoningLevels(crewModel, { ultra: false }) as ThinkingLevel[]).map((level) => (
                      <button
                        key={level}
                        type="button"
                        role="radio"
                        aria-checked={crew.thinkingLevel === level}
                        className={`mode-option ${crew.thinkingLevel === level ? "active" : ""}`}
                        tabIndex={view === "crew" ? 0 : -1}
                        onClick={() => team?.onTeamChange({ ...crew, thinkingLevel: level })}
                      >{level}</button>
                    ))}
                  </div>
                </div>
              )}
            </div>}
          </div>
          {team && (
            <div className="picker-team-footer">
              <button
                type="button"
                role="switch"
                aria-checked={Boolean(crew)}
                aria-label="Team"
                className={`toggle ${crew ? "on" : ""}`}
                disabled={(!crew && Boolean(team.teamAvailable)) || disabled}
                onClick={() => {
                  if (crew) { team.onTeamChange(null); return; }
                  team.onTeamChange(team.teamDefault ?? (() => {
                    const provider = validProviders[0];
                    const model = provider?.models.find(modelIsReady);
                    return provider && model ? { providerId: provider.id, modelId: model.id, thinkingLevel: pickThinkingLevel(model, undefined, undefined, { ultra: false }) } : null;
                  })());
                }}
              >
                <span />
              </button>
              <div className="picker-team-copy">
                <span className="picker-team-title">Team</span>
                {team.teamAvailable ? (
                  <small>{team.teamAvailable}{" "}
                    <button type="button" className="picker-team-link" onClick={() => { setOpen(false); team.onOpenSettings(); }}>Open Settings</button>
                  </small>
                ) : (
                  <small>Lead with this model; the crew does the work.</small>
                )}
              </div>
              {crew && (
                <button
                  type="button"
                  className="picker-item picker-crew-entry"
                  onClick={() => setView("crew")}
                  aria-label={`Team crew: ${crewLabel ?? "choose a model"}`}
                >
                  <span className="picker-model-label">
                    <span>{crewLabel}</span>
                    {crew?.thinkingLevel !== "off" && <span className="picker-model-provider">{crew?.thinkingLevel}</span>}
                  </span>
                  <Icon name="chevron" className="picker-chevron" />
                </button>
              )}
            </div>
          )}
        </div>
      </Popover>
    </>
  );
}

export function ReasoningToggle({
  providers,
  providerId,
  modelId,
  thinkingLevel = "off",
  allowUltra = true,
  disabled,
  popoverSide = "top",
  glass = false,
  onConfigure
}: ReasoningToggleProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const thumbRef = useRef<HTMLDivElement>(null);

  const provider = providers.find((item) => item.id === providerId);
  const model = provider?.models.find((item) => item.id === modelId);
  const levels = reasoningLevels(model, { ultra: allowUltra });

  const activeLevel: ReasoningEffort = levels.includes(thinkingLevel) ? thinkingLevel : levels[0] ?? "off";
  const currentIndex = Math.max(0, levels.indexOf(activeLevel));

  // Little "pop" on the thumb whenever the level changes (and when the popover opens)
  useEffect(() => {
    if (window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches) return;
    thumbRef.current?.animate?.(
      [{ transform: "scale(.72)" }, { transform: "scale(1.14)" }, { transform: "scale(1)" }],
      { duration: 320, easing: "cubic-bezier(.34, 1.4, .64, 1)" }
    );
  }, [currentIndex, open]);

  // Only visible when the selected model supports reasoning (more than just "off")
  if (levels.length <= 1) {
    return null;
  }

  const isReasoningOn = activeLevel !== "off";
  // Max's effect stays on the model's own top level; Ultra, above it, has its own.
  const isUltra = activeLevel === "ultra";
  const topOwnIndex = levels.length - (levels.includes("ultra") ? 2 : 1);
  const isTopLevel = currentIndex === topOwnIndex;
  const peak = isUltra ? " at-ultra" : isTopLevel ? " at-max" : "";
  const position = currentIndex / (levels.length - 1);

  function pick(idx: number) {
    const nextLevel = levels[idx];
    if (nextLevel) {
      onConfigure({ thinkingLevel: nextLevel });
    }
  }

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className={`model-pill reasoning-toggle${isReasoningOn ? " active" : ""}${peak}`}
        disabled={disabled}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        title="Reasoning effort"
      >
        {isUltra ? <UltraMark animated={!disabled} className="reasoning-icon" /> : <Icon name="brain" className="reasoning-icon" />}
        <span className="reasoning-toggle-label">{isUltra ? <UltraLabel text={activeLevel} /> : activeLevel}</span>
      </button>

      <Popover
        anchor={triggerRef}
        open={open}
        onClose={() => setOpen(false)}
        side={popoverSide}
        align="start"
        glass={glass}
        className="reasoning-pop"
      >
        <div className={`reasoning-panel${peak}`}>
          <div className="reasoning-header">
            <span className="reasoning-head-mark" aria-hidden="true">
              {isUltra ? <UltraMark animated className="reasoning-head-icon" /> : <Icon name="brain" className="reasoning-head-icon" />}
            </span>
            <div className="reasoning-head-text">
              <span key={activeLevel} className={`reasoning-level-name${isReasoningOn ? " on" : ""}`}>
                {isUltra ? <UltraLabel text={activeLevel} /> : activeLevel}
              </span>
              <span className="reasoning-model-sub">{model?.name || model?.id}</span>
            </div>
          </div>
          <div
            className="reasoning-slider-wrap"
            style={{ "--p": position } as CSSProperties}
          >
            <div className="reasoning-track" aria-hidden="true">
              <div className="reasoning-fill">
                {isTopLevel && (
                  <div className="reasoning-max-fx">
                    {MAX_FX_DOTS.map((dot, idx) => (
                      <span
                        key={idx}
                        style={{
                          left: dot.left, top: dot.top, width: dot.size, height: dot.size,
                          animationDuration: dot.dur, animationDelay: dot.delay
                        }}
                      />
                    ))}
                  </div>
                )}
                {isUltra && <UltraGlyphField />}
              </div>
              {levels.map((level, idx) => (
                <span
                  key={level}
                  className={`reasoning-dot${idx <= currentIndex ? " on" : ""}`}
                  style={{ "--p": idx / (levels.length - 1) } as CSSProperties}
                />
              ))}
              <div className="reasoning-thumb" ref={thumbRef}>
                {isUltra && <UltraMark tone="ink" className="reasoning-thumb-mark" />}
              </div>
            </div>
            <input
              type="range"
              className="reasoning-slider-input"
              min={0}
              max={levels.length - 1}
              step={1}
              value={currentIndex}
              aria-label="Reasoning effort"
              onChange={(event) => pick(Number(event.target.value))}
            />
            <div className="reasoning-ticks">
              {levels.map((level, idx) => (
                <button
                  type="button"
                  key={level}
                  className={`reasoning-tick${idx === currentIndex ? " selected" : ""}`}
                  style={{ "--p": idx / (levels.length - 1) } as CSSProperties}
                  onClick={() => pick(idx)}
                >
                  <span className="reasoning-tick-label">{level}</span>
                </button>
              ))}
            </div>
          </div>
        </div>
      </Popover>
    </>
  );
}
