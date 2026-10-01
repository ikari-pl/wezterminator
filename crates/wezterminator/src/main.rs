//! The `wezterminator` binary.
//!
//! Subcommands land unit by unit. `stats` (U6), `art generate|check` (U7),
//! `doctor` (U11), `tui` (U12), `install`/`uninstall` (U15), `fleet`/`push`
//! (U16) are real. Remaining art subcommands stay stubbed.

use std::path::PathBuf;
use std::process::ExitCode;

mod art;
mod doctor;
mod fleet;
mod install;
mod push;

use clap::{Args, Parser, Subcommand};
use wzt_model::Paths;
use wzt_tui::{PreviewMode, RunOptions, run as run_tui};

#[derive(Debug, Parser)]
#[command(
    name = "wezterminator",
    version,
    about = "Composable WezTerm presets: TUI, art engine, installer and fleet tooling",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Trailing-arg sink for subcommands still stubbed (art import/pack).
#[derive(Debug, Args)]
pub(crate) struct Pending {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    pub(crate) args: Vec<String>,
}

#[derive(Debug, Args)]
struct StatsArgs {
    /// Print the line to stdout instead of writing the status cache.
    #[arg(long)]
    stdout: bool,
}

#[derive(Debug, Args)]
struct TuiArgs {
    /// Directory of a wezterminator checkout holding built-in presets/themes.
    /// Defaults to the current directory when it has a `presets/` folder.
    #[arg(long, value_name = "DIR")]
    checkout: Option<PathBuf>,
    /// Force browser-mode preview (skip WezTerm OSC handshake).
    #[arg(long)]
    browser: bool,
    /// Force WezTerm OSC mode without waiting for an ack (for scripting/tests).
    #[arg(long)]
    wezterm: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Browse presets and edit theme parts, previewing live in WezTerm.
    Tui(TuiArgs),
    /// Generate, import and pack theme art.
    Art(art::ArtArgs),
    /// Write one key=value line of system stats to the status cache.
    Stats(StatsArgs),
    /// Check fonts, art, screens and the install, and say what is wrong.
    Doctor(doctor::DoctorArgs),
    /// Install wezterminator in add-on, replace or replace-and-import mode.
    Install(install::InstallArgs),
    /// Undo an install from its manifest.
    Uninstall(install::UninstallArgs),
    /// Attach, pull and promote presets for a private fleet repo.
    Fleet(fleet::FleetArgs),
    /// Push the fleet layer to another machine over SSH.
    Push(push::PushArgs),
}

impl Command {
    /// The subcommand name and the unit that implements it, or `None` once
    /// the subcommand is real.
    fn stub(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Command::Tui(_) => None,
            Command::Art(args) => args.command.stub(),
            Command::Stats(_) => None,
            Command::Doctor(_) => None,
            Command::Install(_) => None,
            Command::Uninstall(_) => None,
            Command::Fleet(_) => None,
            Command::Push(_) => None,
        }
    }
}

/// `wezterminator stats`: sample this machine and write the status cache.
fn stats(args: &StatsArgs) -> ExitCode {
    let paths = match Paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("wezterminator stats: {error}");
            return ExitCode::FAILURE;
        }
    };

    // Skipped settings are worth a note, but never stop the line from being written.
    let report = if args.stdout {
        Ok(wzt_stats::sample(&paths))
    } else {
        wzt_stats::run(&paths)
    };
    match report {
        Ok(report) => {
            for warning in &report.warnings {
                eprintln!("wezterminator stats: {warning}");
            }
            if args.stdout {
                print!("{}", report.sample.render());
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("wezterminator stats: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `wezterminator art generate|check`.
fn art_command(args: &art::ArtArgs) -> ExitCode {
    let paths = match Paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("wezterminator art: {error}");
            return ExitCode::FAILURE;
        }
    };
    match art::run(&paths, args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("wezterminator art: {message}");
            ExitCode::FAILURE
        }
    }
}

/// `wezterminator doctor`.
fn doctor_command(args: &doctor::DoctorArgs) -> ExitCode {
    let paths = match Paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("wezterminator doctor: {error}");
            return ExitCode::FAILURE;
        }
    };
    doctor::run(&paths, args)
}

/// `wezterminator tui`.
fn tui_command(args: &TuiArgs) -> ExitCode {
    let paths = match Paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("wezterminator tui: {error}");
            return ExitCode::FAILURE;
        }
    };
    let checkout = args.checkout.clone().or_else(default_checkout);
    let force_mode = if args.browser {
        Some(PreviewMode::Browser)
    } else if args.wezterm {
        Some(PreviewMode::WezTerm)
    } else {
        None
    };
    let opts = RunOptions {
        checkout,
        force_mode,
        skip_handshake: args.wezterm || args.browser,
    };
    match run_tui(&paths, opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("wezterminator tui: {error}");
            ExitCode::FAILURE
        }
    }
}

fn default_checkout() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    if cwd.join("presets").is_dir() && cwd.join("themes").is_dir() {
        Some(cwd)
    } else {
        None
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Command::Stats(args) = &cli.command {
        return stats(args);
    }
    if let Command::Art(args) = &cli.command
        && args.command.stub().is_none()
    {
        return art_command(args);
    }
    if let Command::Doctor(args) = &cli.command {
        return doctor_command(args);
    }
    if let Command::Tui(args) = &cli.command {
        return tui_command(args);
    }
    if let Command::Install(args) = &cli.command {
        return install_command(args);
    }
    if let Command::Uninstall(args) = &cli.command {
        return uninstall_command(args);
    }
    if let Command::Fleet(args) = &cli.command {
        return fleet_command(args);
    }
    if let Command::Push(args) = &cli.command {
        return push_command(args);
    }
    if let Some((name, unit)) = cli.command.stub() {
        eprintln!("wezterminator {name}: not implemented yet (planned in {unit})");
    }
    ExitCode::FAILURE
}

