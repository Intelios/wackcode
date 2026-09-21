import { useRef, useState } from "react";
import type { ProviderRecord, ThinkingLevel } from "../types";
import { modelIsReady } from "../model-utils";
import { Icon } from "./Icons";
import { Popover } from "./ui/Popover";

interface ModelPickerProps {
  providers: ProviderRecord[];
  providerId: string;
  modelId: string;
  thinkingLevel: ThinkingLevel;
  disabled?: boolean;
  onConfigure: (patch: { providerId?: string; modelId?: string; thinkingLevel?: ThinkingLevel }) => void;
}

const LEVEL_ORDER: Record<ThinkingLevel, number> = { off: 0, minimal: 1, low: 2, medium: 3, high: 4, xhigh: 5, max: 6 };

export function ModelPicker({ providers, providerId, modelId, thinkingLevel, disabled, onConfigure }: ModelPickerProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const provider = providers.find((item) => item.id === providerId);
  const model = provider?.models.find((item) => item.id === modelId);
  const levels: ThinkingLevel[] = (model?.thinkingLevels.length ? model.thinkingLevels : (["off"] as ThinkingLevel[]))
    .slice()
    .sort((a, b) => LEVEL_ORDER[a] - LEVEL_ORDER[b]);
  const label = model ? `${model.name || model.id}` : "Choose model";
  const showLevels = levels.length > 1;

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className="model-pill"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <span className="model-pill-name">{label}</span>
        {showLevels && <span className="model-pill-level">{thinkingLevel === "off" ? "reasoning off" : thinkingLevel}</span>}
        <svg className="select-chevron" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
      </button>
      <Popover anchor={triggerRef} open={open} onClose={() => setOpen(false)} side="top" align="start" className="model-picker-pop">
        <div className="picker">
          {providers.map((item) => {
            const models = item.models.filter(modelIsReady);
            if (models.length === 0) return null;
            return (
              <div className="picker-group" key={item.id}>
                <div className="picker-heading">{item.name}</div>
                {models.map((entry) => (
                  <button
                    type="button"
                    key={entry.id}
                    className={`picker-item ${item.id === providerId && entry.id === modelId ? "selected" : ""}`}
                    onClick={() => {
                      onConfigure({ providerId: item.id, modelId: entry.id });
                      setOpen(false);
                    }}
                  >
                    <span>{entry.name || entry.id}</span>
                    {item.id === providerId && entry.id === modelId && <Icon name="check" />}
                  </button>
                ))}
              </div>
            );
          })}
          {showLevels && (
            <div className="picker-group">
              <div className="picker-heading">Reasoning effort</div>
              <div className="picker-chips">
                {levels.map((level) => (
                  <button
                    type="button"
                    key={level}
                    className={`picker-chip ${level === thinkingLevel ? "selected" : ""}`}
                    onClick={() => onConfigure({ thinkingLevel: level })}
                  >{level}</button>
                ))}
              </div>
            </div>
          )}
        </div>
      </Popover>
    </>
  );
}
