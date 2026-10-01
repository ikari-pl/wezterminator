//! Push the fleet layer to another machine over SSH.
//!
//! Hosts from the named push target are tried in order with
//! `ssh -o ConnectTimeout`. On the first reachable host the fleet directory is
//! rsynced (local layers are never pushed). When `wezterminator` exists on the
//! remote, `fleet pull` and `doctor` run there; otherwise those steps are
//! reported as skipped.

use std::path::{Path, PathBuf};

use thiserror::Error;
use wzt_model::{Machine, Paths, PushTarget, read_document};

use crate::fleet::{CommandOutput, CommandRunner};

#[derive(Debug, Error)]
pub enum PushError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Model(#[from] wzt_model::Error),
}

pub type Result<T> = std::result::Result<T, PushError>;

impl PushError {
    pub fn msg(s: impl Into<String>) -> Self {
        Self::Message(s.into())
    }
}

/// Default SSH connect timeout when the target omits one.
pub const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 5;

/// Relative remote path for the fleet layer under the user's home (XDG data).
pub const REMOTE_FLEET_REL: &str = ".local/share/wezterminator/fleet";

/// One host that was tried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostAttempt {
    pub host: String,
    pub reachable: bool,
    pub detail: String,
}

/// Report from [`push_to_target`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReport {
    pub target: String,
    pub host: String,
    pub attempts: Vec<HostAttempt>,
    pub rsynced: bool,
    pub remote_fleet_pull: StepStatus,
    pub remote_doctor: StepStatus,
}

/// Whether a remote step ran, was skipped, or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepStatus {
    Ran { detail: String },
    Skipped { reason: String },
    Failed { detail: String },
}

/// Resolve a named push target from local (then fleet) machine settings.
pub fn resolve_push_target(paths: &Paths, name: &str) -> Result<PushTarget> {
    let local = paths.local_layer_dir().join("machine.json");
    let fleet = paths.fleet_layer_dir().join("machine.json");
    for path in [&local, &fleet] {
        if !path.is_file() {
            continue;
        }
        let machine: Machine = read_document(path)?;
        if let Some(targets) = &machine.push_targets
            && let Some(t) = targets.get(name)
        {
            return Ok(t.clone());
        }
    }
    Err(PushError::msg(format!(
        "push target `{name}` not found in local or fleet machine.json"
    )))
}

/// Push the fleet layer to `target_name`.
pub fn push_to_target(
    paths: &Paths,
    target_name: &str,
    runner: &dyn CommandRunner,
) -> Result<PushReport> {
    let target = resolve_push_target(paths, target_name)?;
    push_with_target(paths, target_name, &target, runner)
}

/// Same as [`push_to_target`] with an already-resolved [`PushTarget`] (tests).
pub fn push_with_target(
    paths: &Paths,
    target_name: &str,
    target: &PushTarget,
    runner: &dyn CommandRunner,
) -> Result<PushReport> {
    let fleet = paths.fleet_layer_dir();
    if !fleet.is_dir() {
        return Err(PushError::msg(format!(
            "fleet layer missing at {}; attach a fleet repo first",
            fleet.display()
        )));
    }

    let timeout = target
        .connect_timeout_seconds
        .unwrap_or(DEFAULT_CONNECT_TIMEOUT_SECS);
    let user = target.user.as_deref();
    let port = target.port;

    let mut attempts = Vec::new();
    let mut chosen: Option<String> = None;

    for host in &target.hosts {
        let (reachable, detail) = probe_host(runner, host, user, port, timeout);
        attempts.push(HostAttempt {
            host: host.clone(),
            reachable,
            detail,
        });
        if reachable {
            chosen = Some(host.clone());
            break;
        }
    }

    let Some(host) = chosen else {
        return Err(PushError::msg(format!(
            "no reachable host for push target `{target_name}` (tried {})",
            target.hosts.join(", ")
        )));
    };

    rsync_fleet(runner, &fleet, &host, user, port, timeout)?;

    let binary_ok = remote_has_binary(runner, &host, user, port, timeout);
    let (remote_fleet_pull, remote_doctor) = if binary_ok {
        let pull = remote_cmd(
            runner,
            &host,
            user,
            port,
            timeout,
            "wezterminator fleet pull",
        );
        let doctor = remote_cmd(runner, &host, user, port, timeout, "wezterminator doctor");
        (pull, doctor)
    } else {
        (
            StepStatus::Skipped {
                reason: "wezterminator binary not found on remote".into(),
            },
            StepStatus::Skipped {
                reason: "wezterminator binary not found on remote".into(),
            },
        )
    };

    Ok(PushReport {
        target: target_name.to_string(),
        host,
        attempts,
        rsynced: true,
        remote_fleet_pull,
        remote_doctor,
    })
}

