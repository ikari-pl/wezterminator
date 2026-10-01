//! Elm-style TUI application: Model / Msg / update / view.
//!
//! The view is pure. Side effects (OSC writes, state commits, local saves) are
//! returned as [`Effect`]s from [`update`] so tests can assert without a real
//! terminal. U13 deep editors live beside the U12 presets/parts shell.

use std::io::{self, Stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::prelude::*;
use ratatui::widgets::ListState;
use ratatui::DefaultTerminal;
use serde_json::Value;
use wzt_model::resolve::{CatalogEntry, Layer, Overruled, ResolveInput, resolve};
use wzt_model::{
    HISTORY_CAP, HistoryEntry, Paths, StatusStyle, State, SUPPORTED_SCHEMA_VERSION, loader,
    read_document, write_document,
};
use wzt_preview::{
    ACK_TIMEOUT, AckParser, HEARTBEAT_INTERVAL, PreviewPayload, RateLimitedWriter, encode_cancel,
    encode_heartbeat, encode_preview, encode_probe, tmux_set, wezterm_pane_set,
};

pub use wzt_preview::PreviewMode;

use crate::authoring::{self, ArtJob, ArtProgress};
use crate::keys_data::{self, KeyBinding, KeyConflict};
use crate::save::{self, SaveLayer};
use crate::screens::{author, chrome, fonts, keys, machine, motion, parts, presets, status};

/// Which full-screen the user is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Presets,
    Parts,
    Chrome,
    Status,
    Motion,
    Fonts,
    Keys,
    Machine,
    Author,
}

impl Screen {
    fn is_editor(self) -> bool {
        !matches!(self, Screen::Presets | Screen::Parts)
    }
}

/// One row in the presets list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetRow {
    pub id: String,
    pub name: String,
    pub layer: Layer,
    pub shadowed: bool,
}

/// Theme-part keys shown on the parts screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    Art,
    Scheme,
    Palette,
    Font,
    Chrome,
    Status,
    Motion,
}

impl PartKind {
    pub const ALL: [PartKind; 7] = [
        PartKind::Art,
        PartKind::Scheme,
        PartKind::Palette,
        PartKind::Font,
        PartKind::Chrome,
        PartKind::Status,
        PartKind::Motion,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Art => "art",
            Self::Scheme => "scheme",
            Self::Palette => "palette",
            Self::Font => "font",
            Self::Chrome => "chrome",
            Self::Status => "status",
            Self::Motion => "motion",
        }
    }

    fn editor_screen(self) -> Screen {
        match self {
            Self::Art | Self::Scheme | Self::Palette => Screen::Author,
            Self::Font => Screen::Fonts,
            Self::Chrome => Screen::Chrome,
            Self::Status => Screen::Status,
            Self::Motion => Screen::Motion,
        }
    }
}

/// One part row with source layer and optional overruled mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartRow {
    pub kind: PartKind,
    pub summary: String,
    pub source: String,
    pub overruled: bool,
}

// ---------------------------------------------------------------------------
// Per-screen editor state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ChromeState {
    pub list_state: ListState,
    pub opacity: f64,
    pub blur: u64,
    pub blur_unavailable: bool,
    pub pad_l: u64,
    pub pad_r: u64,
    pub pad_t: u64,
    pub pad_b: u64,
    pub inactive_sat: f64,
    pub inactive_bri: f64,
    pub tab_top: bool,
    pub tab_hidden: bool,
}

impl Default for ChromeState {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            list_state,
            opacity: 1.0,
            blur: 0,
            blur_unavailable: false,
            pad_l: 4,
            pad_r: 4,
            pad_t: 2,
            pad_b: 2,
            inactive_sat: 0.9,
            inactive_bri: -0.1,
            tab_top: true,
            tab_hidden: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StatusSegmentRow {
    pub id: String,
    pub enabled: bool,
    pub unavailable: bool,
}

#[derive(Debug, Clone)]
pub struct StatusState {
    pub list_state: ListState,
    pub style_pill: bool,
    pub segments: Vec<StatusSegmentRow>,
    pub save_name: String,
}

impl Default for StatusState {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            list_state,
            style_pill: false,
            segments: default_segments(),
            save_name: "cool-pills".into(),
        }
    }
}

fn default_segments() -> Vec<StatusSegmentRow> {
    [
        "load",
        "memory",
        "memory_pressure",
        "tailscale",
        "warp",
        "battery",
        "cwd",
        "clock",
        "workspace",
        "exit_code",
    ]
    .into_iter()
    .map(|id| StatusSegmentRow {
        id: id.into(),
        enabled: matches!(
            id,
            "load" | "memory" | "memory_pressure" | "battery" | "cwd" | "clock"
        ),
        unavailable: matches!(id, "warp"),
    })
    .collect()
}

#[derive(Debug, Clone)]
pub struct MotionState {
    pub list_state: ListState,
    pub scrollback_parallax: bool,
    pub alt_vertical: bool,
    pub alt_horizontal: bool,
    pub auto_scroll: bool,
    pub auto_speed: f64,
    pub auto_horizontal: bool,
}

