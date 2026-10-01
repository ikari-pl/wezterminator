# wezterminator data model

This document is the contract that the Lua engine (`plugin/wzt/resolve.lua`) and the Rust model (`crates/wzt-model`) both implement. The JSON Schemas in `schema/` define what a valid document looks like. The fixtures in `tests/fixtures/resolution/` define what resolution does with valid documents. When the two runtimes disagree, the fixtures win.

All paths in this document are relative to the repository root unless they name a user directory by role.

## Documents

Every document is JSON with an integer `schema_version`. This schema generation is version 1. Files shipped in the repo live under the paths below. Fleet and local layers use the same relative layout under their own directories.

| Document | File | Schema | Layers |
|---|---|---|---|
| Preset | `presets/<slug>.json` | `schema/preset.schema.json` | built-in, fleet, local |
| Theme | `themes/<slug>/theme.json` | `schema/theme.schema.json` | built-in, fleet, local |
| Overrides | `overrides.json` | `schema/preset.schema.json#/$defs/overrides_document` | fleet, local |
| Machine settings | `machine.json` | `schema/machine.schema.json` | fleet, local |
| Engine state | `state.json` (state directory) | `schema/state.schema.json` | none (per machine) |
| Recorded screens | `screens.json` (state directory) | `schema/state.schema.json#/$defs/screens_document` | none (per machine) |

The built-in layer ships presets and themes only. It never ships overrides or machine settings. If a built-in layer is supplied with them, the loader does not read them.

`schema/theme.schema.json` and `schema/state.schema.json` reference shared definitions (ids, the motion part, parallax) in `schema/preset.schema.json` by relative `$ref`. Validators need all four files side by side.

### Preset

A preset names one choice for each of seven parts: `art`, `scheme`, `palette`, `font`, `chrome`, `status` and `motion`. It also carries `id`, `name` and `based_on`.

