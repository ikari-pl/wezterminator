//! `wezterminator doctor`: structured health findings.
//!
//! Every finding has a stable [`FindingCode`] so tests and tooling can match
//! without scraping prose. The runner accepts a fully injectable
//! [`DoctorInput`] so unit tests never need real fonts, screens or WezTerm.

use std::fmt;
use std::path::{Path, PathBuf};

use wzt_art::{MANIFEST_FILE, Manifest, recipe_hash};
use wzt_fonts::{
    CoverageSets, FontCatalog, NERD_FONT_SAMPLE, POLISH_DIACRITICS, check_coverage_sets,
};
use wzt_model::{
    EngineInfo, Paths, Preset, Screens, Theme, ThemeArt, VpnKind, VpnProbe,
    SUPPORTED_SCHEMA_VERSION, paths::shipped_art_dir, read_document,
};

/// Soft cap on WezTerm background layers per theme (GPU / reload cost).
pub const MAX_THEME_LAYERS: usize = 8;

/// Stable finding identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FindingCode {
    /// Preferred font missing for a preset (AE6 / R7 / R35).
    MissingPreferredFont,
    /// Font lacks Polish diacritics or Nerd Font sample glyphs (R20 / R35).
    FailedCoverage,
    /// No art at the recorded screen resolution (AE8 / R35).
    ArtMissingAtResolution,
    /// Screens document missing, empty or unusable.
    UnknownScreenResolution,
    /// Theme art recipe exceeds [`MAX_THEME_LAYERS`].
    ThemeLayerBudgetExceeded,
    /// User art manifest recipe hash no longer matches the theme.
    StaleUserArt,
    /// A configured VPN / status probe binary is missing.
    UnavailableProbe,
    /// Chrome blur (or similar) is requested but unsupported here.
    UnavailableChromeOption,
    /// Binary schema support differs from the recorded Lua engine.
    SchemaVersionMismatch,
    /// Binary version differs from the recorded Lua engine version.
    EngineVersionMismatch,
    /// Install is missing, unknown, or not current.
    InstallNotCurrent,
}

impl FindingCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingPreferredFont => "missing_preferred_font",
            Self::FailedCoverage => "failed_coverage",
            Self::ArtMissingAtResolution => "art_missing_at_resolution",
            Self::UnknownScreenResolution => "unknown_screen_resolution",
            Self::ThemeLayerBudgetExceeded => "theme_layer_budget_exceeded",
            Self::StaleUserArt => "stale_user_art",
            Self::UnavailableProbe => "unavailable_probe",
            Self::UnavailableChromeOption => "unavailable_chrome_option",
            Self::SchemaVersionMismatch => "schema_version_mismatch",
            Self::EngineVersionMismatch => "engine_version_mismatch",
            Self::InstallNotCurrent => "install_not_current",
        }
    }
}

impl fmt::Display for FindingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One doctor finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub code: FindingCode,
    pub message: String,
}

impl Finding {
    pub fn new(code: FindingCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Collected doctor output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DoctorReport {
    pub findings: Vec<Finding>,
}

impl DoctorReport {
    pub fn is_ok(&self) -> bool {
        self.findings.is_empty()
    }

    pub fn codes(&self) -> Vec<FindingCode> {
        self.findings.iter().map(|f| f.code).collect()
    }

    /// Human-readable summary lines (`code: message`).
    pub fn render(&self) -> String {
        if self.findings.is_empty() {
            return "ok: no findings\n".to_string();
        }
        let mut out = String::new();
        for f in &self.findings {
            out.push_str(f.code.as_str());
            out.push_str(": ");
            out.push_str(&f.message);
            out.push('\n');
        }
        out
    }
}

/// Font lists from one preset, already extracted for injection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetFonts {
    pub id: String,
    pub name: String,
    pub preferred: Vec<String>,
    pub fallback: Vec<String>,
    /// Theme slug used for art (without `builtin:`), when known.
    pub art_theme_slug: Option<String>,
    /// Whether chrome.blur is set and non-zero.
    pub wants_blur: bool,
}

/// Theme layer count for the budget check.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeLayers {
    pub id: String,
    pub slug: String,
    pub layer_count: usize,
    pub art: ThemeArt,
}

