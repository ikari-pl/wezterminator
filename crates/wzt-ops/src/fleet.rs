//! Fleet layer: attach a private git clone, pull updates, promote local
//! presets, and push the fleet remote.
//!
//! All git work shells out with `GIT_TERMINAL_PROMPT=0` so a missing credential
//! helper never hangs an interactive agent. Pull is `--ff-only` and refuses a
//! dirty or diverged tree. Promote copies (never moves) into the fleet clone
//! and commits locally; pushing the remote is a separate step.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use thiserror::Error;
use wzt_model::paths::{PRESETS_DIR, THEME_FILE, THEMES_DIR};
use wzt_model::{Paths, Preset, Theme, read_document, write_document};

/// How an external program ended when started through a [`CommandRunner`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.code == 0
    }
}

/// Injectable process runner. Production uses [`SystemRunner`]; tests supply
/// fakes for ssh/rsync and may still call real `git` for AE2.
pub trait CommandRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
        env: &[(&str, &str)],
    ) -> std::result::Result<CommandOutput, String>;
}

/// Shells out with no shell, capturing stdout and stderr.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
        env: &[(&str, &str)],
    ) -> std::result::Result<CommandOutput, String> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        let output = cmd
            .output()
            .map_err(|e| format!("failed to run `{program}`: {e}"))?;
        Ok(CommandOutput {
            code: output.status.code().unwrap_or(1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[derive(Debug, Error)]
pub enum FleetError {
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

pub type Result<T> = std::result::Result<T, FleetError>;

impl FleetError {
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

const GIT_ENV: &[(&str, &str)] = &[("GIT_TERMINAL_PROMPT", "0")];

fn git(
    runner: &dyn CommandRunner,
    cwd: Option<&Path>,
    args: &[&str],
) -> Result<CommandOutput> {
    runner
        .run("git", args, cwd, GIT_ENV)
        .map_err(FleetError::msg)
}

fn git_ok(runner: &dyn CommandRunner, cwd: Option<&Path>, args: &[&str]) -> Result<CommandOutput> {
    let out = git(runner, cwd, args)?;
    if out.success() {
        Ok(out)
    } else {
        let detail = first_nonempty([&out.stderr, &out.stdout]);
        Err(FleetError::msg(format!(
            "git {} failed: {detail}",
            args.first().copied().unwrap_or("?")
        )))
    }
}

fn first_nonempty(parts: [&str; 2]) -> &str {
    for p in parts {
        let t = p.trim();
        if !t.is_empty() {
            return t;
        }
    }
    "(no output)"
}

/// Report from [`attach`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachReport {
    pub fleet_dir: PathBuf,
    pub url: String,
}

/// Clone `url` into the fleet layer directory.
pub fn attach(
    paths: &Paths,
    url: &str,
    runner: &dyn CommandRunner,
) -> Result<AttachReport> {
    let fleet = paths.fleet_layer_dir();
    if fleet.exists() {
        let nonempty = fs::read_dir(&fleet)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
        if nonempty {
            return Err(FleetError::msg(format!(
                "fleet directory already exists and is not empty: {}",
                fleet.display()
            )));
        }
        // Empty dir: remove so clone can create it.
        fs::remove_dir(&fleet).map_err(|e| FleetError::io(&fleet, e))?;
    }
    if let Some(parent) = fleet.parent() {
        fs::create_dir_all(parent).map_err(|e| FleetError::io(parent, e))?;
    }
    let fleet_s = fleet.to_string_lossy();
    git_ok(runner, None, &["clone", url, fleet_s.as_ref()])?;
    Ok(AttachReport {
        fleet_dir: fleet,
        url: url.to_string(),
    })
}

/// Report from [`pull`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullReport {
    pub fleet_dir: PathBuf,
    pub touched_state: bool,
}

/// Fast-forward pull. Refuses a dirty working tree or diverged history.
/// On success, touches `state.json` so WezTerm reloads watched config.
pub fn pull(paths: &Paths, runner: &dyn CommandRunner) -> Result<PullReport> {
    let fleet = paths.fleet_layer_dir();
    ensure_fleet_repo(&fleet)?;

    let status = git_ok(runner, Some(&fleet), &["status", "--porcelain"])?;
    if !status.stdout.trim().is_empty() {
        return Err(FleetError::msg(format!(
            "fleet working tree is dirty; commit or stash first, then retry.\n\
             Resolve with: cd {} && git status",
            fleet.display()
        )));
    }

    // Fetch first so we can detect divergence without mutating on failure of pull.
    git_ok(runner, Some(&fleet), &["fetch", "--quiet"])?;

    let diverged = git(
        runner,
        Some(&fleet),
        &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
    )?;
    if diverged.success() {
        let counts = parse_left_right(diverged.stdout.trim());
        if let Some((ahead, behind)) = counts
            && ahead > 0
            && behind > 0
        {
            return Err(FleetError::msg(format!(
                "fleet has diverged from upstream (ahead {ahead}, behind {behind}); \
                 refusing to pull. Resolve with: cd {} && git status",
                fleet.display()
            )));
        }
    }

    let pull_out = git(runner, Some(&fleet), &["pull", "--ff-only"])?;
    if !pull_out.success() {
        let detail = first_nonempty([&pull_out.stderr, &pull_out.stdout]);
        return Err(FleetError::msg(format!(
            "fleet pull --ff-only refused: {detail}\n\
             Resolve with: cd {} && git status",
            fleet.display()
        )));
    }

    let touched_state = touch_state(paths)?;
    Ok(PullReport {
        fleet_dir: fleet,
        touched_state,
    })
}

fn parse_left_right(s: &str) -> Option<(u64, u64)> {
    let mut parts = s.split_whitespace();
    let left = parts.next()?.parse().ok()?;
    let right = parts.next()?.parse().ok()?;
    Some((left, right))
}

fn ensure_fleet_repo(fleet: &Path) -> Result<()> {
    if !fleet.join(".git").exists() {
        return Err(FleetError::msg(format!(
            "no fleet git repo at {}; run `wezterminator fleet attach <url>` first",
            fleet.display()
        )));
    }
    Ok(())
}

/// Bump `state.json` mtime when it exists so WezTerm's reload watch fires.
fn touch_state(paths: &Paths) -> Result<bool> {
    let state = paths.state_file();
    if !state.is_file() {
        return Ok(false);
    }
    let file = fs::File::options()
        .write(true)
        .open(&state)
        .map_err(|e| FleetError::io(&state, e))?;
    file.set_modified(SystemTime::now())
        .map_err(|e| FleetError::io(&state, e))?;
    Ok(true)
}

/// Report from [`promote`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromoteReport {
    pub local_path: PathBuf,
    pub fleet_path: PathBuf,
    pub fleet_id: String,
    pub commit: String,
}

/// Copy a local preset into the fleet clone (does not delete local) and commit.
///
/// When the preset references `local:…` themes, those theme directories are
/// copied into the fleet layer and the preset's theme ids are rewritten to
/// `fleet:…`. References to missing local themes refuse the promote.
pub fn promote(
    paths: &Paths,
    preset_ref: &str,
    runner: &dyn CommandRunner,
) -> Result<PromoteReport> {
    let fleet = paths.fleet_layer_dir();
    ensure_fleet_repo(&fleet)?;

    let slug = strip_layer_prefix(preset_ref);
    let local_path = paths
        .local_layer_dir()
        .join(PRESETS_DIR)
        .join(format!("{slug}.json"));
    if !local_path.is_file() {
        return Err(FleetError::msg(format!(
            "local preset not found: {}",
            local_path.display()
        )));
    }

    let mut preset: Preset = read_document(&local_path)?;
    let fleet_id = format!("fleet:{slug}");
    preset.id = fleet_id.clone();

    let mut git_paths = Vec::new();
    promote_local_themes(paths, &fleet, &mut preset, &mut git_paths)?;

    let fleet_presets = fleet.join(PRESETS_DIR);
    fs::create_dir_all(&fleet_presets).map_err(|e| FleetError::io(&fleet_presets, e))?;
    let fleet_path = fleet_presets.join(format!("{slug}.json"));
    write_document(&fleet_path, &preset)?;
    git_paths.push(format!("{PRESETS_DIR}/{slug}.json"));

    for rel in &git_paths {
        git_ok(runner, Some(&fleet), &["add", "--", rel])?;
    }
    let msg = format!("promote {fleet_id}");
    git_ok(runner, Some(&fleet), &["commit", "-m", &msg])?;
    let head = git_ok(runner, Some(&fleet), &["rev-parse", "--short", "HEAD"])?;

    Ok(PromoteReport {
        local_path,
        fleet_path,
        fleet_id,
        commit: head.stdout.trim().to_string(),
    })
}

fn promote_local_themes(
    paths: &Paths,
    fleet: &Path,
    preset: &mut Preset,
    git_paths: &mut Vec<String>,
) -> Result<()> {
    let refs = collect_theme_refs(preset);
    for theme_ref in refs {
        let Some(local_slug) = theme_ref.strip_prefix("local:") else {
            continue;
        };
        let src = paths
            .local_layer_dir()
            .join(THEMES_DIR)
            .join(local_slug)
            .join(THEME_FILE);
        if !src.is_file() {
            return Err(FleetError::msg(format!(
                "preset references local theme `{theme_ref}` but {} is missing; \
                 create the theme or point the preset at a fleet/builtin theme before promoting",
                src.display()
            )));
        }
        let mut theme: Theme = read_document(&src)?;
        let fleet_theme_id = format!("fleet:{local_slug}");
        theme.id = fleet_theme_id.clone();

        let dest_dir = fleet.join(THEMES_DIR).join(local_slug);
        fs::create_dir_all(&dest_dir).map_err(|e| FleetError::io(&dest_dir, e))?;
        let dest = dest_dir.join(THEME_FILE);
        write_document(&dest, &theme)?;
        git_paths.push(format!("{THEMES_DIR}/{local_slug}/{THEME_FILE}"));
        rewrite_theme_refs(preset, &theme_ref, &fleet_theme_id);
    }
    Ok(())
}

fn collect_theme_refs(preset: &Preset) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(v) = serde_json::to_value(&preset.parts) {
        collect_theme_strings(&v, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn collect_theme_strings(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(s)) = map.get("theme") {
                out.push(s.clone());
            }
            for val in map.values() {
                collect_theme_strings(val, out);
            }
        }
        serde_json::Value::Array(arr) => {
            for val in arr {
                collect_theme_strings(val, out);
            }
        }
        _ => {}
    }
}

fn rewrite_theme_refs(preset: &mut Preset, from: &str, to: &str) {
    if let Ok(mut v) = serde_json::to_value(&preset.parts) {
        rewrite_theme_string_value(&mut v, from, to);
        if let Ok(parts) = serde_json::from_value(v) {
            preset.parts = parts;
        }
    }
}

fn rewrite_theme_string_value(v: &mut serde_json::Value, from: &str, to: &str) {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(s)) = map.get_mut("theme")
                && s == from
            {
                *s = to.to_string();
            }
            for val in map.values_mut() {
                rewrite_theme_string_value(val, from, to);
            }
        }
        serde_json::Value::Array(arr) => {
            for val in arr {
                rewrite_theme_string_value(val, from, to);
            }
        }
        _ => {}
    }
}

