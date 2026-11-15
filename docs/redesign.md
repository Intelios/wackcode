# The UI redesign

Agreed with the owner on 2026-11-15, on a design canvas (claude.ai artifact "WackCode 3 UI Redesign", https://claude.ai/artifact/FCzbBYRTkzpCUx6huFips9). The canvas is the picture to build toward. This file is what was decided and the order it lands in. Each phase is its own commit, checked in the gallery and the dev app, light and dark, over a picture and without one.

## Decisions

- **The window is a frame and a stage.** The sidebar and the stage are panes with an 8-point gutter around them, and the chat tabs sit on pills in the title strip between them.
- **Over a picture, the panes are frosted glass.** The picture fills the window, blurred and dimmed as the user chose; the owner wants to keep seeing it. The glass is never thinner than `theme::GLASS_MIN` and thickens only where text would not read. A picture behind just the title strip, with plain ground below it, was considered and rejected.
- **The user sets blur and dim** in Settings › Appearance, as in WackCode 2.
- **New chat and welcome: the picture is shown, not dimmed.** The whole stage shows it clear (optionally through a texture). The smaller words sit on frosted pills and the display line has a faint halo of the window's colour; nothing is laid over the picture (a feathered patch of glass behind the heading was tried and removed on the owner's call). When the first message is sent, the blur and dim ease in as the picture fades to the glass (and back on New chat). Decided 2026-11-15 after phase 1: a stage full of glass on the new-chat screen read as dimming.
- **Themes are chosen, never made.** The user picks one of a fixed set of themes, WackCode 2's (WackCode, Midnight, Lagoon, Verdant, Grape, Nebula, Rosé, Solar, Ember, Mono; four with gradient accents), plus light or dark. No hand-picked colours: the paint chips and hex fields go. Nothing taken from the picture overrides the theme. Decided 2026-11-15, reversing the earlier cut of theme presets.
- **Pills and circles are fine.** The serif is used sparingly: a screen's one display line, not every label.
- **Composer:** round controls and a mode track. The model gets its own pill, and **effort gets its own pill** opening WackCode 2's slider (Low, Medium, High, Xhigh, Max, Ultra) rather than living inside the model menu. While a run is going, status pills sit above the composer (time and what it is doing, the todo list, the queue). Ultra Plan wears the prism around the composer.
- **Transcript:** the user's message is a bubble on the right. "Worked for 19s" is a chip with a dotted rule and a one-line summary, and opens into compact rows: a round icon, the verb, the target in mono, its meta. Diffs and code are sheets with a head band.
- **Settings:** grouped section navigation, sized for about 14 sections. A page opens with a title and its primary action, then cards of rows: label and description on the left, the control on the right. Connections are rows with a switch.

## Phases

1. **Frame, stage, glass, blur and dim** (done): `shell.rs`, `glass.rs`, `backdrop.rs`, `theme.rs` (`Theme::glass`), and the Blur choice on Settings › Appearance.
2. **Sidebar and title strip** (done): the New chat pill, round project monograms, chats on a dotted rail, tab pills with the sidebar's state marks and a new-tab button; and the new-chat screen showing the picture clear, easing to glass when a chat opens. The canvas's Search chats field and Archived button wait until those exist.
3. **Transcript:** bubbles, the work chip and rows, sheets.
4. **Composer:** round controls, the effort pill and slider, status pills, the Ultra prism.
5. **Settings:** grouped navigation, cards and rows, switches (a connection's on and off is new engine state), and sliders for Blur and Dim replacing the words.
6. **Themes:** WackCode 2's set ported from the legacy source (never its data files), replacing theme colour and accent in the engine's appearance and on Settings › Appearance.

## Open with the owner

- **Binding a theme to a picture.** WackCode 2 bound a theme to each picture in a library. Themes are coming back (phase 6); whether the picture library comes back with the binding is still to agree.
- **Themes in light mode.** WackCode 2's theme backgrounds are all near-black; how each theme reads in light is to agree before phase 6.
- **Gradient accents.** Four of WackCode 2's themes paint the accent as a gradient; whether that comes back with them.
