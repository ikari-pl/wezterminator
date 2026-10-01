//! Typed mirrors of the JSON documents in `schema/`.
//!
//! Every object type ends in a flattened [`Comments`] map. `_` keys are kept
//! and written back; any other unknown key fails to deserialize. Optional
//! fields are omitted on write when unset, and `null` appears only in
//! `based_on`, as the data model requires.
//!
//! These types are for reading and editing files. Resolution itself works on
//! raw JSON (see [`crate::resolve`]) because the version gate has to run
//! before anything assumes a shape.
//!
//! Number fields that the schema calls `number` are [`serde_json::Number`], so
//! `1` stays `1` and `0.5` stays `0.5` through a read and write.

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use crate::comments::{Comments, Keyed};

// ---------------------------------------------------------------------------
// Preset parts (shared by presets, themes' motion defaults and overrides)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parallax {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizontal: Option<Number>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerTweak {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallax: Option<Parallax>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Keyed by layer id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_tweaks: Option<Keyed<LayerTweak>>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemeTheme {
    pub theme: String,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemeNamed {
    pub wezterm_scheme: String,
    #[serde(flatten)]
    pub comments: Comments,
}

/// Either a theme's own terminal scheme or a named WezTerm scheme, never both.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SchemePart {
    Theme(SchemeTheme),
    WezTerm(SchemeNamed),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PalettePart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Number>,
    /// Per-font size correction in points, keyed by family name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corrections: Option<Keyed<Number>>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Padding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<u64>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InactivePane {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<Number>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabBarPosition {
    Top,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabBar {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<TabBarPosition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChromePart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blur: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<Padding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inactive_pane: Option<InactivePane>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_bar: Option<TabBar>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusStyle {
    Sparkline,
    Pill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Segment {
    Load,
    Memory,
    MemoryPressure,
    Tailscale,
    Warp,
    Tunnels,
    AwsVpn,
    Cwd,
    Font,
    Battery,
    ExitCode,
    Clock,
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StatusStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segments: Option<Vec<Segment>>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AltWheelScroll {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizontal: Option<bool>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoScroll {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Pixels per tick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<Axis>,
    #[serde(flatten)]
    pub comments: Comments,
}

/// Motion settings. Themes carry these as defaults; presets and overrides win
/// field by field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrollback_parallax: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_wheel_scroll: Option<AltWheelScroll>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_scroll: Option<AutoScroll>,
    #[serde(flatten)]
    pub comments: Comments,
}

/// One choice per part. All seven are required in a preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parts {
    pub art: ArtPart,
    pub scheme: SchemePart,
    pub palette: PalettePart,
    pub font: FontPart,
    pub chrome: ChromePart,
    pub status: StatusPart,
    pub motion: Motion,
    #[serde(flatten)]
    pub comments: Comments,
}

/// Same shape as [`Parts`] with every part optional, as used by overrides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartialParts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art: Option<ArtPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<SchemePart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<PalettePart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<FontPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chrome: Option<ChromePart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,
    #[serde(flatten)]
    pub comments: Comments,
}

// ---------------------------------------------------------------------------
// Preset and overrides documents
// ---------------------------------------------------------------------------

/// `presets/<slug>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub schema_version: u64,
    pub id: String,
    pub name: String,
    /// The preset this was snapshotted from. `null` for an original. The
    /// parent may no longer exist and resolution does not care.
    pub based_on: Option<String>,
    pub parts: Parts,
    #[serde(flatten)]
    pub comments: Comments,
}

/// `overrides.json` in the fleet or local layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Overrides {
    pub schema_version: u64,
    pub parts: PartialParts,
    #[serde(flatten)]
    pub comments: Comments,
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    Dark,
    Light,
}

