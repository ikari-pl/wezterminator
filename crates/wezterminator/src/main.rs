//! The `wezterminator` binary.
//!
//! Subcommands are stubs until their unit lands (`stats` is real, from U6).
//! Each stub accepts and ignores trailing arguments, so scripts written
//! against the planned interface fail with the "not implemented" message
//! rather than a usage error.

use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use wzt_model::Paths;

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

/// Arguments the real subcommand will define; accepted and ignored for now.
#[derive(Debug, Args)]
struct Pending {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    args: Vec<String>,
}

#[derive(Debug, Args)]
struct StatsArgs {
    /// Print the line to stdout instead of writing the status cache.
    #[arg(long)]
    stdout: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Browse presets and edit theme parts, previewing live in WezTerm.
    Tui(Pending),
    /// Generate, import and pack theme art.
    Art(Pending),
    /// Write one key=value line of system stats to the status cache.
    Stats(StatsArgs),
    /// Check fonts, art, screens and the install, and say what is wrong.
    Doctor(Pending),
    /// Install wezterminator in add-on, replace or replace-and-import mode.
    Install(Pending),
    /// Undo an install from its manifest.
    Uninstall(Pending),
    /// Attach, pull and promote presets for a private fleet repo.
    Fleet(Pending),
    /// Push the fleet layer to another machine over SSH.
    Push(Pending),
}

impl Command {
    /// The subcommand name and the unit that implements it, or `None` once
    /// the subcommand is real.
    fn stub(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Command::Tui(_) => Some(("tui", "U12")),
            Command::Art(_) => Some(("art", "U7")),
            Command::Stats(_) => None,
            Command::Doctor(_) => Some(("doctor", "U11")),
            Command::Install(_) => Some(("install", "U15")),
            Command::Uninstall(_) => Some(("uninstall", "U15")),
            Command::Fleet(_) => Some(("fleet", "U16")),
            Command::Push(_) => Some(("push", "U16")),
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

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Command::Stats(args) = &cli.command {
        return stats(args);
    }
    if let Some((name, unit)) = cli.command.stub() {
        eprintln!("wezterminator {name}: not implemented yet (planned in {unit})");
    }
    ExitCode::FAILURE
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
    fn stats_is_real_and_every_other_subcommand_is_still_a_stub() {
        let stub = |args: &[&str]| Cli::try_parse_from(args).unwrap().command.stub();
        assert_eq!(stub(&["wezterminator", "stats"]), None);
        assert_eq!(stub(&["wezterminator", "stats", "--stdout"]), None);
        assert_eq!(stub(&["wezterminator", "doctor"]), Some(("doctor", "U11")));
    }

    #[test]
    fn stats_rejects_unknown_flags() {
        assert!(Cli::try_parse_from(["wezterminator", "stats", "--nope"]).is_err());
    }

    #[test]
    fn stubs_accept_trailing_arguments() {
        let cli = Cli::try_parse_from(["wezterminator", "fleet", "pull", "--ff-only"]).unwrap();
        assert!(matches!(cli.command, Command::Fleet(Pending { ref args }) if args == &["pull", "--ff-only"]));
    }
}
