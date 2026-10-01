//! Export a preset (+ its theme) as a public contribution bundle.
//!
//! Machine settings are never included (they live in a separate schema). The
//! bundle is still scanned for hostname and email patterns — including `_`
//! comment values — because comments are free text and can leak personal data.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use thiserror::Error;
use wzt_model::paths::{PRESETS_DIR, THEME_FILE, THEMES_DIR};
use wzt_model::{
    Paths, Preset, SUPPORTED_SCHEMA_VERSION, SchemePart, Theme, read_document, write_document,
};

use crate::fleet::strip_layer_prefix;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("{0}")]
    Message(String),
    #[error("io error at {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Model(#[from] wzt_model::Error),
}

pub type Result<T> = std::result::Result<T, ExportError>;

impl ExportError {
    pub fn msg(s: impl Into<String>) -> Self {
        Self::Message(s.into())
    }

    fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// Patterns that must not appear in an exported public bundle.
#[derive(Debug, Clone, Default)]
pub struct Denylist {
    /// Exact substrings (typically the machine hostname), matched case-insensitively.
    pub hostnames: Vec<String>,
    /// When true, also reject string values that look like email addresses.
    pub reject_emails: bool,
}

impl Denylist {
    /// Build a denylist from a known hostname. Empty hostname is rejected so
    /// export cannot silently skip hostname scanning.
    pub fn from_hostname(hostname: impl Into<String>) -> Result<Self> {
        let host = hostname.into();
        let host = host.trim();
        if host.is_empty() {
            return Err(ExportError::msg(
                "cannot export: machine hostname is unknown; \
                 refusing so personal hostnames in comments cannot slip through",
            ));
        }
        Ok(Self {
            hostnames: vec![host.to_string()],
            reject_emails: true,
        })
    }
}

/// One denylist hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenylistHit {
    pub path: String,
    pub value: String,
    pub reason: String,
}

/// Report from [`export_bundle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    pub out_dir: PathBuf,
    pub preset_path: PathBuf,
    pub theme_path: Option<PathBuf>,
    pub preset_id: String,
}

/// Export `preset_ref` (local/fleet/builtin slug or id) into `out_dir` as a
/// public PR bundle: `presets/<slug>.json` and optionally `themes/<slug>/theme.json`.
///
/// Machine settings are never written. The resulting JSON is scanned against
/// `denylist` (including `_` comment values).
pub fn export_bundle(
    paths: &Paths,
    preset_ref: &str,
    out_dir: &Path,
    denylist: &Denylist,
    builtin_checkout: Option<&Path>,
) -> Result<ExportReport> {
    let slug = strip_layer_prefix(preset_ref);
    let (preset, _source_layer) = load_preset(paths, slug, builtin_checkout)?;

    // Public contribution uses the builtin namespace.
    let mut exported = preset.clone();
    exported.id = format!("builtin:{slug}");
    exported.schema_version = SUPPORTED_SCHEMA_VERSION;

    let theme_slug = theme_slug_from_preset(&exported);
    let theme = match theme_slug.as_deref() {
        Some(ts) => load_theme(paths, ts, builtin_checkout)?,
        None => None,
    };

    let mut theme_export = theme.clone();
    if let Some(ref mut t) = theme_export {
        let tslug = strip_layer_prefix(&t.id);
        t.id = format!("builtin:{tslug}");
        t.schema_version = SUPPORTED_SCHEMA_VERSION;
        // Point preset parts at the exported theme id when they referenced the old one.
        rewrite_theme_refs(&mut exported, &t.id);
    }

    // Scan before writing so a dirty denylist never leaves partial files.
    let preset_value = serde_json::to_value(&exported)
        .map_err(|e| ExportError::msg(format!("serialize preset: {e}")))?;
    let mut hits = scan_value(&preset_value, "$", denylist);
    if let Some(ref t) = theme_export {
        let theme_value = serde_json::to_value(t)
            .map_err(|e| ExportError::msg(format!("serialize theme: {e}")))?;
        hits.extend(scan_value(&theme_value, "$", denylist));
    }
    if !hits.is_empty() {
        let summary = hits
            .iter()
            .map(|h| format!("{}: {} ({})", h.path, h.value, h.reason))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ExportError::msg(format!(
            "export refused: personal data in bundle: {summary}"
        )));
    }

    let presets_out = out_dir.join(PRESETS_DIR);
    fs::create_dir_all(&presets_out).map_err(|e| ExportError::io(&presets_out, e))?;
    let preset_path = presets_out.join(format!("{slug}.json"));
    write_document(&preset_path, &exported)?;

    // Never copy machine.json — assert absence for the test contract.
    let machine_out = out_dir.join("machine.json");
    if machine_out.exists() {
        let _ = fs::remove_file(&machine_out);
    }

    let theme_path = if let Some(ref t) = theme_export {
        let tslug = strip_layer_prefix(&t.id);
        let theme_dir = out_dir.join(THEMES_DIR).join(tslug);
        fs::create_dir_all(&theme_dir).map_err(|e| ExportError::io(&theme_dir, e))?;
        let path = theme_dir.join(THEME_FILE);
        write_document(&path, t)?;
        Some(path)
    } else {
        None
    };

    Ok(ExportReport {
        out_dir: out_dir.to_path_buf(),
        preset_path,
        theme_path,
        preset_id: exported.id,
    })
}

