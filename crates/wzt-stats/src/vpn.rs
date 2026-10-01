//! VPN probes, driven by `vpn_probes` in machine settings.
//!
//! Each enabled probe contributes one key named after its `id`:
//!
//! | kind | value |
//! |---|---|
//! | `tailscale`, `warp`, `command` | `up`, `wait` or `off` |
//! | `aws_vpn` | `up` or `off`, plus `<id>_profile=<names>` while connected |
//! | `interface` with a name | `up` (the interface has an address) or `off` |
//! | `interface` without a name | the number of tunnel interfaces carrying an address |
//!
//! A probe that outlives its `timeout_ms` is killed and reports `wait`.
//! Results are cached per probe for `ttl_seconds`, in the spirit of
//! `collect-stats.sh`: the Tailscale query alone costs about 200 ms, far too
//! much to repeat on every one-second tick.

use std::collections::BTreeMap;
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wzt_model::{VpnKind, VpnProbe, write_atomic};

use crate::exec::{self, Exec};

pub const UP: &str = "up";
pub const WAIT: &str = "wait";
pub const OFF: &str = "off";

/// Cache lifetime when a probe sets no `ttl_seconds` (the metis value).
pub const DEFAULT_TTL: Duration = Duration::from_secs(5);
/// Hard limit when a probe sets no `timeout_ms`.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(1000);
/// An AWS VPN client log older than this is a client that is not running.
pub const AWS_LOG_MAX_AGE: Duration = Duration::from_secs(600);

/// Keys the collector writes itself; a probe may not reuse them.
pub const RESERVED_KEYS: [&str; 5] = ["load", "ncpu", "memused", "memtotal", "pressure"];

const TAILSCALE_FALLBACKS: [&str; 3] = [
    "/opt/homebrew/bin/tailscale",
    "/usr/local/bin/tailscale",
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
];
const WARP_FALLBACKS: [&str; 2] = ["/usr/local/bin/warp-cli", "/opt/homebrew/bin/warp-cli"];
const IFCONFIG: &str = "/sbin/ifconfig";
const IP_FALLBACKS: [&str; 4] = ["/sbin/ip", "/usr/sbin/ip", "/usr/bin/ip", "/bin/ip"];
const TUNNEL_PREFIXES: [&str; 4] = ["utun", "tun", "wg", "ppp"];

/// A probe that is enabled and well formed, with defaults filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub id: String,
    pub kind: VpnKind,
    pub ttl: Duration,
    pub timeout: Duration,
    pub interface: Option<String>,
    pub command: Vec<String>,
    pub profile: Option<String>,
}

impl Probe {
    /// `None` when the probe is disabled or cannot produce a valid key.
    ///
    /// A probe without `enabled` counts as enabled: listing it is the opt-in,
    /// and `enabled: false` is how a layer switches one off.
    pub fn from_settings(settings: &VpnProbe) -> Option<Probe> {
        if settings.enabled == Some(false) || !is_valid_key(&settings.id) {
            return None;
        }
        let command = settings.command.clone().unwrap_or_default();
        if settings.kind == VpnKind::Command && command.is_empty() {
            return None;
        }
        Some(Probe {
            id: settings.id.clone(),
            kind: settings.kind,
            ttl: settings
                .ttl_seconds
                .map_or(DEFAULT_TTL, Duration::from_secs),
            timeout: settings
                .timeout_ms
                .map_or(DEFAULT_TIMEOUT, Duration::from_millis),
            interface: settings.interface.clone(),
            command,
            profile: settings.profile.clone(),
        })
    }

    /// Identifies what the probe measures, so a cached reading is dropped
    /// when the settings behind it change.
    pub fn fingerprint(&self) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for byte in bytes.iter().chain(&[0]) {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        };
        feed(kind_name(self.kind).as_bytes());
        feed(self.interface.as_deref().unwrap_or("").as_bytes());
        feed(self.profile.as_deref().unwrap_or("").as_bytes());
        for arg in &self.command {
            feed(arg.as_bytes());
        }
        format!("{hash:016x}")
    }
}

