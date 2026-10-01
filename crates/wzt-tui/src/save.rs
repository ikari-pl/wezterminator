//! Persist TUI edits to the chosen layer.
//!
//! A saved preset is always a **full snapshot** with `based_on` metadata, never
//! a diff — upstream edits to the parent must not silently change the save.

use std::path::{Path, PathBuf};

use serde_json::{Number, Value};
use wzt_model::{
    Parts, Preset, StatusStyle, SUPPORTED_SCHEMA_VERSION, write_document,
};

use crate::app::AppError;

/// Which layer a save targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveLayer {
    /// `local/presets/<slug>.json` — full preset snapshot.
    LocalPreset,
    /// `local/overrides.json` — partial parts merge (not used by status-style
    /// snapshot saves; reserved for field-level tweaks).
    LocalOverrides,
}

/// Build a local preset id / filename slug from a display name.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if (c == ' ' || c == '-' || c == '_') && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "unnamed".into()
    } else {
        trimmed
    }
}

/// Path for a local preset file.
pub fn local_preset_path(local_layer: &Path, slug: &str) -> PathBuf {
    local_layer.join("presets").join(format!("{slug}.json"))
}

/// Write a full preset snapshot under the local layer.
///
/// `parts` must already be a complete parts object (as resolved). `based_on`
/// records the parent preset id the user started from.
pub fn save_local_preset(
    local_layer: &Path,
    name: &str,
    based_on: Option<String>,
    parts: Parts,
) -> Result<Preset, AppError> {
    let slug = slugify(name);
    let id = format!("local:{slug}");
    let preset = Preset {
        schema_version: SUPPORTED_SCHEMA_VERSION,
        id,
        name: name.to_string(),
        based_on,
        parts,
        comments: Default::default(),
    };
    let path = local_preset_path(local_layer, &slug);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_document(&path, &preset)?;
    Ok(preset)
}

/// Apply a status-style change onto a parts JSON value and deserialize.
pub fn parts_with_status_style(parts: &Value, style: StatusStyle) -> Result<Parts, AppError> {
    let mut v = parts.clone();
    let style_str = match style {
        StatusStyle::Sparkline => "sparkline",
        StatusStyle::Pill => "pill",
    };
    let status = v
        .as_object_mut()
        .ok_or_else(|| AppError::Message("parts must be an object".into()))?
        .entry("status")
        .or_insert_with(|| Value::Object(Default::default()));
    status
        .as_object_mut()
        .ok_or_else(|| AppError::Message("status must be an object".into()))?
        .insert("style".into(), Value::String(style_str.into()));
    serde_json::from_value(v).map_err(|e| AppError::Message(e.to_string()))
}

/// Set font size (global base) on a parts value.
pub fn parts_with_font_size(parts: &Value, size: f64) -> Result<Parts, AppError> {
    let mut v = parts.clone();
    let font = v
        .as_object_mut()
        .ok_or_else(|| AppError::Message("parts must be an object".into()))?
        .entry("font")
        .or_insert_with(|| Value::Object(Default::default()));
    let num = Number::from_f64(size).ok_or_else(|| AppError::Message("bad font size".into()))?;
    font.as_object_mut()
        .ok_or_else(|| AppError::Message("font must be an object".into()))?
        .insert("size".into(), Value::Number(num));
    serde_json::from_value(v).map_err(|e| AppError::Message(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wzt_model::read_document;

    #[test]
    fn saving_status_style_writes_snapshot_with_based_on() {
        let dir = tempfile::tempdir().unwrap();
        let parts = json!({
            "art": { "theme": "builtin:cpc-cool" },
            "scheme": { "theme": "builtin:cpc-cool" },
            "palette": { "theme": "builtin:cpc-cool" },
            "font": {
                "preferred": ["Terminess Nerd Font Mono"],
                "fallback": ["Menlo"],
                "size": 14.0
            },
            "chrome": { "opacity": 1.0 },
            "status": { "style": "sparkline", "segments": ["load", "clock"] },
            "motion": { "scrollback_parallax": true }
        });
        let typed = parts_with_status_style(&parts, StatusStyle::Pill).unwrap();
        assert_eq!(typed.status.style, Some(StatusStyle::Pill));
        let saved = save_local_preset(
            dir.path(),
            "Cool Pills",
            Some("builtin:cpc-cool".into()),
            typed,
        )
        .unwrap();
        assert_eq!(saved.id, "local:cool-pills");
        assert_eq!(saved.based_on.as_deref(), Some("builtin:cpc-cool"));
        let path = local_preset_path(dir.path(), "cool-pills");
        let round: Preset = read_document(&path).unwrap();
        assert_eq!(round.based_on.as_deref(), Some("builtin:cpc-cool"));
        assert_eq!(round.parts.status.style, Some(StatusStyle::Pill));
        // Full snapshot: every part present.
        assert!(round.parts.art.theme.is_some());
        assert!(round.parts.font.preferred.is_some());
    }
}