fn load_preset(
    paths: &Paths,
    slug: &str,
    builtin_checkout: Option<&Path>,
) -> Result<(Preset, &'static str)> {
    let candidates: Vec<(PathBuf, &'static str)> = {
        let mut v = vec![
            (
                paths.local_layer_dir().join(PRESETS_DIR).join(format!("{slug}.json")),
                "local",
            ),
            (
                paths
                    .fleet_layer_dir()
                    .join(PRESETS_DIR)
                    .join(format!("{slug}.json")),
                "fleet",
            ),
        ];
        if let Some(checkout) = builtin_checkout {
            v.push((
                checkout.join(PRESETS_DIR).join(format!("{slug}.json")),
                "builtin",
            ));
        }
        v
    };
    for (path, layer) in candidates {
        if path.is_file() {
            return Ok((read_document(&path)?, layer));
        }
    }
    Err(ExportError::msg(format!(
        "preset `{slug}` not found in local, fleet or builtin layers"
    )))
}

fn load_theme(
    paths: &Paths,
    theme_ref: &str,
    builtin_checkout: Option<&Path>,
) -> Result<Option<Theme>> {
    let slug = strip_layer_prefix(theme_ref);
    let candidates: Vec<PathBuf> = {
        let mut v = vec![
            paths
                .local_layer_dir()
                .join(THEMES_DIR)
                .join(slug)
                .join(THEME_FILE),
            paths
                .fleet_layer_dir()
                .join(THEMES_DIR)
                .join(slug)
                .join(THEME_FILE),
        ];
        if let Some(checkout) = builtin_checkout {
            v.push(checkout.join(THEMES_DIR).join(slug).join(THEME_FILE));
        }
        v
    };
    for path in candidates {
        if path.is_file() {
            return Ok(Some(read_document(&path)?));
        }
    }
    Ok(None)
}

fn theme_slug_from_preset(preset: &Preset) -> Option<String> {
    if let Some(ref t) = preset.parts.art.theme {
        return Some(t.clone());
    }
    if let Some(ref t) = preset.parts.palette.theme {
        return Some(t.clone());
    }
    if let SchemePart::Theme(s) = &preset.parts.scheme {
        return Some(s.theme.clone());
    }
    let v = serde_json::to_value(&preset.parts).ok()?;
    find_theme_string(&v)
}

fn find_theme_string(v: &Value) -> Option<String> {
    match v {
        Value::Object(map) => {
            if let Some(Value::String(s)) = map.get("theme") {
                return Some(s.clone());
            }
            for val in map.values() {
                if let Some(s) = find_theme_string(val) {
                    return Some(s);
                }
            }
            None
        }
        Value::Array(arr) => arr.iter().find_map(find_theme_string),
        _ => None,
    }
}