/// Turn the settings list into runnable probes. A repeated `id`, or one that
/// collides with a built-in key, is dropped (first wins).
pub fn resolve_probes(settings: &[VpnProbe]) -> Vec<Probe> {
    let mut probes: Vec<Probe> = Vec::new();
    // Every key a probe may write, its own and its `_profile` companion.
    let mut used: Vec<String> = RESERVED_KEYS.iter().map(|k| (*k).to_owned()).collect();
    for setting in settings {
        let Some(probe) = Probe::from_settings(setting) else {
            continue;
        };
        let keys = [probe.id.clone(), detail_key(&probe.id)];
        if keys.iter().any(|key| used.contains(key)) {
            continue;
        }
        used.extend(keys);
        probes.push(probe);
    }
    probes
}

fn is_valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn kind_name(kind: VpnKind) -> &'static str {
    match kind {
        VpnKind::Tailscale => "tailscale",
        VpnKind::Warp => "warp",
        VpnKind::AwsVpn => "aws_vpn",
        VpnKind::Interface => "interface",
        VpnKind::Command => "command",
    }
}

/// The extra key an `aws_vpn` probe writes while connected.
pub fn detail_key(id: &str) -> String {
    format!("{id}_profile")
}

/// What one probe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub state: String,
    pub detail: Option<String>,
}

impl Reading {
    fn state(state: &str) -> Self {
        Reading {
            state: state.to_owned(),
            detail: None,
        }
    }

    fn with_detail(state: &str, detail: &[String]) -> Self {
        let detail = detail
            .iter()
            .map(|d| sanitize(d))
            .collect::<Vec<_>>()
            .join(",");
        Reading {
            state: state.to_owned(),
            detail: (!detail.is_empty()).then_some(detail),
        }
    }
}

/// Keep a profile name usable as a `key=value` value: no whitespace, no `=`.
fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.:@/+".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Everything that varies between machines and tests.
#[derive(Debug, Clone)]
pub struct Context {
    pub now: SystemTime,
    /// Directory with `aws_vpn_client_gui_*.log`; `None` disables the AWS probe.
    pub aws_log_dir: Option<PathBuf>,
    /// Logs older than this are ignored; `None` accepts any age.
    pub aws_max_age: Option<Duration>,
    /// Overrides the `tailscale` lookup.
    pub tailscale_bin: Option<PathBuf>,
    /// Overrides the `warp-cli` lookup.
    pub warp_bin: Option<PathBuf>,
}

impl Context {
    pub fn system() -> Self {
        Context {
            now: SystemTime::now(),
            aws_log_dir: std::env::home_dir().map(|home| home.join(".config/AWSVPNClient/logs")),
            aws_max_age: Some(AWS_LOG_MAX_AGE),
            tailscale_bin: None,
            warp_bin: None,
        }
    }

