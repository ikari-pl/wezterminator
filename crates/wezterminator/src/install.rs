//! `wezterminator install` and `uninstall`.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, ValueEnum};
use wzt_model::{InstallMode, Paths};
use wzt_ops::{
    ConfigEnv, InstallOptions, install as ops_install, uninstall as ops_uninstall,
};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ModeArg {
    /// Keep the user's config; append a marked apply_to_config block.
    #[value(name = "add-on")]
    AddOn,
    /// Back up and replace the config with a checkout shim.
    Replace,
    /// Replace, and import literal font/colours/keys into a local preset.
    #[value(name = "replace-import")]
    ReplaceImport,
}

impl From<ModeArg> for InstallMode {
    fn from(value: ModeArg) -> Self {
        match value {
            ModeArg::AddOn => InstallMode::AddOn,
            ModeArg::Replace => InstallMode::Replace,
            ModeArg::ReplaceImport => InstallMode::ReplaceImport,
        }
    }
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Install mode.
    #[arg(long, value_enum)]
    pub mode: ModeArg,

    /// Directory of a wezterminator checkout holding `plugin/init.lua`.
    /// Defaults to the current directory when it looks like a checkout.
    #[arg(long, value_name = "DIR")]
    pub checkout: Option<PathBuf>,

    /// Plugin URL for add-on `wezterm.plugin.require`. When omitted, the
    /// checkout is loaded with `dofile`.
    #[arg(long, value_name = "URL")]
    pub plugin_url: Option<String>,

    /// Do not migrate `~/.wezterm-*` state files into the local layer.
    #[arg(long)]
    pub skip_migrate: bool,
}

#[derive(Debug, Args)]
pub struct UninstallArgs {}

/// Run `wezterminator install`.
pub fn run_install(paths: &Paths, args: &InstallArgs) -> ExitCode {
    let checkout = match args.checkout.clone().or_else(default_checkout) {
        Some(c) => c,
        None => {
            eprintln!(
                "wezterminator install: pass --checkout DIR (no wezterminator checkout found)"
            );
            return ExitCode::FAILURE;
        }
    };
    let config_env = match ConfigEnv::from_process() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("wezterminator install: {error}");
            return ExitCode::FAILURE;
        }
    };
    let opts = InstallOptions {
        mode: args.mode.into(),
        paths: paths.clone(),
        config_env,
        checkout,
        plugin_url: args.plugin_url.clone(),
        skip_migrate: args.skip_migrate,
    };
    match ops_install(&opts) {
        Ok(report) => {
            if report.idempotent {
                println!(
                    "already installed ({:?}) at {}",
                    report.mode,
                    report.config_path.display()
                );
            } else {
                println!(
                    "installed ({:?}) into {}",
                    report.mode,
                    report.config_path.display()
                );
                println!("manifest: {}", report.manifest_path.display());
                println!("backup:   {}", report.backup_dir.display());
                if let Some(p) = &report.imported_preset {
                    println!("imported: {}", p.display());
                }
            }
            for w in &report.warnings {
                eprintln!("warning: {w}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("wezterminator install: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Run `wezterminator uninstall`.
pub fn run_uninstall(paths: &Paths, _args: &UninstallArgs) -> ExitCode {
    match ops_uninstall(paths) {
        Ok(report) => {
            println!("uninstalled; restored {}", report.config_path.display());
            for w in &report.warnings {
                eprintln!("warning: {w}");
            }
            for kept in &report.kept_edited {
                eprintln!("kept edited copy: {}", kept.display());
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("wezterminator uninstall: {error}");
            ExitCode::FAILURE
        }
    }
}

fn default_checkout() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    if cwd.join("plugin").join("init.lua").is_file()
        && cwd.join("presets").is_dir()
        && cwd.join("themes").is_dir()
    {
        Some(cwd)
    } else {
        None
    }
}
