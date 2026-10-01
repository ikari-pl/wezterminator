---
date: 2026-10-01
topic: wezterminator
---

# wezterminator — requirements

## Summary

wezterminator is a public WezTerm theming and configuration project. A data-driven Lua engine applies composable presets that can be switched instantly from inside WezTerm. A Rust toolkit with a TUI configures everything, authors themes, generates all their art for any display, and installs the setup. Each person's own machines run it through a private layer on top of the public one, and each machine keeps its own local tweaks.

---

## Problem Frame

ikari's WezTerm setups grew separately on each machine and drifted apart. metis runs a modular config with CPC pixel-art parallax, a non-blocking sparkline status bar, and fzf pickers that preview live. OD-Cezar, a work laptop, runs one 925-line file with a softer translucent nebula, a pill status bar that blocks the GUI thread on every probe, and a smaller font list. Features were ported between them by hand, and nobody else can use any of it, because it is full of one person's hostnames, VPNs, work Linear org and display resolution.

"Theme" already means three things in these configs: a WezTerm colour scheme, a background-art bundle (`cool` / `warm`), and the whole machine's look. None of them can be mixed freely. Making a new look means editing Lua and Python by hand.

---

## Key Decisions

- **Public project.** wezterminator is for anyone who installs WezTerm. Nothing in the public repo assumes a particular person, machine, network, employer or display.
- **macOS and Linux fully supported, Windows best-effort.** Every feature works on macOS and Linux. On Windows the presets, art and TUI work, and status segments without a Windows probe are hidden.
- **A theme is a preset of independent parts.** A preset names a choice for each part (background art, colour scheme, palette, font, window chrome, status-bar style, motion), and any part can be swapped on its own.
- **Switching runs in Lua; making runs in Rust.** Daily preset switching works from inside WezTerm with no dependency on the Rust binary. The Rust tool writes the data and art the Lua engine reads, and previews by signalling the running WezTerm.
- **Three settings layers.** Public built-ins ship in the repo. An optional private fleet layer is shared across one person's machines, for example through a private git repo. Per-machine local overrides are never committed. A tweak can be promoted from local to the fleet layer, or offered to the public repo.
- **Installer offers three modes.** The user chooses at install time: add-on (their existing config stays, and their settings win where they overlap), replace with backup, or replace and import their colours, font and keys as a personal preset.
- **Every asset is generated.** The Rust tool generates all background layers, sprites and motifs procedurally at the user's real display resolution. External images, such as Retro Diffusion output, are an optional import. No theme depends on them, and no asset is copied from anywhere.
- **Rust.** The TUI, art generator and installer are written in Rust.
- **Origin looks ship neutral.** The metis and OD-Cezar looks become public presets under neutral names, carrying looks only. ikari's real machine profiles live in his private fleet layer.

---

## Requirements

**Presets and switching**

- R1. The repo ships twelve built-in presets (see Theme Library).
- R2. A preset can be switched from inside WezTerm (command palette, and a key binding to cycle presets or open a picker) without launching the TUI.
- R3. Switching a preset applies live to the window without restarting WezTerm. The switch persists across restarts.
- R4. The user can save the current combination of parts as a named preset in either their fleet layer or the local layer. Saved presets appear alongside built-ins everywhere presets are listed.
- R5. A local preset or tweak can be promoted to the fleet layer, or exported as a contribution to the public repo.
- R6. Switching presets keeps the "undo last change" behaviour the colour-scheme picker has today.
- R7. Every preset declares fallback fonts, so it renders acceptably when its preferred font is not installed.

**Theme parts**

- R8. Each of these parts can be chosen independently of the preset: background art, colour scheme, UI palette (tab bar and status colours), font, window chrome, status-bar style, and motion.
- R9. Window chrome covers background opacity, blur, padding, inactive-pane dimming and tab-bar placement. A chrome option the platform does not support is shown as unavailable rather than silently ignored.
- R10. Status-bar style covers at least a sparkline style and a pill style. Segments and their order can be configured.
- R11. Available status segments include load, memory, memory pressure, Tailscale, WARP, other tunnels, AWS VPN profile, cwd, font, battery, last exit code, clock and workspace.
- R12. Every status segment works on macOS and Linux. A segment with no probe on the current platform is hidden and listed as unavailable in the TUI.
- R13. Status collection never blocks the WezTerm GUI thread.
- R14. Status colours are semantic (ok / warn / bad / accent and so on) and come from the active palette.

