import { useEffect, useRef, useState, type CSSProperties } from "react";
import { DEFAULT_ACCENT, DEFAULT_BACKGROUND, THEME_PRESETS, clampBackground, resolveTheme } from "../theme";
import type { AppearanceConfig, BackdropMode, GlassStyle } from "../types";
import { Icon } from "./Icons";
import { Tooltip } from "./ui/Tooltip";

interface AppearanceSectionProps {
  config: AppearanceConfig;
  /** macOS 26+: `NSGlassEffectView` exists. */
  glassSupported: boolean;
  /** The stored background image, if any. */
  backgroundImageUrl?: string;
  /** Persist a change. */
  onChange: (config: AppearanceConfig) => Promise<void>;
  /** Show a change live without saving it: a colour picker or slider mid-drag. */
  onPreview: (config: AppearanceConfig) => void;
  /** Rust opens the picker, stores a copy and switches the backdrop to it. */
  onChooseImage: () => Promise<void>;
  onRemoveImage: () => Promise<void>;
}

const BACKDROPS: { value: BackdropMode; label: string; note: string }[] = [
  { value: "solid", label: "Solid", note: "One flat colour" },
  { value: "image", label: "Image", note: "A picture of your own" },
  { value: "glass", label: "Liquid Glass", note: "The desktop shows through" }
];

const GLASS_STYLES: { value: GlassStyle; label: string; note: string }[] = [
  { value: "frosted", label: "Frosted", note: "Heavier blur" },
  { value: "clear", label: "Clear", note: "More desktop" }
];

/** The chat switches each show a miniature of what they change, so the effect is visible
 *  before it is switched on. */
const CHAT_OPTIONS: { key: "thinkingPreview" | "messageBubbles" | "groupExploration"; label: string; description: string; preview: "bubble" | "thinking" | "explore" }[] = [
  {
    key: "messageBubbles",
    label: "Message bubbles",
    description: "Give the assistant's replies a bubble like yours. Tool rows, thinking and plan cards stay outside it.",
    preview: "bubble"
  },
  {
    key: "thinkingPreview",
    label: "Thinking preview",
    description: "Show a one-line gist of the model's reasoning beside “Thinking…”. Only models that stream their reasoning show one.",
    preview: "thinking"
  },
  {
    key: "groupExploration",
    label: "Group exploration",
    description: "Fold runs of file reads, searches and listings into one “Explored” row you can expand.",
    preview: "explore"
  }
];

/** The miniature exploration: one folded row when grouped, each call on its own row when not. */
const MINI_EXPLORATION: { icon: "file" | "search"; verb: string; subject: string }[] = [
  { icon: "file", verb: "Read", subject: "App.tsx" },
  { icon: "search", verb: "Searched", subject: "useTheme" },
  { icon: "search", verb: "Listed", subject: "src" }
];

const same = (first: string | null | undefined, second: string) => (first ?? "").toLowerCase() === second.toLowerCase();

/** What to save for a picked background: darkened for readability, and null for the default. */
function storedBackground(picked: string): string | null {
  const shown = clampBackground(picked);
  return same(shown, DEFAULT_BACKGROUND) ? null : shown;
}