/// Where art was found for a theme × resolution, if anywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtPresence {
    /// No user or shipped art directory with a manifest.
    Missing,
    /// User art present; `recipe_ok` is whether the manifest hash matches.
    User { dir: PathBuf, recipe_ok: bool },
    /// Only shipped art is present.
    Shipped { dir: PathBuf },
}

/// Whether a probe's backing command is available on PATH / as configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeAvailability {
    pub id: String,
    pub kind: VpnKind,
    pub available: bool,
    pub detail: String,
}

/// Install currency relative to this binary / checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallCurrency {
    /// No install recorded yet (typical before U15 / first WezTerm load).
    NotInstalled,
    /// Cannot tell (no manifest / no checkout remote).
    Unknown { reason: String },
    Current,
    Outdated { reason: String },
}

/// Everything doctor needs. All fields are injectable for tests.
#[derive(Debug, Clone)]
pub struct DoctorInput {
    pub presets: Vec<PresetFonts>,
    pub themes: Vec<ThemeLayers>,
    /// Recorded screens, or `None` when the file is absent.
    pub screens: Option<Screens>,
    pub catalog: FontCatalog,
    /// Optional per-family coverage (family → sets). Missing entries are skipped
    /// unless [`Self::check_coverage_for`] is set to load bytes.
    pub coverage: Vec<(String, CoverageSets)>,
    /// Resolve art at `(theme_slug, width, height)`.
    pub art: Vec<ArtAt>,
    pub engine: Option<EngineInfo>,
    pub binary_version: String,
    pub binary_schema: u64,
    pub probes: Vec<ProbeAvailability>,
    /// `Some(false)` when blur is requested but unsupported.
    pub chrome_blur_supported: bool,
    pub install: InstallCurrency,
}

/// One art lookup result already resolved by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtAt {
    pub theme_slug: String,
    pub width: u64,
    pub height: u64,
    pub presence: ArtPresence,
}

impl Default for DoctorInput {
    fn default() -> Self {
        Self {
            presets: Vec::new(),
            themes: Vec::new(),
            screens: None,
            catalog: FontCatalog::default(),
            coverage: Vec::new(),
            art: Vec::new(),
            engine: None,
            binary_version: env!("CARGO_PKG_VERSION").to_string(),
            binary_schema: SUPPORTED_SCHEMA_VERSION,
            probes: Vec::new(),
            chrome_blur_supported: true,
            install: InstallCurrency::Unknown {
                reason: "no install manifest".into(),
            },
        }
    }
}

/// Run all doctor checks against `input`.
pub fn run(input: &DoctorInput) -> DoctorReport {
    let mut findings = Vec::new();

    check_fonts(input, &mut findings);
    check_coverage(input, &mut findings);
    check_screens_and_art(input, &mut findings);
    check_layer_budget(input, &mut findings);
    check_probes(input, &mut findings);
    check_chrome(input, &mut findings);
    check_engine_versions(input, &mut findings);
    check_install(input, &mut findings);

    DoctorReport { findings }
}

fn check_fonts(input: &DoctorInput, findings: &mut Vec<Finding>) {
    for preset in &input.presets {
        let mut missing = Vec::new();
        let mut present_preferred = Vec::new();
        for family in &preset.preferred {
            if input.catalog.contains(family) {
                present_preferred.push(family.clone());
            } else {
                missing.push(family.clone());
            }
        }
        if missing.is_empty() {
            continue;
        }
        // Effective list: installed preferred, then declared fallback (AE6).
        let mut effective = present_preferred;
        for family in &preset.fallback {
            if !effective.iter().any(|e| e.eq_ignore_ascii_case(family)) {
                effective.push(family.clone());
            }
        }
        let fallback_note = if preset.fallback.is_empty() {
            "no fallback list declared".to_string()
        } else {
            format!("fallback in use: {}", preset.fallback.join(", "))
        };
        findings.push(Finding::new(
            FindingCode::MissingPreferredFont,
            format!(
                "preset `{}` ({}) missing preferred font{} {}; {}",
                preset.name,
                preset.id,
                if missing.len() == 1 { "" } else { "s" },
                missing
                    .iter()
                    .map(|m| format!("`{m}`"))
                    .collect::<Vec<_>>()
                    .join(", "),
                fallback_note
            ),
        ));
        let _ = effective; // reserved for richer output / TUI later
    }
}

