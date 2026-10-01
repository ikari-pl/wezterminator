//! Linux probes, read from `/proc`.
//!
//! `/proc/loadavg` is what `getloadavg(3)` reads, and `MemAvailable` is the
//! kernel's own estimate of memory a new workload can use without swapping,
//! which is the closest counterpart of Activity Monitor's "Memory Used".

use std::fs;
use std::path::Path;
use std::thread;

use crate::{GIB, Metrics, Pressure};

/// Pressure-stall `some avg10` (percent of the last 10 s that at least one task
/// stalled on memory) at which the level becomes "warning" (2).
pub const PSI_WARN: f64 = 5.0;
/// ... and "critical" (4). The scale matches macOS: 1 normal, 2 warning, 4 critical.
pub const PSI_CRITICAL: f64 = 20.0;

/// Probe the running system.
pub fn collect() -> Metrics {
    collect_from(Path::new("/proc"))
}

/// Probe a `/proc`-shaped directory. Tests point this at a fixture tree.
pub fn collect_from(proc_root: &Path) -> Metrics {
    let read = |name: &str| fs::read_to_string(proc_root.join(name)).ok();
    let mem = read("meminfo").and_then(|text| parse_meminfo(&text));
    Metrics {
        load: read("loadavg").and_then(|text| parse_loadavg(&text)),
        ncpu: thread::available_parallelism().ok().map(|n| n.get()),
        mem_used_gib: mem.map(|m| m.used_bytes() as f64 / GIB),
        mem_total_gib: mem.map(|m| m.total_bytes() as f64 / GIB),
        // A kernel without PSI (or with it disabled) has no such file.
        pressure: Some(
            read("pressure/memory")
                .and_then(|text| parse_pressure(&text))
                .unwrap_or(Pressure::Unavailable),
        ),
    }
}

/// First field of `/proc/loadavg`: the 1-minute load.
pub fn parse_loadavg(text: &str) -> Option<f64> {
    text.split_whitespace().next()?.parse().ok()
}

/// The `/proc/meminfo` values the used-memory calculation needs, in kB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemInfo {
    pub total_kb: u64,
    pub available_kb: u64,
}

impl MemInfo {
    pub fn total_bytes(&self) -> u64 {
        self.total_kb * 1024
    }

    /// Total minus available.
    pub fn used_bytes(&self) -> u64 {
        self.total_kb.saturating_sub(self.available_kb) * 1024
    }
}

/// Parse `/proc/meminfo`. Without `MemAvailable` (kernels before 3.14) the
/// available figure is `MemFree + Buffers + Cached`.
pub fn parse_meminfo(text: &str) -> Option<MemInfo> {
    let field = |name: &str| -> Option<u64> {
        text.lines().find_map(|line| {
            let (key, rest) = line.split_once(':')?;
            if key != name {
                return None;
            }
            rest.split_whitespace().next()?.parse::<u64>().ok()
        })
    };
    let total_kb = field("MemTotal")?;
    let available_kb = field("MemAvailable").or_else(|| {
        Some(field("MemFree")? + field("Buffers").unwrap_or(0) + field("Cached").unwrap_or(0))
    })?;
    Some(MemInfo {
        total_kb,
        available_kb,
    })
}

/// Parse `/proc/pressure/memory`: the `some avg10=` figure, mapped to the
/// 1 / 2 / 4 scale. `None` when the line is missing or malformed.
pub fn parse_pressure(text: &str) -> Option<Pressure> {
    let some = text.lines().find(|line| line.starts_with("some "))?;
    let avg10: f64 = some
        .split_whitespace()
        .find_map(|field| field.strip_prefix("avg10="))?
        .parse()
        .ok()?;
    Some(Pressure::Level(pressure_level(avg10)))
}

/// Map a stall percentage to the 1 / 2 / 4 scale.
pub fn pressure_level(avg10: f64) -> u8 {
    if avg10 >= PSI_CRITICAL {
        4
    } else if avg10 >= PSI_WARN {
        2
    } else {
        1
    }
}
