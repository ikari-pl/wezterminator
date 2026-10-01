//! `wezterminator doctor`: fonts, art, screens and install health.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use wzt_model::Paths;
use wzt_ops::doctor::{self, DoctorReport};

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Directory of a wezterminator checkout holding built-in presets/themes.
    /// Defaults to the current directory when it has a `themes/` folder.
    #[arg(long, value_name = "DIR")]
    pub checkout: Option<PathBuf>,
}

/// Run doctor and print findings. Exit 0 when clean, 1 when findings exist.
pub fn run(paths: &Paths, args: &DoctorArgs) -> ExitCode {
    let input = doctor::collect(paths, args.checkout.as_deref());
    let report = doctor::run(&input);
    print_report(&report);
    if report.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn print_report(report: &DoctorReport) {
    if report.is_ok() {
        println!("ok: no findings");
        return;
    }
    println!("wezterminator doctor: {} finding(s)", report.findings.len());
    for finding in &report.findings {
        println!("  {}: {}", finding.code, finding.message);
    }
}