/// Strip a `local:` / `fleet:` / `builtin:` prefix when present.
pub fn strip_layer_prefix(preset_ref: &str) -> &str {
    preset_ref
        .strip_prefix("local:")
        .or_else(|| preset_ref.strip_prefix("fleet:"))
        .or_else(|| preset_ref.strip_prefix("builtin:"))
        .unwrap_or(preset_ref)
}

/// Report from [`push_fleet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushFleetReport {
    pub fleet_dir: PathBuf,
}

/// Push the fleet clone to its configured remote (`git push`).
pub fn push_fleet(paths: &Paths, runner: &dyn CommandRunner) -> Result<PushFleetReport> {
    let fleet = paths.fleet_layer_dir();
    ensure_fleet_repo(&fleet)?;
    git_ok(runner, Some(&fleet), &["push"])?;
    Ok(PushFleetReport { fleet_dir: fleet })
}

/// Path of a fleet preset file for `slug`.
pub fn fleet_preset_path(paths: &Paths, slug: &str) -> PathBuf {
    paths
        .fleet_layer_dir()
        .join(PRESETS_DIR)
        .join(format!("{slug}.json"))
}

/// Path of a local preset file for `slug`.
pub fn local_preset_path(paths: &Paths, slug: &str) -> PathBuf {
    paths
        .local_layer_dir()
        .join(PRESETS_DIR)
        .join(format!("{slug}.json"))
}

#[cfg(test)]
mod touch_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn touch_state_returns_false_when_absent() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::from_roots(
            tmp.path().join("c"),
            tmp.path().join("d"),
            tmp.path().join("s"),
        );
        assert!(!touch_state(&paths).unwrap());
    }
}
