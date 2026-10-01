//! Preset resolution: the Rust twin of `plugin/wzt/resolve.lua`.
//!
//! `docs/data-model.md` ("Resolution") is the contract and the fixtures in
//! `tests/fixtures/resolution/` are the arbiter. Resolution is a pure function
//! with no filesystem access. It works on raw JSON, not on [`crate::model`]
//! types, because the `schema_version` gate must run before anything assumes a
//! document's shape, and because resolution assumes documents are
//! schema-valid apart from that gate.
//!
//! Step numbers in the comments refer to the "Steps" list in the data model.
//!
//! One deliberate difference from Lua: Lua has no `null`, so its output omits
//! absent values. Here the output types serialize `None` as `null`, which is
//! how the data model documents the output (`"shadowed_by": null`) and how the
//! fixtures spell it.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::json::{merge_into, merge_parts_into, strip_comments};
use crate::version::{VersionProblem, check_schema_version};

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// The three layers, lowest precedence first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    Builtin,
    Fleet,
    Local,
}

impl Layer {
    pub const ALL: [Layer; 3] = [Layer::Builtin, Layer::Fleet, Layer::Local];

    pub fn as_str(self) -> &'static str {
        match self {
            Layer::Builtin => "builtin",
            Layer::Fleet => "fleet",
            Layer::Local => "local",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EngineInput {
    #[serde(default = "default_supported")]
    pub supported_schema_version: u64,
    #[serde(default)]
    pub default_preset: Option<String>,
}

fn default_supported() -> u64 {
    crate::version::SUPPORTED_SCHEMA_VERSION
}

impl Default for EngineInput {
    fn default() -> Self {
        EngineInput {
            supported_schema_version: default_supported(),
            default_preset: None,
        }
    }
}

/// One layer's documents, as the loader read them (files sorted by path).
/// Documents are raw JSON so that the version gate can run first.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct LayerInput {
    #[serde(default)]
    pub presets: Vec<Value>,
    #[serde(default)]
    pub themes: Vec<Value>,
    #[serde(default)]
    pub overrides: Option<Value>,
    #[serde(default)]
    pub machine: Option<Value>,
}

/// Alias used by the layer loader.
pub type LayerDocs = LayerInput;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct LayersInput {
    #[serde(default)]
    pub builtin: Option<LayerInput>,
    #[serde(default)]
    pub fleet: Option<LayerInput>,
    #[serde(default)]
    pub local: Option<LayerInput>,
}

/// Alias used by the layer loader.
pub type Layers = LayersInput;

/// Alias kept for call sites that say "engine spec".
pub type EngineSpec = EngineInput;

impl LayersInput {
    fn get(&self, layer: Layer) -> Option<&LayerInput> {
        match layer {
            Layer::Builtin => self.builtin.as_ref(),
            Layer::Fleet => self.fleet.as_ref(),
            Layer::Local => self.local.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct EnvironmentInput {
    /// Installed family names, or `None` when unknown.
    #[serde(default)]
    pub installed_fonts: Option<Vec<String>>,
}

/// Add-on mode: the WezTerm config keys the user's config set before the
/// engine ran.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct AddonInput {
    #[serde(default)]
    pub owned_keys: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ResolveInput {
    #[serde(default)]
    pub engine: EngineInput,
    #[serde(default)]
    pub layers: LayersInput,
    /// The state document, or `None`.
    #[serde(default)]
    pub state: Option<Value>,
    #[serde(default)]
    pub environment: EnvironmentInput,
    /// `None` in replace modes.
    #[serde(default)]
    pub addon: Option<AddonInput>,
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Active {
    pub requested: Option<String>,
    pub id: Option<String>,
    pub fell_back: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub layer: Layer,
    pub shadowed_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Resolved {
    pub id: String,
    pub name: String,
    pub based_on: Option<String>,
    /// Expanded parts. Raw JSON because the shape depends on which parts and
    /// fields the layers set; resolution never fills in defaults.
    pub parts: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Overruled {
    pub path: String,
    pub config_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IgnoreKind {
    Preset,
    Theme,
    Overrides,
    Machine,
    State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IgnoreReason {
    InvalidDocument,
    InvalidSchemaVersion,
    UnsupportedSchemaVersion,
    IdLayerMismatch,
    DuplicateId,
}

/// A document that was ignored, and why. It behaves as if the file did not
/// exist.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ignored {
    /// Absent for the state document, which belongs to no layer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<Layer>,
    pub kind: IgnoreKind,
    /// The document id, or `overrides`, `machine` or `state`.
    #[serde(rename = "ref")]
    pub reference: String,
    pub reason: IgnoreReason,
    /// The offending `schema_version`, for the two version reasons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found: Option<Value>,
}

/// Something the engine shows as a toast.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum Notice {
    NoActivePreset {
        #[serde(skip_serializing_if = "Option::is_none")]
        used: Option<String>,
    },
    ActivePresetMissing {
        requested: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        used: Option<String>,
    },
    ThemeMissing {
        preset: String,
        theme: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NoResolvablePreset,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolveError {
    pub code: ErrorCode,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Resolution {
    pub active: Active,
    pub catalog: Vec<CatalogEntry>,
    pub resolved: Option<Resolved>,
    pub machine: Value,
    pub overruled: Vec<Overruled>,
    pub ignored: Vec<Ignored>,
    pub notices: Vec<Notice>,
    pub error: Option<ResolveError>,
}

impl Resolution {
    /// The output as JSON, exactly as the data model documents it.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("resolution serializes to JSON")
    }
}

// ---------------------------------------------------------------------------
// Add-on ownership
// ---------------------------------------------------------------------------

/// Config key the user set, and the dotted paths of the resolved `parts` it
/// overrules. Table order matters: when several keys map to one path, the
/// first one here is reported.
///
/// `wayland_window_background_blur` is not in the data-model table. It is the
/// nightly Wayland key the platform probe looks for, with the same meaning as
/// the KDE key, and `resolve.lua` carries it too.
pub const ADDON_RULES: &[(&str, &[&str])] = &[
    ("font", &["font"]),
    ("font_size", &["font"]),
    ("font_rules", &["font"]),
    ("color_scheme", &["scheme"]),
    ("colors", &["scheme", "palette"]),
    ("window_frame", &["palette"]),
    ("background", &["art"]),
    ("window_background_opacity", &["chrome.opacity"]),
    ("macos_window_background_blur", &["chrome.blur"]),
    ("kde_window_background_blur", &["chrome.blur"]),
    ("wayland_window_background_blur", &["chrome.blur"]),
    ("window_padding", &["chrome.padding"]),
    ("inactive_pane_hsb", &["chrome.inactive_pane"]),
    ("enable_tab_bar", &["chrome.tab_bar.hidden"]),
    ("tab_bar_at_bottom", &["chrome.tab_bar.position"]),
];

/// Remove the dotted `path` from `root` if present. Empty parents stay.
fn remove_path(root: &mut Value, path: &str) -> bool {
    let mut segments: Vec<&str> = path.split('.').collect();
    let Some(last) = segments.pop() else {
        return false;
    };
    let mut node = root;
    for segment in segments {
        match node.get_mut(segment) {
            Some(next) if next.is_object() => node = next,
            _ => return false,
        }
    }
    node.as_object_mut()
        .is_some_and(|map| map.shift_remove(last).is_some())
}

/// Step 8: drop the parts the user's config owns, and list them.
fn drop_owned(parts: &mut Value, owned_keys: &[String]) -> Vec<Overruled> {
    let owned: HashSet<&str> = owned_keys.iter().map(String::as_str).collect();
    let mut reported: HashSet<&str> = HashSet::new();
    let mut list = Vec::new();
    for (config_key, paths) in ADDON_RULES {
        if !owned.contains(config_key) {
            continue;
        }
        for path in *paths {
            if !reported.contains(path) && remove_path(parts, path) {
                reported.insert(path);
                list.push(Overruled {
                    path: (*path).to_owned(),
                    config_key: (*config_key).to_owned(),
                });
            }
        }
    }
    list.sort_by(|a, b| a.path.cmp(&b.path));
    list
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Preset and theme ids look like `<layer>:<slug>`.
fn id_namespace(id: &str) -> Option<&str> {
    id.split_once(':').map(|(ns, _)| ns).filter(|ns| !ns.is_empty())
}

/// Names compare trimmed and ASCII-lowercased.
fn name_key(name: &Value) -> String {
    const WS: &[char] = &[' ', '\t', '\n', '\x0b', '\x0c', '\r'];
    name.as_str()
        .map(|s| s.trim_matches(WS).to_ascii_lowercase())
        .unwrap_or_default()
}

/// The `schema_version` gate (step 2). `Ok` for a usable document.
fn gate(doc: &Value, supported: u64) -> Result<(), (IgnoreReason, Option<Value>)> {
    if !doc.is_object() {
        return Err((IgnoreReason::InvalidDocument, None));
    }
    match check_schema_version(doc, supported) {
        Ok(_) => Ok(()),
        Err(VersionProblem::Invalid(found)) => {
            // An absent version has nothing to report as `found`.
            let found = (!found.is_null()).then_some(found);
            Err((IgnoreReason::InvalidSchemaVersion, found))
        }
        Err(VersionProblem::Unsupported(found)) => {
            Err((IgnoreReason::UnsupportedSchemaVersion, Some(json!(found))))
        }
    }
}

/// An accepted preset, comments already stripped.
struct PresetEntry {
    id: String,
    name: Value,
    layer: Layer,
    doc: Value,
}

fn compute_font(font: &Value, installed: Option<&[String]>) -> Value {
    let mut out = font.clone();
    let strings = |key: &str| -> Vec<String> {
        font.get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };

    let have: Option<HashSet<&str>> = installed.map(|fonts| fonts.iter().map(String::as_str).collect());
    let mut seen: HashSet<String> = HashSet::new();
    let mut effective: Vec<Value> = Vec::new();
    let mut missing: Vec<Value> = Vec::new();

    for family in strings("preferred") {
        if have.as_ref().is_none_or(|have| have.contains(family.as_str())) {
            if seen.insert(family.clone()) {
                effective.push(Value::String(family));
            }
        } else {
            missing.push(Value::String(family));
        }
    }
    for family in strings("fallback") {
        if seen.insert(family.clone()) {
            effective.push(Value::String(family));
        }
    }

    if let Some(map) = out.as_object_mut() {
        map.insert("effective".into(), Value::Array(effective));
        map.insert("missing".into(), Value::Array(missing));
    }
    out
}

/// Steps 6 and 7 for one candidate preset. On a missing theme, returns the
/// notice and the caller tries the next candidate.
fn build(
    preset: &PresetEntry,
    overrides: &[&Value],
    themes: &HashMap<String, Value>,
    installed: Option<&[String]>,
) -> Result<Resolved, Notice> {
    // Step 6: preset parts, then fleet overrides, then local overrides.
    let mut parts = preset
        .doc
        .get("parts")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    for over in overrides {
        if let Some(over_parts) = over.get("parts").filter(|p| p.is_object()) {
            merge_parts_into(&mut parts, over_parts);
        }
    }

    // Step 7: every theme the preset names must exist.
    let theme_of = |part: &str| -> Option<String> {
        parts
            .get(part)
            .and_then(|p| p.get("theme"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let art_id = theme_of("art");
    let palette_id = theme_of("palette");
    let scheme_id = theme_of("scheme");
    for id in [&art_id, &palette_id, &scheme_id].into_iter().flatten() {
        if !themes.contains_key(id) {
            return Err(Notice::ThemeMissing {
                preset: preset.id.clone(),
                theme: id.clone(),
            });
        }
    }

    let art_theme = art_id.as_ref().and_then(|id| themes.get(id));
    let mut out = Map::new();

    if let (Some(art_part), Some(art_id), Some(theme)) = (parts.get("art"), &art_id, art_theme) {
        let recipe = theme.get("art");
        let tweaks = art_part.get("layer_tweaks");
        let layers: Vec<Value> = recipe
            .and_then(|a| a.get("layers"))
            .and_then(Value::as_array)
            .map(|layers| {
                layers
                    .iter()
                    .map(|layer| {
                        let tweak = layer
                            .get("id")
                            .and_then(Value::as_str)
                            .and_then(|id| tweaks.and_then(|t| t.get(id)));
                        let mut layer = layer.clone();
                        if let Some(tweak) = tweak {
                            merge_into(&mut layer, tweak);
                        }
                        layer
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut art = Map::new();
        art.insert("theme".into(), json!(art_id));
        for key in ["seed", "base_color"] {
            if let Some(v) = recipe.and_then(|a| a.get(key)) {
                art.insert(key.into(), v.clone());
            }
        }
        art.insert("layers".into(), Value::Array(layers));
        art.insert(
            "fallback_layers".into(),
            theme
                .get("fallback_layers")
                .cloned()
                .unwrap_or_else(|| json!([])),
        );
        out.insert("art".into(), Value::Object(art));
    }

    if let Some(scheme) = parts.get("scheme") {
        if let Some(id) = &scheme_id {
            let mut s = Map::new();
            s.insert("theme".into(), json!(id));
            if let Some(colors) = themes[id].get("palette").and_then(|p| p.get("scheme")) {
                s.insert("colors".into(), colors.clone());
            }
            out.insert("scheme".into(), Value::Object(s));
        } else {
            out.insert("scheme".into(), scheme.clone());
        }
    }

    if let (Some(_), Some(id)) = (parts.get("palette"), &palette_id) {
        let mut p = Map::new();
        p.insert("theme".into(), json!(id));
        if let Some(ui) = themes[id].get("palette").and_then(|p| p.get("ui")) {
            p.insert("ui".into(), ui.clone());
        }
        out.insert("palette".into(), Value::Object(p));
    }

    if let Some(font) = parts.get("font") {
        out.insert("font".into(), compute_font(font, installed));
    }
    for key in ["chrome", "status"] {
        if let Some(v) = parts.get(key) {
            out.insert(key.into(), v.clone());
        }
    }

    // `motion` is the art theme's defaults merged with the part's own.
    if art_theme.is_some() || parts.get("motion").is_some() {
        let mut motion = art_theme
            .and_then(|t| t.get("motion"))
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        if let Some(over) = parts.get("motion") {
            merge_into(&mut motion, over);
        }
        out.insert("motion".into(), motion);
    }

    Ok(Resolved {
        id: preset.id.clone(),
        name: preset.name.as_str().unwrap_or_default().to_owned(),
        based_on: preset
            .doc
            .get("based_on")
            .and_then(Value::as_str)
            .map(str::to_owned),
        parts: Value::Object(out),
    })
}

/// Resolve the active preset. See `docs/data-model.md`.
pub fn resolve(input: &ResolveInput) -> Resolution {
    let supported = input.engine.supported_schema_version;
    let default_id = input.engine.default_preset.clone();

    let mut ignored: Vec<Ignored> = Vec::new();
    let mut presets: Vec<PresetEntry> = Vec::new();
    let mut themes: HashMap<String, Value> = HashMap::new();
    let mut overrides: HashMap<Layer, Value> = HashMap::new();
    let mut machines: HashMap<Layer, Value> = HashMap::new();

    // Steps 1 to 3: strip comments, gate versions, check identity.
    for layer in Layer::ALL {
        let Some(input_layer) = input.layers.get(layer) else {
            continue;
        };

        let mut take = |docs: &[Value], kind: IgnoreKind| -> Vec<(String, Value)> {
            let mut seen: HashSet<String> = HashSet::new();
            let mut accepted = Vec::new();
            for raw in docs {
                let id = raw.get("id").and_then(Value::as_str);
                let reference = id.unwrap_or("<unknown>").to_owned();
                let mut ignore = |reason, found| {
                    ignored.push(Ignored {
                        layer: Some(layer),
                        kind,
                        reference: reference.clone(),
                        reason,
                        found,
                    });
                };
                if let Err((reason, found)) = gate(raw, supported) {
                    ignore(reason, found);
                } else if id.and_then(id_namespace) != Some(layer.as_str()) {
                    ignore(IgnoreReason::IdLayerMismatch, None);
                } else if !seen.insert(reference.clone()) {
                    ignore(IgnoreReason::DuplicateId, None);
                } else {
                    let mut doc = raw.clone();
                    strip_comments(&mut doc);
                    accepted.push((reference.clone(), doc));
                }
            }
            accepted
        };

        for (id, doc) in take(&input_layer.presets, IgnoreKind::Preset) {
            presets.push(PresetEntry {
                name: doc.get("name").cloned().unwrap_or(Value::Null),
                id,
                layer,
                doc,
            });
        }
        for (id, doc) in take(&input_layer.themes, IgnoreKind::Theme) {
            themes.insert(id, doc);
        }

        // The built-in layer never ships overrides or machine settings, and
        // the loader does not read them even if supplied.
        if layer == Layer::Builtin {
            continue;
        }
        let mut single = |raw: &Option<Value>, kind: IgnoreKind, reference: &str| -> Option<Value> {
            let raw = raw.as_ref()?;
            match gate(raw, supported) {
                Err((reason, found)) => {
                    ignored.push(Ignored {
                        layer: Some(layer),
                        kind,
                        reference: reference.to_owned(),
                        reason,
                        found,
                    });
                    None
                }
                Ok(()) => {
                    let mut doc = raw.clone();
                    strip_comments(&mut doc);
                    Some(doc)
                }
            }
        };
        if let Some(doc) = single(&input_layer.overrides, IgnoreKind::Overrides, "overrides") {
            overrides.insert(layer, doc);
        }
        if let Some(doc) = single(&input_layer.machine, IgnoreKind::Machine, "machine") {
            machines.insert(layer, doc);
        }
    }

    // The state document is gated last.
    let state = input.state.as_ref().and_then(|raw| match gate(raw, supported) {
        Err((reason, found)) => {
            ignored.push(Ignored {
                layer: None,
                kind: IgnoreKind::State,
                reference: "state".to_owned(),
                reason,
                found,
            });
            None
        }
        Ok(()) => {
            let mut doc = raw.clone();
            strip_comments(&mut doc);
            Some(doc)
        }
    });

    // Step 4: the catalog, by layer and then id. The highest layer holding a
    // name wins it; lower entries record the lowest id of that layer.
    presets.sort_by(|a, b| a.layer.cmp(&b.layer).then_with(|| a.id.cmp(&b.id)));

    let mut top: HashMap<String, (Layer, &str)> = HashMap::new();
    for p in &presets {
        let entry = top.entry(name_key(&p.name)).or_insert((p.layer, &p.id));
        if p.layer > entry.0 {
            *entry = (p.layer, &p.id);
        }
    }
    let catalog: Vec<CatalogEntry> = presets
        .iter()
        .map(|p| {
            let (top_layer, top_id) = top[&name_key(&p.name)];
            CatalogEntry {
                id: p.id.clone(),
                name: p.name.as_str().unwrap_or_default().to_owned(),
                layer: p.layer,
                shadowed_by: (top_layer > p.layer).then(|| top_id.to_owned()),
            }
        })
        .collect();

    // Step 5: pick the preset, with the default as the fallback.
    let by_id: HashMap<&str, &PresetEntry> = presets.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut notices: Vec<Notice> = Vec::new();
    let requested: Option<String> = state
        .as_ref()
        .and_then(|s| s.get("active_preset"))
        .and_then(Value::as_str)
        .map(str::to_owned);

    let candidates: Vec<Option<String>> = match &requested {
        None => {
            notices.push(Notice::NoActivePreset {
                used: default_id.clone(),
            });
            vec![default_id.clone()]
        }
        Some(req) if !by_id.contains_key(req.as_str()) => {
            notices.push(Notice::ActivePresetMissing {
                requested: req.clone(),
                used: default_id.clone(),
            });
            vec![default_id.clone()]
        }
        Some(req) if Some(req) == default_id.as_ref() => vec![Some(req.clone())],
        Some(req) => vec![Some(req.clone()), default_id.clone()],
    };

    // Steps 6 and 7: overrides and theme expansion, first buildable wins.
    let layered_overrides: Vec<&Value> = [Layer::Fleet, Layer::Local]
        .iter()
        .filter_map(|l| overrides.get(l))
        .collect();
    let mut resolved: Option<Resolved> = None;
    for id in candidates.iter().flatten() {
        let Some(preset) = by_id.get(id.as_str()) else {
            continue;
        };
        match build(
            preset,
            &layered_overrides,
            &themes,
            input.environment.installed_fonts.as_deref(),
        ) {
            Ok(r) => {
                resolved = Some(r);
                break;
            }
            Err(notice) => notices.push(notice),
        }
    }

    // Step 8: parts the user's config owns.
    let mut overruled = Vec::new();
    if let (Some(r), Some(addon)) = (resolved.as_mut(), input.addon.as_ref()) {
        overruled = drop_owned(&mut r.parts, &addon.owned_keys);
    }

    // Step 9: machine settings, fleet then local.
    let mut machine = Value::Object(Map::new());
    for layer in [Layer::Fleet, Layer::Local] {
        if let Some(doc) = machines.get(&layer) {
            let mut doc = doc.clone();
            if let Some(map) = doc.as_object_mut() {
                map.shift_remove("schema_version");
            }
            merge_into(&mut machine, &doc);
        }
    }

    let active_id = resolved.as_ref().map(|r| r.id.clone());
    let fell_back = active_id.is_some() && active_id != requested;
    let error = resolved.is_none().then_some(ResolveError {
        code: ErrorCode::NoResolvablePreset,
    });

    Resolution {
        active: Active {
            requested,
            id: active_id,
            fell_back,
        },
        catalog,
        resolved,
        machine,
        overruled,
        ignored,
        notices,
        error,
    }
}

/// Resolve from the JSON shape the fixtures use for `input`.
pub fn resolve_value(input: &Value) -> Result<Value, serde_json::Error> {
    let input = ResolveInput::deserialize(input)?;
    Ok(resolve(&input).to_value())
}
