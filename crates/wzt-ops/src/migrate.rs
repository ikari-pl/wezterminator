//! Migrate legacy `~/.wezterm-*` state files into the local layer.
//!
//! Originals are left in place until uninstall removes created local files.
//! Unknown values are reported as warnings.

use std::fs;
use std::path::{Path, PathBuf};

use wzt_model::{
    ArtPart, AutoScroll, ChromePart, FontPart, Keyed, Motion, Padding, PalettePart, Parts,
    Paths, Preset, SchemeNamed, SchemePart, SchemeTheme, Segment, State, StatusPart, StatusStyle,
    AltWheelScroll, write_document,
};

use crate::install::{InstallError, Result};

/// Theme id mapping from metis `.wezterm-bgtheme` values.
fn map_bgtheme(name: &str) -> Option<&'static str> {
    match name.trim() {
        "cool" => Some("builtin:cpc-cool"),
        "warm" => Some("builtin:ember"),
        "soft-nebula" | "nebula" => Some("builtin:soft-nebula"),
        _ => None,
    }
}

/// Report of what migration wrote / skipped.
#[derive(Debug, Clone, Default)]
pub struct MigrateReport {
    pub created: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub active_preset: Option<String>,
    pub font_preferred: Option<String>,
    pub font_corrections: Vec<(String, f64)>,
    pub theme_id: Option<String>,
}

/// Read `~/.wezterm-*` under `home` and write a local preset + state update.
pub fn migrate_home_state(paths: &Paths, home: &Path) -> Result<MigrateReport> {
    let mut report = MigrateReport::default();

    let bgtheme = read_line(&home.join(".wezterm-bgtheme"));
    let font = read_line(&home.join(".wezterm-font"));
    let font_size = read_line(&home.join(".wezterm-font-size"));
    let offsets_path = home.join(".wezterm-font-offsets");
    let offsets = read_offsets(&offsets_path, &mut report.warnings)?;
    let scheme = read_line(&home.join(".wezterm-scheme"));

    let any = bgtheme.is_some()
        || font.is_some()
        || font_size.is_some()
        || !offsets.is_empty()
        || scheme.is_some();
    if !any {
        return Ok(report);
    }

    let theme_id = match bgtheme.as_deref().and_then(map_bgtheme) {
        Some(id) => {
            report.theme_id = Some(id.to_string());
            id.to_string()
        }
        None => {
            if let Some(raw) = &bgtheme {
                report.warnings.push(format!(
                    "unknown .wezterm-bgtheme value `{raw}`; defaulting art to builtin:cpc-cool"
                ));
            }
            "builtin:cpc-cool".into()
        }
    };

    let size = font_size
        .as_deref()
        .and_then(|s| s.parse::<f64>().ok())
        .and_then(serde_json::Number::from_f64)
        .unwrap_or_else(|| serde_json::Number::from_f64(14.0).unwrap());

    let preferred = font.clone().unwrap_or_else(|| "JetBrains Mono".into());
    report.font_preferred = Some(preferred.clone());
    report.font_corrections = offsets
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();

    let mut corrections: Keyed<serde_json::Number> = Keyed::default();
    for (name, value) in &offsets {
        if let Some(n) = serde_json::Number::from_f64(*value) {
            corrections.entries.insert(name.clone(), n);
        } else {
            report
                .warnings
                .push(format!("skipped non-finite font offset for `{name}`"));
        }
    }

    let scheme_part = if let Some(name) = scheme {
        // Named WezTerm schemes stay as wezterm_scheme; theme ids from bgtheme
        // already drive art/palette.
        SchemePart::WezTerm(SchemeNamed {
            wezterm_scheme: name,
            comments: Default::default(),
        })
    } else {
        SchemePart::Theme(SchemeTheme {
            theme: theme_id.clone(),
            comments: Default::default(),
        })
    };

    let slug = "migrated";
    let preset_id = format!("local:{slug}");
    let preset = Preset {
        schema_version: 1,
        id: preset_id.clone(),
        name: "Migrated".into(),
        based_on: Some(theme_id.clone()),
        parts: Parts {
            art: ArtPart {
                theme: Some(theme_id.clone()),
                layer_tweaks: None,
                comments: Default::default(),
            },
            scheme: scheme_part,
            palette: PalettePart {
                theme: Some(theme_id.clone()),
                comments: Default::default(),
            },
            font: FontPart {
                preferred: Some(vec![preferred]),
                fallback: Some(vec![
                    "FiraCode Nerd Font Mono".into(),
                    "JetBrains Mono".into(),
                    "Menlo".into(),
                ]),
                size: Some(size),
                corrections: if corrections.entries.is_empty() {
                    None
                } else {
                    Some(corrections)
                },
                comments: Default::default(),
            },
            chrome: ChromePart {
                opacity: Some(serde_json::Number::from_f64(1.0).unwrap()),
                blur: None,
                padding: Some(Padding {
                    left: Some(4),
                    right: Some(4),
                    top: Some(2),
                    bottom: Some(2),
                    comments: Default::default(),
                }),
                inactive_pane: None,
                tab_bar: None,
                comments: Default::default(),
            },
            status: StatusPart {
                style: Some(StatusStyle::Sparkline),
                segments: Some(vec![
                    Segment::Load,
                    Segment::Memory,
                    Segment::MemoryPressure,
                    Segment::Battery,
                    Segment::Cwd,
                    Segment::Clock,
                ]),
                comments: Default::default(),
            },
            motion: Motion {
                scrollback_parallax: Some(true),
                alt_wheel_scroll: Some(AltWheelScroll {
                    vertical: Some(true),
                    horizontal: Some(false),
                    comments: Default::default(),
                }),
                auto_scroll: Some(AutoScroll {
                    enabled: Some(false),
                    speed: Some(serde_json::Number::from(0)),
                    axis: None,
                    comments: Default::default(),
                }),
                comments: Default::default(),
            },
            comments: Default::default(),
        },
        comments: Default::default(),
    };

    let preset_path = paths.local_layer_dir().join("presets").join(format!("{slug}.json"));
    write_document(&preset_path, &preset)?;
    report.created.push(preset_path);
    report.active_preset = Some(preset_id.clone());

    // Point state at the migrated preset (preserve install_mode if already set).
    let state_path = paths.state_file();
    let mut state = if state_path.is_file() {
        wzt_model::read_document::<State>(&state_path).unwrap_or_else(|_| blank_state())
    } else {
        blank_state()
    };
    state.active_preset = preset_id;
    write_document(&state_path, &state)?;

    Ok(report)
}

fn blank_state() -> State {
    State {
        schema_version: 1,
        active_preset: "builtin:cpc-cool".into(),
        history: Vec::new(),
        install_mode: None,
        engine: None,
        comments: Default::default(),
    }
}

fn read_line(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

fn read_offsets(path: &Path, warnings: &mut Vec<String>) -> Result<Vec<(String, f64)>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path).map_err(|e| InstallError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let mut out = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.rsplit_once('=') else {
            warnings.push(format!(
                "{}:{}: expected `Family=offset`, got `{line}`",
                path.display(),
                lineno + 1
            ));
            continue;
        };
        match value.trim().parse::<f64>() {
            Ok(v) => out.push((name.trim().to_string(), v)),
            Err(_) => warnings.push(format!(
                "{}:{}: bad offset `{value}`",
                path.display(),
                lineno + 1
            )),
        }
    }
    Ok(out)
}