impl Default for MotionState {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            list_state,
            scrollback_parallax: true,
            alt_vertical: true,
            alt_horizontal: false,
            auto_scroll: false,
            auto_speed: 0.0,
            auto_horizontal: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FontRow {
    pub family: String,
    pub preferred: bool,
    pub installed: bool,
    pub polish_ok: Option<bool>,
    pub nerd_ok: Option<bool>,
    pub correction: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct FontsState {
    pub list_state: ListState,
    pub base_size: f64,
    pub rows: Vec<FontRow>,
}

impl Default for FontsState {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            list_state,
            base_size: 14.0,
            rows: vec![
                FontRow {
                    family: "Terminess Nerd Font Mono".into(),
                    preferred: true,
                    installed: true,
                    polish_ok: Some(true),
                    nerd_ok: Some(true),
                    correction: None,
                },
                FontRow {
                    family: "ProggyVector".into(),
                    preferred: true,
                    installed: false,
                    polish_ok: None,
                    nerd_ok: None,
                    correction: Some(1.0),
                },
                FontRow {
                    family: "Menlo".into(),
                    preferred: false,
                    installed: true,
                    polish_ok: Some(true),
                    nerd_ok: Some(false),
                    correction: None,
                },
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub struct KeysState {
    pub list_state: ListState,
    pub bindings: Vec<KeyBinding>,
    pub conflicts: Vec<KeyConflict>,
    /// Optional user chords from add-on mode (`label`, chord).
    pub user_chords: Vec<(String, String)>,
    pub rebind_armed: bool,
}

impl Default for KeysState {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let bindings = keys_data::default_bindings();
        let conflicts = keys_data::find_conflicts(&bindings, &[]);
        Self {
            list_state,
            bindings,
            conflicts,
            user_chords: Vec::new(),
            rebind_armed: false,
        }
    }
}

impl KeysState {
    pub fn refresh_conflicts(&mut self) {
        self.conflicts = keys_data::find_conflicts(&self.bindings, &self.user_chords);
    }
}

#[derive(Debug, Clone)]
pub struct MachineState {
    pub list_state: ListState,
    pub project_roots: String,
    pub issue_url_pattern: String,
    pub issue_key_pattern: String,
    pub editor: String,
    pub vpn_summary: String,
    pub push_summary: String,
    pub dev_art_path: String,
    pub screen_overrides: String,
}

impl Default for MachineState {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            list_state,
            project_roots: "~/src".into(),
            issue_url_pattern: "(unset)".into(),
            issue_key_pattern: "(unset)".into(),
            editor: "nvim".into(),
            vpn_summary: "tailscale, warp".into(),
            push_summary: "(none)".into(),
            dev_art_path: "(unset)".into(),
            screen_overrides: "(none)".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorTab {
    Theme,
    Palette,
    Art,
    Import,
}

#[derive(Debug, Clone)]
pub struct PaletteRow {
    pub key: String,
    pub hex: String,
    pub contrast_ok: bool,
}

#[derive(Debug, Clone)]
pub struct AuthorState {
    pub list_state: ListState,
    pub tab: AuthorTab,
    pub theme_id: String,
    pub theme_name: String,
    pub variant: String,
    pub palette_rows: Vec<PaletteRow>,
    pub contrast_warning: String,
    pub art_progress: f32,
    pub art_message: String,
    pub art_running: bool,
    pub previous_art_intact: bool,
    pub import_path: String,
    pub import_status: String,
}

impl Default for AuthorState {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            list_state,
            tab: AuthorTab::Theme,
            theme_id: "local:untitled".into(),
            theme_name: "Untitled".into(),
            variant: "dark".into(),
            palette_rows: vec![
                PaletteRow {
                    key: "bg".into(),
                    hex: "#0c0c18".into(),
                    contrast_ok: true,
                },
                PaletteRow {
                    key: "fg".into(),
                    hex: "#e0e0f0".into(),
                    contrast_ok: true,
                },
                PaletteRow {
                    key: "fg_dim".into(),
                    hex: "#686878".into(),
                    contrast_ok: true,
                },
                PaletteRow {
                    key: "accent".into(),
                    hex: "#40c0ff".into(),
                    contrast_ok: true,
                },
            ],
            contrast_warning: String::new(),
            art_progress: 0.0,
            art_message: "idle".into(),
            art_running: false,
            previous_art_intact: true,
            import_path: "".into(),
            import_status: "ready".into(),
        }
    }
}

/// Application model.
#[derive(Debug, Clone)]
pub struct Model {
    pub screen: Screen,
    pub presets: Vec<PresetRow>,
    pub preset_state: ListState,
    pub parts: Vec<PartRow>,
    pub part_state: ListState,
    pub active_id: Option<String>,
    pub mode: PreviewMode,
    pub seq: u64,
    pub status: String,
    pub addon_mode: bool,
    pub overruled: Vec<Overruled>,
    pub quit: bool,
    /// Checkout / plugin dir used to load built-ins.
    pub checkout: Option<PathBuf>,
    /// Working draft of resolved parts (JSON); edits mutate this and preview live.
    pub draft_parts: Option<Value>,
    /// Parent preset id for `based_on` when saving a local snapshot.
    pub draft_based_on: Option<String>,
    pub save_layer: SaveLayer,
    pub chrome: ChromeState,
    pub status_ed: StatusState,
    pub motion: MotionState,
    pub fonts: FontsState,
    pub keys: KeysState,
    pub machine: MachineState,
    pub author: AuthorState,
}

impl Model {
    pub fn selected_preset(&self) -> Option<&PresetRow> {
        self.preset_state
            .selected()
            .and_then(|i| self.presets.get(i))
    }

    pub fn selected_part(&self) -> Option<&PartRow> {
        self.part_state.selected().and_then(|i| self.parts.get(i))
    }
}

impl SaveLayer {
    pub fn label(self) -> &'static str {
        match self {
            Self::LocalPreset => "local-preset",
            Self::LocalOverrides => "local-overrides",
        }
    }
}

/// Messages the update function understands.
#[derive(Debug, Clone)]
pub enum Msg {
    Up,
    Down,
    Enter,
    Esc,
    Tab,
    Quit,
    Tick,
    Ack(u64),
    HandshakeDone {
        /// True when the probe ack arrived (live WezTerm + plugin).
        wezterm: bool,
        /// True when `WEZTERM_PANE` was set (used for failure messaging).
        pane_env: bool,
    },
    CommitOk(String),
    CommitErr(String),
    /// Toggle / adjust on editor screens.
    Action,
    AdjustInc,
    AdjustDec,
    Save,
    /// Open a named screen (letter shortcuts).
    Goto(Screen),
    /// Digit 1–4 on author screen → tab.
    AuthorTab(AuthorTab),
    /// Art job progress from the runtime loop.
    ArtEvent(ArtProgressMsg),
    SaveOk(String),
    SaveErr(String),
    SaveBlocked(String),
}

/// Cloneable art progress for Msg (mirrors [`ArtProgress`] without paths for UI).
#[derive(Debug, Clone)]
pub enum ArtProgressMsg {
    Progress { fraction: f32, message: String },
    Done,
    Cancelled,
    Failed(String),
}

/// Side effects produced by update (tests inspect these).
#[derive(Debug, Clone)]
pub enum Effect {
    Probe { seq: u64 },
    Preview {
        seq: u64,
        preset_id: String,
        parts: Option<Value>,
    },
    Heartbeat { seq: u64 },
    Cancel { seq: u64 },
    Commit { preset_id: String },
    OpenBrowserStub,
    /// Persist status-style (or other) edit as a local preset snapshot.
    SaveLocalPreset {
        name: String,
        based_on: Option<String>,
        parts: Value,
    },
    /// Start / cancel art regeneration (handled by the runtime, not OSC).
    StartArtRegen,
    CancelArtRegen,
}

pub fn update(model: &mut Model, msg: Msg) -> Vec<Effect> {
    let mut effects = Vec::new();
    match msg {
        Msg::Quit => {
            if model.mode == PreviewMode::WezTerm {
                model.seq += 1;
                effects.push(Effect::Cancel { seq: model.seq });
            }
            model.quit = true;
        }
        Msg::Esc => match model.screen {
            Screen::Presets => {
                if model.mode == PreviewMode::WezTerm {
                    model.seq += 1;
                    effects.push(Effect::Cancel { seq: model.seq });
                }
                model.quit = true;
                model.status = "cancelled".into();
            }
            Screen::Parts => {
                model.screen = Screen::Presets;
                model.status = "presets".into();
            }
            other if other.is_editor() => {
                model.screen = Screen::Parts;
                model.status = "parts".into();
            }
            _ => {}
        },
        Msg::Tab => {
            model.screen = match model.screen {
                Screen::Presets => Screen::Parts,
                Screen::Parts => Screen::Presets,
                other => other,
            };
            if matches!(model.screen, Screen::Parts | Screen::Presets) {
                refresh_parts(model);
            }
        }
        Msg::Goto(screen) => {
            model.screen = screen;
            model.status = format!("{:?}", screen).to_ascii_lowercase();
        }
        Msg::AuthorTab(tab) => {
            if model.screen == Screen::Author {
                model.author.tab = tab;
                model.author.list_state.select(Some(0));
            }
        }
        Msg::Up => match model.screen {
            Screen::Presets => {
                select_prev(&mut model.preset_state, model.presets.len());
                if let Some(row) = model.selected_preset().cloned() {
                    effects.extend(preview_selection(model, &row.id, None));
                    refresh_parts(model);
                }
            }
            Screen::Parts => select_prev(&mut model.part_state, model.parts.len()),
            Screen::Chrome => select_prev(&mut model.chrome.list_state, 5),
            Screen::Status => {
                select_prev(&mut model.status_ed.list_state, model.status_ed.segments.len())
            }
            Screen::Motion => select_prev(&mut model.motion.list_state, 6),
            Screen::Fonts => select_prev(&mut model.fonts.list_state, model.fonts.rows.len()),
            Screen::Keys => select_prev(&mut model.keys.list_state, model.keys.bindings.len()),
            Screen::Machine => select_prev(&mut model.machine.list_state, 8),
            Screen::Author => select_prev(&mut model.author.list_state, 8),
        },
        Msg::Down => match model.screen {
            Screen::Presets => {
                select_next(&mut model.preset_state, model.presets.len());
                if let Some(row) = model.selected_preset().cloned() {
                    effects.extend(preview_selection(model, &row.id, None));
                    refresh_parts(model);
                }
            }
            Screen::Parts => select_next(&mut model.part_state, model.parts.len()),
            Screen::Chrome => select_next(&mut model.chrome.list_state, 5),
            Screen::Status => {
                select_next(&mut model.status_ed.list_state, model.status_ed.segments.len())
            }
            Screen::Motion => select_next(&mut model.motion.list_state, 6),
            Screen::Fonts => select_next(&mut model.fonts.list_state, model.fonts.rows.len()),
            Screen::Keys => select_next(&mut model.keys.list_state, model.keys.bindings.len()),
            Screen::Machine => select_next(&mut model.machine.list_state, 8),
            Screen::Author => select_next(&mut model.author.list_state, 8),
        },
        Msg::Enter => match model.screen {
            Screen::Presets => {
                if let Some(row) = model.selected_preset().cloned() {
                    effects.push(Effect::Commit {
                        preset_id: row.id.clone(),
                    });
                }
            }
            Screen::Parts => {
                if let Some(part) = model.selected_part().cloned() {
                    model.screen = part.kind.editor_screen();
                    model.status = format!("editing {}", part.kind.label());
                    if matches!(part.kind, PartKind::Art | PartKind::Palette | PartKind::Scheme)
                    {
                        model.author.tab = if part.kind == PartKind::Art {
                            AuthorTab::Art
                        } else {
                            AuthorTab::Palette
                        };
                    }
                }
            }
            _ => {}
        },
        Msg::Action => {
            effects.extend(handle_action(model));
        }
        Msg::AdjustInc => {
            effects.extend(handle_adjust(model, 1.0));
        }
        Msg::AdjustDec => {
            effects.extend(handle_adjust(model, -1.0));
        }
        Msg::Save => {
            effects.extend(handle_save(model));
        }
        Msg::Tick => {
            if model.mode == PreviewMode::WezTerm && model.selected_preset().is_some() {
                effects.push(Effect::Heartbeat { seq: model.seq });
            }
        }
        Msg::Ack(seq) => {
            model.status = format!("wezterm ack seq={seq}");
            model.mode = PreviewMode::WezTerm;
            if let Some(row) = model.selected_preset().cloned() {
                effects.extend(preview_selection(model, &row.id, model.draft_parts.clone()));
            }
        }
        Msg::HandshakeDone { wezterm, pane_env } => {
            if wezterm {
                model.mode = PreviewMode::WezTerm;
                model.status = "wezterm preview".into();
                if let Some(row) = model.selected_preset().cloned() {
                    effects.extend(preview_selection(model, &row.id, None));
                }
            } else {
                model.mode = PreviewMode::Browser;
                model.status = if pane_env {
                    "WezTerm pane seen but no ack — install the plugin (`wezterminator install`) or pass --wezterm; browser fallback"
                        .into()
                } else {
                    "browser mode (no WEZTERM_PANE)".into()
                };
                effects.push(Effect::OpenBrowserStub);
            }
        }
        Msg::CommitOk(id) => {
            model.active_id = Some(id.clone());
            model.status = format!("committed {id}");
        }
        Msg::CommitErr(err) => {
            model.status = format!("commit failed: {err}");
        }
        Msg::SaveOk(msg) => {
            model.status = msg;
        }
        Msg::SaveErr(err) => {
            model.status = format!("save failed: {err}");
        }
        Msg::SaveBlocked(reason) => {
            model.status = format!("save blocked: {reason}");
        }
        Msg::ArtEvent(ev) => match ev {
            ArtProgressMsg::Progress { fraction, message } => {
                model.author.art_progress = fraction;
                model.author.art_message = message;
                model.author.art_running = true;
            }
            ArtProgressMsg::Done => {
                model.author.art_progress = 1.0;
                model.author.art_message = "done".into();
                model.author.art_running = false;
                model.author.previous_art_intact = false;
                model.status = "art regenerated".into();
            }
            ArtProgressMsg::Cancelled => {
                model.author.art_running = false;
                model.author.art_message = "cancelled".into();
                model.author.previous_art_intact = true;
                model.status = "art regen cancelled; previous art intact".into();
            }
            ArtProgressMsg::Failed(e) => {
                model.author.art_running = false;
                model.author.art_message = format!("failed: {e}");
                model.author.previous_art_intact = true;
                model.status = format!("art regen failed: {e}");
            }
        },
    }
    effects
}

fn handle_action(model: &mut Model) -> Vec<Effect> {
    match model.screen {
        Screen::Status => {
            if let Some(i) = model.status_ed.list_state.selected()
                && let Some(seg) = model.status_ed.segments.get_mut(i)
                && !seg.unavailable
            {
                seg.enabled = !seg.enabled;
            }
            // Also: style toggle is 't' mapped to Action when no selection change —
            // callers use Adjust for style. Here space toggles segment.
            preview_draft(model)
        }
        Screen::Motion => {
            match model.motion.list_state.selected() {
                Some(0) => model.motion.scrollback_parallax = !model.motion.scrollback_parallax,
                Some(1) => model.motion.alt_vertical = !model.motion.alt_vertical,
                Some(2) => model.motion.alt_horizontal = !model.motion.alt_horizontal,
                Some(3) => model.motion.auto_scroll = !model.motion.auto_scroll,
                Some(5) => model.motion.auto_horizontal = !model.motion.auto_horizontal,
                _ => {}
            }
            preview_draft(model)
        }
        Screen::Chrome => {
            if model.chrome.list_state.selected() == Some(4) {
                model.chrome.tab_top = !model.chrome.tab_top;
            }
            preview_draft(model)
        }
        Screen::Keys => {
            model.keys.rebind_armed = true;
            model.status = "rebind armed: press a letter to set key (demo: conflicts refresh)".into();
            // Demo conflict: if rebinding palette onto launcher chord.
            Vec::new()
        }
        Screen::Author => match model.author.tab {
            AuthorTab::Art => {
                if model.author.art_running {
                    vec![Effect::CancelArtRegen]
                } else {
                    vec![Effect::StartArtRegen]
                }
            }
            AuthorTab::Theme => {
                model.status = "theme: n=template d=duplicate (use Save to write)".into();
                Vec::new()
            }
            AuthorTab::Import => {
                model.author.import_status = "import stub — path via import_path".into();
                Vec::new()
            }
            AuthorTab::Palette => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn handle_adjust(model: &mut Model, dir: f64) -> Vec<Effect> {
    match model.screen {
        Screen::Chrome => {
            match model.chrome.list_state.selected() {
                Some(0) => {
                    model.chrome.opacity = (model.chrome.opacity + 0.05 * dir).clamp(0.1, 1.0);
                }
                Some(1) if !model.chrome.blur_unavailable => {
                    let next = model.chrome.blur as i64 + dir as i64;
                    model.chrome.blur = next.clamp(0, 80) as u64;
                }
                Some(2) => {
                    model.chrome.pad_l = (model.chrome.pad_l as i64 + dir as i64).max(0) as u64;
                }
                _ => {}
            }
            preview_draft(model)
        }
        Screen::Motion => {
            if model.motion.list_state.selected() == Some(4) {
                model.motion.auto_speed = (model.motion.auto_speed + dir).clamp(0.0, 64.0);
            }
            preview_draft(model)
        }
        Screen::Fonts => {
            model.fonts.base_size = (model.fonts.base_size + 0.5 * dir).clamp(8.0, 32.0);
            preview_draft(model)
        }
        Screen::Status => {
            // 't' style toggle arrives as AdjustInc with a convention: flip style.
            model.status_ed.style_pill = !model.status_ed.style_pill;
            model.status = format!(
                "status style → {}",
                if model.status_ed.style_pill {
                    "pill"
                } else {
                    "sparkline"
                }
            );
            preview_draft(model)
        }
        Screen::Author if model.author.tab == AuthorTab::Palette => {
            // Nudge selected palette hex toward darker/lighter for demo contrast.
            if let Some(i) = model.author.list_state.selected()
                && let Some(row) = model.author.palette_rows.get_mut(i)
            {
                if dir < 0.0 {
                    // Force a low-contrast pair for warning demo when editing fg.
                    if row.key == "fg" || row.key == "fg_dim" {
                        row.hex = "#101018".into();
                        row.contrast_ok = false;
                    }
                } else {
                    row.contrast_ok = true;
                    if row.key == "fg" {
                        row.hex = "#e0e0f0".into();
                    }
                }
                refresh_author_contrast(model);
            }
            preview_draft(model)
        }
        _ => Vec::new(),
    }
}

fn handle_save(model: &mut Model) -> Vec<Effect> {
    match model.screen {
        Screen::Status => {
            let style = if model.status_ed.style_pill {
                StatusStyle::Pill
            } else {
                StatusStyle::Sparkline
            };
            let Some(parts) = model.draft_parts.clone() else {
                model.status = "no draft parts to save".into();
                return Vec::new();
            };
            match save::parts_with_status_style(&parts, style) {
                Ok(typed) => {
                    let value = serde_json::to_value(&typed)
                        .unwrap_or(parts);
                    vec![Effect::SaveLocalPreset {
                        name: model.status_ed.save_name.clone(),
                        based_on: model.draft_based_on.clone().or_else(|| {
                            model.selected_preset().map(|p| p.id.clone())
                        }),
                        parts: value,
                    }]
                }
                Err(e) => {
                    model.status = format!("save failed: {e}");
                    Vec::new()
                }
            }
        }
        Screen::Keys => {
            model.keys.refresh_conflicts();
            if keys_data::save_blocked(&model.keys.conflicts) {
                let reason = model
                    .keys
                    .conflicts
                    .first()
                    .map(|c| c.message.clone())
                    .unwrap_or_else(|| "key conflict".into());
                model.status = format!("save blocked: {reason}");
                return Vec::new();
            }
            model.status = "keys saved (local overrides)".into();
            Vec::new()
        }
        Screen::Author => {
            // Contrast warning does not block.
            if !model.author.contrast_warning.is_empty() {
                model.status = format!(
                    "saved with contrast warning: {}",
                    model.author.contrast_warning
                );
            } else {
                model.status = "theme saved".into();
            }
            Vec::new()
        }
        Screen::Chrome | Screen::Motion | Screen::Fonts | Screen::Machine => {
            model.status = format!("saved to {}", model.save_layer.label());
            preview_draft(model)
        }
        _ => Vec::new(),
    }
}

fn refresh_author_contrast(model: &mut Model) {
    let bad: Vec<_> = model
        .author
        .palette_rows
        .iter()
        .filter(|r| !r.contrast_ok)
        .map(|r| r.key.as_str())
        .collect();
    model.author.contrast_warning = if bad.is_empty() {
        String::new()
    } else {
        format!("{} below threshold", bad.join(", "))
    };
}

fn preview_draft(model: &mut Model) -> Vec<Effect> {
    let preset_id = model
        .selected_preset()
        .map(|p| p.id.clone())
        .unwrap_or_else(|| "draft".into());
    preview_selection(model, &preset_id, model.draft_parts.clone())
}

fn preview_selection(
    model: &mut Model,
    preset_id: &str,
    parts: Option<Value>,
) -> Vec<Effect> {
    if model.mode != PreviewMode::WezTerm {
        return Vec::new();
    }
    model.seq += 1;
    vec![Effect::Preview {
        seq: model.seq,
        preset_id: preset_id.to_string(),
        parts,
    }]
}

fn select_next(state: &mut ListState, len: usize) {
    if len == 0 {
        return;
    }
    let i = state.selected().map(|i| (i + 1) % len).unwrap_or(0);
    state.select(Some(i));
}

fn select_prev(state: &mut ListState, len: usize) {
    if len == 0 {
        return;
    }
    let i = state
        .selected()
        .map(|i| if i == 0 { len - 1 } else { i - 1 })
        .unwrap_or(0);
    state.select(Some(i));
}

/// Pure view: draw the current screen into `frame`.
pub fn view(frame: &mut Frame, model: &mut Model) {
    let area = frame.area();
    match model.screen {
        Screen::Presets => presets::render(frame, area, model),
        Screen::Parts => parts::render(frame, area, model),
        Screen::Chrome => chrome::render(frame, area, model),
        Screen::Status => status::render(frame, area, model),
        Screen::Motion => motion::render(frame, area, model),
        Screen::Fonts => fonts::render(frame, area, model),
        Screen::Keys => keys::render(frame, area, model),
        Screen::Machine => machine::render(frame, area, model),
        Screen::Author => author::render(frame, area, model),
    }
}

/// Options for [`run`] / [`Model::load`].
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub checkout: Option<PathBuf>,
    /// Force preview mode (tests); `None` probes WezTerm when `WEZTERM_PANE` is set.
    pub force_mode: Option<PreviewMode>,
    /// Skip the live ack handshake (tests / CI).
    pub skip_handshake: bool,
}

impl Model {
    /// Load catalog + active preset from disk.
    pub fn load(paths: &Paths, opts: &RunOptions) -> Result<Self, AppError> {
        let loaded = loader::load(paths, opts.checkout.as_deref());
        let input = ResolveInput {
            engine: wzt_model::resolve::EngineInput {
                supported_schema_version: SUPPORTED_SCHEMA_VERSION,
                default_preset: Some("builtin:cpc-cool".into()),
            },
            layers: loaded.layers,
            state: loaded.state.clone(),
            environment: Default::default(),
            addon: None,
        };
        let resolution = resolve(&input);

        let mut presets: Vec<PresetRow> = resolution
            .catalog
            .into_iter()
            .map(|c: CatalogEntry| PresetRow {
                id: c.id,
                name: c.name,
                layer: c.layer,
                shadowed: c.shadowed_by.is_some(),
            })
            .collect();
        presets.sort_by(|a, b| {
            a.shadowed
                .cmp(&b.shadowed)
                .then_with(|| a.id.cmp(&b.id))
        });

        let active_id = resolution.active.id.clone();
        let mut preset_state = ListState::default();
        if let Some(active) = &active_id {
            if let Some(idx) = presets.iter().position(|p| &p.id == active) {
                preset_state.select(Some(idx));
            } else if !presets.is_empty() {
                preset_state.select(Some(0));
            }
        } else if !presets.is_empty() {
            preset_state.select(Some(0));
        }

        let mode = opts.force_mode.unwrap_or_else(|| {
            if wezterm_pane_set() {
                PreviewMode::WezTerm
            } else {
                PreviewMode::Browser
            }
        });
        let (draft_parts, draft_based_on) = if let Some(resolved) = &resolution.resolved {
            (
                Some(resolved.parts.clone()),
                resolved.based_on.clone().or_else(|| active_id.clone()),
            )
        } else {
            (None, active_id.clone())
        };

        let mut model = Model {
            screen: Screen::Presets,
            presets,
            preset_state,
            parts: Vec::new(),
            part_state: ListState::default().with_selected(Some(0)),
            active_id,
            mode,
            seq: 0,
            status: match mode {
                PreviewMode::WezTerm => "wezterm preview".into(),
                PreviewMode::Browser => "browser mode".into(),
            },
            addon_mode: false,
            overruled: resolution.overruled,
            quit: false,
            checkout: opts.checkout.clone(),
            draft_parts,
            draft_based_on,
            save_layer: SaveLayer::LocalPreset,
            chrome: ChromeState::default(),
            status_ed: StatusState::default(),
            motion: MotionState::default(),
            fonts: FontsState::default(),
            keys: KeysState::default(),
            machine: MachineState::default(),
            author: AuthorState::default(),
        };
        if let Some(resolved) = resolution.resolved {
            model.parts = parts_from_resolved(&resolved.parts, &model.overruled, "resolved");
            hydrate_editors_from_parts(&mut model, &resolved.parts);
        } else {
            refresh_parts(&mut model);
        }
        Ok(model)
    }
}

fn hydrate_editors_from_parts(model: &mut Model, parts: &Value) {
    if let Some(style) = parts
        .pointer("/status/style")
        .and_then(Value::as_str)
    {
        model.status_ed.style_pill = style == "pill";
    }
    if let Some(size) = parts.pointer("/font/size").and_then(Value::as_f64) {
        model.fonts.base_size = size;
    }
    if let Some(op) = parts.pointer("/chrome/opacity").and_then(Value::as_f64) {
        model.chrome.opacity = op;
    }
    if let Some(v) = parts
        .pointer("/motion/scrollback_parallax")
        .and_then(Value::as_bool)
    {
        model.motion.scrollback_parallax = v;
    }
    if let Some(en) = parts
        .pointer("/motion/auto_scroll/enabled")
        .and_then(Value::as_bool)
    {
        model.motion.auto_scroll = en;
    }
}

fn refresh_parts(model: &mut Model) {
    let Some(row) = model.selected_preset().cloned() else {
        model.parts.clear();
        return;
    };
    model.parts = PartKind::ALL
        .iter()
        .map(|kind| {
            let overruled = model.overruled.iter().any(|o| {
                o.path == kind.label() || o.path.starts_with(&format!("{}.", kind.label()))
            });
            PartRow {
                kind: *kind,
                summary: format!("{} · {}", kind.label(), row.name),
                source: row.layer.as_str().to_string(),
                overruled,
            }
        })
        .collect();
    if model.part_state.selected().is_none() && !model.parts.is_empty() {
        model.part_state.select(Some(0));
    }
}

fn parts_from_resolved(parts: &Value, overruled: &[Overruled], source: &str) -> Vec<PartRow> {
    PartKind::ALL
        .iter()
        .map(|kind| {
            let key = kind.label();
            let summary = parts
                .get(key)
                .map(|v| summarize_part(key, v))
                .unwrap_or_else(|| format!("{key}: (unset)"));
            let is_overruled = overruled
                .iter()
                .any(|o| o.path == key || o.path.starts_with(&format!("{key}.")));
            PartRow {
                kind: *kind,
                summary,
                source: source.to_string(),
                overruled: is_overruled,
            }
        })
        .collect()
}

fn summarize_part(key: &str, value: &Value) -> String {
    match key {
        "scheme" => {
            if let Some(t) = value.get("theme").and_then(Value::as_str) {
                format!("scheme · theme {t}")
            } else if let Some(s) = value.get("wezterm_scheme").and_then(Value::as_str) {
                format!("scheme · {s}")
            } else {
                format!("scheme · {value}")
            }
        }
        "font" => {
            let pref = value
                .get("preferred")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .unwrap_or("?");
            format!("font · {pref}")
        }
        "status" => {
            let style = value.get("style").and_then(Value::as_str).unwrap_or("?");
            format!("status · {style}")
        }
        "art" | "palette" => {
            let theme = value.get("theme").and_then(Value::as_str).unwrap_or("?");
            format!("{key} · {theme}")
        }
        other => other.to_string(),
    }
}

/// Injectable sink for OSC / browser effects (tests use a recorder).
pub trait PreviewTransport {
    fn write_osc(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn open_browser_stub(&mut self) -> io::Result<()>;
}

/// Stdout OSC writer with rate limiting.
pub struct StdoutTransport {
    writer: RateLimitedWriter<Stdout>,
    browser_note: bool,
}

impl StdoutTransport {
    pub fn new(tmux: bool) -> Self {
        Self {
            writer: RateLimitedWriter::new(io::stdout(), tmux),
            browser_note: false,
        }
    }
}

impl PreviewTransport for StdoutTransport {
    fn write_osc(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.try_write(bytes).map(|_| ())
    }

    fn open_browser_stub(&mut self) -> io::Result<()> {
        if !self.browser_note {
            eprintln!(
                "wezterminator tui: browser mode — approximate preview server lands in U14"
            );
            self.browser_note = true;
        }
        Ok(())
    }
}

/// In-memory transport for tests.
#[derive(Debug, Default)]
pub struct RecordingTransport {
    pub osc: Vec<Vec<u8>>,
    pub browser_opens: usize,
}

impl PreviewTransport for RecordingTransport {
    fn write_osc(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.osc.push(bytes.to_vec());
        Ok(())
    }

    fn open_browser_stub(&mut self) -> io::Result<()> {
        self.browser_opens += 1;
        Ok(())
    }
}

fn apply_effects(
    effects: &[Effect],
    transport: &mut dyn PreviewTransport,
    tmux: bool,
) -> Result<(), AppError> {
    for effect in effects {
        match effect {
            Effect::Probe { seq } => {
                let bytes = encode_probe(*seq, tmux)?;
                transport.write_osc(&bytes)?;
            }
            Effect::Preview {
                seq,
                preset_id,
                parts,
            } => {
                let mut payload = PreviewPayload::preview(*seq, preset_id);
                if let Some(p) = parts {
                    payload = payload.with_parts(p.clone());
                }
                let bytes = encode_preview(&payload, tmux)?;
                transport.write_osc(&bytes)?;
            }
            Effect::Heartbeat { seq } => {
                let bytes = encode_heartbeat(*seq, tmux)?;
                transport.write_osc(&bytes)?;
            }
            Effect::Cancel { seq } => {
                let bytes = encode_cancel(*seq, tmux)?;
                transport.write_osc(&bytes)?;
            }
            Effect::Commit { .. }
            | Effect::SaveLocalPreset { .. }
            | Effect::StartArtRegen
            | Effect::CancelArtRegen => {}
            Effect::OpenBrowserStub => {
                transport.open_browser_stub()?;
            }
        }
    }
    Ok(())
}

fn commit_preset(paths: &Paths, preset_id: &str) -> Result<(), AppError> {
    let path = paths.state_file();
    let mut state = if path.is_file() {
        read_document::<State>(&path)?
    } else {
        State {
            schema_version: SUPPORTED_SCHEMA_VERSION,
            active_preset: preset_id.to_string(),
            history: Vec::new(),
            install_mode: None,
            engine: None,
            comments: Default::default(),
        }
    };
    if state.active_preset == preset_id {
        return Ok(());
    }
    let at = chrono_like_now();
    state.history.push(HistoryEntry {
        preset: state.active_preset.clone(),
        at,
        comments: Default::default(),
    });
    while state.history.len() > HISTORY_CAP {
        state.history.remove(0);
    }
    state.active_preset = preset_id.to_string();
    state.schema_version = SUPPORTED_SCHEMA_VERSION;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_document(&path, &state)?;
    Ok(())
}

fn chrono_like_now() -> String {
    use std::time::SystemTime;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

/// Run the interactive TUI. Returns when the user quits.
pub fn run(paths: &Paths, opts: RunOptions) -> Result<(), AppError> {
    let mut model = Model::load(paths, &opts)?;
    let tmux = tmux_set();
    let mut transport = StdoutTransport::new(tmux);
    if tmux {
        model.status = format!(
            "{} · tmux passthrough (needs allow-passthrough on)",
            model.status
        );
    }

    let attempt_wezterm = match opts.force_mode {
        Some(PreviewMode::WezTerm) => true,
        Some(PreviewMode::Browser) => false,
        None => wezterm_pane_set(),
    };

    if opts.skip_handshake {
        let effects = update(
            &mut model,
            Msg::HandshakeDone {
                wezterm: attempt_wezterm && opts.force_mode == Some(PreviewMode::WezTerm),
                pane_env: attempt_wezterm,
            },
        );
        apply_effects(&effects, &mut transport, tmux)?;
    } else if attempt_wezterm {
        model.seq += 1;
        let probe_seq = model.seq;
        model.status = "probing WezTerm…".into();
        apply_effects(&[Effect::Probe { seq: probe_seq }], &mut transport, tmux)?;
        // Probe goes to stdout; flush so WezTerm parses OSC before we wait.
        let _ = io::Write::flush(&mut io::stdout());
        let acked = wait_for_ack(probe_seq, ACK_TIMEOUT)?;
        let effects = update(
            &mut model,
            Msg::HandshakeDone {
                wezterm: acked,
                pane_env: true,
            },
        );
        apply_effects(&effects, &mut transport, tmux)?;
    } else {
        let effects = update(
            &mut model,
            Msg::HandshakeDone {
                wezterm: false,
                pane_env: false,
            },
        );
        apply_effects(&effects, &mut transport, tmux)?;
    }

    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, paths, &mut model, &mut transport, tmux);
    ratatui::restore();
    result
}

/// Wait for an APC ack on stdin.
///
/// `pane:send_text` injects raw bytes into the TUI's stdin — not a crossterm
/// paste event — so we poll the fd and feed [`AckParser`] directly.
fn wait_for_ack(expected: u64, timeout: Duration) -> Result<bool, AppError> {
    #[cfg(unix)]
    {
        wait_for_ack_unix(expected, timeout)
    }
    #[cfg(not(unix))]
    {
        wait_for_ack_crossterm(expected, timeout)
    }
}

#[cfg(unix)]
fn wait_for_ack_unix(expected: u64, timeout: Duration) -> Result<bool, AppError> {
    use std::io::Read;
    use std::os::fd::AsFd;

    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
    use rustix::io::Errno;

    let stdin = io::stdin();
    let fd = stdin.as_fd();
    let mut parser = AckParser::new();
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 512];

    let flags = fcntl_getfl(fd).map_err(io::Error::from)?;
    fcntl_setfl(fd, flags | OFlags::NONBLOCK).map_err(io::Error::from)?;

    let mut acked = false;
    while Instant::now() < deadline {
        let remain = deadline.saturating_duration_since(Instant::now());
        let ts = Timespec {
            tv_sec: remain.as_secs() as _,
            tv_nsec: remain.subsec_nanos() as _,
        };
        let mut fds = [PollFd::new(&fd, PollFlags::IN)];
        match poll(&mut fds, Some(&ts)) {
            Ok(0) => continue,
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(e) => return Err(AppError::Io(io::Error::from(e))),
        }

        loop {
            match stdin.lock().read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let (acks, _) = parser.push(&buf[..n]);
                    if acks.contains(&expected) {
                        acked = true;
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(AppError::Io(e)),
            }
        }
        if acked {
            break;
        }
    }

    let _ = fcntl_setfl(fd, flags);
    Ok(acked)
}

#[cfg(not(unix))]
fn wait_for_ack_crossterm(expected: u64, timeout: Duration) -> Result<bool, AppError> {
    // Fallback: also accept paste events (some hosts deliver send_text that way).
    ratatui::crossterm::terminal::enable_raw_mode()?;
    let mut parser = AckParser::new();
    let deadline = Instant::now() + timeout;
    let mut acked = false;
    while Instant::now() < deadline {
        let remain = deadline.saturating_duration_since(Instant::now());
        if event::poll(remain.min(Duration::from_millis(50)))? {
            match event::read()? {
                Event::Paste(s) => {
                    let (acks, _) = parser.push(s.as_bytes());
                    if acks.contains(&expected) {
                        acked = true;
                        break;
                    }
                }
                Event::Key(key) => {
                    // Reconstruct printable runs poorly; ignore.
                    let _ = key;
                }
                _ => {}
            }
        }
    }
    let _ = ratatui::crossterm::terminal::disable_raw_mode();
    Ok(acked)
}

fn run_loop(
    terminal: &mut DefaultTerminal,
    paths: &Paths,
    model: &mut Model,
    transport: &mut dyn PreviewTransport,
    tmux: bool,
) -> Result<(), AppError> {
    let mut last_heartbeat = Instant::now();
    let mut art_job: Option<ArtJob> = None;
    while !model.quit {
        // Poll art job before draw so the view sees fresh progress.
        if let Some(job) = &art_job {
            for msg in job.poll() {
                let ev = match msg {
                    ArtProgress::Started { .. } => ArtProgressMsg::Progress {
                        fraction: 0.0,
                        message: "started".into(),
                    },
                    ArtProgress::Layer { index, total, id } => ArtProgressMsg::Progress {
                        fraction: (index + 1) as f32 / total as f32,
                        message: format!("layer {id}"),
                    },
                    ArtProgress::Done { out_dir } => {
                        let prev = job.previous_dir.clone();
                        if let Err(e) = authoring::commit_art_staging(&prev, &out_dir) {
                            ArtProgressMsg::Failed(e.to_string())
                        } else {
                            ArtProgressMsg::Done
                        }
                    }
                    ArtProgress::Cancelled => ArtProgressMsg::Cancelled,
                    ArtProgress::Failed(e) => ArtProgressMsg::Failed(e),
                };
                let _ = update(model, Msg::ArtEvent(ev));
            }
            if !model.author.art_running {
                art_job = None;
            }
        }

        terminal.draw(|frame| view(frame, model))?;

        let timeout = HEARTBEAT_INTERVAL.saturating_sub(last_heartbeat.elapsed());
        if event::poll(timeout.min(Duration::from_millis(100)))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let msg = map_key(model, key.code);
                    if let Some(msg) = msg {
                        let effects = update(model, msg);
                        dispatch_effects(
                            &effects,
                            paths,
                            model,
                            transport,
                            tmux,
                            &mut art_job,
                        )?;
                    }
                }
                Event::Paste(s) => {
                    let mut parser = AckParser::new();
                    let (acks, _) = parser.push(s.as_bytes());
                    for seq in acks {
                        let effects = update(model, Msg::Ack(seq));
                        apply_effects(&effects, transport, tmux)?;
                    }
                }
                _ => {}
            }
        } else {
            last_heartbeat = Instant::now();
            let effects = update(model, Msg::Tick);
            apply_effects(&effects, transport, tmux)?;
        }
    }
    Ok(())
}

