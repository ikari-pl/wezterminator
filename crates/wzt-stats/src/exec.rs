//! Running external tools without a shell and with a hard timeout.
//!
//! A probe that hangs (a wedged `tailscale` daemon, a VPN client waiting on a
//! login) must never hold up the stats line, so every child gets a deadline
//! and is killed when it passes.

use std::ffi::OsStr;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How a child process ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exec {
    /// The process exited before the deadline.
    Finished { success: bool, stdout: String },
    /// The deadline passed; the process was killed.
    TimedOut,
    /// The program could not be started (missing, not executable).
    SpawnFailed,
}

/// Run `program` with `args`, no shell, no stdin, stderr discarded.
///
/// `capture` keeps stdout; otherwise it is discarded and `stdout` is empty.
pub fn run<S: AsRef<OsStr>>(
    program: impl AsRef<OsStr>,
    args: &[S],
    timeout: Duration,
    capture: bool,
) -> Exec {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(if capture {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let Ok(mut child) = command.spawn() else {
        return Exec::SpawnFailed;
    };
    let deadline = Instant::now() + timeout;

    // Drain stdout on a thread so a chatty child cannot block on a full pipe.
    let reader = child.stdout.take().map(|mut out| {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            let _ = tx.send(buf);
        });
        rx
    });

    let mut poll = Duration::from_millis(1);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Exec::SpawnFailed;
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Exec::TimedOut;
        }
        thread::sleep(poll);
        poll = (poll * 2).min(Duration::from_millis(10));
    };

    // The child is gone, but a grandchild may still hold the pipe open, so
    // the wait for its output is bounded too.
    let stdout = reader
        .map(|rx| {
            let left = deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(50));
            rx.recv_timeout(left)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    Exec::Finished {
        success: status.success(),
        stdout,
    }
}
