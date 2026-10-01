//! Fleet attach / promote / pull scenarios (U16, AE2).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use wzt_model::{
    Paths, Preset, StatusStyle, SUPPORTED_SCHEMA_VERSION, read_document, write_document,
};
use wzt_ops::{
    SystemRunner, attach, export_bundle, promote, pull, push_fleet, Denylist,
};

struct GitEnv {
    _tmp: TempDir,
    bare: PathBuf,
    paths_a: Paths,
    paths_b: Paths,
}

impl GitEnv {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let bare = tmp.path().join("fleet.git");
        fs::create_dir_all(&bare).unwrap();
        run_git(None, &["init", "--bare", bare.to_str().unwrap()]);

        // Seed the bare repo with an empty fleet layout commit.
        let seed = tmp.path().join("seed");
        run_git(None, &["clone", bare.to_str().unwrap(), seed.to_str().unwrap()]);
        configure_identity(&seed);
        fs::create_dir_all(seed.join("presets")).unwrap();
        fs::write(seed.join("presets").join(".gitkeep"), "").unwrap();
        run_git(Some(&seed), &["add", "."]);
        run_git(Some(&seed), &["commit", "-m", "init fleet"]);
        run_git(Some(&seed), &["push", "origin", "HEAD:master"]);
        // Ensure HEAD points at master for clones that expect a default branch.
        run_git(Some(&bare), &["symbolic-ref", "HEAD", "refs/heads/master"]);

        let home_a = tmp.path().join("a");
        let home_b = tmp.path().join("b");
        let paths_a = paths_for(&home_a);
        let paths_b = paths_for(&home_b);
        Self {
            _tmp: tmp,
            bare,
            paths_a,
            paths_b,
        }
    }

    fn bare_url(&self) -> String {
        self.bare.to_string_lossy().into_owned()
    }
}

fn paths_for(home: &Path) -> Paths {
    let config = home.join(".config");
    let data = home.join(".local").join("share");
    let state = home.join(".local").join("state");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    fs::create_dir_all(&state).unwrap();
    Paths::from_roots(config, data, state)
}

fn configure_identity(repo: &Path) {
    run_git(Some(repo), &["config", "user.email", "wzt-test@example.com"]);
    run_git(Some(repo), &["config", "user.name", "wzt-test"]);
}

fn run_git(cwd: Option<&Path>, args: &[&str]) {
    let mut cmd = Command::new("git");
    cmd.args(args).env("GIT_TERMINAL_PROMPT", "0");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output().expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
}

fn sample_local_preset(paths: &Paths, slug: &str, name: &str) -> PathBuf {
    let dir = paths.local_layer_dir().join("presets");
    fs::create_dir_all(&dir).unwrap();
    // Minimal but valid preset: reuse cpc-cool shape via JSON.
    let body = serde_json::json!({
        "schema_version": 1,
        "id": format!("local:{slug}"),
        "name": name,
        "based_on": "builtin:cpc-cool",
        "parts": {
            "art": { "theme": "builtin:cpc-cool" },
            "scheme": { "theme": "builtin:cpc-cool" },
            "palette": { "theme": "builtin:cpc-cool" },
            "font": {
                "preferred": ["Menlo"],
                "fallback": ["monospace"],
                "size": 14.0
            },
            "chrome": { "opacity": 1.0 },
            "status": { "style": "pill", "segments": ["cwd"] },
            "motion": {}
        }
    });
    let preset: Preset = serde_json::from_value(body).unwrap();
    let path = dir.join(format!("{slug}.json"));
    write_document(&path, &preset).unwrap();
    path
}

#[test]
fn ae2_promote_on_a_push_bare_pull_on_b_local_remains() {
    let env = GitEnv::new();
    let runner = SystemRunner;

    // Both machines attach before the promotion so B must pull to see it.
    attach(&env.paths_a, &env.bare_url(), &runner).unwrap();
    configure_identity(&env.paths_a.fleet_layer_dir());
    attach(&env.paths_b, &env.bare_url(), &runner).unwrap();
    configure_identity(&env.paths_b.fleet_layer_dir());

    let local_path = sample_local_preset(&env.paths_a, "cool-pills", "Cool Pills");
    let promote = promote(&env.paths_a, "cool-pills", &runner).unwrap();
    assert_eq!(promote.fleet_id, "fleet:cool-pills");
    assert!(promote.fleet_path.is_file());
    assert!(local_path.is_file(), "promote must copy, not move");

    let local_still: Preset = read_document(&local_path).unwrap();
    assert_eq!(local_still.id, "local:cool-pills");
    assert_eq!(local_still.parts.status.style, Some(StatusStyle::Pill));

    push_fleet(&env.paths_a, &runner).unwrap();

    assert!(
        !env.paths_b
            .fleet_layer_dir()
            .join("presets")
            .join("cool-pills.json")
            .is_file(),
        "B must not see the preset before pull"
    );

    fs::create_dir_all(env.paths_b.state_dir()).unwrap();
    fs::write(
        env.paths_b.state_file(),
        r#"{"schema_version":1,"active_preset":"builtin:cpc-cool","history":[]}"#,
    )
    .unwrap();
    let pulled = pull(&env.paths_b, &runner).unwrap();
    assert!(pulled.touched_state);

    let on_b = env
        .paths_b
        .fleet_layer_dir()
        .join("presets")
        .join("cool-pills.json");
    assert!(on_b.is_file(), "fleet preset must resolve on B after pull");
    let fleet_preset: Preset = read_document(&on_b).unwrap();
    assert_eq!(fleet_preset.id, "fleet:cool-pills");
    assert_eq!(fleet_preset.parts.status.style, Some(StatusStyle::Pill));
    assert_eq!(fleet_preset.schema_version, SUPPORTED_SCHEMA_VERSION);
}

