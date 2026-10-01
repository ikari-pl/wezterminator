//! Linux `/proc` and macOS `sysctl` / `vm_stat` parsing, from fixtures.
//! Both parsers compile on every host, so both run in every CI job.

use std::fs;

use wzt_stats::Pressure;
use wzt_stats::linux::{self, MemInfo};
use wzt_stats::macos::{self, VmStat};

const MEMINFO: &str = "\
MemTotal:       16384000 kB
MemFree:         1024000 kB
MemAvailable:    6144000 kB
Buffers:          512000 kB
Cached:          4096000 kB
SwapTotal:             0 kB
";

const PSI: &str = "\
some avg10=6.25 avg60=3.10 avg300=1.00 total=123456
full avg10=0.00 avg60=0.00 avg300=0.00 total=0
";

fn proc_tree(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in files {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    dir
}

#[test]
fn linux_fixture_tree_parses() {
    let dir = proc_tree(&[
        ("loadavg", "0.52 0.58 0.59 1/467 12345\n"),
        ("meminfo", MEMINFO),
        ("pressure/memory", PSI),
    ]);
    let m = linux::collect_from(dir.path());
    assert_eq!(m.load, Some(0.52));
    // 16384000 kB total is 15.625 GiB; used is total - available = 10240000 kB.
    assert!((m.mem_total_gib.unwrap() - 15.625).abs() < 1e-9);
    assert!((m.mem_used_gib.unwrap() - 9.765625).abs() < 1e-9);
    // 6.25% stalled over 10 s is above the warning threshold, below critical.
    assert_eq!(m.pressure, Some(Pressure::Level(2)));
    assert!(m.ncpu.is_some());
}

#[test]
fn linux_without_psi_reports_pressure_na() {
    let dir = proc_tree(&[("loadavg", "1.00 1.00 1.00 1/1 1\n"), ("meminfo", MEMINFO)]);
    let m = linux::collect_from(dir.path());
    assert_eq!(m.pressure, Some(Pressure::Unavailable));
    let mut sample = wzt_stats::Sample::new();
    m.write_into(&mut sample);
    assert_eq!(sample.get("pressure"), Some("na"));
    assert_eq!(sample.get("load"), Some("1.00"));
}

#[test]
fn linux_garbage_psi_is_na_not_a_failure() {
    let dir = proc_tree(&[("pressure/memory", "not psi\n")]);
    assert_eq!(
        linux::collect_from(dir.path()).pressure,
        Some(Pressure::Unavailable)
    );
}

#[test]
fn linux_missing_files_leave_keys_out() {
    let dir = proc_tree(&[]);
    let m = linux::collect_from(dir.path());
    assert_eq!(
        (m.load, m.mem_used_gib, m.mem_total_gib),
        (None, None, None)
    );
}

#[test]
fn linux_pressure_levels_follow_the_thresholds() {
    assert_eq!(linux::pressure_level(0.0), 1);
    assert_eq!(linux::pressure_level(4.99), 1);
    assert_eq!(linux::pressure_level(5.0), 2);
    assert_eq!(linux::pressure_level(19.99), 2);
    assert_eq!(linux::pressure_level(20.0), 4);
}

#[test]
fn linux_old_kernel_falls_back_without_memavailable() {
    let text = "MemTotal: 1000 kB\nMemFree: 100 kB\nBuffers: 50 kB\nCached: 150 kB\n";
    assert_eq!(
        linux::parse_meminfo(text),
        Some(MemInfo {
            total_kb: 1000,
            available_kb: 300
        })
    );
    assert_eq!(linux::parse_meminfo("MemFree: 1 kB\n"), None);
}

const VM_STAT: &str = "\
Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                               3412.
Pages active:                           500000.
Pages inactive:                         400000.
Pages speculative:                        1000.
Pages purgeable:                         10000.
Anonymous pages:                       2000000.
Pages wired down:                       300000.
Pages occupied by compressor:           100000.
";

#[test]
fn macos_memory_matches_activity_monitor() {
    let vm = macos::parse_vm_stat(VM_STAT).unwrap();
    assert_eq!(
        vm,
        VmStat {
            page_size: 16384,
            anonymous: 2_000_000,
            purgeable: 10_000,
            wired: 300_000,
            compressor: 100_000
        }
    );
    // (anonymous - purgeable + wired + compressed) * page size
    assert_eq!(
        vm.used_bytes(),
        (2_000_000 - 10_000 + 300_000 + 100_000) * 16384
    );
}

#[test]
fn macos_vm_stat_missing_a_counter_is_none() {
    assert_eq!(macos::parse_vm_stat("Pages free: 1.\n"), None);
}

#[test]
fn macos_sysctl_values_parse() {
    assert_eq!(macos::parse_loadavg("{ 12.99 11.50 10.20 }\n"), Some(12.99));
    assert_eq!(macos::parse_loadavg("garbage"), None);
    assert_eq!(
        macos::parse_memsize("137438953472\n"),
        Some(137_438_953_472)
    );
    assert_eq!(macos::parse_pressure("4\n"), Some(4));
    assert_eq!(macos::parse_pressure(""), None);
}
