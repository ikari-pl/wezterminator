//! The wezterminator stats collector.
//!
//! `wezterminator stats` is a one-shot command: it samples the machine, builds
//! one `key=value` line and writes it atomically to the stats cache
//! ([`wzt_model::Paths::stats_file`]). WezTerm launches it fire-and-forget at
//! most once per second and reads the *previous* result, so nothing here ever
//! runs on the GUI thread; there is no resident daemon.
//!
//! ```text
//! load=12.99 ncpu=16 memused=52.1 memtotal=128.0 pressure=1 ts=up warp=off utun=0
//! ```
//!
//! Key order is fixed: the built-in probes ([`Metrics`]), then one key per
//! enabled VPN probe in machine-settings order ([`vpn`]). A probe that is not
//! configured, or has no answer, writes no key, which is how status segments
//! know to hide. Battery is not collected here; the status bar reads it from
//! WezTerm's Lua API.
//!
//! Platforms: [`macos`] and [`linux`] probe; [`windows`] writes no probe keys
//! in v1. Every platform module compiles everywhere, so the parsers are
//! tested on any host.

pub mod exec;
pub mod linux;
pub mod macos;
pub mod settings;
pub mod vpn;
pub mod windows;

use std::path::Path;
use std::thread;

use wzt_model::{Paths, write_atomic};

/// Bytes per GiB, the unit of `memused` and `memtotal`.
pub(crate) const GIB: f64 = 1_073_741_824.0;

/// File name of the VPN probe cache, next to the stats file in the state dir.
pub const VPN_CACHE_FILE: &str = "stats-vpn";

/// Memory pressure, on the macOS scale: 1 normal, 2 warning, 4 critical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressure {
    Level(u8),
    /// The platform has no pressure signal (a Linux kernel without PSI).
    Unavailable,
}

/// The built-in probes. `None` means "no answer": the key is left out.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Metrics {
    /// 1-minute load average.
    pub load: Option<f64>,
    pub ncpu: Option<usize>,
    pub mem_used_gib: Option<f64>,
    pub mem_total_gib: Option<f64>,
    pub pressure: Option<Pressure>,
}

impl Metrics {
    /// Add the keys in their fixed order: `load ncpu memused memtotal pressure`.
    pub fn write_into(&self, sample: &mut Sample) {
        if let Some(load) = self.load {
            sample.push("load", format!("{load:.2}"));
        }
        if let Some(ncpu) = self.ncpu {
            sample.push("ncpu", ncpu.to_string());
        }
        if let Some(used) = self.mem_used_gib {
            sample.push("memused", format!("{used:.1}"));
        }
        if let Some(total) = self.mem_total_gib {
            sample.push("memtotal", format!("{total:.1}"));
        }
        match self.pressure {
            Some(Pressure::Level(level)) => sample.push("pressure", level.to_string()),
            Some(Pressure::Unavailable) => sample.push("pressure", "na"),
            None => {}
        }
    }
}

/// An ordered list of `key=value` pairs, rendered as one line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sample {
    entries: Vec<(String, String)>,
}

impl Sample {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a pair. The line format is whitespace separated, so a key must
    /// be `[a-z0-9_-]+` and a value has whitespace and `=` replaced by `_`.
    /// A pair that cannot be made valid is dropped.
    pub fn push(&mut self, key: &str, value: impl AsRef<str>) {
        let key_ok = !key.is_empty()
            && key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
        let value: String = value
            .as_ref()
            .chars()
            .map(|c| {
                if c.is_whitespace() || c.is_control() || c == '=' {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        if key_ok && !value.is_empty() {
            self.entries.push((key.to_owned(), value));
        }
    }

    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// `k1=v1 k2=v2\n`. An empty sample renders as a bare newline, so the
    /// cache file still exists and says "collector ran, nothing to report".
    pub fn render(&self) -> String {
        let mut line = self
            .entries
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(" ");
        line.push('\n');
        line
    }
}

/// Whether this platform runs any probes at all.
pub const fn probes_supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "linux"))
}

/// Probe the running system's built-in metrics.
pub fn system_metrics() -> Metrics {
    if cfg!(target_os = "macos") {
        macos::collect()
    } else if cfg!(target_os = "linux") {
        linux::collect()
    } else {
        // Windows by design; other platforms have no probes yet.
        windows::collect()
    }
}

/// Build the stats line: built-in metrics, then the VPN probes, concurrently.
///
/// On a platform without probes ([`probes_supported`]) the sample is empty.
pub fn collect(probes: &[vpn::Probe], ctx: &vpn::Context, vpn_cache: &Path) -> Sample {
    let mut sample = Sample::new();
    if !probes_supported() {
        return sample;
    }
    let (metrics, vpn_entries) = thread::scope(|scope| {
        let vpn = scope.spawn(|| vpn::collect(probes, ctx, vpn_cache));
        let metrics = system_metrics();
        (metrics, vpn.join().unwrap_or_default())
    });
    metrics.write_into(&mut sample);
    for (key, value) in vpn_entries {
        sample.push(&key, value);
    }
    sample
}

/// What [`sample`] produced.
#[derive(Debug)]
pub struct Report {
    pub sample: Sample,
    /// Settings that were skipped (unreadable `machine.json`, a bad probe).
    pub warnings: Vec<String>,
}

/// Sample this machine using the current user's machine settings.
pub fn sample(paths: &Paths) -> Report {
    let settings = settings::load(paths);
    let probes = vpn::resolve_probes(&settings.probes);
    let cache = paths.state_dir().join(VPN_CACHE_FILE);
    Report {
        sample: collect(&probes, &vpn::Context::system(), &cache),
        warnings: settings.warnings,
    }
}

/// Write the line to `path` atomically. The directory is created if needed and
/// a reader sees the old line or the new one, never a partial write.
pub fn write_cache(path: &Path, sample: &Sample) -> wzt_model::Result<()> {
    write_atomic(path, sample.render().as_bytes())
}

/// Sample and write the stats cache. This is the whole `stats` command.
pub fn run(paths: &Paths) -> wzt_model::Result<Report> {
    let report = sample(paths);
    write_cache(&paths.stats_file(), &report.sample)?;
    Ok(report)
}
