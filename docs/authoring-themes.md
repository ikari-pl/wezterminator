# Authoring themes

A **theme** is the art + colour document (`themes/<slug>/theme.json`). A
**preset** picks one theme for art / scheme / palette and chooses font,
chrome, status and motion parts. Machine settings never live in either file.

Schemas: `schema/theme.schema.json`, `schema/preset.schema.json`.  
Full field contract: [data-model.md](data-model.md).

## Layout

```text
themes/<slug>/
  theme.json          # required
  art/<W>x<H>/        # optional shipped PNGs (packs deferred)
presets/<slug>.json   # built-in preset that names the theme
```

User-generated art lands under
`$XDG_DATA_HOME/wezterminator/art/<theme>/<W>x<H>/` (also on macOS) with a
blake3 recipe hash in a sibling manifest. Lookup order: matching user art →
shipped repo art → theme fallback `Color` / `Gradient` layers.

## Minimal theme

Required top-level fields: `schema_version`, `id`, `name`, `variant`,
`palette`, `art`, `fallback_layers`, `legibility`. Copy an existing theme
(for example `themes/phosphor/theme.json`) and trim layers rather than
inventing shape from memory.

```json
{
  "_": "Short note about the look. Keys starting with _ are comments.",
  "schema_version": 1,
  "id": "builtin:my-theme",
  "name": "My Theme",
  "variant": "dark",
  "palette": {
    "ui": {
      "bg": "#0a0a12",
      "surface": "#12121c",
      "fg": "#e8e8f0",
      "fg_dim": "#8888a0",
      "accent": "#7aa2f7",
      "accent_alt": "#bb9af7",
      "ok": "#9ece6a",
      "warn": "#e0af68",
      "bad": "#f7768e",
      "info": "#7dcfff",
      "tab_bar_bg": "#0a0a12",
      "tab_active_bg": "#12121c",
      "tab_active_fg": "#e8e8f0",
      "tab_inactive_bg": "#0a0a12",
      "tab_inactive_fg": "#8888a0",
      "tab_hover_bg": "#12121c",
      "tab_hover_fg": "#e8e8f0"
    },
    "scheme": {
      "foreground": "#e8e8f0",
      "background": "#0a0a12",
      "cursor_bg": "#e8e8f0",
      "cursor_fg": "#0a0a12",
      "selection_bg": "#12121c",
      "selection_fg": "#e8e8f0",
      "ansi": ["#0a0a12", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6"],
      "brights": ["#8888a0", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#e8e8f0"]
    }
  },
  "art": {
    "seed": 1,
    "base_color": "#0a0a12",
    "layers": [
      {
        "id": "wash",
        "primitive": "dither_wash",
        "scale": 4,
        "opacity": 0.4,
        "parallax": { "vertical": 0.05, "horizontal": 0 },
        "params": {
          "direction": "top",
          "colors": ["surface", "accent"],
          "alpha": 70,
          "strength": 40
        }
      }
    ]
  },
  "fallback_layers": [
    { "kind": "color", "color": "#0a0a12" },
    {
      "kind": "gradient",
      "shape": "radial",
      "colors": ["#12121c", "#0a0a12"],
      "opacity": 0.45
    }
  ],
  "motion": {
    "scrollback_parallax": true,
    "alt_wheel_scroll": { "vertical": true, "horizontal": false },
    "auto_scroll": { "enabled": false, "speed": 0 }
  },
  "legibility": {
    "text": "#e8e8f0",
    "dim_text": "#8888a0",
    "min_contrast": 3.0
  }
}
```

`variant` is `dark` or `light` (Washi is the light built-in). Comments are any
key whose name starts with `_`; Lua strips them on load, Rust preserves them
on rewrite.

## Art recipe

Each layer names a **primitive**, integer **scale**, opacity, parallax factors
and primitive-specific `params`. Generation is deterministic for a given seed
and thread count.

Primitives include: `starfield`, `dither_wash`, `cloud_blobs`, perspective
grid, scanlines / vignette, tiled / scattered sprites, isometric tiles,
silhouette bands. Prefer procedural layers; do not ship copied or trademarked
sprites.

```bash
# Legibility only (no files written)
wezterminator art check my-theme

# Render at a device resolution into the art cache / --out
wezterminator art generate my-theme --size 3024x1964 --checkout .
```

`doctor` reports missing art for recorded screens, stale user art whose recipe
hash no longer matches, and themes over the per-theme layer budget.

## Local and fleet themes

Hand-edit under the local layer directory (see data-model paths), or use the
TUI authoring screen to duplicate a template, edit the palette with contrast
readout, and regenerate art. Promote a preset into the **fleet** layer when
you want it on your other machines; export a public PR bundle separately so
machine settings cannot leak.

## Preset pairing

Ship a preset that points art / scheme / palette at the theme id and sets
font fallbacks, chrome, status style and motion. Parts live under `parts`
(see `presets/phosphor.json`):

```json
{
  "schema_version": 1,
  "id": "builtin:my-theme",
  "name": "My Theme",
  "based_on": null,
  "parts": {
    "art": { "theme": "builtin:my-theme" },
    "scheme": { "theme": "builtin:my-theme" },
    "palette": { "theme": "builtin:my-theme" },
    "font": {
      "preferred": ["JetBrains Mono"],
      "fallback": ["JetBrains Mono", "Menlo"],
      "size": 13.0
    },
    "chrome": { "opacity": 1.0 },
    "status": { "style": "sparkline", "segments": ["clock", "cwd", "battery"] },
    "motion": {
      "scrollback_parallax": true,
      "alt_wheel_scroll": { "vertical": true, "horizontal": false },
      "auto_scroll": { "enabled": false, "speed": 0 }
    }
  }
}
```

Saved user presets are full snapshots with `based_on` metadata, not diffs.

## Validation

- Validate JSON against `schema/theme.schema.json` / `schema/preset.schema.json`.
- Shared resolution fixtures live in `tests/fixtures/resolution/` and run in
  both Lua (`lua tests/lua/run.lua`) and Rust (`cargo test -p wzt-model`).
- Prefer `wezterminator art check` before committing a density change.