| Part | Contents |
|---|---|
| `art` | `theme` (the theme whose layer stack is used) and optional `layer_tweaks`, a map from layer id to `{enabled, opacity, parallax}` |
| `scheme` | Either `{theme}` (that theme's terminal scheme) or `{wezterm_scheme}` (a named WezTerm scheme) |
| `palette` | `{theme}` (that theme's semantic UI colours) |
| `font` | `preferred`, `fallback` (the preset's declared fallback list, required), `size`, `corrections` (per-font size deltas) |
| `chrome` | `opacity`, `blur`, `padding`, `inactive_pane`, `tab_bar` |
| `status` | `style` (`sparkline` or `pill`) and the ordered `segments` list |
| `motion` | `scrollback_parallax`, `alt_wheel_scroll`, `auto_scroll` |

Fallback fonts belong to the preset's `font` part, not to the theme. Swapping the font part therefore swaps the fallback list with it.

A saved preset is a full snapshot, not a diff. `based_on` records the preset it was snapshotted from, or `null`. The parent may be gone and resolution does not care. The snapshot covers every part choice. Art, scheme and palette are references to a theme by id, so editing that theme is visible to every preset that names it. Copy the theme into the local layer first to freeze it.

### Theme

A theme holds:

- `palette.ui`: semantic UI keys (`bg`, `fg`, `accent`, `ok`, `warn`, `bad`, tab-bar keys and so on). Status segments and the tab bar read these, never raw hex.
- `palette.scheme`: the terminal colour scheme.
- `art`: the layer recipe (`seed`, `base_color`, and the `layers` list from bottom to top). The recipe hash used for stale-art detection is blake3 over this object, with comment keys removed, as canonical JSON with sorted keys.
- `fallback_layers`: WezTerm `Color` and `Gradient` layers for when no art exists at the recorded resolution.
- `motion`: defaults, same shape as a preset's motion part.
- `legibility`: sample text colours for the contrast check.

Primitive parameters (`art.layers[].params`) are free-form in the schema and validated by the art engine.

### Overrides

`overrides.json` carries tweaks that follow the user across presets, such as "my chrome opacity is 0.85 whatever preset is active" or "stars at 0.1 opacity". It has the shape `{schema_version, parts}`, where `parts` has the same shape as a preset's parts with every part and field optional. Layer parallax factors and opacities (R17) are set through `parts.art.layer_tweaks`.

### Machine settings

Personal values live only here. Presets and themes have no field for them, so exporting a preset cannot leak them. No field has a personal default.

| Field | Meaning |
|---|---|
| `project_roots` | Directories scanned by the project picker |
| `issue_url_pattern` | URL template containing `{key}`, used by quick-select. Absent means the key is only copied |
| `issue_key_pattern` | Regex recognising an issue key |
| `vpn_probes` | List of probes (`tailscale`, `warp`, `aws_vpn`, `interface`, `command`) with `enabled`, TTL and timeout |
| `editor` | `{command, args}` used by "Edit config" |
| `push_targets` | Map of target name to `{hosts, user, port, connect_timeout_seconds}`. Hosts are tried in order |
| `dev_art_path` | **Development only.** Directory with hand-made art as `<dev_art_path>/<theme slug>/<layer id>.png`. When set it is consulted before every other art source. It is never exported and `doctor` flags it |
| `screen_overrides` | List of `{name?, width, height}` device-pixel resolutions, used when WezTerm cannot report device pixels. An entry without `name` applies to every screen |

### Engine state and screens

`state.json` holds `active_preset`, the undo `history`, `install_mode` and `engine` (the resolved plugin directory, the engine version and the highest `schema_version` the engine supports). Only the TUI and CLI write it, so the Lua engine can put it on WezTerm's reload watch list without a reload loop.

`history` is oldest first and capped at 20. A commit pushes the outgoing preset and drops the oldest entry beyond the cap. Undo pops the newest entry and makes it active.

`screens.json` records device-pixel screen sizes and is **not** watched. Lua writes it from a GUI event, only when the list changed. The Rust tool reads it, and art is generated for the screen with the largest pixel area. Its schema is `schema/state.schema.json#/$defs/screens_document` so the state schema file covers both documents of the state directory.

## Conventions

### Comments are `_` fields

JSON has no comments, so any object key starting with `_` is a comment, at every nesting level:

- `"_"` describes the enclosing object.
- `"_<key>"` describes the sibling `<key>`.

```json
{
  "_": "Abyssal: bioluminescent deep sea.",
  "_palette": "Teal with magenta and coral accents.",
  "palette": { "ui": { "_": "Semantic keys only." } }
}
```

Lua strips comment keys recursively right after decoding, so nothing after the loader sees them and they never reach the WezTerm config. Rust keeps them through read and write so TUI edits do not erase hand-written notes. Resolution output contains no key starting with `_`.

Because every object may carry comments, **keys in keyed maps (font names in `corrections`, push target names, layer ids in `layer_tweaks`, `params` keys) must not start with `_`**.

### Closed objects

Every object schema has `"patternProperties": { "^_": {} }` and `"additionalProperties": false`. Comments pass and any other unknown key fails, so a misspelled key is an error and never a silent no-op. Keyed maps use `patternProperties` for `^_` (comments) and `^[^_]` (the value schema), with `additionalProperties: false`.

Comment values are unconstrained by the schemas. They are free text, so the export check scans them for personal data too.

### Ids and versions

- Preset and theme ids are namespaced: `builtin:<slug>`, `fleet:<slug>` or `local:<slug>`. The namespace must match the layer holding the file. A slug is lowercase letters, digits and hyphens.
- `schema_version` is an integer starting at 1. The schemas accept exactly the version they describe. The engine gates on the integer before reading any other field of a document.
- JSON `null` is used in exactly one place, `based_on`. Everywhere else, absent means unset.

### Defaults

Resolution never fills in defaults for absent fields. Defaults (an absent `opacity` means 1, an absent `enabled` means true) belong to the code that applies the result to WezTerm.

## Resolution

Resolution is a pure function with no filesystem and no `wezterm` calls. Its inputs are:

- `engine`: `supported_schema_version` and `default_preset` (an id).
- `layers`: for `builtin`, `fleet` and `local`, either `null` or `{presets, themes, overrides, machine}`. `presets` and `themes` are arrays of documents in the order the loader read them, files sorted by path. `overrides` and `machine` are a document or `null`.
- `state`: the state document or `null`.
- `environment.installed_fonts`: a list of installed family names, or `null` when unknown.
- `addon`: `null` in replace modes, or `{owned_keys}`, the WezTerm config keys the user's config set before the engine ran.

Resolution assumes documents are schema-valid apart from the version gate. Validation is a separate concern (CI, `doctor`, the TUI).

### Steps

1. **Strip comments.** Remove `_` keys recursively from each document.
2. **Gate versions.** A document whose `schema_version` is not an integer ≥ 1 is ignored with reason `invalid_schema_version`. One above `supported_schema_version` is ignored with `unsupported_schema_version`. Both carry the `found` value. Nothing else in the document is read. An ignored document behaves as if the file did not exist.
3. **Check identity.** A preset or theme whose id namespace differs from its layer is ignored with `id_layer_mismatch`. A second document with an id already seen in the same layer is ignored with `duplicate_id` (the first wins).
4. **Build the catalog.** Accepted presets, sorted by layer (built-in, fleet, local) and then id. Names collide across layers and the higher layer wins the name: lower entries get `shadowed_by` set to the id of the lowest-id entry of the highest layer with that name. Names compare trimmed and ASCII-lowercased. Shadowed presets remain addressable by id. Entries in the same layer never shadow each other.
5. **Pick the preset.** The requested id is `state.active_preset`. If it is `null`, absent or not in the catalog, the default preset is used and a notice is recorded (`no_active_preset` or `active_preset_missing`). If the chosen preset cannot be built (step 7), the default is tried next.
6. **Apply overrides.** `effective_parts = preset.parts`, then the fleet overrides' `parts`, then the local overrides' `parts`, each merged with the rules below.
7. **Expand themes.** Every theme named by `art`, `palette` and a theme-based `scheme` must exist in some layer. If one is missing, a `theme_missing` notice is recorded and the next candidate preset is tried. If no candidate builds, `resolved` is `null` and `error` is `{"code": "no_resolvable_preset"}`.
   - `art` becomes `{theme, seed, base_color, layers, fallback_layers}`. Each theme layer is merged with `layer_tweaks[<layer id>]`. Tweaks for layer ids the theme does not have are ignored.
   - `scheme` becomes `{theme, colors}` or is passed through as `{wezterm_scheme}`.
   - `palette` becomes `{theme, ui}`.
   - `motion` is the art theme's `motion` defaults merged with the part's `motion`.
   - `font` is passed through with two computed fields. `effective` is the preferred fonts that are installed (all of them when `installed_fonts` is `null`), then the declared `fallback` list, duplicates removed keeping the first. `missing` is the preferred fonts that are not installed.
   - `chrome` and `status` are passed through.
8. **Drop add-on-owned parts.** When `addon` is set, anything the user's config owns is removed from the result and listed in `overruled` as `{path, config_key}`, sorted by path. Only paths present in the result are reported, each once. Empty objects left behind are not pruned.
9. **Merge machine settings.** Fleet machine settings, then local, with the merge rules below. `schema_version` is not part of the output.

### Merge rules

`merge(base, over)`:

- Two objects merge key by key, recursively. A key only in `over` is copied.
- Anything else replaces: scalars, arrays, and object-over-non-object.
- An **empty array replaces** (it clears the list). An **empty object merges as a no-op** over an object and creates an empty object where nothing existed.
- The `scheme` part is atomic. A layer that sets it replaces the whole part, so `{theme}` and `{wezterm_scheme}` never combine.

### Add-on ownership table

| Config key the user set | Overrules |
|---|---|
| `font`, `font_size`, `font_rules` | part `font` (whole) |
| `color_scheme` | part `scheme` |
| `colors` | parts `scheme` and `palette` |
| `window_frame` | part `palette` |
| `background` | part `art` |
| `window_background_opacity` | `chrome.opacity` |
| `macos_window_background_blur`, `kde_window_background_blur` | `chrome.blur` |
| `window_padding` | `chrome.padding` |
| `inactive_pane_hsb` | `chrome.inactive_pane` |
| `enable_tab_bar` | `chrome.tab_bar.hidden` |
| `tab_bar_at_bottom` | `chrome.tab_bar.position` |

`status` and `motion` are engine-owned and never overruled. When several owned keys map to one path, the first in table order is reported.

### Output

```json
{
  "active":   { "requested": "<id|null>", "id": "<id|null>", "fell_back": false },
  "catalog":  [{ "id": "", "name": "", "layer": "builtin", "shadowed_by": null }],
  "resolved": { "id": "", "name": "", "based_on": null, "parts": {} },
  "machine":  {},
  "overruled": [{ "path": "chrome.opacity", "config_key": "window_background_opacity" }],
  "ignored":  [{ "layer": "fleet", "kind": "preset", "ref": "fleet:x", "reason": "id_layer_mismatch" }],
  "notices":  [{ "code": "active_preset_missing", "requested": "", "used": "" }],
  "error":    null
}
```

- `fell_back` is true when `active.id` is non-null and differs from `requested`. Any notice is what the engine shows as a toast.
- `ignored` entries are ordered layer by layer (built-in, fleet, local), within a layer presets then themes then overrides then machine, and the state document last. `ref` is the document id, or `overrides`, `machine` or `state`.
- Notice codes: `no_active_preset`, `active_preset_missing`, `theme_missing`.
- Art source selection (dev path, user art, shipped art, fallback layers) depends on the filesystem and is not part of these fixtures. It is tested where the directories exist (U4 and U7).

## Fixtures

Fixtures live in `tests/fixtures/resolution/`. Each case is **one JSON file**, named `<kind>-<slug>.json` with a lowercase hyphenated slug, and the `name` field equals the file name without `.json`. There are two kinds, told apart by the prefix and by the `kind` field. Harnesses glob `resolve-*.json` and `validate-*.json`.

Top-level keys starting with `_` are comments, as everywhere. Inside `input` the harness strips comments from documents exactly as the real loader does.

### `resolve-*.json` (`kind: "resolution"`)

```json
{
  "_": "What this case proves.",
  "name": "resolve-merge-order",
  "kind": "resolution",
  "covers": ["merge-order"],
  "input": {
    "engine": { "supported_schema_version": 1, "default_preset": "builtin:neon-night" },
    "layers": { "builtin": {}, "fleet": null, "local": null },
    "state": {},
    "environment": { "installed_fonts": null },
    "addon": null
  },
  "expected": {},
  "expect_at": { "/resolved/parts/chrome/opacity": 0.85 },
  "expect_absent": ["/resolved/parts/font"]
}
```

A case asserts only what it exists to prove. All three assertion keys are optional:

- `expected`: top-level output keys, each compared by exact deep equality. Output keys not listed are not checked.
- `expect_at`: JSON Pointer (RFC 6901) into the output, mapped to an exact expected value.
- `expect_absent`: JSON Pointers that must not exist in the output.

Comparison is exact and normalised:

- Object keys are unordered. Array order matters.
- An empty array never equals an empty object. A runtime whose table type cannot tell them apart must use the schema to know which paths are arrays. In the WezTerm-hosted Lua run, which decodes through `wezterm.json_parse`, empty arrays go through an explicit marker that is normalised back to `[]` before comparison.
- Numbers compare by value.
- The output must contain no key starting with `_`.

### `validate-*.json` (`kind: "validation"`)

```json
{
  "name": "validate-overrides-scheme-both-selectors",
  "kind": "validation",
  "schema": "schema/preset.schema.json",
  "def": "overrides_document",
  "document": {},
  "valid": false,
  "error_path": "/parts/scheme"
}
```

`def` is optional and selects `#/$defs/<def>` inside `schema` instead of the root. `valid` says whether the document must validate. For invalid documents, `error_path` (optional, a JSON Pointer to the instance) must appear among the reported error locations, including nested ones under `oneOf` and `allOf`. Keyword names are not asserted because validators differ.

### Cases

| File | Covers |
|---|---|
| `resolve-basic.json` | Baseline with all three layers, theme expansion and both merges. Twin of `resolve-comment-keys.json`. |
| `resolve-comment-keys.json` | `_` comments injected at every depth. Same expected output as `resolve-basic.json`. |
| `resolve-merge-order.json` | Built-in, fleet, local order for overrides and machine settings, field by field |
| `resolve-shadowing.json` | Same name in three layers. Ids never collide and the shadowed built-in stays addressable |
| `resolve-ignored-documents.json` | Id and layer mismatch, duplicate ids |
| `resolve-missing-active-preset.json` | Active id not in the catalog |
| `resolve-no-state.json` | No state document |
| `resolve-missing-theme.json` | Active preset's theme is gone |
| `resolve-no-resolvable-preset.json` | Not even the default exists, typed error |
| `resolve-future-schema-version.json` | Unsupported and invalid `schema_version` in presets, themes, overrides and machine files |
| `resolve-addon-owned-keys.json` | Add-on mode field-level and part-level overruling (AE5) |
| `resolve-addon-owned-font-size.json` | `font_size` alone overrules the whole font part |
| `resolve-empty-array-vs-object.json` | Empty array replaces, empty object merges, both survive in the output |
| `resolve-snapshot-deleted-parent.json` | `based_on` pointing at a preset that does not exist |
| `resolve-font-fallback-missing.json` | Preferred font missing, declared fallback list used (AE6) |
| `resolve-font-installed-unknown.json` | Installed font list unknown |
| `validate-theme-comments-valid.json` | Comments at several depths validate |
| `validate-theme-misspelled-key.json` | Misspelled non-underscore key fails |
| `validate-theme-comment-without-underscore.json` | A comment-looking key without `_` fails |
| `validate-preset-comments-valid.json` | Preset comments validate |
| `validate-preset-bad-namespace.json` | Unnamespaced id fails |
| `validate-preset-missing-fallback-font.json` | Font part without fallback fonts fails |
| `validate-preset-machine-key-leak.json` | Machine settings field in a preset fails |
| `validate-preset-future-version.json` | Version 2 fails the version 1 schema |
| `validate-machine-valid.json` | Every machine field, including the development art path and screen overrides |
| `validate-machine-bad-issue-pattern.json` | Issue URL pattern without `{key}` fails |
| `validate-state-valid.json` | Full state document |
| `validate-state-history-over-cap.json` | 21 history entries fail |
| `validate-overrides-valid.json` | Partial-parts overrides document |
| `validate-overrides-scheme-both-selectors.json` | Scheme with both selectors fails |
| `validate-screens-valid.json` | Recorded screens document |

### Who consumes them

- U2 runs the resolution cases through `plugin/wzt/resolve.lua` under stock Lua and again inside WezTerm.
- U5 runs the same files through `crates/wzt-model`.
- CI validates every built-in preset and theme against the schemas, and runs the validation cases against whichever validators it has.