**Backgrounds and motion**

- R15. Each theme's background is a stack of layers, with parallax wherever depth reads well.
- R16. Background motion supports vertical parallax from scrollback, vertical and horizontal virtual scroll driven by ALT+wheel, and optional time-driven auto-scroll. Auto-scroll can be on or off, with a speed setting.
- R17. Parallax factors, layer opacities and auto-scroll can be configured per theme and overridden in the fleet or local layer.
- R18. Backgrounds can be paused (base colour only) and restored.
- R19. Text stays legible over the densest part of every background, verified by measurement rather than by eye.

**Fonts**

- R20. The TUI lists the fonts installed on the machine and marks the ones each preset prefers. It flags any font missing full Polish diacritic coverage (ąćęłńóśżź / ĄĆĘŁŃÓŚŻŹ) or Nerd Font glyphs the status bar needs.
- R21. Font size is a global base plus a per-font correction, both adjustable live.

**TUI**

- R22. The TUI configures theme parts, fonts, chrome, status bar, motion, key bindings, machine settings, theme authoring, and install or sync.
- R23. When the TUI runs inside WezTerm, the WezTerm window previews the selected combination live while the user moves through choices. Cancelling reverts to what was there before.
- R24. When the TUI runs outside WezTerm, it shows an approximate visual preview in a local browser.
- R25. For every setting, the TUI shows which layer the value comes from (built-in, fleet, local), and in add-on mode whether the user's own config overrules it.
- R26. Key bindings can be viewed and rebound, and conflicts are surfaced, including conflicts with the user's own config in add-on mode.
- R27. Machine settings cover project roots, the issue-tracker URL pattern used by quick-select, which VPN probes to run, and the editor used by "Edit config". None of these has a personal default.

**Theme authoring and art**

- R28. The TUI can create a theme, edit its palette and parts, and regenerate its art.
- R29. The Rust tool generates every built-in theme's layers, sprites and motifs procedurally, deterministically from a seed, at the target display's device resolution.
- R30. External images can be imported as layers. They are quantised and dithered to the theme's palette when imported.
- R31. Game-inspired themes use only generated original art and carry no trademarked names in user-facing preset names.

**Install and machines**

- R32. The installer asks which mode to use: add-on, replace with backup, or replace and import. Every mode can be undone with one command.
- R33. Installing keeps WezTerm's live reload working, whatever config location the user's platform and existing setup use.
- R34. Installing migrates any existing wezterminator-style state files into the machine's local layer.
- R35. A health check reports missing preferred fonts, failed glyph coverage, missing art at the current resolution, unavailable status probes, and an out-of-date install.
- R36. A user can attach a private fleet layer from a git URL and pull updates to it.
- R37. The tool can push the user's setup to another of their machines over SSH, reaching it by LAN hostname when the overlay VPN reports it offline.
- R38. Everything metis has today keeps working: the persistent mux domain, project-workspace picker, quick-select, CPU, memory and workspace menus, the command palette and leader keys.

---

## Theme Library

| Preset | Origin | Direction |
|---|---|---|
| CPC Cool | metis `cool` | Amstrad CPC 27-colour palette, dithered nebula, stars, perspective grid, scanlines |
| Ember | metis `warm` | Gruvbox-flavoured ember nebula over a luminance-8 base |
| Soft Nebula | OD-Cezar | Gaussian nebula, twinkling stars, translucent and blurred window, Catppuccin-like palette, pill status |
| Phosphor | new | P1 green CRT: green-led but not strictly monochrome, with afterglow, rolling bar, bloom and vignette |
| Amber | new | Amber P3 phosphor CRT companion to Phosphor |
| Abyssal | new | Bioluminescent deep sea: caustics, drifting plankton, jellyfish glows; teal with magenta and coral |
| Washi | new | Light theme: paper fibre, sumi-ink washes, vermilion stamp; ink text |
| Wycinanki | new | Łowicz paper-cut folk motifs on black, in saturated folk colours |
| Platformer | new, inspired by Mario Bros | Side-scrolling world with horizontal parallax: hills, clouds, brick strata |
| Brickfield | new, inspired by Arkanoid | Space-brick field with paddle-and-ball geometry over a starfield |
| Gravekeep | new, inspired by Graveyard Keeper | Moody pixel graveyard: fog banks, gravestones, crooked trees, lantern glow |
| Isoville | new, inspired by SimCity 2000 | Isometric city skyline and terrain in that era's palette |