fn ssh_base_args(host: &str, user: Option<&str>, port: Option<u16>, timeout: u64) -> Vec<String> {
    let mut args = vec![
        "-o".into(),
        format!("ConnectTimeout={timeout}"),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
    ];
    if let Some(p) = port {
        args.push("-p".into());
        args.push(p.to_string());
    }
    let dest = match user {
        Some(u) => format!("{u}@{host}"),
        None => host.to_string(),
    };
    args.push(dest);
    args
}

fn probe_host(
    runner: &dyn CommandRunner,
    host: &str,
    user: Option<&str>,
    port: Option<u16>,
    timeout: u64,
) -> (bool, String) {
    let mut args = ssh_base_args(host, user, port, timeout);
    args.push("true".into());
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match runner.run("ssh", &arg_refs, None, &[]) {
        Ok(out) if out.success() => (true, "ok".into()),
        Ok(out) => (
            false,
            trim_detail(&out.stderr).or_else(|| trim_detail(&out.stdout)).unwrap_or_else(|| {
                format!("exit {}", out.code)
            }),
        ),
        Err(e) => (false, e),
    }
}

fn trim_detail(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.lines().next().unwrap_or(t).to_string())
    }
}

fn rsync_fleet(
    runner: &dyn CommandRunner,
    fleet: &Path,
    host: &str,
    user: Option<&str>,
    port: Option<u16>,
    timeout: u64,
) -> Result<()> {
    let dest_userhost = match user {
        Some(u) => format!("{u}@{host}"),
        None => host.to_string(),
    };
    let remote = format!("{dest_userhost}:{REMOTE_FLEET_REL}/");

    // Ensure trailing slash so rsync syncs contents into the remote fleet dir.
    let mut src = fleet.as_os_str().to_string_lossy().into_owned();
    if !src.ends_with('/') {
        src.push('/');
    }

    let mut ssh_cmd = format!("ssh -o ConnectTimeout={timeout} -o BatchMode=yes");
    if let Some(p) = port {
        ssh_cmd.push_str(&format!(" -p {p}"));
    }

    let args = [
        "-az",
        "--delete",
        "-e",
        &ssh_cmd,
        &src,
        &remote,
    ];
    let out = runner
        .run("rsync", &args, None, &[])
        .map_err(PushError::msg)?;
    if !out.success() {
        return Err(PushError::msg(format!(
            "rsync failed: {}",
            first_line(&out)
        )));
    }
    Ok(())
}

fn remote_has_binary(
    runner: &dyn CommandRunner,
    host: &str,
    user: Option<&str>,
    port: Option<u16>,
    timeout: u64,
) -> bool {
    let mut args = ssh_base_args(host, user, port, timeout);
    args.push("command -v wezterminator".into());
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    matches!(runner.run("ssh", &arg_refs, None, &[]), Ok(out) if out.success())
}

fn remote_cmd(
    runner: &dyn CommandRunner,
    host: &str,
    user: Option<&str>,
    port: Option<u16>,
    timeout: u64,
    remote: &str,
) -> StepStatus {
    let mut args = ssh_base_args(host, user, port, timeout);
    args.push(remote.to_string());
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match runner.run("ssh", &arg_refs, None, &[]) {
        Ok(out) if out.success() => StepStatus::Ran {
            detail: first_line(&out),
        },
        Ok(out) => StepStatus::Failed {
            detail: first_line(&out),
        },
        Err(e) => StepStatus::Failed { detail: e },
    }
}

fn first_line(out: &CommandOutput) -> String {
    trim_detail(&out.stderr)
        .or_else(|| trim_detail(&out.stdout))
        .unwrap_or_else(|| format!("exit {}", out.code))
}

/// Helper for tests: remote data path under an explicit home.
pub fn remote_fleet_under_home(home: &Path) -> PathBuf {
    home.join(".local")
        .join("share")
        .join("wezterminator")
        .join("fleet")
}