/// Semantic UI colours. Colours are `#rrggbb` or `#rrggbbaa` strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiPalette {
    pub bg: String,
    pub surface: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<String>,
    pub fg: String,
    pub fg_dim: String,
    pub accent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_alt: Option<String>,
    pub ok: String,
    pub warn: String,
    pub bad: String,
    pub info: String,
    pub tab_bar_bg: String,
    pub tab_active_bg: String,
    pub tab_active_fg: String,
    pub tab_inactive_bg: String,
    pub tab_inactive_fg: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_hover_bg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_hover_fg: Option<String>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalScheme {
    pub foreground: String,
    pub background: String,
    pub cursor_bg: String,
    pub cursor_fg: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_border: Option<String>,
    pub selection_bg: String,
    pub selection_fg: String,
    pub ansi: [String; 8],
    pub brights: [String; 8],
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemePalette {
    pub ui: UiPalette,
    pub scheme: TerminalScheme,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Primitive {
    Starfield,
    DitherWash,
    CloudBlobs,
    PerspectiveGrid,
    Scanlines,
    Vignette,
    SpriteStrip,
    ScatteredSprites,
    IsometricTiles,
    SilhouetteBands,
    Image,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    None,
    X,
    Y,
    Xy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtLayer {
    pub id: String,
    pub primitive: Primitive,
    /// Free-form primitive parameters, validated by the art engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Keyed<Value>>,
    pub scale: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallax: Option<Parallax>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Repeat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(flatten)]
    pub comments: Comments,
}

/// The layer recipe. Its recipe hash is blake3 over this object with comment
/// keys removed, as canonical JSON with sorted keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeArt {
    pub seed: u64,
    pub base_color: String,
    /// Bottom to top.
    pub layers: Vec<ArtLayer>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradientShape {
    Linear,
    Radial,
}

/// WezTerm `Color` and `Gradient` layers used when no art exists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FallbackLayer {
    Color {
        color: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opacity: Option<Number>,
        #[serde(flatten)]
        comments: Comments,
    },
    Gradient {
        shape: GradientShape,
        /// Degrees, linear gradients only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        angle: Option<Number>,
        colors: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opacity: Option<Number>,
        #[serde(flatten)]
        comments: Comments,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Legibility {
    pub text: String,
    pub dim_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_contrast: Option<Number>,
    #[serde(flatten)]
    pub comments: Comments,
}

/// `themes/<slug>/theme.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub schema_version: u64,
    pub id: String,
    pub name: String,
    pub variant: Variant,
    pub palette: ThemePalette,
    pub art: ThemeArt,
    pub fallback_layers: Vec<FallbackLayer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,
    pub legibility: Legibility,
    #[serde(flatten)]
    pub comments: Comments,
}

// ---------------------------------------------------------------------------
// Machine settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VpnKind {
    Tailscale,
    Warp,
    AwsVpn,
    Interface,
    Command,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VpnProbe {
    pub id: String,
    pub kind: VpnKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Network interface name, for kind `interface`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
    /// Argument vector run without a shell, for kind `command`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    /// AWS VPN profile to watch, for kind `aws_vpn`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Editor {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushTarget {
    /// Tried in order.
    pub hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_seconds: Option<u64>,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub width: u64,
    pub height: u64,
    #[serde(flatten)]
    pub comments: Comments,
}

/// `machine.json` in the fleet and local layers. Personal values live only
/// here, so exporting a preset cannot leak them. No field has a default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    pub schema_version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_roots: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_url_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_key_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vpn_probes: Option<Vec<VpnProbe>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<Editor>,
    /// Keyed by target name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_targets: Option<Keyed<PushTarget>>,
    /// Development only. Never exported; `doctor` flags it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev_art_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_overrides: Option<Vec<ScreenOverride>>,
    #[serde(flatten)]
    pub comments: Comments,
}

// ---------------------------------------------------------------------------
// Engine state and screens
// ---------------------------------------------------------------------------

/// Undo history is capped at this many entries.
pub const HISTORY_CAP: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub preset: String,
    /// RFC 3339 date-time.
    pub at: String,
    #[serde(flatten)]
    pub comments: Comments,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstallMode {
    #[serde(rename = "add-on")]
    AddOn,
    #[serde(rename = "replace")]
    Replace,
    #[serde(rename = "replace-import")]
    ReplaceImport,
}

/// Written by the Lua engine so the Rust side reads the same built-ins the
/// running Lua uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineInfo {
    pub plugin_dir: String,
    pub version: String,
    /// Highest document `schema_version` the engine supports.
    pub schema_version: u64,
    #[serde(flatten)]
    pub comments: Comments,
}

/// `state.json`. Watched by WezTerm, so only the TUI and CLI write it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub schema_version: u64,
    pub active_preset: String,
    /// Oldest first, capped at [`HISTORY_CAP`].
    pub history: Vec<HistoryEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_mode: Option<InstallMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineInfo>,
    #[serde(flatten)]
    pub comments: Comments,
}

impl State {
    /// The plugin directory the Lua engine recorded, if it has run.
    pub fn plugin_dir(&self) -> Option<&str> {
        self.engine.as_ref().map(|e| e.plugin_dir.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub width: u64,
    pub height: u64,
    #[serde(flatten)]
    pub comments: Comments,
}

/// `screens.json`. Not watched by WezTerm; written by Lua from a GUI event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Screens {
    pub schema_version: u64,
    pub screens: Vec<Screen>,
    #[serde(flatten)]
    pub comments: Comments,
}
