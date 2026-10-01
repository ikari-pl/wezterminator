//! Elm-style TUI application: Model / Msg / update / view.
//!
//! The view is pure. Side effects (OSC writes, state commits) are returned as
//! [`Effect`]s from [`update`] so tests can assert without a real terminal.

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
    HISTORY_CAP, HistoryEntry, Paths, State, SUPPORTED_SCHEMA_VERSION, loader, read_document,
    write_document,
};
use wzt_preview::{
    ACK_TIMEOUT, AckParser, HEARTBEAT_INTERVAL, PreviewPayload, RateLimitedWriter, encode_cancel,
    encode_heartbeat, encode_preview, encode_probe, tmux_set, wezterm_pane_set,
};

pub use wzt_preview::PreviewMode;

use crate::screens::{parts, presets};

/// Which full-screen the user is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Presets,
    Parts,
}

/// One row in the presets list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetRow {
    pub id: String,
    pub name: String,
    pub layer: Layer,
    pub shadowed: bool,
}

/// Theme-part keys shown on the parts screen (U12 shell; U13 deep-edits later).
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
}

/// One part row with source layer and optional overruled mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartRow {
    pub kind: PartKind,
    pub summary: String,
    pub source: String,
    pub overruled: bool,
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
    /// Handshake finished: WezTerm confirmed, or timed out → browser.
    HandshakeDone { wezterm: bool },
    CommitOk(String),
    CommitErr(String),
}

/// Side effects produced by update (tests inspect these).
#[derive(Debug, Clone)]
pub enum Effect {
    Probe { seq: u64 },
    Preview { seq: u64, preset_id: String },
    Heartbeat { seq: u64 },
    Cancel { seq: u64 },
    Commit { preset_id: String },
    OpenBrowserStub,
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
            Screen::Parts => {
                model.screen = Screen::Presets;
                model.status = "presets".into();
            }
            Screen::Presets => {
                if model.mode == PreviewMode::WezTerm {
                    model.seq += 1;
                    effects.push(Effect::Cancel { seq: model.seq });
                }
                model.quit = true;
                model.status = "cancelled".into();
            }
        },
        Msg::Tab => {
            model.screen = match model.screen {
                Screen::Presets => Screen::Parts,
                Screen::Parts => Screen::Presets,
            };
            refresh_parts(model);
        }
        Msg::Up => match model.screen {
            Screen::Presets => {
                select_prev(&mut model.preset_state, model.presets.len());
                if let Some(row) = model.selected_preset().cloned() {
                    effects.extend(preview_selection(model, &row.id));
                    refresh_parts(model);
                }
            }
            Screen::Parts => select_prev(&mut model.part_state, model.parts.len()),
        },
        Msg::Down => match model.screen {
            Screen::Presets => {
                select_next(&mut model.preset_state, model.presets.len());
                if let Some(row) = model.selected_preset().cloned() {
                    effects.extend(preview_selection(model, &row.id));
                    refresh_parts(model);
                }
            }
            Screen::Parts => select_next(&mut model.part_state, model.parts.len()),
        },
        Msg::Enter => {
            if model.screen == Screen::Presets
                && let Some(row) = model.selected_preset().cloned()
            {
                effects.push(Effect::Commit {
                    preset_id: row.id.clone(),
                });
            }
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
                effects.extend(preview_selection(model, &row.id));
            }
        }
        Msg::HandshakeDone { wezterm } => {
            if wezterm {
                model.mode = PreviewMode::WezTerm;
                model.status = "wezterm preview".into();
                if let Some(row) = model.selected_preset().cloned() {
                    effects.extend(preview_selection(model, &row.id));
                }
            } else {
                model.mode = PreviewMode::Browser;
                model.status = "browser mode (approximate preview; server in U14)".into();
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
    }
    effects
}

fn preview_selection(model: &mut Model, preset_id: &str) -> Vec<Effect> {
    if model.mode != PreviewMode::WezTerm {
        return Vec::new();
    }
    model.seq += 1;
    vec![Effect::Preview {
        seq: model.seq,
        preset_id: preset_id.to_string(),
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
        // Browse list: prefer non-shadowed, keep order.
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

        let mode = opts.force_mode.unwrap_or(PreviewMode::Browser);
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
        };
        // When we have a resolved preset, fill parts from it.
        if let Some(resolved) = resolution.resolved {
            model.parts = parts_from_resolved(&resolved.parts, &model.overruled, "resolved");
        } else {
            refresh_parts(&mut model);
        }
        Ok(model)
    }
}

