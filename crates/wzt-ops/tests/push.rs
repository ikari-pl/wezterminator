//! SSH push scenarios (U16): host fallthrough and missing remote binary.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tempfile::TempDir;
use wzt_model::{Paths, PushTarget, write_document};
use wzt_ops::fleet::{CommandOutput, CommandRunner};
use wzt_ops::{StepStatus, push_with_target};

/// Scripted runner: matches on (program, joined args substring) FIFO responses.
struct FakeRunner {
    /// Each entry: predicate on program+args, then the output to return.
    script: Mutex<VecDeque<Scripted>>,
    log: RefCell<Vec<String>>,
}

struct Scripted {
    /// Return true when this response should be used.
    when: Box<dyn Fn(&str, &[&str]) -> bool + Send>,
    output: Result<CommandOutput, String>,
}

impl FakeRunner {
    fn new(script: Vec<Scripted>) -> Self {
        Self {
            script: Mutex::new(VecDeque::from(script)),
            log: RefCell::new(Vec::new()),
        }
    }

    fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
}

impl CommandRunner for FakeRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        _cwd: Option<&Path>,
        _env: &[(&str, &str)],
    ) -> Result<CommandOutput, String> {
        let joined = format!("{program} {}", args.join(" "));
        self.log.borrow_mut().push(joined.clone());

        let mut script = self.script.lock().unwrap();
        let idx = script
            .iter()
            .position(|s| (s.when)(program, args))
            .ok_or_else(|| format!("unexpected command: {joined}"))?;
        let entry = script.remove(idx).unwrap();
        entry.output
    }
}

fn ok(stdout: &str) -> Result<CommandOutput, String> {
    Ok(CommandOutput {
        code: 0,
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

fn fail(code: i32, stderr: &str) -> Result<CommandOutput, String> {
    Ok(CommandOutput {
        code,
        stdout: String::new(),
        stderr: stderr.into(),
    })
}

fn when_ssh_host(host: &str) -> Box<dyn Fn(&str, &[&str]) -> bool + Send> {
    let host = host.to_string();
    Box::new(move |program, args| {
        program == "ssh"
            && args.iter().any(|a| *a == host || a.ends_with(&format!("@{host}")))
            && args.last().copied() == Some("true")
    })
}

fn when_ssh_cmd(host: &str, remote_cmd: &str) -> Box<dyn Fn(&str, &[&str]) -> bool + Send> {
    let host = host.to_string();
    let remote_cmd = remote_cmd.to_string();
    Box::new(move |program, args| {
        program == "ssh"
            && args.iter().any(|a| *a == host || a.ends_with(&format!("@{host}")))
            && args.last().copied() == Some(remote_cmd.as_str())
    })
}

fn when_rsync() -> Box<dyn Fn(&str, &[&str]) -> bool + Send> {
    Box::new(|program, _| program == "rsync")
}

fn harness() -> (TempDir, Paths, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let config = home.join(".config");
    let data = home.join(".local").join("share");
    let state = home.join(".local").join("state");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    fs::create_dir_all(&state).unwrap();
    let paths = Paths::from_roots(config, data, state);
    let fleet = paths.fleet_layer_dir();
    fs::create_dir_all(fleet.join("presets")).unwrap();
    fs::write(fleet.join("presets").join(".gitkeep"), "").unwrap();
    (tmp, paths, fleet)
}

#[test]
fn push_falls_through_unreachable_first_host_to_second() {
    let (_tmp, paths, _fleet) = harness();

    let target = PushTarget {
        hosts: vec!["dead.example".into(), "live.example".into()],
        user: Some("ikari".into()),
        port: None,
        connect_timeout_seconds: Some(1),
        comments: Default::default(),
    };

    let runner = FakeRunner::new(vec![
        Scripted {
            when: when_ssh_host("dead.example"),
            output: fail(255, "Connection timed out"),
        },
        Scripted {
            when: when_ssh_host("live.example"),
            output: ok(""),
        },
        Scripted {
            when: when_rsync(),
            output: ok(""),
        },
        Scripted {
            when: when_ssh_cmd("live.example", "command -v wezterminator"),
            output: ok("/usr/local/bin/wezterminator\n"),
        },
        Scripted {
            when: when_ssh_cmd("live.example", "wezterminator fleet pull"),
            output: ok("pulled\n"),
        },
        Scripted {
            when: when_ssh_cmd("live.example", "wezterminator doctor"),
            output: ok("ok\n"),
        },
    ]);

    let report = push_with_target(&paths, "od-cezar", &target, &runner).unwrap();
    assert_eq!(report.host, "live.example");
    assert_eq!(report.attempts.len(), 2);
    assert!(!report.attempts[0].reachable);
    assert!(report.attempts[1].reachable);
    assert!(report.rsynced);
    assert!(matches!(report.remote_fleet_pull, StepStatus::Ran { .. }));
    assert!(matches!(report.remote_doctor, StepStatus::Ran { .. }));

    let log = runner.log();
    assert!(log.iter().any(|l| l.contains("dead.example") && l.contains("true")));
    assert!(log.iter().any(|l| l.contains("live.example") && l.contains("true")));
    assert!(log.iter().any(|l| l.starts_with("rsync ")));
    // Local layer path must never appear in rsync args.
    assert!(
        !log.iter().any(|l| l.contains(".config/wezterminator")),
        "must never push local layer: {log:?}"
    );
}

#[test]
fn push_without_remote_binary_syncs_and_reports_skipped_steps() {
    let (_tmp, paths, _fleet) = harness();

    let target = PushTarget {
        hosts: vec!["fresh.example".into()],
        user: None,
        port: Some(2222),
        connect_timeout_seconds: Some(2),
        comments: Default::default(),
    };

    let runner = FakeRunner::new(vec![
        Scripted {
            when: when_ssh_host("fresh.example"),
            output: ok(""),
        },
        Scripted {
            when: when_rsync(),
            output: ok(""),
        },
        Scripted {
            when: when_ssh_cmd("fresh.example", "command -v wezterminator"),
            output: fail(1, ""),
        },
    ]);

    let report = push_with_target(&paths, "fresh", &target, &runner).unwrap();
    assert!(report.rsynced);
    match &report.remote_fleet_pull {
        StepStatus::Skipped { reason } => assert!(reason.contains("binary")),
        other => panic!("expected skipped pull, got {other:?}"),
    }
    match &report.remote_doctor {
        StepStatus::Skipped { reason } => assert!(reason.contains("binary")),
        other => panic!("expected skipped doctor, got {other:?}"),
    }

    // No remote fleet pull / doctor invocations beyond the binary probe.
    let log = runner.log();
    assert!(!log.iter().any(|l| l.contains("fleet pull")));
    assert!(!log.iter().any(|l| l.contains("doctor")));
}

#[test]
fn resolve_push_target_reads_local_machine_settings() {
    let (_tmp, paths, _) = harness();
    let machine = serde_json::json!({
        "schema_version": 1,
        "push_targets": {
            "od-cezar": {
                "hosts": ["vpn.example", "lan.example"],
                "user": "ikari",
                "connect_timeout_seconds": 3
            }
        }
    });
    write_document(&paths.local_layer_dir().join("machine.json"), &machine).unwrap();

    let t = wzt_ops::resolve_push_target(&paths, "od-cezar").unwrap();
    assert_eq!(t.hosts, vec!["vpn.example", "lan.example"]);
    assert_eq!(t.user.as_deref(), Some("ikari"));
    assert_eq!(t.connect_timeout_seconds, Some(3));
}
