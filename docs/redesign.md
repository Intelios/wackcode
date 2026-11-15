# The UI redesign

Agreed with the owner on 2026-11-15, on a design canvas (claude.ai artifact "WackCode 3 UI Redesign", https://claude.ai/artifact/FCzbBYRTkzpCUx6huFips9). The canvas is the picture to build toward. This file is what was decided and the order it lands in. Each phase is its own commit, checked in the gallery and the dev app, light and dark, over a picture and without one.

## Decisions

- **The window is a frame and a stage.** The sidebar and the stage are panes with an 8-point gutter around them, and the chat tabs sit on pills in the title strip between them.
- **Over a picture, the panes are frosted glass.** The picture fills the window, blurred and dimmed as the user chose; the owner wants to keep seeing it. The glass is never thinner than `theme::GLASS_MIN` and thickens only where text would not read. A picture behind just the title strip, with plain ground below it, was considered and rejected.
- **The user sets blur and dim** in Settings › Appearance, as in WackCode 2.
- **New chat:** the picture shows clear across the top of the stage, then dissolves (optionally through a texture) into the glass around the centred block.
- **The theme colour is always the user's.** Nothing taken from the picture overrides it.
- **Pills and circles are fine.** The serif is used sparingly: a screen's one display line, not every label.
- **Composer:** round controls and a mode track. The model gets its own pill, and **effort gets its own pill** opening WackCode 2's slider (Low, Medium, High, Xhigh, Max, Ultra) rather than living inside the model menu. While a run is going, status pills sit above the composer (time and what it is doing, the todo list, the queue). Ultra Plan wears the prism around the composer.
- **Transcript:** the user's message is a bubble on the right. "Worked for 19s" is a chip with a dotted rule and a one-line summary, and opens into compact rows: a round icon, the verb, the target in mono, its meta. Diffs and code are sheets with a head band.
- **Settings:** grouped section navigation, sized for about 14 sections. A page opens with a title and its primary action, then cards of rows: label and description on the left, the control on the right. Connections are rows with a switch.

## Phases

1. **Frame, stage, glass, blur and dim** (done): `shell.rs`, `glass.rs`, `backdrop.rs`, `theme.rs` (`Theme::glass`), and the Blur choice on Settings › Appearance.
2. **Sidebar and title strip:** the New chat pill, round project monograms, chats on a dotted rail, and tab pills with state marks.
3. **Transcript:** bubbles, the work chip and rows, sheets.
4. **Composer:** round controls, the effort pill and slider, status pills, the Ultra prism.
5. **Settings:** grouped navigation, cards and rows, switches (a connection's on and off is new engine state), and sliders for Blur and Dim replacing the words.

## Open with the owner

- **Binding a theme colour to a picture.** WackCode 2 bound a theme preset to each picture in a library. Version 3 has no presets and one picture, so its shape needs agreeing before Settings is built.