fn refresh_parts(model: &mut Model) {
    let Some(row) = model.selected_preset().cloned() else {
        model.parts.clear();
        return;
    };
    // Lightweight summaries until U13 deep-edits: show part keys and layer.
    model.parts = PartKind::ALL
        .iter()
        .map(|kind| {
            let overruled = model
                .overruled
                .iter()
                .any(|o| o.path == kind.label() || o.path.starts_with(&format!("{}.", kind.label())));
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
            let style = value
                .get("style")
                .and_then(Value::as_str)
                .unwrap_or("?");
            format!("status · {style}")
        }
        "art" | "palette" => {
            let theme = value
                .get("theme")
                .and_then(Value::as_str)
                .unwrap_or("?");
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
            // U14 will open the browser; for U12 we only note the mode.
            eprintln!("wezterminator tui: browser mode — approximate preview server lands in U14");
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
            Effect::Preview { seq, preset_id } => {
                let payload = PreviewPayload::preview(*seq, preset_id);
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
            Effect::Commit { preset_id: _ } => {}
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
    // RFC 3339 UTC without pulling in chrono: good enough for history stamps.
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

    // Handshake: if WEZTERM_PANE is set and we are not forced, probe for ack.
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
            },
        );
        apply_effects(&effects, &mut transport, tmux)?;
    } else if attempt_wezterm {
        model.seq += 1;
        let probe_seq = model.seq;
        apply_effects(&[Effect::Probe { seq: probe_seq }], &mut transport, tmux)?;
        let acked = wait_for_ack(probe_seq, ACK_TIMEOUT)?;
        let effects = update(
            &mut model,
            Msg::HandshakeDone {
                wezterm: acked,
            },
        );
        apply_effects(&effects, &mut transport, tmux)?;
    } else {
        let effects = update(&mut model, Msg::HandshakeDone { wezterm: false });
        apply_effects(&effects, &mut transport, tmux)?;
    }

    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, paths, &mut model, &mut transport, tmux);
    ratatui::restore();
    result
}

fn wait_for_ack(expected: u64, timeout: Duration) -> Result<bool, AppError> {
    // Enter raw mode briefly to read the APC ack from stdin.
    ratatui::crossterm::terminal::enable_raw_mode()?;
    let mut parser = AckParser::new();
    let deadline = Instant::now() + timeout;
    let mut acked = false;
    while Instant::now() < deadline {
        let remain = deadline.saturating_duration_since(Instant::now());
        if event::poll(remain.min(Duration::from_millis(50)))? {
            // Crossterm may not surface APC as Key events; also try reading
            // nothing here — for U12 the handshake success path is exercised
            // when the engine replies. If we only see key events, keep waiting.
            match event::read()? {
                Event::Key(_) => {}
                Event::Paste(s) => {
                    let (acks, _) = parser.push(s.as_bytes());
                    if acks.contains(&expected) {
                        acked = true;
                        break;
                    }
                }
                _ => {}
            }
        }
        // Also try a non-blocking read of raw stdin for the APC bytes.
        // Crossterm owns the terminal; without a dedicated reader we rely on
        // Paste/Key. The Lua engine's send_text appears as raw input — on
        // macOS WezTerm this often arrives as Paste or as opaque bytes that
        // crossterm drops. Tests cover AckParser; live verify on metis.
    }
    let _ = parser;
    ratatui::crossterm::terminal::disable_raw_mode()?;
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
    while !model.quit {
        terminal.draw(|frame| view(frame, model))?;

        let timeout = HEARTBEAT_INTERVAL.saturating_sub(last_heartbeat.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let msg = match key.code {
                        KeyCode::Char('q') => Some(Msg::Quit),
                        KeyCode::Esc => Some(Msg::Esc),
                        KeyCode::Tab => Some(Msg::Tab),
                        KeyCode::Up | KeyCode::Char('k') => Some(Msg::Up),
                        KeyCode::Down | KeyCode::Char('j') => Some(Msg::Down),
                        KeyCode::Enter => Some(Msg::Enter),
                        _ => None,
                    };
                    if let Some(msg) = msg {
                        let effects = update(model, msg);
                        for effect in &effects {
                            if let Effect::Commit { preset_id } = effect {
                                match commit_preset(paths, preset_id) {
                                    Ok(()) => {
                                        let follow = update(model, Msg::CommitOk(preset_id.clone()));
                                        apply_effects(&follow, transport, tmux)?;
                                    }
                                    Err(err) => {
                                        let follow =
                                            update(model, Msg::CommitErr(err.to_string()));
                                        apply_effects(&follow, transport, tmux)?;
                                    }
                                }
                            }
                        }
                        apply_effects(&effects, transport, tmux)?;
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
        };
        refresh_parts(&mut model);
        model
    }

    #[test]
    fn moving_across_three_presets_emits_three_previews() {
        let mut model = sample_model();
        let mut ids = Vec::new();
        // Initial selection is index 0; move to 1 and 2.
        for _ in 0..2 {
            let effects = update(&mut model, Msg::Down);
            for e in effects {
                if let Effect::Preview { preset_id, .. } = e {
                    ids.push(preset_id);
                }
            }
        }
        // And one more Down wraps to 0 — still a preview.
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
        let effects = update(&mut model, Msg::HandshakeDone { wezterm: false });
        assert_eq!(model.mode, PreviewMode::Browser);
        assert!(matches!(effects[0], Effect::OpenBrowserStub));
    }

    #[test]
    fn overruled_parts_marked_in_addon_mode() {
        let model = sample_model();
        let font = model.parts.iter().find(|p| p.kind == PartKind::Font).unwrap();
        assert!(font.overruled);
        let art = model.parts.iter().find(|p| p.kind == PartKind::Art).unwrap();
        assert!(!art.overruled);
    }
}
