import { useId, useMemo, useRef, useState } from "react";
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
  const sliderId = useId();

  const provider = providers.find((item) => item.id === providerId);
  const model = provider?.models.find((item) => item.id === modelId);
  const levels: ThinkingLevel[] = (model?.thinkingLevels.length ? model.thinkingLevels : (["off"] as ThinkingLevel[]))
    .slice()
    .sort((a, b) => LEVEL_ORDER[a] - LEVEL_ORDER[b]);

  // Only visible when the selected model supports reasoning (more than just "off")
  if (levels.length <= 1) {
    return null;
  }

  const activeLevel: ThinkingLevel = levels.includes(thinkingLevel) ? thinkingLevel : levels[0] ?? "off";
  const currentIndex = Math.max(0, levels.indexOf(activeLevel));
  const isReasoningOn = activeLevel !== "off";
  const percent = levels.length > 1 ? (currentIndex / (levels.length - 1)) * 100 : 0;

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className={`model-pill reasoning-toggle ${isReasoningOn ? "active" : ""}`}
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
        <div className="reasoning-panel">
          <div className="reasoning-header">
            <span className="reasoning-title">Reasoning effort</span>
            <span className="reasoning-badge">{activeLevel}</span>
          </div>
          <div className="reasoning-slider-wrap">
            <input
              id={sliderId}
              type="range"
              className="reasoning-slider"
              min={0}
              max={levels.length - 1}
              step={1}
              value={currentIndex}
              style={{
                background: `linear-gradient(to right, var(--wc-accent) 0%, var(--wc-accent) ${percent}%, #242a20 ${percent}%, #242a20 100%)`
              }}
              aria-label="Reasoning effort"
              onChange={(event) => {
                const idx = Number(event.target.value);
                const nextLevel = levels[idx];
                if (nextLevel) {
                  onConfigure({ thinkingLevel: nextLevel });
                }
              }}
            />
            <div className="reasoning-ticks">
              {levels.map((level, idx) => {
                const pct = levels.length > 1 ? (idx / (levels.length - 1)) * 100 : 0;
                return (
                  <button
                    type="button"
                    key={level}
                    className={`reasoning-tick ${idx === currentIndex ? "selected" : ""}`}
                    style={{ left: `${pct}%` }}
                    onClick={() => onConfigure({ thinkingLevel: level })}
                  >
                    <span className="reasoning-tick-pip" />
                    <span className="reasoning-tick-label">{level}</span>
                  </button>
                );
              })}
            </div>
          </div>
        </div>
      </Popover>
    </>
  );
}