export function AppearanceSection({ config, glassSupported, backgroundImageUrl, onChange, onPreview, onChooseImage, onRemoveImage }: AppearanceSectionProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  /** The last picked background was too light and got darkened; cleared by the next pick. */
  const [darkened, setDarkened] = useState(false);
  const theme = resolveTheme(config);
  const accent = config.accent ?? DEFAULT_ACCENT;
  const background = config.background ?? DEFAULT_BACKGROUND;

  async function run(action: () => Promise<void>): Promise<void> {
    setBusy(true);
    setError(undefined);
    try {
      await action();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }
  const save = (next: AppearanceConfig) => run(() => onChange(next));

  function pickBackdrop(backdrop: BackdropMode): void {
    // Image with nothing stored yet goes straight to the picker, which switches on success.
    if (backdrop === "image" && !config.backgroundImage) void run(onChooseImage);
    else void save({ ...config, backdrop });
  }

  function withBackground(picked: string): AppearanceConfig {
    // Stored darkened, so Rust paints the native window the colour the page shows.
    setDarkened(!same(clampBackground(picked), picked));
    return { ...config, background: storedBackground(picked) };
  }

  const withAccent = (picked: string): AppearanceConfig => ({ ...config, accent: same(picked, DEFAULT_ACCENT) ? null : picked.toLowerCase() });

  return (
    <div className="settings-scroll appearance-page">
      <div className="appearance-inner">
        <section className="tool-setting-group theme-group">
          <h4>Theme</h4>
          <div className="theme-presets" role="radiogroup" aria-label="Theme presets">
            {THEME_PRESETS.map((preset) => {
              const variables = resolveTheme(preset).variables;
              const checked = same(accent, preset.accent) && same(background, preset.background);
              return (
                <button
                  type="button"
                  role="radio"
                  aria-checked={checked}
                  key={preset.id}
                  className="theme-preset"
                  disabled={busy}
                  onClick={() => { setDarkened(false); void save({ ...withAccent(preset.accent), background: storedBackground(preset.background) }); }}
                >
                  <span className="theme-preset-preview" style={{ background: preset.background }} aria-hidden="true">
                    <span className="theme-preset-sidebar" style={{ background: variables["--wc-sidebar"], borderColor: variables["--border"] }} />
                    <span className="theme-preset-main">
                      <span className="theme-preset-line" style={{ background: variables["--border-bright"] }} />
                      <span className="theme-preset-line short" style={{ background: variables["--border"] }} />
                      <span className="theme-preset-composer" style={{ background: variables["--wc-composer"], borderColor: variables["--border-bright"] }}>
                        <span className="theme-preset-send" style={{ background: preset.accent }} />
                      </span>
                    </span>
                  </span>
                  <span className="theme-preset-name">{preset.name}</span>
                </button>
              );
            })}
          </div>

          <div className="appearance-pair">
            <ColourRow
              label="Accent colour"
              description={theme.accentAdjusted ? "Lightened slightly on this background so it reads as text." : "Buttons, links, focus rings and highlights."}
              value={accent}
              choices={THEME_PRESETS.map((preset) => ({ name: preset.name, value: preset.accent }))}
              isDefault={!config.accent}
              disabled={busy}
              onPick={(value) => void save(withAccent(value))}
              onPreview={(value) => onPreview(withAccent(value))}
              onReset={() => void save({ ...config, accent: null })}
            />
            <ColourRow
              label="Background colour"
              description={darkened ? "Darkened to keep text readable." : "The app's base colour. Surfaces and borders are shaded from it."}
              value={background}
              choices={THEME_PRESETS.map((preset) => ({ name: preset.name, value: preset.background }))}
              isDefault={!config.background}
              disabled={busy}
              onPick={(value) => void save(withBackground(value))}
              onPreview={(value) => onPreview({ ...config, background: clampBackground(value) })}
              onReset={() => { setDarkened(false); void save({ ...config, background: null }); }}
            />
          </div>
        </section>

        <section className="tool-setting-group">
          <h4>Backdrop</h4>
          <p className="appearance-group-note">What sits behind the window. Chat bubbles, cards and the composer stay solid whatever you pick.</p>
          <div className="appearance-tiles" role="radiogroup" aria-label="Backdrop">
            {BACKDROPS.map((option) => {
              const unavailable = option.value === "glass" && !glassSupported;
              const selected = config.backdrop === option.value;
              const tile = (
                <button
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  aria-label={option.label}
                  key={option.value}
                  className={`appearance-tile backdrop-tile ${selected ? "selected" : ""}`}
                  disabled={busy || unavailable}
                  onClick={() => pickBackdrop(option.value)}
                >
                  {/* Each tile paints the thing it names, so the choice reads at a glance. */}
                  <span className={`tile-art art-${option.value}`} aria-hidden="true">
                    {option.value === "solid" && <span className="art-solid-window" />}
                    {option.value === "image" && (
                      <>
                        <span className="art-sky" />
                        <span className="art-hills" />
                        <span className="art-window" />
                      </>
                    )}
                    {option.value === "glass" && (
                      <>
                        <span className="art-desk" />
                        <span className="art-window glassy" />
                      </>
                    )}
                  </span>
                  <span className="appearance-tile-text">
                    <span className="appearance-tile-label">{option.label}</span>
                    <span className="appearance-tile-note">{option.note}</span>
                  </span>
                </button>
              );
              return unavailable ? <Tooltip key={option.value} label="Liquid Glass needs macOS 26 or later">{tile}</Tooltip> : tile;
            })}
          </div>

          {config.backdrop === "image" && (
            <div className="appearance-subgroup">
              <div className="tool-setting appearance-setting background-image-setting">
                {backgroundImageUrl && <img className="background-thumb" src={backgroundImageUrl} alt="Current background" />}
                <div className="tool-setting-text">
                  <span className="tool-setting-name">Your image</span>
                  <span className="tool-setting-description">Shown clearly on the new-chat screen and dimmed and blurred behind chats. WackCode keeps its own copy; your original is never changed.</span>
                </div>
                <div className="row-actions">
                  <button type="button" className="secondary-button" disabled={busy} onClick={() => void run(onChooseImage)}><Icon name="image" /> Replace…</button>
                  <button type="button" className="secondary-button" disabled={busy} onClick={() => void run(onRemoveImage)}>Remove</button>
                </div>
              </div>
              <div className="appearance-pair">
                <SliderRow
                  label="Dim"
                  description="How much of the background colour covers the image behind a chat."
                  value={config.imageDim}
                  max={90}
                  unit="%"
                  disabled={busy}
                  onPreview={(imageDim) => onPreview({ ...config, imageDim })}
                  onCommit={(imageDim) => void save({ ...config, imageDim })}
                />
                <SliderRow
                  label="Blur"
                  description="Softens the image behind a chat so text stays easy to read."
                  value={config.imageBlur}
                  max={40}
                  unit=" px"
                  disabled={busy}
                  onPreview={(imageBlur) => onPreview({ ...config, imageBlur })}
                  onCommit={(imageBlur) => void save({ ...config, imageBlur })}
                />
              </div>
            </div>
          )}

          {config.backdrop === "glass" && (
            <div className="appearance-subgroup">
              <div className="tool-setting appearance-setting">
                <div className="tool-setting-text">
                  <span className="tool-setting-name">Glass style</span>
                  <span className="tool-setting-description">Frosted blurs the desktop more; Clear shows more of it. While another app is focused the window turns solid.</span>
                </div>
                <div className="segmented" role="radiogroup" aria-label="Glass style">
                  {GLASS_STYLES.map((option) => (
                    <button
                      type="button"
                      role="radio"
                      aria-checked={config.glassStyle === option.value}
                      key={option.value}
                      className={`segmented-option ${config.glassStyle === option.value ? "selected" : ""}`}
                      disabled={busy}
                      onClick={() => void save({ ...config, glassStyle: option.value })}
                    >
                      {option.label}
                    </button>
                  ))}
                </div>
              </div>
              <SliderRow
                label="Tint"
                description="How much of the background colour tints the glass. More tint keeps text readable over busy desktops."
                value={config.glassTint}
                max={90}
                unit="%"
                disabled={busy}
                onPreview={(glassTint) => onPreview({ ...config, glassTint })}
                onCommit={(glassTint) => void save({ ...config, glassTint })}
              />
            </div>
          )}
        </section>

        <section className="tool-setting-group">
          <h4>Chat</h4>
          <p className="appearance-group-note">How the conversation itself is drawn. Each switch shows its effect as you flip it.</p>
          <div className="appearance-pair">
            {CHAT_OPTIONS.map((option) => {
              const on = config[option.key];
              return (
                <div className={`chat-preview-card ${on ? "on" : ""}`} key={option.key}>
                  <div className="chat-preview" aria-hidden="true">
                    {option.preview === "bubble" ? (
                      <>
                        <span className="mini-msg mini-user">Looks good, ship it</span>
                        {/* Assistant prose: bubbled when the switch is on, flat when it is off. */}
                        <span className={`mini-msg mini-assistant ${on ? "bubbled" : "flat"}`}>Done — the tests pass.</span>
                      </>
                    ) : option.preview === "explore" ? (
                      <>
                        {on ? (
                          <span className="mini-tool">
                            <Icon name="search" className="mini-tool-icon" />
                            <span className="mini-tool-verb">Explored</span>
                            <span className="mini-tool-subject">· 1 file, 1 search, 1 list</span>
                            <Icon name="chevron" className="mini-tool-chevron" />
                          </span>
                        ) : MINI_EXPLORATION.map((row) => (
                          <span className="mini-tool" key={row.verb}>
                            <Icon name={row.icon} className="mini-tool-icon" />
                            <span className="mini-tool-verb">{row.verb}</span>
                            <span className="mini-tool-subject">{row.subject}</span>
                          </span>
                        ))}
                        <span className="mini-msg mini-assistant flat">Done — the tests pass.</span>
                      </>
                    ) : (
                      <>
                        <span className="mini-thinking">
                          <Icon name="spark" className="mini-thinking-icon" />
                          <span className="mini-thinking-label">Thinking…</span>
                          {on && <span className="mini-thinking-gist">weighing two approaches</span>}
                        </span>
                        <span className="mini-msg mini-assistant flat">Done — the tests pass.</span>
                      </>
                    )}
                  </div>
                  <div className="chat-preview-text">
                    <span className="tool-setting-name">{option.label}</span>
                    <span className="tool-setting-description">{option.description}</span>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={on}
                    aria-label={option.label}
                    className={`toggle ${on ? "on" : ""}`}
                    disabled={busy}
                    onClick={() => void save({ ...config, [option.key]: !on })}
                  >
                    <span />
                  </button>
                </div>
              );
            })}
          </div>
        </section>
        {error && <div className="error-banner">{error}</div>}
      </div>
    </div>
  );
}

interface ColourRowProps {
  label: string;
  description: string;
  value: string;
  choices: { name: string; value: string }[];
  isDefault: boolean;
  disabled: boolean;
  onPick: (value: string) => void;
  onPreview: (value: string) => void;
  onReset: () => void;
}

/** A settings row of colour swatches plus a native colour picker for anything else. */
function ColourRow({ label, description, value, choices, isDefault, disabled, onPick, onPreview, onReset }: ColourRowProps) {
  const custom = !choices.some((choice) => same(value, choice.value));
  const inputRef = useRef<HTMLInputElement>(null);
  // React's onChange fires on every drag step of the picker; the native change event fires once,
  // when the colour is committed, which is when it's worth saving.
  const commit = useRef(onPick);
  commit.current = onPick;
  useEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    const listener = () => commit.current(input.value);
    input.addEventListener("change", listener);
    return () => input.removeEventListener("change", listener);
  }, []);

  return (
    <div className="tool-setting appearance-setting colour-setting">
      <div className="tool-setting-text">
        <span className="tool-setting-name">{label}</span>
        <span className="tool-setting-description">{description}</span>
      </div>
      <div className="swatch-row" role="radiogroup" aria-label={label}>
        {choices.map((choice) => (
          <button
            type="button"
            role="radio"
            aria-checked={same(value, choice.value)}
            aria-label={`${choice.name} ${label.toLowerCase()}`}
            key={choice.value}
            className="swatch"
            style={{ background: choice.value }}
            disabled={disabled}
            onClick={() => onPick(choice.value)}
          />
        ))}
        <label className={`swatch swatch-custom ${custom ? "selected" : ""}`} style={custom ? { background: value } : undefined} title="Custom colour">
          <input
            ref={inputRef}
            type="color"
            aria-label={`Custom ${label.toLowerCase()}`}
            value={value}
            disabled={disabled}
            onChange={(event) => onPreview(event.target.value)}
          />
        </label>
        <button type="button" className="ghost-button swatch-reset" aria-label={`Reset ${label.toLowerCase()}`} title="Reset to default" disabled={disabled || isDefault} onClick={onReset}>
          <Icon name="refresh" />
        </button>
      </div>
    </div>
  );
}