    fn epoch_secs(&self) -> u64 {
        self.now
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
}

// ---------------------------------------------------------------------------
// Running probes, with the TTL cache
// ---------------------------------------------------------------------------

/// Probe results as `(key, value)` pairs in probe order, using the cache at
/// `cache_path` for readings younger than their TTL.
///
/// Stale probes run side by side, so the call takes as long as the slowest
/// timeout, not the sum. Cache problems never fail a probe run.
pub fn collect(probes: &[Probe], ctx: &Context, cache_path: &Path) -> Vec<(String, String)> {
    let readings = run_probes(probes, ctx, cache_path);
    let mut entries = Vec::new();
    for (probe, reading) in probes.iter().zip(readings) {
        entries.push((probe.id.clone(), reading.state));
        if let Some(detail) = reading.detail {
            entries.push((detail_key(&probe.id), detail));
        }
    }
    entries
}

fn run_probes(probes: &[Probe], ctx: &Context, cache_path: &Path) -> Vec<Reading> {
    if probes.is_empty() {
        return Vec::new();
    }
    let now = ctx.epoch_secs();
    let cache = load_cache(cache_path);

    let mut readings: Vec<Option<Reading>> = probes
        .iter()
        .map(|probe| {
            let fingerprint = probe.fingerprint();
            cache
                .iter()
                .find(|c| c.id == probe.id && c.fingerprint == fingerprint)
                .filter(|c| now >= c.stamp && now - c.stamp < probe.ttl.as_secs())
                .map(|c| c.reading.clone())
        })
        .collect();
    let stamps: Vec<u64> = probes
        .iter()
        .zip(&readings)
        .map(|(probe, reading)| match reading {
            Some(_) => cache
                .iter()
                .find(|c| c.id == probe.id)
                .map_or(now, |c| c.stamp),
            None => now,
        })
        .collect();

    let refreshed = readings.iter().any(Option::is_none);
    if refreshed {
        thread::scope(|scope| {
            let handles: Vec<_> = probes
                .iter()
                .zip(&readings)
                .enumerate()
                .filter(|(_, (_, cached))| cached.is_none())
                .map(|(index, (probe, _))| (index, scope.spawn(move || run_probe(probe, ctx))))
                .collect();
            for (index, handle) in handles {
                readings[index] = Some(handle.join().unwrap_or_else(|_| Reading::state(OFF)));
            }
        });
    }

    let readings: Vec<Reading> = readings
        .into_iter()
        .map(|r| r.unwrap_or_else(|| Reading::state(OFF)))
        .collect();
    if refreshed {
        let entries: Vec<CacheEntry> = probes
            .iter()
            .zip(&readings)
            .zip(&stamps)
            .map(|((probe, reading), stamp)| CacheEntry {
                id: probe.id.clone(),
                fingerprint: probe.fingerprint(),
                stamp: *stamp,
                reading: reading.clone(),
            })
            .collect();
        // A cache that cannot be written only costs the next call a re-probe.
        let _ = store_cache(cache_path, &entries);
    }
    readings
}

/// Run one probe now, ignoring the cache.
pub fn run_probe(probe: &Probe, ctx: &Context) -> Reading {
    match probe.kind {
        VpnKind::Tailscale => run_tailscale(probe, ctx),
        VpnKind::Warp => run_warp(probe, ctx),
        VpnKind::AwsVpn => run_aws(probe, ctx),
        VpnKind::Interface => run_interface(probe),
        VpnKind::Command => run_command(probe),
    }
}

#[derive(Debug)]
struct CacheEntry {
    id: String,
    fingerprint: String,
    stamp: u64,
    reading: Reading,
}

/// One tab-separated line per probe: `id fingerprint stamp state detail`.
/// The stamp is an epoch second, so freshness never depends on file mtimes.
fn load_cache(path: &Path) -> Vec<CacheEntry> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let id = fields.next()?.to_owned();
            let fingerprint = fields.next()?.to_owned();
            let stamp = fields.next()?.parse().ok()?;
            let state = fields.next()?.to_owned();
            let detail = fields.next().filter(|d| !d.is_empty()).map(str::to_owned);
            Some(CacheEntry {
                id,
                fingerprint,
                stamp,
                reading: Reading { state, detail },
            })
        })
        .collect()
}

fn store_cache(path: &Path, entries: &[CacheEntry]) -> wzt_model::Result<()> {
    let mut text = String::new();
    for e in entries {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            e.id,
            e.fingerprint,
            e.stamp,
            e.reading.state,
            e.reading.detail.as_deref().unwrap_or("")
        ));
    }
    write_atomic(path, text.as_bytes())
}

// ---------------------------------------------------------------------------
// Tailscale and WARP
// ---------------------------------------------------------------------------

fn run_tailscale(probe: &Probe, ctx: &Context) -> Reading {
    let bin = ctx
        .tailscale_bin
        .clone()
        .or_else(|| find_binary("tailscale", &TAILSCALE_FALLBACKS));
    let Some(bin) = bin else {
        return Reading::state(OFF);
    };
    match exec::run(
        bin,
        &["status", "--peers=false", "--json"],
        probe.timeout,
        true,
    ) {
        Exec::Finished { stdout, .. } => Reading::state(parse_tailscale_state(&stdout)),
        Exec::TimedOut => Reading::state(WAIT),
        Exec::SpawnFailed => Reading::state(OFF),
    }
}

