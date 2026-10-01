//! VPN probes: parsers from fixtures, timeouts, the TTL cache, AWS logs.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use wzt_model::{VpnKind, VpnProbe};
use wzt_stats::vpn::{self, Context, Iface, Probe};

fn context(now: SystemTime) -> Context {
    Context {
        now,
        aws_log_dir: None,
        aws_max_age: None,
        tailscale_bin: None,
        warp_bin: None,
    }
}

fn settings(id: &str, kind: VpnKind) -> VpnProbe {
    VpnProbe {
        id: id.to_owned(),
        kind,
        enabled: Some(true),
        label: None,
        ttl_seconds: None,
        timeout_ms: None,
        interface: None,
        command: None,
        profile: None,
        comments: Default::default(),
    }
}

fn probe(settings: VpnProbe) -> Probe {
    vpn::resolve_probes(&[settings])
        .pop()
        .expect("probe resolves")
}

fn get<'a>(entries: &'a [(String, String)], key: &str) -> Option<&'a str> {
    entries
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

// --- Tailscale and WARP ----------------------------------------------------

#[test]
fn tailscale_backend_states() {
    let state = |s: &str| {
        vpn::parse_tailscale_state(&format!(r#"{{"Version":"1.2","BackendState":"{s}"}}"#))
    };
    assert_eq!(state("Running"), "up");
    assert_eq!(state("Starting"), "wait");
    assert_eq!(state("NeedsLogin"), "wait");
    assert_eq!(state("NeedsMachineAuth"), "wait");
    assert_eq!(state("Stopped"), "off");
    assert_eq!(vpn::parse_tailscale_state("not json"), "off");
    assert_eq!(vpn::parse_tailscale_state("{}"), "off");
}

#[test]
fn warp_status_lines() {
    assert_eq!(vpn::parse_warp_state("Status update: Connected\n"), "up");
    assert_eq!(vpn::parse_warp_state("Status update: Connecting\n"), "wait");
    // Case matters: "Disconnected" is not "Connected".
    assert_eq!(
        vpn::parse_warp_state("Status update: Disconnected. Reason: Manual\n"),
        "off"
    );
    assert_eq!(vpn::parse_warp_state(""), "off");
}

#[test]
fn a_missing_binary_reads_as_off() {
    let mut ctx = context(SystemTime::now());
    ctx.tailscale_bin = Some("/nonexistent/tailscale".into());
    ctx.warp_bin = Some("/nonexistent/warp-cli".into());
    assert_eq!(
        vpn::run_probe(&probe(settings("ts", VpnKind::Tailscale)), &ctx).state,
        "off"
    );
    assert_eq!(
        vpn::run_probe(&probe(settings("warp", VpnKind::Warp)), &ctx).state,
        "off"
    );
}

// --- Interfaces --------------------------------------------------------------

const IFCONFIG: &str = "\
lo0: flags=8049<UP,LOOPBACK,RUNNING,MULTICAST> mtu 16384
\tinet 127.0.0.1 netmask 0xff000000
\tinet6 ::1 prefixlen 128
en0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
\tinet 192.168.1.20 netmask 0xffffff00 broadcast 192.168.1.255
utun0: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1380
\tinet6 fe80::1%utun0 prefixlen 64 scopeid 0xe
utun1: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1280
\tinet 100.101.102.103 --> 100.101.102.103 netmask 0xffffffff
\tinet 10.20.0.1 --> 10.20.0.1 netmask 0xffffffff
utun2: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1400
\tinet 10.8.0.2 --> 10.8.0.1 netmask 0xffffff00
utun3: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1500
\tinet 100.63.0.9 --> 100.63.0.9 netmask 0xffffffff
";

#[test]
fn utun_count_skips_tailscale_and_addressless_tunnels() {
    let interfaces = vpn::parse_ifconfig(IFCONFIG);
    assert_eq!(interfaces.len(), 6);
    // utun0 has no IPv4 address; utun1 holds a CGNAT address (Tailscale, its
    // extra subnet route notwithstanding); 100.63.x is just below the CGNAT range.
    assert_eq!(vpn::count_tunnels(&interfaces), 2);
}

#[test]
fn ip_addr_output_parses_and_counts() {
    let text = "\
1: lo    inet 127.0.0.1/8 scope host lo\\       valid_lft forever preferred_lft forever
2: eth0    inet 192.168.1.2/24 brd 192.168.1.255 scope global eth0\\       valid_lft forever
5: tun0    inet 10.8.0.2/24 scope global tun0\\       valid_lft forever
6: wg0    inet 10.9.0.2/32 scope global wg0\\       valid_lft forever
6: wg0    inet 10.9.0.3/32 scope global secondary wg0\\       valid_lft forever
7: tailscale0    inet 100.90.1.2/32 scope global tailscale0\\       valid_lft forever
";
    let interfaces = vpn::parse_ip_addr(text);
    // wg0's two address lines merge into one interface.
    assert_eq!(interfaces.len(), 5);
    assert_eq!(
        interfaces.iter().find(|i| i.name == "wg0"),
        Some(&Iface {
            name: "wg0".into(),
            addrs: vec!["10.9.0.2".parse().unwrap(), "10.9.0.3".parse().unwrap()]
        })
    );
    assert_eq!(vpn::count_tunnels(&interfaces), 2);
}

// --- Commands, timeouts and the cache ------------------------------------------

#[cfg(unix)]
mod unix {
    use super::*;

    fn command(id: &str, argv: &[&str], timeout_ms: u64) -> Probe {
        let mut s = settings(id, VpnKind::Command);
        s.command = Some(argv.iter().map(|a| (*a).to_owned()).collect());
        s.timeout_ms = Some(timeout_ms);
        probe(s)
    }

    #[test]
    fn command_probe_follows_the_exit_status() {
        let ctx = context(SystemTime::now());
        assert_eq!(
            vpn::run_probe(&command("a", &["true"], 2000), &ctx).state,
            "up"
        );
        assert_eq!(
            vpn::run_probe(&command("b", &["false"], 2000), &ctx).state,
            "off"
        );
        assert_eq!(
            vpn::run_probe(&command("c", &["/nonexistent/prog"], 2000), &ctx).state,
            "off"
        );
    }

    #[test]
    fn a_probe_over_its_timeout_reports_wait_and_the_run_stays_in_budget() {
        let dir = tempfile::tempdir().unwrap();
        let probes = [
            command("slow1", &["sleep", "5"], 300),
            command("slow2", &["sleep", "5"], 300),
        ];

        let started = Instant::now();
        let entries = vpn::collect(
            &probes,
            &context(SystemTime::now()),
            &dir.path().join("vpn"),
        );
        let elapsed = started.elapsed();

        assert_eq!(get(&entries, "slow1"), Some("wait"));
        assert_eq!(get(&entries, "slow2"), Some("wait"));
        // Both ran side by side and were killed at the deadline, long before
        // the 5 s sleep would have ended.
        assert!(elapsed < Duration::from_secs(3), "took {elapsed:?}");
    }

    #[test]
    fn readings_are_reused_within_the_ttl_and_refreshed_after_it() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("vpn");
        let marker = dir.path().join("runs");
        let script = format!("echo x >> {}", marker.display());
        let mut s = settings("lab", VpnKind::Command);
        s.command = Some(vec!["sh".into(), "-c".into(), script]);
        s.ttl_seconds = Some(10);
        let probes = [probe(s)];
        let runs = || fs::read_to_string(&marker).map_or(0, |t| t.lines().count());

        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let at = |secs: u64| context(t0 + Duration::from_secs(secs));

        assert_eq!(
            get(&vpn::collect(&probes, &at(0), &cache), "lab"),
            Some("up")
        );
        assert_eq!(runs(), 1);
        // Inside the TTL the cached reading is served without running anything.
        assert_eq!(
            get(&vpn::collect(&probes, &at(9), &cache), "lab"),
            Some("up")
        );
        assert_eq!(runs(), 1);
        // At the TTL it is probed again.
        vpn::collect(&probes, &at(10), &cache);
        assert_eq!(runs(), 2);
        // The refresh restarted the clock.
        vpn::collect(&probes, &at(15), &cache);
        assert_eq!(runs(), 2);
    }

    #[test]
    fn changing_a_probe_invalidates_its_cached_reading() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("vpn");
        let ctx = context(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000));

        let up = [command("lab", &["true"], 2000)];
        assert_eq!(get(&vpn::collect(&up, &ctx, &cache), "lab"), Some("up"));
        // Same id, different command, same instant: must not serve "up".
        let down = [command("lab", &["false"], 2000)];
        assert_eq!(get(&vpn::collect(&down, &ctx, &cache), "lab"), Some("off"));
    }

    #[test]
    fn a_damaged_cache_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("vpn");
        fs::write(&cache, "garbage\n\u{0}\u{1}\nlab\tzz\tnotanumber\n").unwrap();
        let probes = [command("lab", &["true"], 2000)];
        assert_eq!(
            get(
                &vpn::collect(&probes, &context(SystemTime::now()), &cache),
                "lab"
            ),
            Some("up")
        );
    }
}