fn check_coverage(input: &DoctorInput, findings: &mut Vec<Finding>) {
    for (family, sets) in &input.coverage {
        if !sets.polish.is_complete() {
            findings.push(Finding::new(
                FindingCode::FailedCoverage,
                format!(
                    "font `{family}` missing Polish diacritics: {}",
                    chars_list(&sets.polish.missing)
                ),
            ));
        }
        if !sets.nerd.is_complete() {
            findings.push(Finding::new(
                FindingCode::FailedCoverage,
                format!(
                    "font `{family}` missing Nerd Font sample glyphs: {}",
                    chars_list(&sets.nerd.missing)
                ),
            ));
        }
    }
}

fn chars_list(chars: &[char]) -> String {
    chars.iter().map(|c| c.to_string()).collect::<String>()
}

fn check_screens_and_art(input: &DoctorInput, findings: &mut Vec<Finding>) {
    let Some(screens) = &input.screens else {
        findings.push(Finding::new(
            FindingCode::UnknownScreenResolution,
            "no screens.json recorded yet; open WezTerm once with the engine loaded",
        ));
        return;
    };
    if screens.screens.is_empty() {
        findings.push(Finding::new(
            FindingCode::UnknownScreenResolution,
            "screens.json lists no screens",
        ));
        return;
    }
    let Some((width, height)) = largest_resolution(screens) else {
        findings.push(Finding::new(
            FindingCode::UnknownScreenResolution,
            "screens.json has no usable width×height",
        ));
        return;
    };

    // Themes referenced by presets, else every known theme.
    let mut slugs: Vec<String> = input
        .presets
        .iter()
        .filter_map(|p| p.art_theme_slug.clone())
        .collect();
    if slugs.is_empty() {
        slugs = input.themes.iter().map(|t| t.slug.clone()).collect();
    }
    slugs.sort();
    slugs.dedup();

    for slug in slugs {
        let presence = input
            .art
            .iter()
            .find(|a| a.theme_slug == slug && a.width == width && a.height == height)
            .map(|a| &a.presence);
        match presence {
            None | Some(ArtPresence::Missing) => {
                findings.push(Finding::new(
                    FindingCode::ArtMissingAtResolution,
                    format!(
                        "theme `{slug}` has no art at recorded resolution {width}x{height}"
                    ),
                ));
            }
            Some(ArtPresence::User { dir, recipe_ok }) => {
                if !recipe_ok {
                    findings.push(Finding::new(
                        FindingCode::StaleUserArt,
                        format!(
                            "user art for `{slug}` at {width}x{height} is stale (recipe hash mismatch) in {}",
                            dir.display()
                        ),
                    ));
                }
            }
            Some(ArtPresence::Shipped { .. }) => {}
        }
    }
}

fn largest_resolution(screens: &Screens) -> Option<(u64, u64)> {
    let mut best: Option<(u64, u64, u64)> = None;
    for s in &screens.screens {
        if s.width == 0 || s.height == 0 {
            continue;
        }
        let area = s.width.saturating_mul(s.height);
        if best.is_none_or(|(a, _, _)| area > a) {
            best = Some((area, s.width, s.height));
        }
    }
    best.map(|(_, w, h)| (w, h))
}

fn check_layer_budget(input: &DoctorInput, findings: &mut Vec<Finding>) {
    for theme in &input.themes {
        if theme.layer_count > MAX_THEME_LAYERS {
            findings.push(Finding::new(
                FindingCode::ThemeLayerBudgetExceeded,
                format!(
                    "theme `{}` has {} layers (budget is {MAX_THEME_LAYERS})",
                    theme.id, theme.layer_count
                ),
            ));
        }
    }
}