/// `tailscale status --json`: `BackendState` of `Running` is up; the states
/// that mean "on its way" are `wait`; everything else, including unparseable
/// output, is off.
pub fn parse_tailscale_state(json: &str) -> &'static str {
    let state = serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.get("BackendState")?.as_str().map(str::to_owned));
    match state.as_deref() {
        Some("Running") => UP,
        Some("Starting" | "NeedsLogin" | "NeedsMachineAuth") => WAIT,
        _ => OFF,
    }
}

fn run_warp(probe: &Probe, ctx: &Context) -> Reading {
    let bin = ctx
        .warp_bin
        .clone()
        .or_else(|| find_binary("warp-cli", &WARP_FALLBACKS));
    let Some(bin) = bin else {
        return Reading::state(OFF);
    };
    match exec::run(bin, &["--accept-tos", "status"], probe.timeout, true) {
        Exec::Finished { stdout, .. } => Reading::state(parse_warp_state(&stdout)),
        Exec::TimedOut => Reading::state(WAIT),
        Exec::SpawnFailed => Reading::state(OFF),
    }
}

/// First line of `warp-cli status`. Case matters: "Disconnected" must not
/// read as "Connected".
pub fn parse_warp_state(output: &str) -> &'static str {
    let first = output.lines().next().unwrap_or("");
    if first.contains("Connected") {
        UP
    } else if first.contains("Connecting") {
        WAIT
    } else {
        OFF
    }
}

/// `name` on `PATH`, then the well-known install locations. A GUI app's
/// `PATH` is minimal, so the fallbacks matter.
fn find_binary(name: &str, fallbacks: &[&str]) -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join(name));
    on_path
        .chain(fallbacks.iter().map(PathBuf::from))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

// ---------------------------------------------------------------------------
// AWS VPN
// ---------------------------------------------------------------------------

fn run_aws(probe: &Probe, ctx: &Context) -> Reading {
    let connected = ctx
        .aws_log_dir
        .as_deref()
        .and_then(|dir| latest_aws_log(dir, ctx.now, ctx.aws_max_age))
        .and_then(|log| fs::read(log).ok())
        .map(|bytes| connected_aws_profiles(&String::from_utf8_lossy(&bytes)))
        .unwrap_or_default();

    match &probe.profile {
        Some(wanted) if connected.contains(wanted) => {
            Reading::with_detail(UP, std::slice::from_ref(wanted))
        }
        Some(_) => Reading::state(OFF),
        None if connected.is_empty() => Reading::state(OFF),
        None => Reading::with_detail(UP, &connected),
    }
}

/// The newest `aws_vpn_client_gui_*.log` in `dir` (by name, as the client
/// stamps them), unless it was last written more than `max_age` ago.
pub fn latest_aws_log(dir: &Path, now: SystemTime, max_age: Option<Duration>) -> Option<PathBuf> {
    let newest = fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("aws_vpn_client_gui_") && n.ends_with(".log"))
        })
        .max()?;
    if let Some(max_age) = max_age {
        let modified = fs::metadata(&newest).and_then(|m| m.modified()).ok()?;
        // A file stamped in the future counts as fresh.
        let age = now.duration_since(modified).unwrap_or_default();
        if age > max_age {
            return None;
        }
    }
    Some(newest)
}

/// Replay `Profile connected:` / `Profile disconnected:` events and return the
/// profiles whose last event was a connect, sorted by name. Port of
/// `sources/od-cezar/config/aws_vpn_profile.zsh`, which takes the text after
/// the last marker on the line.
pub fn connected_aws_profiles(log: &str) -> Vec<String> {
    const CONNECTED: &str = "Profile connected: ";
    const DISCONNECTED: &str = "Profile disconnected: ";

    let mut states: BTreeMap<&str, bool> = BTreeMap::new();
    for line in log.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some((_, profile)) = line.rsplit_once(CONNECTED) {
            states.insert(profile, true);
        } else if let Some((_, profile)) = line.rsplit_once(DISCONNECTED) {
            states.insert(profile, false);
        }
    }
    states
        .into_iter()
        .filter(|(_, connected)| *connected)
        .map(|(profile, _)| profile.to_owned())
        .collect()
}

// ---------------------------------------------------------------------------
// Interfaces and arbitrary commands
// ---------------------------------------------------------------------------