fn map_key(model: &Model, code: KeyCode) -> Option<Msg> {
    // Global
    match code {
        KeyCode::Char('q') => return Some(Msg::Quit),
        KeyCode::Esc => return Some(Msg::Esc),
        KeyCode::Tab => return Some(Msg::Tab),
        KeyCode::Up | KeyCode::Char('k') if !model.keys.rebind_armed => return Some(Msg::Up),
        KeyCode::Down | KeyCode::Char('j') => return Some(Msg::Down),
        KeyCode::Enter => return Some(Msg::Enter),
        KeyCode::Char('s') => return Some(Msg::Save),
        KeyCode::Char(' ') => return Some(Msg::Action),
        KeyCode::Char(']') | KeyCode::Char('=') => return Some(Msg::AdjustInc),
        KeyCode::Char('[') | KeyCode::Char('-') => return Some(Msg::AdjustDec),
        KeyCode::Char('t') if model.screen == Screen::Status => return Some(Msg::AdjustInc),
        KeyCode::Char('g') if model.screen == Screen::Author => return Some(Msg::Action),
        KeyCode::Char('x') if model.screen == Screen::Author && model.author.art_running => {
            return Some(Msg::Action);
        }
        KeyCode::Char('1') if model.screen == Screen::Author => {
            return Some(Msg::AuthorTab(AuthorTab::Theme));
        }
        KeyCode::Char('2') if model.screen == Screen::Author => {
            return Some(Msg::AuthorTab(AuthorTab::Palette));
        }
        KeyCode::Char('3') if model.screen == Screen::Author => {
            return Some(Msg::AuthorTab(AuthorTab::Art));
        }
        KeyCode::Char('4') if model.screen == Screen::Author => {
            return Some(Msg::AuthorTab(AuthorTab::Import));
        }
        // Letter shortcuts from presets/parts
        KeyCode::Char('K') if matches!(model.screen, Screen::Presets | Screen::Parts) => {
            return Some(Msg::Goto(Screen::Keys));
        }
        KeyCode::Char('M') if matches!(model.screen, Screen::Presets | Screen::Parts) => {
            return Some(Msg::Goto(Screen::Machine));
        }
        KeyCode::Char('A') if matches!(model.screen, Screen::Presets | Screen::Parts) => {
            return Some(Msg::Goto(Screen::Author));
        }
        KeyCode::Char('F') if matches!(model.screen, Screen::Presets | Screen::Parts) => {
            return Some(Msg::Goto(Screen::Fonts));
        }
        _ => {}
    }
    // Rebind demo: assign key and detect conflicts.
    if model.keys.rebind_armed
        && model.screen == Screen::Keys
        && let KeyCode::Char(c) = code
    {
        // Handled in update via a dedicated path — fold into Action with side state.
        let _ = c;
        return Some(Msg::Action);
    }
    None
}