fn install_command(args: &install::InstallArgs) -> ExitCode {
    let paths = match Paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("wezterminator install: {error}");
            return ExitCode::FAILURE;
        }
    };
    install::run_install(&paths, args)
}

fn uninstall_command(args: &install::UninstallArgs) -> ExitCode {
    let paths = match Paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("wezterminator uninstall: {error}");
            return ExitCode::FAILURE;
        }
    };
    install::run_uninstall(&paths, args)
}

fn fleet_command(args: &fleet::FleetArgs) -> ExitCode {
    let paths = match Paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("wezterminator fleet: {error}");
            return ExitCode::FAILURE;
        }
    };
    fleet::run(&paths, args)
}

fn push_command(args: &push::PushArgs) -> ExitCode {
    let paths = match Paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("wezterminator push: {error}");
            return ExitCode::FAILURE;
        }
    };
    push::run(&paths, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_planned_subcommand_exists() {
        let cmd = Cli::command();
        let names: Vec<_> = cmd.get_subcommands().map(|c| c.get_name().to_owned()).collect();
        for expected in ["tui", "art", "stats", "doctor", "install", "uninstall", "fleet", "push"] {
            assert!(names.iter().any(|n| n == expected), "missing `{expected}`");
        }
    }

    #[test]
    fn stats_art_doctor_tui_install_fleet_and_push_are_real() {
        let stub = |args: &[&str]| Cli::try_parse_from(args).unwrap().command.stub();
        assert_eq!(stub(&["wezterminator", "stats"]), None);
        assert_eq!(stub(&["wezterminator", "art", "generate", "cpc-cool", "--size", "3840x2160"]), None);
        assert_eq!(stub(&["wezterminator", "art", "check", "ember"]), None);
        assert_eq!(stub(&["wezterminator", "art", "import", "x.png"]), Some(("art import", "U9")));
        assert_eq!(stub(&["wezterminator", "stats", "--stdout"]), None);
        assert_eq!(stub(&["wezterminator", "doctor"]), None);
        assert_eq!(stub(&["wezterminator", "tui"]), None);
        assert_eq!(stub(&["wezterminator", "tui", "--browser"]), None);
        assert_eq!(
            stub(&["wezterminator", "install", "--mode", "add-on", "--checkout", "/tmp/wzt"]),
            None
        );
        assert_eq!(stub(&["wezterminator", "uninstall"]), None);
        assert_eq!(stub(&["wezterminator", "fleet", "pull"]), None);
        assert_eq!(stub(&["wezterminator", "fleet", "attach", "git@example/fleet.git"]), None);
        assert_eq!(stub(&["wezterminator", "fleet", "promote", "cool-pills"]), None);
        assert_eq!(stub(&["wezterminator", "fleet", "push"]), None);
        assert_eq!(
            stub(&["wezterminator", "fleet", "export", "cool-pills", "--out", "/tmp/out"]),
            None
        );
        assert_eq!(stub(&["wezterminator", "push", "od-cezar"]), None);
    }

    #[test]
    fn install_requires_mode() {
        assert!(Cli::try_parse_from(["wezterminator", "install"]).is_err());
        assert!(Cli::try_parse_from(["wezterminator", "install", "--mode", "add-on"]).is_ok());
    }

    #[test]
    fn art_generate_validates_its_arguments() {
        assert!(Cli::try_parse_from(["wezterminator", "art", "generate"]).is_err());
        assert!(Cli::try_parse_from(["wezterminator", "art", "generate", "x", "--size", "big"]).is_err());
        assert!(Cli::try_parse_from(["wezterminator", "art", "generate", "x", "--size", "0x10"]).is_err());
        let ok = Cli::try_parse_from([
            "wezterminator",
            "art",
            "generate",
            "x",
            "--size",
            "6016x3384",
            "--threads",
            "4",
            "--skip-legibility",
        ]);
        assert!(ok.is_ok());
    }

    #[test]
    fn stats_rejects_unknown_flags() {
        assert!(Cli::try_parse_from(["wezterminator", "stats", "--nope"]).is_err());
    }

    #[test]
    fn fleet_and_push_parse_real_args() {
        let cli = Cli::try_parse_from(["wezterminator", "fleet", "pull"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Fleet(fleet::FleetArgs {
                command: fleet::FleetCommand::Pull
            })
        ));
        let cli = Cli::try_parse_from(["wezterminator", "push", "od-cezar"]).unwrap();
        match cli.command {
            Command::Push(args) => assert_eq!(args.target, "od-cezar"),
            other => panic!("expected Push, got {other:?}"),
        }
    }

    #[test]
    fn tui_parses_mode_flags() {
        let cli =
            Cli::try_parse_from(["wezterminator", "tui", "--browser", "--checkout", "/tmp/wzt"])
                .unwrap();
        match cli.command {
            Command::Tui(args) => {
                assert!(args.browser);
                assert_eq!(
                    args.checkout.as_deref(),
                    Some(std::path::Path::new("/tmp/wzt"))
                );
            }
            other => panic!("expected Tui, got {other:?}"),
        }
    }
}