interface SliderRowProps {
  label: string;
  description: string;
  value: number;
  max: number;
  unit: string;
  disabled: boolean;
  onPreview: (value: number) => void;
  onCommit: (value: number) => void;
}

/** A settings row with a range slider: previews while dragging, saves on release. */
function SliderRow({ label, description, value, max, unit, disabled, onPreview, onCommit }: SliderRowProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const commit = useRef(onCommit);
  commit.current = onCommit;
  // The native change event fires on release (or a keyboard step), not on every drag step.
  useEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    const listener = () => commit.current(Number(input.value));
    input.addEventListener("change", listener);
    return () => input.removeEventListener("change", listener);
  }, []);

  return (
    <div className="tool-setting appearance-setting">
      <div className="tool-setting-text">
        <span className="tool-setting-name">{label}</span>
        <span className="tool-setting-description">{description}</span>
      </div>
      <div className="appearance-slider">
        <input
          ref={inputRef}
          type="range"
          min={0}
          max={max}
          step={1}
          value={value}
          aria-label={label}
          disabled={disabled}
          style={{ "--fill": `${(value / max) * 100}%` } as CSSProperties}
          onChange={(event) => onPreview(Number(event.target.value))}
        />
        <output>{value}{unit}</output>
      </div>
    </div>
  );
}