// --- AWS VPN -------------------------------------------------------------------

const AWS_LOG: &str = "\
2026-09-01 10:00:01 INFO Starting client\r
2026-09-01 10:00:05 INFO Profile connected: corp-eu\r
2026-09-01 10:00:09 INFO Profile connected: corp-us
2026-09-01 10:30:00 INFO Profile disconnected: corp-eu\r
2026-09-01 10:31:00 INFO Profile connected: lab vpn
2026-09-01 10:32:00 INFO Profile connected: corp-eu
2026-09-01 10:33:00 INFO Profile disconnected: corp-eu
2026-09-01 10:34:00 INFO Profile disconnected: never-connected
";

#[test]
fn aws_log_reports_only_profiles_still_connected() {
    // corp-eu connected, disconnected, connected, disconnected again; corp-us
    // and "lab vpn" were left connected. Sorted by name.
    assert_eq!(vpn::connected_aws_profiles(AWS_LOG), ["corp-us", "lab vpn"]);
    assert!(vpn::connected_aws_profiles("").is_empty());
}

fn aws_dir(logs: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in logs {
        fs::write(dir.path().join(name), text).unwrap();
    }
    dir
}

fn aws_context(dir: &Path, max_age: Option<Duration>) -> Context {
    Context {
        aws_log_dir: Some(dir.to_path_buf()),
        aws_max_age: max_age,
        ..context(SystemTime::now())
    }
}