fn check_probes(input: &DoctorInput, findings: &mut Vec<Finding>) {
    for probe in &input.probes {
        if !probe.available {
            findings.push(Finding::new(
                FindingCode::UnavailableProbe,
                format!(
                    "probe `{}` ({:?}) unavailable: {}",
                    probe.id, probe.kind, probe.detail
                ),
            ));
        }
    }
}

fn check_chrome(input: &DoctorInput, findings: &mut Vec<Finding>) {
    if input.chrome_blur_supported {
        return;
    }
    for preset in &input.presets {
        if preset.wants_blur {
            findings.push(Finding::new(
                FindingCode::UnavailableChromeOption,
                format!(
                    "preset `{}` requests chrome.blur but blur is unavailable on this platform",
                    preset.name
                ),
            ));
        }
    }
}

fn check_engine_versions(input: &DoctorInput, findings: &mut Vec<Finding>) {
    let Some(engine) = &input.engine else {
        return;
    };
    if engine.schema_version != input.binary_schema {
        findings.push(Finding::new(
            FindingCode::SchemaVersionMismatch,
            format!(
                "Lua engine schema_version is {} but this binary supports {}",
                engine.schema_version, input.binary_schema
            ),
        ));
    }
    if engine.version != input.binary_version {
        findings.push(Finding::new(
            FindingCode::EngineVersionMismatch,
            format!(
                "Lua engine version is `{}` but this binary is `{}`",
                engine.version, input.binary_version
            ),
        ));
    }
}

