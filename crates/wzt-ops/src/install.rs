//! Install wezterminator into a WezTerm config (add-on, replace, replace-import).
//!
//! Resolves the config path with WezTerm's search order, writes a reversible
//! change recorded in an install manifest, and optionally migrates `~/.wezterm-*`
//! state into the local layer.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use wzt_model::{InstallMode, Paths, State, write_atomic, write_document};

use crate::migrate::{self, MigrateReport};

/// Markers wrapping the managed block in add-on mode (and identifying our shim).
pub const BEGIN_MARKER: &str = "-- BEGIN wezterminator";
pub const END_MARKER: &str = "-- END wezterminator";

/// Filename of the install manifest under the state directory.
pub const MANIFEST_FILE: &str = "install-manifest.json";

#[derive(Debug, Error)]
pub enum InstallError {
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
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, InstallError>;

impl InstallError {
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

/// How to locate WezTerm's config for this install.
#[derive(Debug, Clone)]
pub struct ConfigEnv {
    /// `$HOME` (or a temp home in tests).
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME`, or `home/.config` when unset.
    pub xdg_config: PathBuf,
    /// Value of `WEZTERM_CONFIG_FILE`, if set.
    pub wezterm_config_file: Option<PathBuf>,
}

impl ConfigEnv {
    /// Read from the process environment.
    pub fn from_process() -> Result<Self> {
        let home = dirs_home().ok_or_else(|| InstallError::msg("HOME is not set"))?;
        let xdg_config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let wezterm_config_file = std::env::var_os("WEZTERM_CONFIG_FILE").map(PathBuf::from);
        Ok(Self {
            home,
            xdg_config,
            wezterm_config_file,
        })
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Outcome of resolving which config file WezTerm will load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub path: PathBuf,
    /// True when the path came from `WEZTERM_CONFIG_FILE`.
    pub from_env: bool,
    /// True when the file does not exist yet (install will create it).
    pub missing: bool,
    pub warning: Option<String>,
}

/// Resolve the config file WezTerm will load, in WezTerm's search order:
/// 1. `WEZTERM_CONFIG_FILE` when set
/// 2. `$XDG_CONFIG_HOME/wezterm/wezterm.lua`
/// 3. `$HOME/.wezterm.lua`
///
/// When nothing exists yet, the XDG path is chosen as the install target.
pub fn resolve_config_path(env: &ConfigEnv) -> ResolvedConfig {
    if let Some(ref path) = env.wezterm_config_file {
        let missing = !path.is_file();
        return ResolvedConfig {
            path: path.clone(),
            from_env: true,
            missing,
            warning: Some(format!(
                "WEZTERM_CONFIG_FILE is set; installing into {}",
                path.display()
            )),
        };
    }

    let xdg = env.xdg_config.join("wezterm").join("wezterm.lua");
    let home_dot = env.home.join(".wezterm.lua");

    if xdg.is_file() {
        return ResolvedConfig {
            path: xdg,
            from_env: false,
            missing: false,
            warning: None,
        };
    }
    if home_dot.is_file() {
        return ResolvedConfig {
            path: home_dot,
            from_env: false,
            missing: false,
            warning: None,
        };
    }

    // Prefer the XDG location for new installs.
    ResolvedConfig {
        path: xdg,
        from_env: false,
        missing: true,
        warning: None,
    }
}

/// Options for [`install`].
#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub mode: InstallMode,
    pub paths: Paths,
    pub config_env: ConfigEnv,
    /// Checkout that holds `plugin/init.lua`.
    pub checkout: PathBuf,
    /// Optional git/plugin URL for add-on `wezterm.plugin.require`. When
    /// absent, add-on loads the checkout via `dofile` like replace mode.
    pub plugin_url: Option<String>,
    /// Skip migrating `~/.wezterm-*` files.
    pub skip_migrate: bool,
}

/// One file recorded in the install manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestFile {
    pub path: PathBuf,
    /// Role hint: `config`, `created`, etc.
    pub role: String,
    /// Content hash before install (`None` when the file was created).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_hash: Option<String>,
    /// Content hash immediately after install.
    pub post_hash: String,
    /// Relative name under the backup directory (`None` when created new).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_name: Option<String>,
}

/// Persisted install record used by uninstall and doctor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstallManifest {
    pub schema_version: u64,
    pub installed_at: String,
    pub mode: InstallMode,
    pub checkout: PathBuf,
    pub config_path: PathBuf,
    pub backup_dir: PathBuf,
    pub files: Vec<ManifestFile>,
    /// Paths created by this install (local presets, etc.) removed on uninstall.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub created: Vec<PathBuf>,
}

impl InstallManifest {
    pub fn path(paths: &Paths) -> PathBuf {
        paths.state_dir().join(MANIFEST_FILE)
    }

    /// Whether the live config still matches what install wrote.
    pub fn config_matches_disk(&self) -> bool {
        let Some(entry) = self.files.iter().find(|f| f.role == "config") else {
            return false;
        };
        match hash_file(&entry.path) {
            Ok(h) => h == entry.post_hash,
            Err(_) => false,
        }
    }
}

/// Report returned by a successful (or idempotent) install.
#[derive(Debug, Clone)]
pub struct InstallReport {
    pub mode: InstallMode,
    pub config_path: PathBuf,
    pub backup_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub warnings: Vec<String>,
    pub idempotent: bool,
    pub migrate: Option<MigrateReport>,
    pub imported_preset: Option<PathBuf>,
}

/// Install wezterminator according to `opts`.
pub fn install(opts: &InstallOptions) -> Result<InstallReport> {
    if !opts.checkout.join("plugin").join("init.lua").is_file() {
        return Err(InstallError::msg(format!(
            "checkout has no plugin/init.lua: {}",
            opts.checkout.display()
        )));
    }

    let resolved = resolve_config_path(&opts.config_env);
    let mut warnings = Vec::new();
    if let Some(w) = &resolved.warning {
        warnings.push(w.clone());
    }

    // Idempotent: same mode + matching post hash → no-op.
    let manifest_path = InstallManifest::path(&opts.paths);
    if let Ok(existing) = read_manifest(&manifest_path)
        && existing.mode == opts.mode
        && existing.config_path == resolved.path
        && existing.config_matches_disk()
        && block_or_shim_present(&resolved.path, opts.mode)?
    {
        return Ok(InstallReport {
            mode: opts.mode,
            config_path: resolved.path,
            backup_dir: existing.backup_dir,
            manifest_path,
            warnings,
            idempotent: true,
            migrate: None,
            imported_preset: None,
        });
    }

    let stamp = timestamp_stamp();
    let backup_dir = opts.paths.state_dir().join("backups").join(&stamp);
    fs::create_dir_all(&backup_dir).map_err(|e| InstallError::io(&backup_dir, e))?;

    let mut files = Vec::new();
    let mut created = Vec::new();
    let mut imported_preset = None;

    let original = if resolved.path.is_file() {
        Some(fs::read(&resolved.path).map_err(|e| InstallError::io(&resolved.path, e))?)
    } else {
        None
    };
    let pre_hash = original.as_ref().map(|b| blake3_hex(b));

    if let Some(ref bytes) = original {
        let backup_name = "config.lua.bak";
        let backup_path = backup_dir.join(backup_name);
        write_atomic(&backup_path, bytes).map_err(InstallError::from)?;
        // recorded below after writing new content
        let _ = backup_name;
    }

    let new_content = match opts.mode {
        InstallMode::AddOn => {
            let base = original
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_else(default_empty_config);
            ensure_addon_block(&base, &opts.checkout, opts.plugin_url.as_deref())?
        }
        InstallMode::Replace => replace_shim(&opts.checkout),
        InstallMode::ReplaceImport => {
            let source = original
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            let (preset, parse_warnings) = import_literals(&source);
            warnings.extend(parse_warnings);
            if let Some(preset) = preset {
                let preset_path = opts
                    .paths
                    .local_layer_dir()
                    .join("presets")
                    .join("imported.json");
                write_document(&preset_path, &preset).map_err(InstallError::from)?;
                created.push(preset_path.clone());
                imported_preset = Some(preset_path);
            }
            replace_shim(&opts.checkout)
        }
    };

    if let Some(parent) = resolved.path.parent() {
        fs::create_dir_all(parent).map_err(|e| InstallError::io(parent, e))?;
    }
    write_atomic(&resolved.path, new_content.as_bytes()).map_err(InstallError::from)?;
    let post_hash = blake3_hex(new_content.as_bytes());

    files.push(ManifestFile {
        path: resolved.path.clone(),
        role: "config".into(),
        pre_hash,
        post_hash,
        backup_name: original.as_ref().map(|_| "config.lua.bak".into()),
    });

    let migrate = if opts.skip_migrate {
        None
    } else {
        let report = migrate::migrate_home_state(&opts.paths, &opts.config_env.home)?;
        for p in &report.created {
            created.push(p.clone());
        }
        warnings.extend(report.warnings.clone());
        Some(report)
    };

    // Record install mode in state.json (create or update).
    update_state_install_mode(&opts.paths, opts.mode, &opts.checkout, imported_preset.as_deref())?;

    let manifest = InstallManifest {
        schema_version: 1,
        installed_at: stamp,
        mode: opts.mode,
        checkout: opts.checkout.clone(),
        config_path: resolved.path.clone(),
        backup_dir: backup_dir.clone(),
        files,
        created: created.clone(),
    };
    write_document(&manifest_path, &manifest).map_err(InstallError::from)?;

    Ok(InstallReport {
        mode: opts.mode,
        config_path: resolved.path,
        backup_dir,
        manifest_path,
        warnings,
        idempotent: false,
        migrate,
        imported_preset,
    })
}

fn block_or_shim_present(config: &Path, mode: InstallMode) -> Result<bool> {
    if !config.is_file() {
        return Ok(false);
    }
    let text = fs::read_to_string(config).map_err(|e| InstallError::io(config, e))?;
    Ok(match mode {
        InstallMode::AddOn => text.contains(BEGIN_MARKER) && text.contains(END_MARKER),
        InstallMode::Replace | InstallMode::ReplaceImport => {
            text.contains(BEGIN_MARKER) && text.contains("apply_to_config")
        }
    })
}

fn default_empty_config() -> String {
    "local wezterm = require 'wezterm'\nlocal config = wezterm.config_builder()\nreturn config\n"
        .into()
}

/// Append or replace the marked add-on block.
fn ensure_addon_block(existing: &str, checkout: &Path, plugin_url: Option<&str>) -> Result<String> {
    let block = addon_block(checkout, plugin_url);
    if let Some(stripped) = strip_marked_block(existing) {
        // Replace existing block (idempotent update).
        let mut out = stripped;
        if !out.ends_with('\n') && !out.is_empty() {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(&block);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        return Ok(out);
    }
    let mut out = existing.to_string();
    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    // Insert before a trailing `return config` when present.
    if let Some(idx) = find_return_config(&out) {
        let (head, tail) = out.split_at(idx);
        let mut combined = head.to_string();
        if !combined.ends_with('\n') && !combined.is_empty() {
            combined.push('\n');
        }
        combined.push('\n');
        combined.push_str(&block);
        combined.push('\n');
        combined.push_str(tail);
        Ok(combined)
    } else {
        out.push('\n');
        out.push_str(&block);
        out.push('\n');
        Ok(out)
    }
}

fn find_return_config(src: &str) -> Option<usize> {
    // Last line that is `return config` (optional semicolon / whitespace).
    let mut last = None;
    for (idx, line) in src.lines().enumerate() {
        let t = line.trim();
        if t == "return config" || t == "return config;" {
            // byte offset of line start
            let mut pos = 0;
            for (i, l) in src.lines().enumerate() {
                if i == idx {
                    last = Some(pos);
                    break;
                }
                pos += l.len() + 1;
            }
        }
    }
    last
}

fn addon_block(checkout: &Path, plugin_url: Option<&str>) -> String {
    let checkout_lua = lua_string(&checkout.display().to_string());
    let load = match plugin_url {
        Some(url) => format!(
            "local wzt = wezterm.plugin.require {url}",
            url = lua_string(url)
        ),
        None => format!("local wzt = dofile({checkout_lua} .. '/plugin/init.lua')"),
    };
    format!(
        "{BEGIN_MARKER}\n\
         -- Managed by `wezterminator install`; do not edit by hand.\n\
         {load}\n\
         wzt.apply_to_config(config, {{ dir = {checkout_lua}, mode = 'addon' }})\n\
         {END_MARKER}"
    )
}

fn replace_shim(checkout: &Path) -> String {
    let checkout_lua = lua_string(&checkout.display().to_string());
    format!(
        "{BEGIN_MARKER}\n\
         -- wezterminator replace-mode shim. Managed by `wezterminator install`.\n\
         local wezterm = require 'wezterm'\n\
         local config = wezterm.config_builder()\n\
         local wzt = dofile({checkout_lua} .. '/plugin/init.lua')\n\
         wzt.apply_to_config(config, {{ dir = {checkout_lua}, mode = 'replace' }})\n\
         return config\n\
         {END_MARKER}\n"
    )
}

fn lua_string(s: &str) -> String {
    // Prefer single quotes; escape uncommon cases.
    if !s.contains('\'') && !s.contains('\\') && !s.contains('\n') {
        format!("'{s}'")
    } else {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        format!("\"{escaped}\"")
    }
}

/// Remove a previously managed BEGIN/END block, returning the remainder.
pub fn strip_marked_block(src: &str) -> Option<String> {
    let begin = src.find(BEGIN_MARKER)?;
    let end_rel = src[begin..].find(END_MARKER)?;
    let end = begin + end_rel + END_MARKER.len();
    // Also drop a single trailing newline after END.
    let mut end = end;
    if src[end..].starts_with('\n') {
        end += 1;
    }
    let mut out = String::new();
    out.push_str(&src[..begin]);
    out.push_str(&src[end..]);
    // Trim excess blank lines left at the join.
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    Some(out)
}

fn update_state_install_mode(
    paths: &Paths,
    mode: InstallMode,
    checkout: &Path,
    imported: Option<&Path>,
) -> Result<()> {
    let state_path = paths.state_file();
    let mut state = if state_path.is_file() {
        wzt_model::read_document::<State>(&state_path).unwrap_or_else(|_| State {
            schema_version: 1,
            active_preset: "builtin:cpc-cool".into(),
            history: Vec::new(),
            install_mode: None,
            engine: None,
            comments: Default::default(),
        })
    } else {
        State {
            schema_version: 1,
            active_preset: "builtin:cpc-cool".into(),
            history: Vec::new(),
            install_mode: None,
            engine: None,
            comments: Default::default(),
        }
    };
    state.install_mode = Some(mode);
    if state.engine.is_none() {
        state.engine = Some(wzt_model::EngineInfo {
            plugin_dir: checkout.display().to_string(),
            version: env!("CARGO_PKG_VERSION").into(),
            schema_version: wzt_model::SUPPORTED_SCHEMA_VERSION,
            comments: Default::default(),
        });
    }
    if let Some(path) = imported
        && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
    {
        state.active_preset = format!("local:{stem}");
    }
    write_document(&state_path, &state).map_err(InstallError::from)
}

/// Parse literal colour / font / keys assignments from a WezTerm lua config.
/// No evaluation: only simple literal forms are taken.
fn import_literals(source: &str) -> (Option<wzt_model::Preset>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut preferred = None;
    let mut size = None;
    let mut scheme = None;
    let mut keys_literal = false;

    for line in source.lines() {
        let t = line.trim();
        if let Some(name) = match_font_assignment(t) {
            preferred = Some(name);
            continue;
        }
        if let Some(n) = match_font_size(t) {
            size = Some(n);
            continue;
        }
        if let Some(s) = match_color_scheme(t) {
            scheme = Some(s);
            continue;
        }
        if t.starts_with("config.keys") || t.starts_with("config.colors") {
            if t.contains('{') && !t.contains('}') {
                warnings.push(format!(
                    "skipped multi-line assignment starting at: {}",
                    truncate(t, 60)
                ));
            } else if t.starts_with("config.keys") {
                keys_literal = true;
                warnings.push(
                    "keys table present but not imported (only simple literals are taken)".into(),
                );
            } else {
                warnings.push(format!(
                    "colours assignment not imported as literal: {}",
                    truncate(t, 60)
                ));
            }
        }
    }

    if preferred.is_none() && scheme.is_none() && size.is_none() && !keys_literal {
        warnings.push("replace-import found no literal font/scheme/keys to import".into());
        return (None, warnings);
    }

    let font_name = preferred.unwrap_or_else(|| "JetBrains Mono".into());
    let size_num = size.unwrap_or_else(|| serde_json::Number::from_f64(14.0).unwrap());

    let scheme_part = match scheme {
        Some(name) => wzt_model::SchemePart::WezTerm(wzt_model::SchemeNamed {
            wezterm_scheme: name,
            comments: Default::default(),
        }),
        None => wzt_model::SchemePart::Theme(wzt_model::SchemeTheme {
            theme: "builtin:cpc-cool".into(),
            comments: Default::default(),
        }),
    };

    let preset = wzt_model::Preset {
        schema_version: 1,
        id: "local:imported".into(),
        name: "Imported".into(),
        based_on: Some("builtin:cpc-cool".into()),
        parts: wzt_model::Parts {
            art: wzt_model::ArtPart {
                theme: Some("builtin:cpc-cool".into()),
                layer_tweaks: None,
                comments: Default::default(),
            },
            scheme: scheme_part,
            palette: wzt_model::PalettePart {
                theme: Some("builtin:cpc-cool".into()),
                comments: Default::default(),
            },
            font: wzt_model::FontPart {
                preferred: Some(vec![font_name]),
                fallback: Some(vec![
                    "JetBrains Mono".into(),
                    "Menlo".into(),
                    "Consolas".into(),
                ]),
                size: Some(size_num),
                corrections: None,
                comments: Default::default(),
            },
            chrome: wzt_model::ChromePart {
                opacity: Some(serde_json::Number::from_f64(1.0).unwrap()),
                blur: None,
                padding: Some(wzt_model::Padding {
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
            status: wzt_model::StatusPart {
                style: Some(wzt_model::StatusStyle::Sparkline),
                segments: Some(vec![
                    wzt_model::Segment::Load,
                    wzt_model::Segment::Memory,
                    wzt_model::Segment::Battery,
                    wzt_model::Segment::Cwd,
                    wzt_model::Segment::Clock,
                ]),
                comments: Default::default(),
            },
            motion: wzt_model::Motion {
                scrollback_parallax: Some(true),
                alt_wheel_scroll: Some(wzt_model::AltWheelScroll {
                    vertical: Some(true),
                    horizontal: Some(false),
                    comments: Default::default(),
                }),
                auto_scroll: Some(wzt_model::AutoScroll {
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
    (Some(preset), warnings)
}

fn match_font_assignment(line: &str) -> Option<String> {
    // config.font = wezterm.font('Name') / wezterm.font("Name") / wezterm.font { family = 'Name' }
    let t = line.trim().trim_end_matches(',');
    // Require an exact `config.font` key (not `config.font_size`).
    let rest = if let Some(r) = t.strip_prefix("config.font") {
        let r = r.trim_start();
        if r.starts_with('=') {
            r.strip_prefix('=')?.trim_start()
        } else {
            return None;
        }
    } else {
        return None;
    };
    if let Some(inner) = rest.strip_prefix("wezterm.font") {
        let inner = inner.trim_start().trim_start_matches('(').trim_start();
        if let Some(name) = extract_quoted(inner) {
            return Some(name);
        }
        if let Some(idx) = inner.find("family") {
            let after = &inner[idx + "family".len()..];
            let after = after.trim_start().trim_start_matches('=').trim_start();
            if let Some(name) = extract_quoted(after) {
                return Some(name);
            }
        }
    }
    None
}

fn match_font_size(line: &str) -> Option<serde_json::Number> {
    let t = line.trim().trim_end_matches(',');
    let rest = t.strip_prefix("config.font_size")?;
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    let num: f64 = rest.parse().ok()?;
    serde_json::Number::from_f64(num)
}

fn match_color_scheme(line: &str) -> Option<String> {
    let t = line.trim().trim_end_matches(',');
    let rest = t.strip_prefix("config.color_scheme")?;
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    extract_quoted(rest)
}

fn extract_quoted(s: &str) -> Option<String> {
    let s = s.trim_start();
    let quote = s.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let rest = &s[1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

pub fn read_manifest(path: &Path) -> Result<InstallManifest> {
    let bytes = fs::read(path).map_err(|e| InstallError::io(path, e))?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|e| InstallError::io(path, e))?;
    Ok(blake3_hex(&bytes))
}

pub fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn timestamp_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos}")
}

/// Assess install currency for doctor.
pub fn install_currency(
    paths: &Paths,
    checkout: Option<&Path>,
) -> crate::doctor::InstallCurrency {
    use crate::doctor::InstallCurrency;
    let state_path = paths.state_file();
    let state = wzt_model::read_document::<State>(&state_path).ok();
    let mode = state.as_ref().and_then(|s| s.install_mode);
    if mode.is_none() {
        return InstallCurrency::NotInstalled;
    }
    let manifest_path = InstallManifest::path(paths);
    let Ok(manifest) = read_manifest(&manifest_path) else {
        return InstallCurrency::Unknown {
            reason: "install mode set but install manifest is missing".into(),
        };
    };
    if !manifest.config_matches_disk() {
        return InstallCurrency::Outdated {
            reason: format!(
                "config at {} changed since install",
                manifest.config_path.display()
            ),
        };
    }
    if let Some(c) = checkout {
        let c = fs::canonicalize(c).unwrap_or_else(|_| c.to_path_buf());
        let recorded =
            fs::canonicalize(&manifest.checkout).unwrap_or_else(|_| manifest.checkout.clone());
        if c != recorded {
            return InstallCurrency::Outdated {
                reason: format!(
                    "install recorded checkout {} but doctor was given {}",
                    manifest.checkout.display(),
                    c.display()
                ),
            };
        }
    }
    InstallCurrency::Current
}
