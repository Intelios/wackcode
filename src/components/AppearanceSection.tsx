import { useEffect, useRef, useState, type CSSProperties } from "react";
import { DEFAULT_ACCENT, DEFAULT_BACKGROUND, THEME_PRESETS, clampBackground, resolveTheme, swatchTint } from "../theme";
import { DEFAULT_AGENT_NAME, agentName } from "../agentName";
import type { ImageCrop } from "../backdrop-crop";
import type { AppearanceConfig, BackdropMode, GlassStyle, ThinkingTimerPrecision } from "../types";
import { Icon } from "./Icons";
import { PonderingDuck } from "./PonderingDuck";
import { BackdropCropEditor } from "./BackdropCropEditor";
import { Tooltip } from "./ui/Tooltip";

interface AppearanceSectionProps {
  config: AppearanceConfig;
  /** macOS 26+: `NSGlassEffectView` exists. */
  glassSupported: boolean;
  /** Asset URL for a stored background name; undefined before the data path is known. */
  backgroundImageUrlFor: (name: string) => string | undefined;
  /** Persist a change. */
  onChange: (config: AppearanceConfig) => Promise<void>;
  /** Show a change live without saving it: a colour picker or slider mid-drag. */
  onPreview: (config: AppearanceConfig) => void;
  /** Rust opens the picker, stores copies and switches the backdrop to the last one. */
  onChooseImage: () => Promise<void>;
  onRemoveImage: (name?: string) => Promise<void>;
  onSelectImage: (name: string) => Promise<void>;
  onShuffleImage: () => Promise<void>;
}

const BACKDROPS: { value: BackdropMode; label: string; note: string }[] = [
  { value: "solid", label: "Solid", note: "One flat colour" },
  { value: "image", label: "Image", note: "A picture of your own" },
  { value: "glass", label: "Liquid Glass", note: "The desktop shows through" },
  { value: "dynamic", label: "Dynamic Backgrounds", note: "Motion that follows your chat" }
];

const GLASS_STYLES: { value: GlassStyle; label: string; note: string }[] = [
  { value: "frosted", label: "Frosted", note: "Heavier blur" },
  { value: "clear", label: "Clear", note: "More desktop" }
];

/** The Chat group's thinking timer choice (ThinkingRow.tsx counts from it). */
const TIMER_PRECISIONS: { value: ThinkingTimerPrecision; label: string }[] = [
  { value: "second", label: "1s" },
  { value: "tenth", label: "0.1s" }
];

/** The chat switches each show a miniature of what they change, so the effect is visible
 *  before it is switched on. */
const CHAT_OPTIONS: { key: "chatTabs" | "thinkingPreview" | "messageBubbles" | "groupExploration" | "collapseCompletedWork" | "visualBanner"; label: string; description: (name: string) => string; preview: "tabs" | "bubble" | "thinking" | "explore" | "work" | "visual" }[] = [
  {
    key: "chatTabs",
    label: "Chat tabs",
    description: () => "Keep several chats and drafts open, each with its own reading position and panel. Tabs last until you quit.",
    preview: "tabs"
  },
  {
    key: "messageBubbles",
    label: "Message bubbles",
    description: (name) => `Give ${name}'s replies a bubble like yours. Tool rows, thinking and plan cards stay outside it.`,
    preview: "bubble"
  },
  {
    key: "thinkingPreview",
    label: "Thinking preview",
    description: () => "Show the model's reasoning flowing beside “Thinking…” as it streams. Only models that stream their reasoning show one.",
    preview: "thinking"
  },
  {
    key: "groupExploration",
    label: "Group exploration",
    description: () => "Fold runs of file reads, searches and listings into one “Explored” chip you can expand.",
    preview: "explore"
  },
  {
    key: "collapseCompletedWork",
    label: "Collapse completed work",
    description: () => "Keep the final reply visible and fold its steps behind “Worked for…”. Live work stays open.",
    preview: "work"
  },
  {
    key: "visualBanner",
    label: "Visual banner",
    description: () => "The title strip above a visual reply, with Expand and Copy. Switch it off and the card is the visual alone — the actions go with it.",
    preview: "visual"
  }
];

