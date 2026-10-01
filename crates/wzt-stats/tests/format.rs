//! The line format: stable key order, omitted keys, atomic cache write.

use std::time::SystemTime;

use wzt_model::Paths;
use wzt_model::{VpnKind, VpnProbe};
use wzt_stats::vpn::{self, Context};
use wzt_stats::{Metrics, Pressure, Sample};

fn full_metrics() -> Metrics {
    Metrics {
        load: Some(12.99),
        ncpu: Some(16),
        mem_used_gib: Some(52.14),
        mem_total_gib: Some(128.0),
        pressure: Some(Pressure::Level(1)),
    }
}

fn render(metrics: &Metrics, extra: &[(&str, &str)]) -> String {
    let mut sample = Sample::new();
    metrics.write_into(&mut sample);
    for (key, value) in extra {
        sample.push(key, value);
    }
    sample.render()
}

#[test]
fn keys_come_out_in_a_stable_order() {
    assert_eq!(
        render(
            &full_metrics(),
            &[("ts", "up"), ("warp", "off"), ("utun", "0")]
        ),
        "load=12.99 ncpu=16 memused=52.1 memtotal=128.0 pressure=1 ts=up warp=off utun=0\n"
    );
}

#[test]
fn a_probe_without_an_answer_leaves_its_key_out() {
    let metrics = Metrics {
        load: Some(0.5),
        mem_used_gib: None,
        mem_total_gib: None,
        ..full_metrics()
    };
    assert_eq!(render(&metrics, &[]), "load=0.50 ncpu=16 pressure=1\n");
}

#[test]
fn pressure_with_no_signal_is_na() {
    let metrics = Metrics {
        pressure: Some(Pressure::Unavailable),
        ..Metrics::default()
    };
    assert_eq!(render(&metrics, &[]), "pressure=na\n");
}

#[test]
fn an_empty_sample_is_a_bare_newline() {
    assert_eq!(Sample::new().render(), "\n");
    assert_eq!(render(&Metrics::default(), &[]), "\n");
}

#[test]
fn values_cannot_break_the_line() {
    let mut sample = Sample::new();
    sample.push("aws_profile", "my profile=x\ny");
    sample.push("Bad Key", "1");
    sample.push("empty", "");
    assert_eq!(sample.render(), "aws_profile=my_profile_x_y\n");
}

#[test]
fn disabled_probes_write_no_keys() {
    let settings = |id: &str, enabled: Option<bool>| VpnProbe {
        id: id.to_owned(),
        kind: VpnKind::Command,
        enabled,
        label: None,
        ttl_seconds: None,
        timeout_ms: None,
        interface: None,
        command: Some(vec!["true".to_owned()]),
        profile: None,
        comments: Default::default(),
    };
    let probes = vpn::resolve_probes(&[
        settings("on", Some(true)),
        settings("off", Some(false)),
        settings("implicit", None),
        // Reusing a built-in key or an earlier probe's id is dropped.
        settings("load", None),
        settings("on", None),
        settings("Bad Id", None),
    ]);
    let ids: Vec<_> = probes.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["on", "implicit"]);
}

#[test]
fn the_cache_line_is_written_in_place_and_replaces_the_old_one() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::from_roots(
        home.path().join("config"),
        home.path().join("data"),
        home.path().join("state"),
    );
    let mut first = Sample::new();
    first.push("load", "1.00");
    wzt_stats::write_cache(&paths.stats_file(), &first).unwrap();
    assert_eq!(
        std::fs::read_to_string(paths.stats_file()).unwrap(),
        "load=1.00\n"
    );

    let mut second = Sample::new();
    second.push("load", "2.00");
    wzt_stats::write_cache(&paths.stats_file(), &second).unwrap();
    assert_eq!(
        std::fs::read_to_string(paths.stats_file()).unwrap(),
        "load=2.00\n"
    );

    // No temporary file is left behind in the state directory.
    let names: Vec<_> = std::fs::read_dir(paths.state_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["stats"]);
}

/// Windows emits no probe keys in v1; the same holds for any platform
/// without probes. On macOS and Linux the sample is never empty.
#[test]
fn collect_matches_the_platform() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = Context {
        now: SystemTime::now(),
        aws_log_dir: None,
        aws_max_age: None,
        tailscale_bin: None,
        warp_bin: None,
    };
    let sample = wzt_stats::collect(&[], &ctx, &dir.path().join("vpn"));
    if wzt_stats::probes_supported() {
        assert!(sample.get("load").is_some());
        assert!(sample.get("memtotal").is_some());
        assert!(sample.get("pressure").is_some());
    } else {
        assert!(sample.entries().is_empty());
        assert_eq!(sample.render(), "\n");
    }
}

#[test]
fn windows_module_reports_no_metrics() {
    assert_eq!(wzt_stats::windows::collect(), Metrics::default());
}
