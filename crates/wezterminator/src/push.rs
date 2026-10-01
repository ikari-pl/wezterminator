//! `wezterminator push` — sync the fleet layer to another machine over SSH.

use std::process::ExitCode;

use clap::Args;
use wzt_model::Paths;
use wzt_ops::{StepStatus, SystemRunner, push_to_target};

#[derive(Debug, Args)]
pub struct PushArgs {
    /// Named push target from machine.json (`push_targets`).
    pub target: String,
}

/// Run `wezterminator push <target>`.
pub fn run(paths: &Paths, args: &PushArgs) -> ExitCode {
    let runner = SystemRunner;
    match push_to_target(paths, &args.target, &runner) {
        Ok(report) => {
            println!(
                "pushed fleet to {} via {}",
                report.target, report.host
            );
            for attempt in &report.attempts {
                let mark = if attempt.reachable { "ok" } else { "fail" };
                println!("  host {}: {mark} ({})", attempt.host, attempt.detail);
            }
            println!("  rsync: {}", if report.rsynced { "done" } else { "skipped" });
            print_step("fleet pull", &report.remote_fleet_pull);
            print_step("doctor", &report.remote_doctor);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("wezterminator push: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_step(name: &str, step: &StepStatus) {
    match step {
        StepStatus::Ran { detail } => println!("  {name}: ok ({detail})"),
        StepStatus::Skipped { reason } => println!("  {name}: skipped ({reason})"),
        StepStatus::Failed { detail } => println!("  {name}: failed ({detail})"),
    }
}