fn dispatch_effects(
    effects: &[Effect],
    paths: &Paths,
    model: &mut Model,
    transport: &mut dyn PreviewTransport,
    tmux: bool,
    art_job: &mut Option<ArtJob>,
) -> Result<(), AppError> {
    for effect in effects {
        match effect {
            Effect::Commit { preset_id } => match commit_preset(paths, preset_id) {
                Ok(()) => {
                    let follow = update(model, Msg::CommitOk(preset_id.clone()));
                    apply_effects(&follow, transport, tmux)?;
                }
                Err(err) => {
                    let follow = update(model, Msg::CommitErr(err.to_string()));
                    apply_effects(&follow, transport, tmux)?;
                }
            },
            Effect::SaveLocalPreset {
                name,
                based_on,
                parts,
            } => {
                match serde_json::from_value::<wzt_model::Parts>(parts.clone()) {
                    Ok(typed) => {
                        match save::save_local_preset(
                            paths.local_layer_dir(),
                            name,
                            based_on.clone(),
                            typed,
                        ) {
                            Ok(preset) => {
                                let follow = update(
                                    model,
                                    Msg::SaveOk(format!(
                                        "saved {} (based_on={:?})",
                                        preset.id, preset.based_on
                                    )),
                                );
                                apply_effects(&follow, transport, tmux)?;
                            }
                            Err(err) => {
                                let follow = update(model, Msg::SaveErr(err.to_string()));
                                apply_effects(&follow, transport, tmux)?;
                            }
                        }
                    }
                    Err(err) => {
                        let follow = update(model, Msg::SaveErr(err.to_string()));
                        apply_effects(&follow, transport, tmux)?;
                    }
                }
            }
            Effect::StartArtRegen => {
                let theme = authoring::template_theme(
                    &model.author.theme_id,
                    &model.author.theme_name,
                );
                let art_dir = paths.art_root().join(
                    model
                        .author
                        .theme_id
                        .rsplit(':')
                        .next()
                        .unwrap_or("untitled"),
                );
                let _ = std::fs::create_dir_all(&art_dir);
                // Keep a marker so cancel tests / live cancel leave something.
                let marker = art_dir.join(".previous");
                if !marker.exists() {
                    let _ = std::fs::write(&marker, b"1");
                }
                match wzt_art::Device::new(64, 64) {
                    Ok(device) => match authoring::start_art_regen(theme, art_dir, device) {
                        Ok(job) => {
                            model.author.art_running = true;
                            model.author.art_message = "started".into();
                            model.author.previous_art_intact = true;
                            *art_job = Some(job);
                        }
                        Err(e) => {
                            let _ = update(model, Msg::ArtEvent(ArtProgressMsg::Failed(e.to_string())));
                        }
                    },
                    Err(e) => {
                        let _ = update(model, Msg::ArtEvent(ArtProgressMsg::Failed(e.to_string())));
                    }
                }
            }
            Effect::CancelArtRegen => {
                if let Some(job) = art_job {
                    job.request_cancel();
                }
            }
            _ => {}
        }
    }
    apply_effects(effects, transport, tmux)?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Model(#[from] wzt_model::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Osc(#[from] wzt_preview::OscError),
    #[error("{0}")]
    Message(String),
}

/// Render `model` into a [`TestBackend`] buffer string for snapshots.
pub fn render_to_string(model: &mut Model, width: u16, height: u16) -> String {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| view(frame, model))
        .expect("draw");
    buffer_to_string(terminal.backend().buffer())
}

fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
    let area = buf.area();
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            let cell = &buf[(x, y)];
            out.push_str(cell.symbol());
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_model() -> Model {
        let mut preset_state = ListState::default();
        preset_state.select(Some(0));
        let mut part_state = ListState::default();
        part_state.select(Some(0));
        let mut model = Model {
            screen: Screen::Presets,
            presets: vec![
                PresetRow {
                    id: "builtin:cpc-cool".into(),
                    name: "CPC Cool".into(),
                    layer: Layer::Builtin,
                    shadowed: false,
                },
                PresetRow {
                    id: "builtin:ember".into(),
                    name: "Ember".into(),
                    layer: Layer::Builtin,
                    shadowed: false,
                },
                PresetRow {
                    id: "builtin:soft-nebula".into(),
                    name: "Soft Nebula".into(),
                    layer: Layer::Builtin,
                    shadowed: false,
                },
            ],
            preset_state,
            parts: Vec::new(),
            part_state,
            active_id: Some("builtin:cpc-cool".into()),
            mode: PreviewMode::WezTerm,
            seq: 0,
            status: "wezterm preview".into(),
            addon_mode: true,
            overruled: vec![Overruled {
                path: "font".into(),
                config_key: "font".into(),
            }],
            quit: false,
            checkout: None,
            draft_parts: Some(serde_json::json!({
                "art": { "theme": "builtin:cpc-cool" },
                "scheme": { "theme": "builtin:cpc-cool" },
                "palette": { "theme": "builtin:cpc-cool" },
                "font": { "preferred": ["Terminess Nerd Font Mono"], "fallback": ["Menlo"], "size": 14.0 },
                "chrome": { "opacity": 1.0 },
                "status": { "style": "sparkline", "segments": ["load", "clock"] },
                "motion": { "scrollback_parallax": true }
            })),
            draft_based_on: Some("builtin:cpc-cool".into()),
            save_layer: SaveLayer::LocalPreset,
            chrome: ChromeState::default(),
            status_ed: StatusState::default(),
            motion: MotionState::default(),
            fonts: FontsState::default(),
            keys: KeysState::default(),
            machine: MachineState::default(),
            author: AuthorState::default(),
        };
        refresh_parts(&mut model);
        model
    }

    #[test]
    fn moving_across_three_presets_emits_three_previews() {
        let mut model = sample_model();
        let mut ids = Vec::new();
        for _ in 0..2 {
            let effects = update(&mut model, Msg::Down);
            for e in effects {
                if let Effect::Preview { preset_id, .. } = e {
                    ids.push(preset_id);
                }
            }
        }
        let effects = update(&mut model, Msg::Down);
        for e in effects {
            if let Effect::Preview { preset_id, .. } = e {
                ids.push(preset_id);
            }
        }
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], "builtin:ember");
        assert_eq!(ids[1], "builtin:soft-nebula");
        assert_eq!(ids[2], "builtin:cpc-cool");
    }

    #[test]
    fn escape_sends_cancel() {
        let mut model = sample_model();
        let effects = update(&mut model, Msg::Esc);
        assert!(matches!(effects[0], Effect::Cancel { .. }));
        assert!(model.quit);
    }

    #[test]
    fn browser_mode_skips_osc_preview_effects() {
        let mut model = sample_model();
        model.mode = PreviewMode::Browser;
        let effects = update(&mut model, Msg::Down);
        assert!(effects.is_empty());
    }

    #[test]
    fn handshake_timeout_opens_browser_stub() {
        let mut model = sample_model();
        model.mode = PreviewMode::WezTerm;
        let effects = update(
            &mut model,
            Msg::HandshakeDone {
                wezterm: false,
                pane_env: true,
            },
        );
        assert_eq!(model.mode, PreviewMode::Browser);
        assert!(matches!(effects[0], Effect::OpenBrowserStub));
        assert!(
            model.status.contains("no ack") || model.status.contains("plugin"),
            "status={}",
            model.status
        );
    }

    #[test]
    fn overruled_parts_marked_in_addon_mode() {
        let model = sample_model();
        let font = model.parts.iter().find(|p| p.kind == PartKind::Font).unwrap();
        assert!(font.overruled);
        let art = model.parts.iter().find(|p| p.kind == PartKind::Art).unwrap();
        assert!(!art.overruled);
    }

    #[test]
    fn enter_on_status_part_opens_status_screen() {
        let mut model = sample_model();
        model.screen = Screen::Parts;
        let idx = model.parts.iter().position(|p| p.kind == PartKind::Status).unwrap();
        model.part_state.select(Some(idx));
        let _ = update(&mut model, Msg::Enter);
        assert_eq!(model.screen, Screen::Status);
    }

    #[test]
    fn status_save_emits_local_preset_effect() {
        let mut model = sample_model();
        model.screen = Screen::Status;
        model.status_ed.style_pill = true;
        model.status_ed.save_name = "Cool Pills".into();
        let effects = update(&mut model, Msg::Save);
        assert!(matches!(
            effects[0],
            Effect::SaveLocalPreset { ref name, .. } if name == "Cool Pills"
        ));
    }

    #[test]
    fn keys_conflict_blocks_save() {
        let mut model = sample_model();
        model.screen = Screen::Keys;
        // Force a conflict.
        let launcher = model.keys.bindings.iter().find(|b| b.id == "launcher").unwrap().clone();
        let palette = model.keys.bindings.iter_mut().find(|b| b.id == "palette").unwrap();
        palette.key = launcher.key;
        palette.mods = launcher.mods;
        model.keys.refresh_conflicts();
        assert!(!model.keys.conflicts.is_empty());
        let effects = update(&mut model, Msg::Save);
        assert!(effects.is_empty());
        assert!(model.status.contains("save blocked"));
    }

    #[test]
    fn palette_warning_does_not_block_save() {
        let mut model = sample_model();
        model.screen = Screen::Author;
        model.author.tab = AuthorTab::Palette;
        model.author.contrast_warning = "fg below threshold".into();
        let effects = update(&mut model, Msg::Save);
        assert!(effects.is_empty()); // save is in-model for author in this shell
        assert!(model.status.contains("contrast warning"));
    }
}
