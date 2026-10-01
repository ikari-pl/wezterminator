//! macOS probes: the same commands as `sources/metis/config/bin/collect-stats.sh`.
//!
//! The plan names `getloadavg` and Mach `host_statistics64`, but both are FFI
//! and the workspace forbids `unsafe`. `sysctl` and `vm_stat` report the same
//! numbers; the parsers are pure so they are tested on every platform.

use std::thread;
use std::time::Duration;

use crate::exec::{self, Exec};
use crate::{GIB, Metrics, Pressure};

const SYSCTL: &str = "/usr/sbin/sysctl";
const VM_STAT: &str = "/usr/bin/vm_stat";
const TOOL_TIMEOUT: Duration = Duration::from_millis(1500);

/// Probe the running system. The four tools run side by side.
pub fn collect() -> Metrics {
    thread::scope(|scope| {
        let load = scope.spawn(|| sysctl("vm.loadavg").and_then(|s| parse_loadavg(&s)));
        let total = scope.spawn(|| sysctl("hw.memsize").and_then(|s| parse_memsize(&s)));
        let pressure = scope.spawn(|| {
            // Like the shell script, a failing sysctl means "normal".
            sysctl("kern.memorystatus_vm_pressure_level")
                .and_then(|s| parse_pressure(&s))
                .unwrap_or(1)
        });
        let used = scope.spawn(|| {
            let out = capture(VM_STAT, &[])?;
            parse_vm_stat(&out).map(|vm| vm.used_bytes())
        });

        Metrics {
            load: load.join().ok().flatten(),
            ncpu: thread::available_parallelism().ok().map(|n| n.get()),
            mem_used_gib: used.join().ok().flatten().map(|b| b as f64 / GIB),
            mem_total_gib: total.join().ok().flatten().map(|b| b as f64 / GIB),
            pressure: pressure.join().ok().map(Pressure::Level),
        }
    })
}

fn sysctl(name: &str) -> Option<String> {
    capture(SYSCTL, &["-n", name])
}

fn capture(program: &str, args: &[&str]) -> Option<String> {
    match exec::run(program, args, TOOL_TIMEOUT, true) {
        Exec::Finished {
            success: true,
            stdout,
        } => Some(stdout),
        _ => None,
    }
}

/// `sysctl -n vm.loadavg` prints `{ 12.99 11.50 10.20 }`; the 1-minute load
/// is the first number.
pub fn parse_loadavg(text: &str) -> Option<f64> {
    text.trim()
        .trim_start_matches('{')
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// `sysctl -n hw.memsize`: total memory in bytes.
pub fn parse_memsize(text: &str) -> Option<u64> {
    text.trim().parse().ok()
}

/// `sysctl -n kern.memorystatus_vm_pressure_level`: 1 normal, 2 warning, 4 critical.
pub fn parse_pressure(text: &str) -> Option<u8> {
    text.trim().parse().ok()
}

/// The `vm_stat` counters behind Activity Monitor's "Memory Used".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmStat {
    pub page_size: u64,
    pub anonymous: u64,
    pub purgeable: u64,
    pub wired: u64,
    pub compressor: u64,
}

impl VmStat {
    /// `anonymous - purgeable + wired + compressed`, in bytes, the formula
    /// from `collect-stats.sh`. Purgeable pages are a subset of anonymous
    /// ones, so the difference cannot go below zero on a sane system; it is
    /// clamped anyway.
    pub fn used_bytes(&self) -> u64 {
        let pages = (self.anonymous + self.wired + self.compressor).saturating_sub(self.purgeable);
        pages * self.page_size
    }
}

/// Parse `vm_stat` output. Every counter must be present.
pub fn parse_vm_stat(text: &str) -> Option<VmStat> {
    let mut page_size = None;
    let (mut anonymous, mut purgeable, mut wired, mut compressor) = (None, None, None, None);

    for line in text.lines() {
        if let Some((_, rest)) = line.split_once("page size of ") {
            page_size = rest.split_whitespace().next().and_then(|n| n.parse().ok());
            continue;
        }
        let Some((label, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_end_matches('.').parse::<u64>().ok();
        match label.trim() {
            "Anonymous pages" => anonymous = value,
            "Pages purgeable" => purgeable = value,
            "Pages wired down" => wired = value,
            "Pages occupied by compressor" => compressor = value,
            _ => {}
        }
    }

    Some(VmStat {
        page_size: page_size?,
        anonymous: anonymous?,
        purgeable: purgeable?,
        wired: wired?,
        compressor: compressor?,
    })
}