Preset names in this table are working names.

---

## Acceptance Examples

- AE1. **Covers R2, R3.** Given Soft Nebula is active, when the user picks Abyssal from the WezTerm command palette, the window re-themes without a restart. After relaunching WezTerm, Abyssal is still active.
- AE2. **Covers R4, R5, R25.** Given ikari switches CPC Cool's status style to pills and saves it as "cool-pills" in his local layer, then it is listed on that machine only, and the TUI shows the status style as local. When he promotes it to his fleet layer, it appears on his other machines after they pull.
- AE3. **Covers R23.** Given the TUI runs in a WezTerm pane, when the user moves across presets, the window previews each one. Pressing escape restores the preset that was active before.
- AE4. **Covers R24.** Given the TUI runs in another terminal, the preview opens as an approximate rendering in the browser, and no WezTerm window changes.
- AE5. **Covers R32, R25.** Given a stranger with an existing config installs in add-on mode, their own font setting stays in effect when they switch to Phosphor. The TUI shows Phosphor's font as overruled by their config.
- AE6. **Covers R7, R35.** Given a fresh Linux machine without Terminess installed, CPC Cool renders with its fallback font, and the health check names the missing font.
- AE7. **Covers R12, R13.** Given a Linux machine with Tailscale and no WARP, the Tailscale segment works, WARP is listed as unavailable, and the GUI never waits on a probe.
- AE8. **Covers R29, R35.** Given a display whose resolution differs from the generated art, the health check reports the mismatch. Regenerating produces art at that display's device resolution, with no Python required.

---

## Scope Boundaries

**Deferred for later**

- Peer-to-peer sync of fleet layers or art over a BitTorrent-based protocol (tracked as `wzt-lz6`). v1 moves settings between machines with git and SSH push.
- Full Windows parity for status probes and chrome.

**Outside this product's identity**

- A resident background daemon. The setup must work with the Rust binary absent.
- Copied or ripped sprites, assets or trademarks from any game.

---

## Dependencies / Assumptions

- Retro Diffusion credits may not be available. Every built-in theme stands on procedural art alone.
- WezTerm's Linux builds support layered backgrounds and config overrides as on macOS. Window blur and some chrome options vary by Linux compositor, and R9 covers that gap.
- Time-driven auto-scroll is limited by WezTerm's update tick and by what config overrides can change. Smoothness and CPU cost have to be validated on hardware.
- OD-Cezar's display resolution is unverified (`wzt-8xg`).

---

## Outstanding Questions

**Deferred to Planning**

- How preset and theme data is expressed so that both Lua and Rust read it, given what WezTerm's Lua can parse natively.
- Whether the Lua engine is also distributed through WezTerm's plugin system for add-on mode.
- Whether horizontal auto-scroll uses the update tick, animated layers, or both.
- How the browser preview approximates layered art and chrome.
- Which Linux probes back each status segment (load, memory pressure, VPNs, battery).
- Final public preset names.

---

## Sources / Research

`sources/` is gitignored because it contains personal and work details. It moves to ikari's private fleet layer before the public repo gets a remote (`wzt-0bj`).

- `sources/metis/` — snapshot of the metis config, its `~/.wezterm.lua` shim and its state files, taken 2026-10-01.
- `sources/od-cezar/` — snapshot of OD-Cezar's 925-line `~/.wezterm.lua`, its config directory (art, generators, AWS VPN probe, theme picker) and its state files, taken 2026-10-01.
- `sources/metis/config/backgrounds.lua` — layer stack, solved base luminances, and the existing per-theme palette table.
- `sources/metis/config/parallax.lua` — why ALT+wheel drives virtual scroll: full-screen apps swallow plain wheel events, and WezTerm cannot forward them.
- `sources/metis/config/themes.lua` and `font_picker.zsh` — live preview through OSC 1337 SetUserVar, and base-plus-correction font sizing.
- `sources/metis/config/status.lua` and `bin/collect-stats.sh` — the non-blocking status pattern R13 relies on.
- `sources/od-cezar/wezterm.lua` — pill status segments, chrome values and the blocking probes that R13 replaces.
