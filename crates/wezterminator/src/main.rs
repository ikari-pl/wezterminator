//! The `wezterminator` binary.
//!
//! Every subcommand is a stub until its unit lands. Each accepts and ignores
//! trailing arguments, so scripts written against the planned interface fail
//! with the "not implemented" message rather than a usage error.

use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

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

#[derive(Debug, Subcommand)]
enum Command {
    /// Browse presets and edit theme parts, previewing live in WezTerm.
    Tui(Pending),
    /// Generate, import and pack theme art.
    Art(Pending),
    /// Write one key=value line of system stats to the status cache.
    Stats(Pending),
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
    /// The subcommand name and the unit that implements it.
    fn stub(&self) -> (&'static str, &'static str) {
        match self {
            Command::Tui(_) => ("tui", "U12"),
            Command::Art(_) => ("art", "U7"),
            Command::Stats(_) => ("stats", "U6"),
            Command::Doctor(_) => ("doctor", "U11"),
            Command::Install(_) => ("install", "U15"),
            Command::Uninstall(_) => ("uninstall", "U15"),
            Command::Fleet(_) => ("fleet", "U16"),
            Command::Push(_) => ("push", "U16"),
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let (name, unit) = cli.command.stub();
    eprintln!("wezterminator {name}: not implemented yet (planned in {unit})");
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
    fn stubs_accept_trailing_arguments() {
        let cli = Cli::try_parse_from(["wezterminator", "fleet", "pull", "--ff-only"]).unwrap();
        assert!(matches!(cli.command, Command::Fleet(Pending { ref args }) if args == &["pull", "--ff-only"]));
    }
}