#[test]
fn aws_probe_reads_the_newest_log() {
    let dir = aws_dir(&[
        (
            "aws_vpn_client_gui_20260831.log",
            "Profile connected: stale\n",
        ),
        ("aws_vpn_client_gui_20260901.log", AWS_LOG),
        ("unrelated.log", "Profile connected: other\n"),
    ]);
    let ctx = aws_context(dir.path(), Some(Duration::from_secs(600)));

    let any = vpn::run_probe(&probe(settings("aws", VpnKind::AwsVpn)), &ctx);
    assert_eq!(any.state, "up");
    // The profile list is exposed with whitespace made safe for the line format.
    assert_eq!(any.detail.as_deref(), Some("corp-us,lab_vpn"));

    let mut named = settings("aws", VpnKind::AwsVpn);
    named.profile = Some("corp-us".into());
    assert_eq!(vpn::run_probe(&probe(named.clone()), &ctx).state, "up");
    named.profile = Some("corp-eu".into());
    let reading = vpn::run_probe(&probe(named), &ctx);
    assert_eq!((reading.state.as_str(), reading.detail), ("off", None));
}

#[test]
fn aws_probe_is_off_without_a_log_or_with_a_stale_one() {
    let aws = probe(settings("aws", VpnKind::AwsVpn));

    let empty = aws_dir(&[]);
    assert_eq!(
        vpn::run_probe(&aws, &aws_context(empty.path(), None)).state,
        "off"
    );

    let missing = aws_context(Path::new("/nonexistent/aws-logs"), None);
    assert_eq!(vpn::run_probe(&aws, &missing).state, "off");

    // A log last written long ago means the client is not running.
    let dir = aws_dir(&[("aws_vpn_client_gui_1.log", "Profile connected: corp\n")]);
    let mut late = aws_context(dir.path(), Some(Duration::from_secs(600)));
    late.now = SystemTime::now() + Duration::from_secs(3600);
    assert_eq!(vpn::run_probe(&aws, &late).state, "off");
    // Without an age limit the same log counts.
    assert_eq!(
        vpn::run_probe(&aws, &aws_context(dir.path(), None)).state,
        "up"
    );
}

#[test]
fn aws_collect_writes_the_profile_key_only_while_connected() {
    let dir = aws_dir(&[("aws_vpn_client_gui_1.log", "Profile connected: corp\n")]);
    let cache = tempfile::tempdir().unwrap();
    let probes = [probe(settings("aws", VpnKind::AwsVpn))];
    let entries = vpn::collect(
        &probes,
        &aws_context(dir.path(), None),
        &cache.path().join("vpn"),
    );
    assert_eq!(get(&entries, "aws"), Some("up"));
    assert_eq!(get(&entries, "aws_profile"), Some("corp"));

    let off_dir = aws_dir(&[]);
    let off_cache = tempfile::tempdir().unwrap();
    let entries = vpn::collect(
        &probes,
        &aws_context(off_dir.path(), None),
        &off_cache.path().join("vpn"),
    );
    assert_eq!(get(&entries, "aws"), Some("off"));
    assert_eq!(get(&entries, "aws_profile"), None);
}
