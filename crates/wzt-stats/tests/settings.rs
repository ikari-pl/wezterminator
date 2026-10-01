//! Reading `vpn_probes` from machine settings.

use std::fs;

use wzt_stats::settings;

fn layer(json: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("machine.json"), json).unwrap();
    dir
}

fn ids(s: &settings::Settings) -> Vec<&str> {
    s.probes.iter().map(|p| p.id.as_str()).collect()
}

#[test]
fn no_machine_file_means_no_probes_and_no_complaints() {
    let empty = tempfile::tempdir().unwrap();
    let s = settings::load_layers(&[empty.path().to_path_buf(), empty.path().join("absent")]);
    assert!(s.probes.is_empty());
    assert!(s.warnings.is_empty());
}

#[test]
fn local_probes_replace_the_fleet_list() {
    let fleet = layer(
        r#"{"schema_version":1,"vpn_probes":[{"id":"ts","kind":"tailscale"},{"id":"warp","kind":"warp"}]}"#,
    );
    let local = layer(r#"{"schema_version":1,"vpn_probes":[{"id":"utun","kind":"interface"}]}"#);
    let s = settings::load_layers(&[fleet.path().to_path_buf(), local.path().to_path_buf()]);
    assert_eq!(ids(&s), ["utun"]);

    // A local file that says nothing about probes leaves the fleet list alone.
    let quiet = layer(r#"{"schema_version":1,"project_roots":["~/src"]}"#);
    let s = settings::load_layers(&[fleet.path().to_path_buf(), quiet.path().to_path_buf()]);
    assert_eq!(ids(&s), ["ts", "warp"]);

    // An empty local list clears it.
    let cleared = layer(r#"{"schema_version":1,"vpn_probes":[]}"#);
    let s = settings::load_layers(&[fleet.path().to_path_buf(), cleared.path().to_path_buf()]);
    assert!(s.probes.is_empty());
}

#[test]
fn comment_keys_are_accepted() {
    let dir = layer(
        r#"{"schema_version":1,"_":"note","vpn_probes":[{"_":"x","id":"ts","kind":"tailscale","ttl_seconds":10}]}"#,
    );
    let s = settings::load_layers(&[dir.path().to_path_buf()]);
    assert_eq!(ids(&s), ["ts"]);
    assert_eq!(s.probes[0].ttl_seconds, Some(10));
    assert!(s.warnings.is_empty());
}

#[test]
fn a_broken_file_costs_its_probes_not_the_run() {
    let good = layer(r#"{"schema_version":1,"vpn_probes":[{"id":"ts","kind":"tailscale"}]}"#);
    let broken = layer("{ not json");
    let future = layer(r#"{"schema_version":99,"vpn_probes":[{"id":"x","kind":"warp"}]}"#);
    let s = settings::load_layers(&[
        good.path().to_path_buf(),
        broken.path().to_path_buf(),
        future.path().to_path_buf(),
    ]);
    assert_eq!(ids(&s), ["ts"]);
    assert_eq!(s.warnings.len(), 2);
}

#[test]
fn one_bad_probe_does_not_drop_the_others() {
    let dir = layer(
        r#"{"schema_version":1,"vpn_probes":[{"id":"ts","kind":"tailscale"},{"id":"x","kind":"nope"},{"id":"warp","kind":"warp"}]}"#,
    );
    let s = settings::load_layers(&[dir.path().to_path_buf()]);
    assert_eq!(ids(&s), ["ts", "warp"]);
    assert_eq!(s.warnings.len(), 1);
}