fn rewrite_theme_refs(preset: &mut Preset, new_theme_id: &str) {
    if let Some(ref mut t) = preset.parts.art.theme {
        *t = new_theme_id.to_string();
    }
    // scheme / palette hold theme via untagged enums — rewrite through JSON.
    if let Ok(mut v) = serde_json::to_value(&preset.parts) {
        rewrite_theme_strings(&mut v, new_theme_id);
        if let Ok(parts) = serde_json::from_value(v) {
            preset.parts = parts;
        }
    }
}

fn rewrite_theme_strings(v: &mut Value, new_theme_id: &str) {
    match v {
        Value::Object(map) => {
            if let Some(Value::String(s)) = map.get_mut("theme") {
                *s = new_theme_id.to_string();
            }
            for val in map.values_mut() {
                rewrite_theme_strings(val, new_theme_id);
            }
        }
        Value::Array(arr) => {
            for val in arr {
                rewrite_theme_strings(val, new_theme_id);
            }
        }
        _ => {}
    }
}

/// Walk a JSON value and collect denylist hits (including `_` comment keys).
pub fn scan_value(value: &Value, path: &str, denylist: &Denylist) -> Vec<DenylistHit> {
    let mut hits = Vec::new();
    scan_inner(value, path, denylist, &mut hits);
    hits
}

fn scan_inner(value: &Value, path: &str, denylist: &Denylist, hits: &mut Vec<DenylistHit>) {
    match value {
        Value::String(s) => {
            if let Some(reason) = match_denylist(s, denylist) {
                hits.push(DenylistHit {
                    path: path.to_string(),
                    value: s.clone(),
                    reason,
                });
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                scan_inner(v, &format!("{path}[{i}]"), denylist, hits);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                let child = format!("{path}.{k}");
                scan_inner(v, &child, denylist, hits);
            }
        }
        _ => {}
    }
}

fn match_denylist(s: &str, denylist: &Denylist) -> Option<String> {
    let lower = s.to_ascii_lowercase();
    for host in &denylist.hostnames {
        if host.is_empty() {
            continue;
        }
        if lower.contains(&host.to_ascii_lowercase()) {
            return Some(format!("hostname `{host}`"));
        }
    }
    if denylist.reject_emails && contains_email(s) {
        return Some("email address".into());
    }
    None
}

fn contains_email(s: &str) -> bool {
    for token in s.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '<' | '>' | '"' | '\'')) {
        if looks_like_email(token) {
            return true;
        }
    }
    false
}

fn looks_like_email(s: &str) -> bool {
    // Conservative: local@domain.tld with no spaces.
    let s = s.trim().trim_matches(|c| matches!(c, '.' | '!' | '?'));
    if s.is_empty() || s.contains(' ') || s.contains('\n') {
        return false;
    }
    let Some((local, rest)) = s.split_once('@') else {
        return false;
    };
    if local.is_empty() || !rest.contains('.') {
        return false;
    }
    let domain = rest;
    !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scan_finds_hostname_in_comment() {
        let v = json!({
            "schema_version": 1,
            "id": "builtin:x",
            "name": "X",
            "_": "built on metis overnight",
            "parts": {}
        });
        let denylist = Denylist::from_hostname("metis").unwrap();
        let hits = scan_value(&v, "$", &denylist);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].reason.contains("hostname"));
    }

    #[test]
    fn from_hostname_refuses_empty() {
        let err = Denylist::from_hostname("").unwrap_err();
        assert!(err.to_string().contains("hostname is unknown"));
    }

    #[test]
    fn scan_finds_email() {
        let v = json!({ "_note": "contact me@example.com please" });
        let denylist = Denylist {
            hostnames: vec![],
            reject_emails: true,
        };
        let hits = scan_value(&v, "$", &denylist);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].reason, "email address");
    }
}