fn check_install(input: &DoctorInput, findings: &mut Vec<Finding>) {
    match &input.install {
        InstallCurrency::Current => {}
        InstallCurrency::NotInstalled => {
            findings.push(Finding::new(
                FindingCode::InstallNotCurrent,
                "wezterminator is not installed (no install mode in state.json)",
            ));
        }
        InstallCurrency::Unknown { reason } => {
            findings.push(Finding::new(
                FindingCode::InstallNotCurrent,
                format!("install currency unknown: {reason}"),
            ));
        }
        InstallCurrency::Outdated { reason } => {
            findings.push(Finding::new(
                FindingCode::InstallNotCurrent,
                format!("install is not current: {reason}"),
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Live helpers: build DoctorInput from Paths + checkout
// ---------------------------------------------------------------------------

/// Build a [`DoctorInput`] by reading the user's paths and an optional checkout.
pub fn collect(paths: &Paths, checkout: Option<&Path>) -> DoctorInput {
    let state = read_document::<wzt_model::State>(&paths.state_file()).ok();
    let engine = state.as_ref().and_then(|s| s.engine.clone());
    let screens = read_document::<Screens>(&paths.screens_file()).ok();

    let cwd_checkout = std::env::current_dir()
        .ok()
        .filter(|cwd| cwd.join("themes").is_dir());
    let checkout_owned = checkout.map(Path::to_path_buf).or(cwd_checkout);
    let builtin = wzt_model::paths::builtin_dir(
        checkout_owned.as_deref(),
        engine.as_ref().map(|e| e.plugin_dir.as_str()),
    );

    let layer_dirs: Vec<(String, PathBuf)> = [
        ("local".into(), paths.local_layer_dir().to_path_buf()),
        ("fleet".into(), paths.fleet_layer_dir()),
    ]
    .into_iter()
    .chain(builtin.map(|d| ("builtin".into(), d)))
    .collect();

    let (presets, themes) = load_presets_and_themes(&layer_dirs);
    let catalog = wzt_fonts::system_catalog();
    let coverage = coverage_for_preferred(&presets, &catalog);

    let resolution = screens.as_ref().and_then(largest_resolution);
    let art = match resolution {
        Some((w, h)) => resolve_art(paths, &layer_dirs, &themes, w, h),
        None => Vec::new(),
    };

    let machine = load_merged_machine(paths);
    let probes = probe_availability(machine.as_ref());
    let chrome_blur_supported = blur_supported();

    let install = match state.as_ref().and_then(|s| s.install_mode) {
        None => InstallCurrency::NotInstalled,
        Some(_) => InstallCurrency::Unknown {
            reason: "install manifest not available yet (U15)".into(),
        },
    };

    DoctorInput {
        presets,
        themes,
        screens,
        catalog,
        coverage,
        art,
        engine,
        binary_version: env!("CARGO_PKG_VERSION").to_string(),
        binary_schema: SUPPORTED_SCHEMA_VERSION,
        probes,
        chrome_blur_supported,
        install,
    }
}

fn load_presets_and_themes(layers: &[(String, PathBuf)]) -> (Vec<PresetFonts>, Vec<ThemeLayers>) {
    let mut presets = Vec::new();
    let mut themes = Vec::new();
    let mut seen_presets = std::collections::HashSet::new();
    let mut seen_themes = std::collections::HashSet::new();

    // Later layers win for identity; walk local → fleet → builtin so first wins
    // if we insert only once — actually we want all presets for doctor (missing
    // fonts per preset). Load every preset we can find, keyed by id.
    for (_layer, dir) in layers {
        let presets_dir = dir.join("presets");
        if let Ok(entries) = std::fs::read_dir(&presets_dir) {
            let mut files: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
            files.sort();
            for file in files {
                if file.extension().is_none_or(|e| e != "json") {
                    continue;
                }
                let Ok(preset) = read_document::<Preset>(&file) else {
                    continue;
                };
                if !seen_presets.insert(preset.id.clone()) {
                    continue;
                }
                presets.push(preset_fonts_from(&preset));
            }
        }

        let themes_dir = dir.join("themes");
        if let Ok(entries) = std::fs::read_dir(&themes_dir) {
            let mut dirs: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
            dirs.sort();
            for theme_dir in dirs {
                let file = theme_dir.join("theme.json");
                if !file.is_file() {
                    continue;
                }
                let Ok(theme) = read_document::<Theme>(&file) else {
                    continue;
                };
                if !seen_themes.insert(theme.id.clone()) {
                    continue;
                }
                let slug = theme_dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| theme.id.clone());
                themes.push(ThemeLayers {
                    id: theme.id.clone(),
                    slug,
                    layer_count: theme.art.layers.len(),
                    art: theme.art.clone(),
                });
            }
        }
    }
    (presets, themes)
}

fn preset_fonts_from(preset: &Preset) -> PresetFonts {
    let preferred = preset.parts.font.preferred.clone().unwrap_or_default();
    let fallback = preset.parts.font.fallback.clone().unwrap_or_default();
    let art_theme_slug = preset
        .parts
        .art
        .theme
        .as_ref()
        .map(|id| id.rsplit_once(':').map(|(_, s)| s).unwrap_or(id).to_string());
    let wants_blur = preset
        .parts
        .chrome
        .blur
        .is_some_and(|b| b > 0);
    PresetFonts {
        id: preset.id.clone(),
        name: preset.name.clone(),
        preferred,
        fallback,
        art_theme_slug,
        wants_blur,
    }
}

fn coverage_for_preferred(
    presets: &[PresetFonts],
    catalog: &FontCatalog,
) -> Vec<(String, CoverageSets)> {
    let mut families = Vec::new();
    for preset in presets {
        for family in preset.preferred.iter().chain(preset.fallback.iter()) {
            if catalog.contains(family) && !families.iter().any(|f: &String| f.eq_ignore_ascii_case(family))
            {
                families.push(family.clone());
            }
        }
    }
    let mut out = Vec::new();
    for family in families {
        if let Some(bytes) = wzt_fonts::discovery::load_family_bytes(&family)
            && let Ok(sets) = check_coverage_sets(&bytes)
        {
            out.push((family, sets));
        }
    }
    out
}

fn resolve_art(
    paths: &Paths,
    layers: &[(String, PathBuf)],
    themes: &[ThemeLayers],
    width: u64,
    height: u64,
) -> Vec<ArtAt> {
    let mut out = Vec::new();
    for theme in themes {
        let user_dir = paths.art_dir(&theme.slug, width, height);
        let presence = if user_dir.join(MANIFEST_FILE).is_file() {
            let recipe_ok = match read_document::<Manifest>(&user_dir.join(MANIFEST_FILE)) {
                Ok(manifest) => manifest.recipe_hash == recipe_hash(&theme.art),
                Err(_) => false,
            };
            ArtPresence::User {
                dir: user_dir,
                recipe_ok,
            }
        } else {
            let mut shipped = None;
            for (_layer, dir) in layers {
                let candidate = shipped_art_dir(dir, &theme.slug, width, height);
                if candidate.join(MANIFEST_FILE).is_file() {
                    shipped = Some(candidate);
                    break;
                }
            }
            match shipped {
                Some(dir) => ArtPresence::Shipped { dir },
                None => ArtPresence::Missing,
            }
        };
        out.push(ArtAt {
            theme_slug: theme.slug.clone(),
            width,
            height,
            presence,
        });
    }
    out
}

fn load_merged_machine(paths: &Paths) -> Option<wzt_model::Machine> {
    // Prefer local over fleet when both exist; doctor only needs probes.
    let local = paths.local_layer_dir().join("machine.json");
    if local.is_file() {
        return read_document(&local).ok();
    }
    let fleet = paths.fleet_layer_dir().join("machine.json");
    if fleet.is_file() {
        return read_document(&fleet).ok();
    }
    None
}

fn probe_availability(machine: Option<&wzt_model::Machine>) -> Vec<ProbeAvailability> {
    let Some(machine) = machine else {
        return Vec::new();
    };
    let Some(probes) = &machine.vpn_probes else {
        return Vec::new();
    };
    probes
        .iter()
        .filter(|p| p.enabled != Some(false))
        .map(probe_status)
        .collect()
}

fn probe_status(probe: &VpnProbe) -> ProbeAvailability {
    match probe.kind {
        VpnKind::Tailscale => {
            let ok = command_on_path("tailscale");
            ProbeAvailability {
                id: probe.id.clone(),
                kind: probe.kind,
                available: ok,
                detail: if ok {
                    "tailscale on PATH".into()
                } else {
                    "tailscale not found on PATH".into()
                },
            }
        }
        VpnKind::Warp => {
            let ok = command_on_path("warp-cli");
            ProbeAvailability {
                id: probe.id.clone(),
                kind: probe.kind,
                available: ok,
                detail: if ok {
                    "warp-cli on PATH".into()
                } else {
                    "warp-cli not found on PATH".into()
                },
            }
        }
        VpnKind::AwsVpn => ProbeAvailability {
            id: probe.id.clone(),
            kind: probe.kind,
            available: true,
            detail: "aws_vpn checked via stats (no binary required)".into(),
        },
        VpnKind::Interface => {
            let name = probe.interface.clone().unwrap_or_default();
            ProbeAvailability {
                id: probe.id.clone(),
                kind: probe.kind,
                available: !name.is_empty(),
                detail: if name.is_empty() {
                    "interface probe missing interface name".into()
                } else {
                    format!("interface `{name}`")
                },
            }
        }
        VpnKind::Command => {
            let cmd = probe
                .command
                .as_ref()
                .and_then(|c| c.first())
                .cloned()
                .unwrap_or_default();
            let ok = !cmd.is_empty() && (Path::new(&cmd).is_file() || command_on_path(&cmd));
            ProbeAvailability {
                id: probe.id.clone(),
                kind: probe.kind,
                available: ok,
                detail: if ok {
                    format!("command `{cmd}` available")
                } else if cmd.is_empty() {
                    "command probe has empty argv".into()
                } else {
                    format!("command `{cmd}` not found")
                },
            }
        }
    }
}

fn command_on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let candidate = dir.join(name);
                candidate.is_file()
            })
        })
        .unwrap_or(false)
}

fn blur_supported() -> bool {
    // macOS and Windows always expose a blur key the engine can set. Linux
    // needs KDE or a nightly Wayland blur API; without probing WezTerm itself
    // we treat non-macOS Unix as unsupported until the engine records capability.
    cfg!(target_os = "macos") || cfg!(target_os = "windows")
}

/// Coverage set constants re-exported for callers that want the same samples.
pub fn coverage_sample_note() -> String {
    format!(
        "polish={} nerd_sample_len={}",
        POLISH_DIACRITICS.chars().count(),
        NERD_FONT_SAMPLE.chars().count()
    )
}