/// A network interface and its IPv4 addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Iface {
    pub name: String,
    pub addrs: Vec<Ipv4Addr>,
}

fn run_interface(probe: &Probe) -> Reading {
    let output = if cfg!(target_os = "macos") {
        exec::run(IFCONFIG, &[] as &[&str], probe.timeout, true)
    } else if cfg!(target_os = "linux") {
        match find_binary("ip", &IP_FALLBACKS) {
            Some(ip) => exec::run(ip, &["-o", "-4", "addr", "show"], probe.timeout, true),
            None => Exec::SpawnFailed,
        }
    } else {
        Exec::SpawnFailed
    };
    let stdout = match output {
        Exec::Finished {
            success: true,
            stdout,
        } => stdout,
        Exec::TimedOut => return Reading::state(WAIT),
        _ => return Reading::state(OFF),
    };
    let interfaces = if cfg!(target_os = "macos") {
        parse_ifconfig(&stdout)
    } else {
        parse_ip_addr(&stdout)
    };

    match &probe.interface {
        Some(name) => {
            let up = interfaces
                .iter()
                .any(|i| &i.name == name && !i.addrs.is_empty());
            Reading::state(if up { UP } else { OFF })
        }
        None => Reading {
            state: count_tunnels(&interfaces).to_string(),
            detail: None,
        },
    }
}

/// Parse `ifconfig` (BSD / macOS). Headers start in column 0; addresses are
/// indented `inet` lines.
pub fn parse_ifconfig(text: &str) -> Vec<Iface> {
    let mut interfaces: Vec<Iface> = Vec::new();
    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) {
            if let Some((name, _)) = line.split_once(':') {
                interfaces.push(Iface {
                    name: name.to_owned(),
                    addrs: Vec::new(),
                });
            }
        } else if let Some(rest) = line.trim_start().strip_prefix("inet ")
            && let (Some(iface), Some(addr)) = (
                interfaces.last_mut(),
                rest.split_whitespace().next().and_then(|a| a.parse().ok()),
            )
        {
            iface.addrs.push(addr);
        }
    }
    interfaces
}

/// Parse `ip -o -4 addr show` (Linux): `2: eth0    inet 192.168.1.2/24 ...`.
pub fn parse_ip_addr(text: &str) -> Vec<Iface> {
    let mut interfaces: Vec<Iface> = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(_index), Some(name), Some("inet"), Some(cidr)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let name = name.trim_end_matches(':').split('@').next().unwrap_or(name);
        let Some(addr) = cidr.split('/').next().and_then(|a| a.parse().ok()) else {
            continue;
        };
        match interfaces.iter_mut().find(|i| i.name == name) {
            Some(iface) => iface.addrs.push(addr),
            None => interfaces.push(Iface {
                name: name.to_owned(),
                addrs: vec![addr],
            }),
        }
    }
    interfaces
}

/// Tunnel interfaces (`utunN`, `tunN`, `wgN`, `pppN`) that carry an address,
/// leaving out any interface holding a Tailscale CGNAT address (100.64.0.0/10):
/// Tailscale advertises subnet routes as extra addresses on its own interface,
/// which must not count as separate tunnels.
pub fn count_tunnels(interfaces: &[Iface]) -> usize {
    interfaces
        .iter()
        .filter(|i| is_tunnel_name(&i.name))
        .filter(|i| !i.addrs.is_empty() && !i.addrs.iter().any(is_cgnat))
        .count()
}

fn is_tunnel_name(name: &str) -> bool {
    TUNNEL_PREFIXES.iter().any(|prefix| {
        name.strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
    })
}

fn is_cgnat(addr: &Ipv4Addr) -> bool {
    let [a, b, ..] = addr.octets();
    a == 100 && (64..=127).contains(&b)
}

fn run_command(probe: &Probe) -> Reading {
    let Some((program, args)) = probe.command.split_first() else {
        return Reading::state(OFF);
    };
    match exec::run(program, args, probe.timeout, false) {
        Exec::Finished { success: true, .. } => Reading::state(UP),
        Exec::Finished { .. } | Exec::SpawnFailed => Reading::state(OFF),
        Exec::TimedOut => Reading::state(WAIT),
    }
}
