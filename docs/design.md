# Design toolkit

The principles are in [AGENTS.md](../AGENTS.md#design). This is how to follow them with what the app already has.

## Character

WackCode is a dark, compact macOS workspace with a lime accent (`#c2ee4a` on `#111310` by default), a duck mascot (the app icon and the menu bar; `DuckMark` bobs as if afloat on the boot screen and ponders — head tilting, bubbles rising — beside reasoning rows in `PonderingDuck`), and a robot for sub-agents (`RobotMark`). Ultra Plan gets its own warm, fiery treatment so it never reads as ordinary Plan. New surfaces should feel like they belong to this cast rather than to a component library.

## Type

- Inter when installed, otherwise the system font. No web fonts: the CSP only allows `font-src 'self'`. Base size is 14px; rows sit around 13px and metadata at 10.5 to 12px.
- `"SFMono-Regular", Consolas, monospace` for code, file paths, branch names, and the names of tools, skills, commands and MCP servers.

## Colour

- **Adding a colour:** add a token to `TOKENS` in `src/theme.ts` (a recipe anchored on its default hex), add the same default under `:root` in `styles.css`, and use `var(--wc-…)`. `theme.test.ts` fails if the two disagree or a raw hex appears below `:root`.
- **Glows and tints:** `color-mix(in srgb, var(--wc-accent) N%, transparent)`, never an `rgb()` of the default accent.
- **Fixed meaning:** danger, warning, diff, success, the stop button and Ultra Plan (`--wc-ultra*`) keep their colours in every theme. The literals allowed below `:root` are listed in `SEMANTIC_COLOURS` in `theme.test.ts`.
- **Contrast is automatic:** a background too light for the app's fixed light text is darkened, and an accent too dark to read is lightened. Text on the accent uses `--wc-on-accent`.
- **Surfaces**, darkest to lightest: `--wc-well`, `--wc-sidebar`, `--wc-input`, `--wc-panel`, `--bg`, `--surface`, `--wc-composer`, `--wc-elevated`, `--surface-raised`, `--wc-menu`. Text: `--text`, `--wc-text-body`, `--text-soft`, `--text-dim`, `--text-faint`. Lines: `--border`, `--border-bright`.
- **See-through shell:** only `--wc-shell`, `--wc-header`, `--wc-sidebar`, `--wc-panel` and `--wc-composer-fade` turn translucent over an image or Liquid Glass (`shellVariables` in `theme.ts`). Content (bubbles, composer, cards, dialogs) never does.
- Code highlighting (`--wc-code-*`) and the terminal palette (`--wc-term-*`, bright variants from `terminal-theme.ts`) are derived from the accent, so check them in a few presets.

Test a new surface in at least the default, Mono and one colourful preset, with an image backdrop, and with Liquid Glass if you're on macOS 26.

## Motion

- **motion/react** (`motion`, `AnimatePresence`, `LayoutGroup`) for enter/exit, layout moves and anything tied to state: panel swaps, list reordering, the composer's hero→dock glide. Use springs for things that move. Read `useReducedMotion()` and drop transforms when it's true.
- **CSS** for hover feedback, ambient loops and choreography: transitions use `var(--ease)`; the shell's staggered `rebuild-in` entrance, the sidebar tiles' liquid outline, pulsing status dots, the send comet, the Ultra Plan flame with its embers.
- **Git mode's cast:** the composer ducks under the window as the mode opens (`composer-duck`, on the layer) and surfaces on the way out; the sync button's arrow launches on click and the send comet's ring runs round it while Git talks to the remote; committed rows zip up toward History as a "+1" floats off its tab; the duck splashes down on a clean tree.
- **Direction means something:** side-panel views slide by their order (`swapDirection` in `side-panel.ts`); sub-agent tabs slide only the transcript.
- **Names get a beat:** when the auto-title model names a chat, the header's title swipes to the new name with an accent glint passing through the letters, and its sidebar row crossfades at the same moment (`TextSwap`, driven by `titlePulses`).
- **Cheap properties:** animate `transform` and `opacity`. The image backdrop crossfades two pre-blurred layers rather than animating the blur.
- **Reduced motion:** a global rule collapses every duration to `.01ms`. A looping animation therefore also needs its own `@media (prefers-reduced-motion: reduce)` override beside it, setting a still end state so it doesn't flicker. The welcome comet is the one deliberate ambient exception.
- **Don't fight another animation.** Where Framer owns an element's transform (the composer), a CSS animation moves a wrapper instead.

## Components

- **Primitives** (`src/components/ui/`): `ConfirmDialog` (use `danger` for destructive actions), `Menu` / `MenuButton`, `Popover`, `Select`, `Tooltip`, `Checkbox` (a native checkbox, tri-state, with a drawn tick), `ImageLightbox`, and `useConfirmAction` (the sidebar's two-click "Delete?" arm).
- **Buttons** (`styles.css`): `.primary-button`, `.secondary-button`, `.danger-button` (add `.compact` for dense rows), `.icon-button`, `.ghost-button`, `.text-button`, `.panel-button` in the chat header.
- **Icons** (`Icons.tsx`): one hand-drawn set on a 24px grid, 1.8 stroke, round caps, `currentColor`. Add a path and a name to `IconName`; don't pull in an icon library.
- **Settings pages** take the finished shape: a centred `.settings-page` column that opens with `SettingsHero` (an illustrated stage whose loop shows the page's state, a status pill, the page's main switch or action) and continues in solid `.settings-block` cards. Every page uses the shared parts except Computer use and Prompts (the same shape in their own classes) and Appearance (a centred column of its own). A page's detail views (a skill, a connection, an editor) open in the same column under a back link or the hero; an editor that saves explicitly shows its save bar only while it has changes (Settings › Providers). An empty state explains the feature and offers the ways in, rather than one grey line.
- **The agent asking the user** (questions, extension dialogs, the computer-use access card) renders inline above the composer (`InlineDialog`), not as a modal. Live status (goal loop, computer use) goes in a banner above the composer.

## Interaction and accessibility

- Icon-only buttons have an `aria-label`; square tiles put their label in a `Tooltip`.
- Actions revealed on hover (message actions, file rows) also appear on keyboard focus. Focus rings come from the global `:focus-visible` rule.
- Escape closes dialogs, popovers and menus.
- Show keyboard shortcuts with macOS glyphs (⌘⇧⌥⌃) in `<kbd>`.
- Components are tested with Testing Library role-based queries, so real roles and labels matter.

## Copy

- Short, plain sentences in the second person. Errors say what happened and what to do; Rust errors are shown to users verbatim, so write them as sentences.
- Name settings as `Settings › Section`.
- The agent is `agentName(...)` / `AssistantNameContext` (`src/agentName.ts`), never "Pi". Only genuine Pi product references (the package catalogue, subscription sign-in, the worker process) say Pi.
- User-facing behaviour changes also update `README.md`.