#[test]
fn diverged_pull_refuses_and_leaves_trees_untouched() {
    let env = GitEnv::new();
    let runner = SystemRunner;

    attach(&env.paths_a, &env.bare_url(), &runner).unwrap();
    configure_identity(&env.paths_a.fleet_layer_dir());
    attach(&env.paths_b, &env.bare_url(), &runner).unwrap();
    configure_identity(&env.paths_b.fleet_layer_dir());

    // Divergent commits on A and B.
    fs::write(
        env.paths_a.fleet_layer_dir().join("presets").join("a-only.json"),
        "{}\n",
    )
    .unwrap();
    // Invalid JSON is fine for divergence test — we only care about git trees.
    // Use a valid-enough text file that is not a preset.
    fs::write(
        env.paths_a.fleet_layer_dir().join("from-a.txt"),
        "a\n",
    )
    .unwrap();
    let _ = fs::remove_file(env.paths_a.fleet_layer_dir().join("presets").join("a-only.json"));
    run_git(
        Some(&env.paths_a.fleet_layer_dir()),
        &["add", "from-a.txt"],
    );
    run_git(
        Some(&env.paths_a.fleet_layer_dir()),
        &["commit", "-m", "from a"],
    );

    fs::write(
        env.paths_b.fleet_layer_dir().join("from-b.txt"),
        "b\n",
    )
    .unwrap();
    run_git(
        Some(&env.paths_b.fleet_layer_dir()),
        &["add", "from-b.txt"],
    );
    run_git(
        Some(&env.paths_b.fleet_layer_dir()),
        &["commit", "-m", "from b"],
    );

    // Push A's commit so B's upstream moves... actually for divergence we need
    // both to have published different tips. Push A first, then B cannot ff-push;
    // for pull on B: fetch A's tip while B has its own commit → diverge.
    push_fleet(&env.paths_a, &runner).unwrap();

    let before_b = fs::read_to_string(env.paths_b.fleet_layer_dir().join("from-b.txt")).unwrap();
    let before_a_missing = env.paths_b.fleet_layer_dir().join("from-a.txt");
    assert!(!before_a_missing.exists());

    let err = pull(&env.paths_b, &runner).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("diverged") || msg.contains("ff-only") || msg.contains("refused"),
        "unexpected error: {msg}"
    );

    // Trees untouched: B still has from-b, still lacks from-a.
    assert_eq!(
        fs::read_to_string(env.paths_b.fleet_layer_dir().join("from-b.txt")).unwrap(),
        before_b
    );
    assert!(!env.paths_b.fleet_layer_dir().join("from-a.txt").exists());
}

#[test]
fn export_strips_hostname_from_machine_settings() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let paths = paths_for(&home);
    sample_local_preset(&paths, "cool-pills", "Cool Pills");

    // Machine settings with a hostname — must never appear in the export bundle.
    let machine = serde_json::json!({
        "schema_version": 1,
        "project_roots": ["/Users/ikari/src"],
        "_": "hostname of this box is metis",
        "push_targets": {
            "od-cezar": {
                "hosts": ["metis.local", "192.168.1.10"],
                "user": "ikari"
            }
        }
    });
    write_document(
        &paths.local_layer_dir().join("machine.json"),
        &machine,
    )
    .unwrap();

    let out = tmp.path().join("bundle");
    // Denylist uses a different hostname so the preset itself is allowed;
    // the assertion is that machine.json content is absent from the bundle.
    let denylist = Denylist::from_hostname("not-this-host");
    let report = export_bundle(&paths, "cool-pills", &out, &denylist, None).unwrap();
    assert!(report.preset_path.is_file());
    assert!(!out.join("machine.json").exists());

    let preset_text = fs::read_to_string(&report.preset_path).unwrap();
    assert!(!preset_text.contains("metis"));
    assert!(!preset_text.contains("192.168.1.10"));
    assert!(!preset_text.contains("ikari"));
    assert!(!preset_text.contains("project_roots"));

    // If a comment on the preset itself carries the hostname, export refuses.
    let mut leaked: Preset = read_document(&report.preset_path).unwrap();
    // Write a local preset with a hostname in a comment and re-export.
    leaked.id = "local:cool-pills".into();
    leaked.comments.insert("_", "built on metis");
    write_document(
        &paths.local_layer_dir().join("presets").join("cool-pills.json"),
        &leaked,
    )
    .unwrap();
    let denylist = Denylist::from_hostname("metis");
    let err = export_bundle(&paths, "cool-pills", &out.join("bad"), &denylist, None).unwrap_err();
    assert!(err.to_string().contains("personal data") || err.to_string().contains("hostname"));
}
