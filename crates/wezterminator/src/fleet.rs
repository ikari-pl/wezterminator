//! `wezterminator fleet` — attach, pull, promote, push, export.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use wzt_model::Paths;
use wzt_ops::{
    Denylist, SystemRunner, attach, export_bundle, promote, pull, push_fleet,
};

#[derive(Debug, Args)]
pub struct FleetArgs {
    #[command(subcommand)]
    pub command: FleetCommand,
}

#[derive(Debug, Subcommand)]
pub enum FleetCommand {
    /// Clone a private fleet repo into the fleet layer directory.
    Attach {
        /// Git URL (or local path) of the fleet repository.
        url: String,
    },
    /// Fast-forward pull; refuses dirty or diverged trees.
    Pull,
    /// Copy a local preset into the fleet clone and commit (does not delete local).
    Promote {
        /// Local preset id or slug (`cool-pills` or `local:cool-pills`).
        preset: String,
    },
    /// `git push` the fleet clone to its remote.
    Push,
    /// Write a public PR bundle (preset + theme); never includes machine settings.
    Export {
        /// Preset id or slug to export.
        preset: String,
        /// Output directory for the bundle.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// Optional checkout holding built-in themes/presets.
        #[arg(long, value_name = "DIR")]
        checkout: Option<PathBuf>,
    },
}

/// Run `wezterminator fleet …`.
pub fn run(paths: &Paths, args: &FleetArgs) -> ExitCode {
    let runner = SystemRunner;
    match &args.command {
        FleetCommand::Attach { url } => match attach(paths, url, &runner) {
            Ok(report) => {
                println!(
                    "attached fleet from {} into {}",
                    report.url,
                    report.fleet_dir.display()
                );
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("wezterminator fleet attach: {e}");
                ExitCode::FAILURE
            }
        },
        FleetCommand::Pull => match pull(paths, &runner) {
            Ok(report) => {
                println!("pulled fleet at {}", report.fleet_dir.display());
                if report.touched_state {
                    println!("touched engine state for reload");
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("wezterminator fleet pull: {e}");
                ExitCode::FAILURE
            }
        },
        FleetCommand::Promote { preset } => match promote(paths, preset, &runner) {
            Ok(report) => {
                println!(
                    "promoted {} → {} (commit {})",
                    report.local_path.display(),
                    report.fleet_path.display(),
                    report.commit
                );
                println!("local copy kept at {}", report.local_path.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("wezterminator fleet promote: {e}");
                ExitCode::FAILURE
            }
        },
        FleetCommand::Push => match push_fleet(paths, &runner) {
            Ok(report) => {
                println!("pushed fleet at {}", report.fleet_dir.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("wezterminator fleet push: {e}");
                ExitCode::FAILURE
            }
        },
        FleetCommand::Export {
            preset,
            out,
            checkout,
        } => {
            let hostname = hostname_string();
            let denylist = Denylist::from_hostname(hostname);
            let checkout = checkout.clone().or_else(default_checkout);
            match export_bundle(paths, preset, out, &denylist, checkout.as_deref()) {
                Ok(report) => {
                    println!("exported {} to {}", report.preset_id, report.out_dir.display());
                    if let Some(theme) = &report.theme_path {
                        println!("theme: {}", theme.display());
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("wezterminator fleet export: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

fn hostname_string() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

fn default_checkout() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    if cwd.join("presets").is_dir() && cwd.join("themes").is_dir() {
        Some(cwd)
    } else {
        None
    }
}