/** The miniature exploration: one folded chip when grouped, each call its own chip when not. */
const MINI_EXPLORATION: { icon: "file" | "search" | "folder"; verb: string; subject: string }[] = [
  { icon: "file", verb: "Read", subject: "App.tsx" },
  { icon: "search", verb: "Searched", subject: "useTheme" },
  { icon: "folder", verb: "Listed", subject: "src" }
];

/** A group's place in the page's entrance: they rise in one after another (styles.css). */
const enter = (index: number) => ({ "--i": index } as CSSProperties);

const same = (first: string | null | undefined, second: string) => (first ?? "").toLowerCase() === second.toLowerCase();

/** What to save for a picked background: darkened for readability, and null for the default. */
function storedBackground(picked: string): string | null {
  const shown = clampBackground(picked);
  return same(shown, DEFAULT_BACKGROUND) ? null : shown;
}

export function AppearanceSection({ config, glassSupported, backgroundImageUrlFor, onChange, onPreview, onChooseImage, onRemoveImage, onSelectImage, onShuffleImage }: AppearanceSectionProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  /** The last picked background was too light and got darkened; cleared by the next pick. */
  const [darkened, setDarkened] = useState(false);
  /** The name field's live text; committed on blur, so typing never writes `wackcode.json`. */
  const [draftName, setDraftName] = useState(config.agentName ?? "");
  const theme = resolveTheme(config);
  const accent = config.accent ?? DEFAULT_ACCENT;
  const background = config.background ?? DEFAULT_BACKGROUND;
  const name = agentName(config);
  /** The name the miniature draws: the draft as you type, falling back to the default an
   *  empty field saves. */
  const previewName = draftName.trim() || DEFAULT_AGENT_NAME;

  function commitName(): void {
    const next = draftName.trim() || null;
    if (next === (config.agentName ?? null)) return;
    void save({ ...config, agentName: next });
  }

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

  /** Save a crop onto the entry it belongs to: each saved image keeps its own crop. */
  const withCrop = (crop: ImageCrop): AppearanceConfig => ({
    ...config,
    backgroundImages: config.backgroundImages.map((image) =>
      image.name === config.backgroundImage ? { ...image, zoom: crop.zoom, x: crop.x, y: crop.y } : image)
  });

  const withAccent = (picked: string): AppearanceConfig => ({ ...config, accent: same(picked, DEFAULT_ACCENT) ? null : picked.toLowerCase() });

  return (
    <div className="settings-scroll appearance-page">
      <div className="appearance-inner">
        <section className="tool-setting-group theme-group" style={enter(0)}>
          <h4>Theme</h4>
          <div className="theme-presets" role="radiogroup" aria-label="Theme presets">
            {THEME_PRESETS.map((preset, index) => {
              const variables = resolveTheme(preset).variables;
              const checked = same(accent, preset.accent) && same(background, preset.background);
              return (
                <button
                  type="button"
                  role="radio"
                  aria-checked={checked}
                  key={preset.id}
                  className="theme-preset"
                  style={{ "--k": index } as CSSProperties}
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
              tinted
              onPick={(value) => void save(withBackground(value))}
              onPreview={(value) => onPreview({ ...config, background: clampBackground(value) })}
              onReset={() => { setDarkened(false); void save({ ...config, background: null }); }}
            />
          </div>
        </section>

        <section className="tool-setting-group" style={enter(1)}>
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
                    {option.value === "dynamic" && <><span className="art-colour-drift" /><span className="art-dynamic-orbit" /></>}
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

          {config.backdrop === "dynamic" && (
            <div className="appearance-subgroup">
              <div className="appearance-tiles" role="radiogroup" aria-label="Dynamic background style">
                {([
                  { id: "colourDrift", name: "Colour Drift", note: "Soft colour blooms gather energy as your chat works." },
                  { id: "glyphFlow", name: "Glyph Flow", note: "Click empty background to ripple through drifting characters." }
                ] as const).map((style) => (
                  <button type="button" role="radio" aria-checked={config.dynamicStyle === style.id}
                    aria-label={style.name} key={style.id} disabled={busy}
                    className={`appearance-tile ${config.dynamicStyle === style.id ? "selected" : ""}`}
                    onClick={() => void save({ ...config, dynamicStyle: style.id })}>
                    <span className="tile-art dynamic-style-art" aria-hidden="true">
                      {style.id === "colourDrift" ? <span className="art-colour-drift" /> : <span className="art-glyph-flow">{'. : + x 0 1 * . : + x 0 1 *\n : + x 0 1 * . : + x 0 1 *\n. : + x 0 1 * . : + x 0 1 *\n : + x 0 1 * . : + x 0 1 *'}</span>}
                    </span>
                    <span className="appearance-tile-text">
                      <span className="appearance-tile-label">{style.name}{style.id === "glyphFlow" && <span className="dynamic-reactive-tag">Reactive</span>}</span>
                      <span className="appearance-tile-note">{style.note}</span>
                    </span>
                  </button>
                ))}
              </div>
              <div className="tool-setting appearance-setting">
                <div className="tool-setting-text"><span className="tool-setting-name">Palette</span>
                  <span className="tool-setting-description">Follow your accent, or give the background its own colours.</span></div>
                <div className="segmented" role="radiogroup" aria-label="Dynamic background palette">
                  {(["theme", "aurora", "sunset"] as const).map((palette) => <button key={palette} type="button" role="radio"
                    aria-checked={config.dynamicPalette === palette} disabled={busy}
                    className={`segmented-option ${config.dynamicPalette === palette ? "selected" : ""}`}
                    onClick={() => void save({ ...config, dynamicPalette: palette })}>
                    <span className={`dynamic-palette-swatch palette-${palette}`} aria-hidden="true" />
                    {palette === "theme" ? "Theme" : palette === "aurora" ? "Aurora" : "Sunset"}
                  </button>)}
                </div>
              </div>
              <SliderRow label="Intensity" description="Expressive on the welcome screen, softer behind conversations. Zero turns the effect off."
                value={config.dynamicIntensity} max={100} unit="%" disabled={busy}
                onPreview={(dynamicIntensity) => onPreview({ ...config, dynamicIntensity })}
                onCommit={(dynamicIntensity) => void save({ ...config, dynamicIntensity })} />
              <p className="appearance-group-note">Motion pauses when you leave the app. With Reduce Motion, backgrounds stay still.</p>
            </div>
          )}

          {config.backdrop === "image" && (
            <div className="appearance-subgroup">
              <div className="tool-setting appearance-setting background-image-setting">
                <div className="tool-setting-text">
                  <span className="tool-setting-name">Your images</span>
                  <span className="tool-setting-description">Click one to show it — clearly on the new-chat screen, dimmed and blurred behind chats. WackCode keeps its own copies; your originals are never changed.</span>
                </div>
                <div className="row-actions">
                  <button type="button" className="secondary-button" disabled={busy} onClick={() => void run(onChooseImage)}><Icon name="plus" /> Add…</button>
                </div>
              </div>
              {config.backgroundImages.length > 0 && (
                <div className="background-strip" role="list" aria-label="Saved backgrounds">
                  {config.backgroundImages.map((image, index) => {
                    const url = backgroundImageUrlFor(image.name);
                    const current = image.name === config.backgroundImage;
                    return (
                      <div className="background-item" role="listitem" key={image.name}>
                        <button
                          type="button"
                          className={`background-thumb ${current ? "current" : ""}`}
                          aria-label={`Background ${index + 1}${current ? " (showing)" : ""}`}
                          aria-pressed={current}
                          disabled={busy || !url}
                          style={url ? { backgroundImage: `url("${url}")` } : undefined}
                          onClick={() => { if (!current) void run(() => onSelectImage(image.name)); }}
                        />
                        {/* Revealed on hover and on keyboard focus, like every hover-only control. */}
                        <button
                          type="button"
                          className="background-thumb-remove"
                          aria-label={`Remove background ${index + 1}`}
                          disabled={busy}
                          onClick={() => void run(() => onRemoveImage(image.name))}
                        >
                          <Icon name="close" />
                        </button>
                      </div>
                    );
                  })}
                </div>
              )}
              {config.backgroundImages.length > 1 && (
                <div className="tool-setting appearance-setting">
                  <div className="tool-setting-text">
                    <span className="tool-setting-name">Shuffle</span>
                    <span className="tool-setting-description">Show a different background each time you open WackCode. Shuffle now switches to another one straight away.</span>
                  </div>
                  <div className="row-actions">
                    <button type="button" className="secondary-button" disabled={busy} onClick={() => void run(onShuffleImage)}><Icon name="shuffle" /> Shuffle now</button>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={config.shuffleBackgrounds}
                      aria-label="Shuffle"
                      className={`toggle ${config.shuffleBackgrounds ? "on" : ""}`}
                      disabled={busy}
                      onClick={() => void save({ ...config, shuffleBackgrounds: !config.shuffleBackgrounds })}
                    >
                      <span />
                    </button>
                  </div>
                </div>
              )}
              {(() => {
                const shown = config.backgroundImages.find((image) => image.name === config.backgroundImage);
                const url = shown && backgroundImageUrlFor(shown.name);
                return shown && url ? (
                  <BackdropCropEditor
                    imageUrl={url}
                    crop={{ zoom: shown.zoom, x: shown.x, y: shown.y }}
                    disabled={busy}
                    onPreview={(crop) => onPreview(withCrop(crop))}
                    onCommit={(crop) => void save(withCrop(crop))}
                  />
                ) : null;
              })()}
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

        <section className="tool-setting-group" style={enter(2)}>
          <h4>Chat</h4>
          <p className="appearance-group-note">How the conversation itself is drawn. Each switch shows its effect as you flip it.</p>
          <div className="appearance-pair">
            {CHAT_OPTIONS.map((option) => {
              const on = config[option.key];
              return (
                <div className={`chat-preview-card ${on ? "on" : ""}`} key={option.key}>
                  <div className="chat-preview" aria-hidden="true">
                    {option.preview === "tabs" ? (
                      <>
                        <span className={`mini-tabs ${on ? "enabled" : ""}`}><span>Build the app</span>{on && <><span>Fix the tests</span><Icon name="plus" /></>}</span>
                        <span className="mini-msg mini-assistant flat">Ready when you are.</span>
                      </>
                    ) : option.preview === "bubble" ? (
                      <>
                        <span className="mini-msg mini-user">Looks good, ship it</span>
                        {/* Assistant prose: bubbled when the switch is on, flat when it is off. */}
                        <span className={`mini-msg mini-assistant ${on ? "bubbled" : "flat"}`}>Done — the tests pass.</span>
                      </>
                    ) : option.preview === "work" ? (
                      <>
                        {on ? (
                          <span className="mini-work">
                            <Icon name="clock" className="mini-tool-icon" />
                            <span className="mini-work-label">Worked for 12s</span>
                            <Icon name="chevron" className="mini-tool-chevron" />
                          </span>
                        ) : (
                          <span className="mini-work-steps">
                            <span className="mini-thinking">
                              <PonderingDuck live={false} className="mini-thinking-icon" />
                              <span>Thought for 2s</span>
                            </span>
                            <span className="mini-tool">
                              <Icon name="quill" className="mini-tool-icon" />
                              <span className="mini-tool-verb">Edited</span>
                              <span className="mini-tool-subject">App.tsx</span>
                            </span>
                            <span className="mini-tool">
                              <Icon name="terminal" className="mini-tool-icon" />
                              <span className="mini-tool-verb">Ran</span>
                              <span className="mini-tool-subject">pnpm test</span>
                            </span>
                          </span>
                        )}
                        <span className="mini-msg mini-assistant flat">Done — the tests pass.</span>
                      </>
                    ) : option.preview === "explore" ? (
                      <>
                        {on ? (
                          <span className="mini-tool mini-chip mini-fold">
                            <span className="mini-stack">
                              <Icon name="file" /><Icon name="search" /><Icon name="folder" />
                            </span>
                            <span className="mini-tool-verb">Explored</span>
                            <span className="mini-tool-subject">· 1 file, 1 search, 1 list</span>
                            <Icon name="chevron" className="mini-tool-chevron" />
                          </span>
                        ) : (
                          <span className="mini-chips">
                            {MINI_EXPLORATION.map((row) => (
                              <span className="mini-tool mini-chip" key={row.verb}>
                                <Icon name={row.icon} className="mini-tool-icon" />
                                <span className="mini-tool-verb">{row.verb}</span>
                                <span className="mini-tool-subject">{row.subject}</span>
                              </span>
                            ))}
                          </span>
                        )}
                        <span className="mini-msg mini-assistant flat">Done — the tests pass.</span>
                      </>
                    ) : option.preview === "visual" ? (
                      /* The card's head band wears the switch: present when on, gone when off. */
                      <span className="mini-visual">
                        {on && (
                          <span className="mini-visual-head">
                            <span className="mini-visual-mark" />
                            <span className="mini-visual-title" />
                            <span className="mini-visual-action" />
                          </span>
                        )}
                        <span className="mini-visual-body">
                          <span style={{ height: "50%" }} />
                          <span style={{ height: "85%" }} />
                          <span style={{ height: "35%" }} />
                          <span style={{ height: "65%" }} />
                        </span>
                      </span>
                    ) : (
                      <>
                        <span className="mini-thinking">
                          <PonderingDuck live className="mini-thinking-icon" />
                          <span className="mini-thinking-label">Thinking…</span>
                          {on && (
                            <span className="thinking-stream">
                              <span className="thinking-stream-text">weighing two approaches, the second keeps the</span>
                              <span className="thinking-stream-caret" />
                            </span>
                          )}
                        </span>
                        <span className="mini-msg mini-assistant flat">Done — the tests pass.</span>
                      </>
                    )}
                  </div>
                  <div className="chat-preview-text">
                    <span className="tool-setting-name">{option.label}</span>
                    <span className="tool-setting-description">{option.description(name)}</span>
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
          <div className="tool-setting appearance-setting">
            <div className="tool-setting-text">
              <span className="tool-setting-name">Thinking timer</span>
              <span className="tool-setting-description">Count the time beside “Thinking…” in whole seconds or tenths. Finished thoughts match: “Thought for 4.2s”.</span>
            </div>
            <div className="segmented" role="radiogroup" aria-label="Thinking timer">
              {TIMER_PRECISIONS.map((option) => (
                <button
                  type="button"
                  role="radio"
                  aria-checked={config.thinkingTimerPrecision === option.value}
                  key={option.value}
                  className={`segmented-option ${config.thinkingTimerPrecision === option.value ? "selected" : ""}`}
                  disabled={busy}
                  onClick={() => void save({ ...config, thinkingTimerPrecision: option.value })}
                >
                  {option.label}
                </button>
              ))}
            </div>
          </div>
        </section>

        <section className="tool-setting-group" style={enter(3)}>
          <h4>Agent</h4>
          <p className="appearance-group-note">The name the app uses for your assistant. The miniature follows the field as you type.</p>
          {/* Same card as the chat switches, showing the empty chat the name will meet. */}
          <div className="chat-preview-card agent-name-card">
            <div className="chat-preview" aria-hidden="true">
              <span className="agent-greeting">What should we build?</span>
              <span className="agent-greeting-sub">Describe the change, bug, or question — {previewName} can read this project, run commands, and edit files.</span>
              <span className="mini-composer">
                <span className="mini-composer-text">Ask {previewName} to inspect, change, or run something…</span>
                <span className="mini-composer-send" />
              </span>
            </div>
            <div className="chat-preview-text">
              <span className="tool-setting-name">Name</span>
              <span className="tool-setting-description">What the app calls your agent — in the composer, while it works, and in the empty chat.</span>
            </div>
            <input
              className="agent-name-input"
              value={draftName}
              onChange={(event) => setDraftName(event.target.value)}
              onBlur={commitName}
              onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }}
              maxLength={40}
              disabled={busy}
              placeholder={DEFAULT_AGENT_NAME}
              aria-label="Agent name"
            />
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
  /** Ring each swatch in its own lifted tint: the backgrounds are near-black and would
   *  otherwise read as one row of identical dark dots. */
  tinted?: boolean;
  onPick: (value: string) => void;
  onPreview: (value: string) => void;
  onReset: () => void;
}

/** A settings row of colour swatches plus a native colour picker for anything else. */
function ColourRow({ label, description, value, choices, isDefault, disabled, tinted, onPick, onPreview, onReset }: ColourRowProps) {
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

  // A tinted row paints the fill as picked and hands the CSS its rim as a custom property,
  // which keeps the two in one place instead of splitting them across a class and a style.
  const fill = (colour: string): CSSProperties =>
    tinted ? ({ background: colour, "--swatch-tint": swatchTint(colour) } as CSSProperties) : { background: colour };

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
            className={`swatch ${tinted ? "swatch-tint" : ""}`}
            style={fill(choice.value)}
            disabled={disabled}
            onClick={() => onPick(choice.value)}
          />
        ))}
        <label
          className={`swatch swatch-custom ${custom ? "selected" : ""} ${tinted && custom ? "swatch-tint" : ""}`}
          style={custom ? fill(value) : undefined}
          title="Custom colour"
        >
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
