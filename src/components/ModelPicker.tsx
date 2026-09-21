import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import type { ProviderRecord, ThinkingLevel } from "../types";
import { modelIsReady } from "../model-utils";
import { Icon } from "./Icons";
import { Popover } from "./ui/Popover";

export interface ModelPickerProps {
  providers: ProviderRecord[];
  providerId?: string;
  modelId?: string;
  disabled?: boolean;
  popoverSide?: "top" | "bottom";
  onConfigure: (patch: { providerId?: string; modelId?: string; thinkingLevel?: ThinkingLevel }) => void;
}

export interface ReasoningToggleProps {
  providers: ProviderRecord[];
  providerId?: string;
  modelId?: string;
  thinkingLevel?: ThinkingLevel;
  disabled?: boolean;
  popoverSide?: "top" | "bottom";
  onConfigure: (patch: { thinkingLevel: ThinkingLevel }) => void;
}

const LEVEL_ORDER: Record<ThinkingLevel, number> = { off: 0, minimal: 1, low: 2, medium: 3, high: 4, xhigh: 5, max: 6 };

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
  popoverSide = "top",
  onConfigure
}: ModelPickerProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);

  const validProviders = useMemo(() => {
    return providers.filter((item) => item.models.some(modelIsReady));
  }, [providers]);

  const currentProvider = validProviders.find((item) => item.id === providerId);
  const currentModel = currentProvider?.models.find((item) => item.id === modelId && modelIsReady(item));
  const label = currentModel ? `${currentModel.name || currentModel.id}` : "Choose model";

  const [activeProviderId, setActiveProviderId] = useState<string>(() => {
    return currentProvider?.id ?? validProviders[0]?.id ?? "";
  });

  const [view, setView] = useState<"providers" | "models">(() => {
    return currentProvider ? "models" : "providers";
  });

  function handleOpen() {
    if (!open) {
      if (currentProvider) {
        setActiveProviderId(currentProvider.id);
        setView("models");
      } else if (validProviders.length > 0) {
        setActiveProviderId(validProviders[0].id);
        setView("providers");
      }
    }
    setOpen((val) => !val);
  }

  const activeProvider = validProviders.find((p) => p.id === activeProviderId) ?? validProviders[0];
  const activeModels = activeProvider ? activeProvider.models.filter(modelIsReady) : [];

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className="model-pill"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={handleOpen}
      >
        <span className="model-pill-name">{label}</span>
        <svg className="select-chevron" viewBox="0 0 12 12" aria-hidden="true">
          <path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>

      <Popover
        anchor={triggerRef}
        open={open}
        onClose={() => setOpen(false)}
        side={popoverSide}
        align="start"
        className="model-picker-pop"
      >
        <div className="picker-viewport">
          <div className={`picker-panels ${view === "models" ? "show-models" : "show-providers"}`}>
            {/* Panel 1: Provider selection */}
            <div className="picker-panel picker-panel-providers">
              <div className="picker-heading">Providers</div>
              <div className="picker-list">
                {validProviders.map((item) => {
                  const readyCount = item.models.filter(modelIsReady).length;
                  const isSelected = item.id === (currentProvider?.id ?? activeProviderId);
                  return (
                    <button
                      type="button"
                      key={item.id}
                      className={`picker-item picker-provider-item ${isSelected ? "selected" : ""}`}
                      onClick={() => {
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

            {/* Panel 2: Model list for active provider */}
            <div className="picker-panel picker-panel-models">
              <div className="picker-header">
                <button
                  type="button"
                  className="picker-back-btn"
                  onClick={() => setView("providers")}
                  aria-label="Back to providers"
                >
                  <Icon name="back" />
                  <span className="picker-back-title">{activeProvider?.name ?? "Providers"}</span>
                </button>
              </div>
              <div className="picker-list">
                {activeModels.map((entry) => {
                  const isSelected = activeProvider?.id === providerId && entry.id === modelId;
                  return (
                    <button
                      type="button"
                      key={entry.id}
                      className={`picker-item ${isSelected ? "selected" : ""}`}
                      onClick={() => {
                        onConfigure({ providerId: activeProvider.id, modelId: entry.id });
                        setOpen(false);
                      }}
                    >
                      <span>{entry.name || entry.id}</span>
                      {isSelected && <Icon name="check" />}
                    </button>
                  );
                })}
              </div>
            </div>
          </div>
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
  disabled,
  popoverSide = "top",
  onConfigure
}: ReasoningToggleProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const thumbRef = useRef<HTMLDivElement>(null);

  const provider = providers.find((item) => item.id === providerId);
  const model = provider?.models.find((item) => item.id === modelId);
  const levels: ThinkingLevel[] = (model?.thinkingLevels.length ? model.thinkingLevels : (["off"] as ThinkingLevel[]))
    .slice()
    .sort((a, b) => LEVEL_ORDER[a] - LEVEL_ORDER[b]);

  const activeLevel: ThinkingLevel = levels.includes(thinkingLevel) ? thinkingLevel : levels[0] ?? "off";
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
  const isTopLevel = currentIndex === levels.length - 1;
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
        className={`model-pill reasoning-toggle${isReasoningOn ? " active" : ""}${isTopLevel ? " at-max" : ""}`}
        disabled={disabled}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        title="Reasoning effort"
      >
        <Icon name="brain" className="reasoning-icon" />
        <span className="reasoning-toggle-label">{activeLevel}</span>
      </button>

      <Popover
        anchor={triggerRef}
        open={open}
        onClose={() => setOpen(false)}
        side={popoverSide}
        align="start"
        className="reasoning-pop"
      >
        <div className={`reasoning-panel${isTopLevel ? " at-max" : ""}`}>
          <div className="reasoning-header">
            <Icon name="brain" className="reasoning-head-icon" />
            <div className="reasoning-head-text">
              <span key={activeLevel} className={`reasoning-level-name${isReasoningOn ? " on" : ""}`}>{activeLevel}</span>
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
              </div>
              {levels.map((level, idx) => (
                <span
                  key={level}
                  className={`reasoning-dot${idx <= currentIndex ? " on" : ""}`}
                  style={{ "--p": idx / (levels.length - 1) } as CSSProperties}
                />
              ))}
              <div className="reasoning-thumb" ref={thumbRef} />
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
