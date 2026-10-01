//! Behaviour of the typed model, atomic writes and the layer loader.

use std::fs;
use std::path::Path;

use serde_json::{Number, Value, json};
use tempfile::TempDir;
use wzt_model::Error;
use wzt_model::io::{read_document, write_atomic, write_document};
use wzt_model::json::{comment_key_paths, semantic_eq};
use wzt_model::loader;
use wzt_model::model::{Preset, State};
use wzt_model::resolve::{EngineSpec, ResolveInput, resolve};
use wzt_model::Paths;

fn commented_preset() -> Value {
    json!({
        "_": "A hand-written note about the whole preset.",
        "schema_version": 1,
        "id": "local:mine",
        "name": "Mine",
        "_based_on": "Snapshot of a built-in.",
        "based_on": "builtin:neon-night",
        "parts": {
            "_": "One choice per part.",
            "art": {"theme": "builtin:neon", "layer_tweaks": {"_": "Tweaks.", "stars": {"_": "Dimmer.", "opacity": 0.1}}},
            "scheme": {"_": "Atomic.", "theme": "builtin:neon"},
            "palette": {"theme": "builtin:neon"},
            "font": {
                "_": "Terminess first.",
                "preferred": ["Terminess Nerd Font"],
                "fallback": ["JetBrains Mono"],
                "size": 14,
                "corrections": {"_": "Per font.", "Terminess Nerd Font": 1.5}
            },
            "chrome": {"opacity": 1, "padding": {"_": "Roomy.", "left": 4, "right": 4}},
            "status": {"style": "sparkline", "segments": ["load", "clock"]},
            "motion": {"auto_scroll": {"_": "Off for now.", "enabled": false, "speed": 0}}
        }
    })
}

#[test]
fn unknown_keys_error_but_comment_keys_pass() {
    let mut doc = commented_preset();
    serde_json::from_value::<Preset>(doc.clone()).expect("comments are allowed at every depth");

    doc["parts"]["chrome"]["opacitty"] = json!(0.9);
    let err = serde_json::from_value::<Preset>(doc).unwrap_err().to_string();
    assert!(err.contains("opacitty"), "{err}");

    let mut doc = commented_preset();
    doc["notes"] = json!("looks like a comment but is not one");
    assert!(serde_json::from_value::<Preset>(doc).is_err());
}

#[test]
fn unknown_keys_error_inside_keyed_maps_too() {
    // `corrections` entries are font names, so a non-numeric value is wrong,
    // while a `_` key is a comment.
    let mut doc = commented_preset();
    doc["parts"]["font"]["corrections"]["Terminess Nerd Font"] = json!("big");
    assert!(serde_json::from_value::<Preset>(doc).is_err());
}

#[test]
fn editing_one_field_keeps_comments_on_untouched_objects() {
    let original = commented_preset();
    let mut preset: Preset = serde_json::from_value(original.clone()).unwrap();

    preset.parts.chrome.opacity = Some(Number::from_f64(0.7).unwrap());
    let edited = serde_json::to_value(&preset).unwrap();

    assert_eq!(edited["parts"]["chrome"]["opacity"], json!(0.7));
    let (mut before, mut after) = (comment_key_paths(&original), comment_key_paths(&edited));
    before.sort();
    after.sort();
    assert_eq!(before, after, "every comment survives, at the same path");
    assert_eq!(edited["_"], original["_"]);
    assert_eq!(edited["parts"]["font"]["corrections"]["_"], json!("Per font."));
    assert_eq!(edited["parts"]["art"]["layer_tweaks"]["stars"]["_"], json!("Dimmer."));

    // Nothing else moved.
    let mut expected = original;
    expected["parts"]["chrome"]["opacity"] = json!(0.7);
    assert!(semantic_eq(&edited, &expected));
}

#[test]
fn integers_and_floats_keep_their_form_through_a_round_trip() {
    let preset: Preset = serde_json::from_value(commented_preset()).unwrap();
    let out = serde_json::to_string(&preset).unwrap();
    assert!(out.contains(r#""opacity":1,"#) || out.contains(r#""opacity":1}"#), "{out}");
    assert!(out.contains(r#""size":14"#), "{out}");
    assert!(out.contains("1.5"), "{out}");
}

#[test]
fn based_on_null_is_written_and_nothing_else_is_null() {
    let mut doc = commented_preset();
    doc["based_on"] = Value::Null;
    let preset: Preset = serde_json::from_value(doc).unwrap();
    let out = serde_json::to_value(&preset).unwrap();
    assert_eq!(out["based_on"], Value::Null);
    assert_eq!(out.to_string().matches("null").count(), 1);
}

// ----- schema_version gate ---------------------------------------------------

fn write_json(path: &Path, value: &Value) {
    write_atomic(path, serde_json::to_string_pretty(value).unwrap().as_bytes()).unwrap();
}

#[test]
fn unknown_future_schema_version_is_a_typed_error_not_a_panic() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("preset.json");
    // The rest of this document is deliberately nonsense for version 1.
    write_json(&path, &json!({"schema_version": 2, "id": 7, "mystery": [1, 2, 3]}));

    match read_document::<Preset>(&path) {
        Err(Error::UnsupportedSchemaVersion { found: 2, supported: 1, .. }) => {}
        other => panic!("expected UnsupportedSchemaVersion, got {other:?}"),
    }
}

#[test]
fn missing_or_malformed_schema_version_is_a_typed_error() {
    let dir = TempDir::new().unwrap();
    for (name, version) in [("zero", json!(0)), ("string", json!("1")), ("float", json!(1.5)), ("null", Value::Null)] {
        let path = dir.path().join(format!("{name}.json"));
        write_json(&path, &json!({"schema_version": version}));
        assert!(
            matches!(read_document::<Preset>(&path), Err(Error::InvalidSchemaVersion { .. })),
            "{name}"
        );
    }
    let path = dir.path().join("absent.json");
    write_json(&path, &json!({"id": "local:x"}));
    assert!(matches!(read_document::<Preset>(&path), Err(Error::InvalidSchemaVersion { .. })));
}

#[test]
fn shape_and_syntax_errors_are_distinguished() {
    let dir = TempDir::new().unwrap();
    let broken = dir.path().join("broken.json");
    fs::write(&broken, "{ not json").unwrap();
    assert!(matches!(read_document::<Preset>(&broken), Err(Error::Json { .. })));

    let wrong = dir.path().join("wrong.json");
    write_json(&wrong, &json!({"schema_version": 1, "id": "local:x", "typo": true}));
    assert!(matches!(read_document::<Preset>(&wrong), Err(Error::Shape { .. })));

    assert!(matches!(
        read_document::<Preset>(&dir.path().join("nope.json")),
        Err(Error::Io { .. })
    ));
}

// ----- atomic writes -----------------------------------------------------------

#[test]
fn atomic_write_creates_parents_replaces_and_leaves_no_temp_files() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("deep/er/state.json");

    let state: State = serde_json::from_value(json!({
        "_": "kept", "schema_version": 1, "active_preset": "builtin:a", "history": []
    }))
    .unwrap();
    write_document(&path, &state).unwrap();
    assert_eq!(read_document::<State>(&path).unwrap(), state);
    assert!(fs::read_to_string(&path).unwrap().ends_with("}\n"));

    let mut newer = state.clone();
    newer.active_preset = "builtin:b".into();
    write_document(&path, &newer).unwrap();
    assert_eq!(read_document::<State>(&path).unwrap().active_preset, "builtin:b");

    let names: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["state.json"], "no stray temp files");
}

#[test]
fn failed_atomic_write_leaves_the_target_and_no_temp_file() {
    let dir = TempDir::new().unwrap();
    // The target is a directory, so the final rename must fail.
    let target = dir.path().join("target.json");
    fs::create_dir(&target).unwrap();
    assert!(write_atomic(&target, b"{}").is_err());

    assert!(target.is_dir());
    let leftovers: Vec<_> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(leftovers, ["target.json"]);
}

#[cfg(unix)]
#[test]
fn atomic_write_keeps_the_mode_of_the_file_it_replaces() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("a.json");
    write_atomic(&path, b"{}").unwrap();
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o644);

    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    write_atomic(&path, b"{ }").unwrap();
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
}

// ----- loading layers from disk and resolving -------------------------------

#[test]
fn built_ins_load_from_the_plugin_dir_recorded_in_state() {
    let fixture: Value = serde_json::from_str(
        &fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/resolution/resolve-basic.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let builtin = &fixture["input"]["layers"]["builtin"];

    let home = TempDir::new().unwrap();
    let paths = Paths::from_roots(
        home.path().join("config"),
        home.path().join("data"),
        home.path().join("state"),
    );

    // The built-in layer ships presets and themes in the plugin directory.
    let plugin = home.path().join("plugin-checkout");
    for preset in builtin["presets"].as_array().unwrap() {
        let slug = preset["id"].as_str().unwrap().strip_prefix("builtin:").unwrap();
        write_json(&plugin.join(format!("presets/{slug}.json")), preset);
    }
    for theme in builtin["themes"].as_array().unwrap() {
        let slug = theme["id"].as_str().unwrap().strip_prefix("builtin:").unwrap();
        write_json(&plugin.join(format!("themes/{slug}/theme.json")), theme);
    }
    // A built-in layer must never contribute machine settings.
    write_json(&plugin.join("machine.json"), &json!({"schema_version": 1, "editor": {"command": "evil"}}));

    // What the Lua engine would have recorded.
    write_json(
        &paths.state_file(),
        &json!({
            "schema_version": 1,
            "active_preset": "builtin:dusk-calm",
            "history": [],
            "engine": {"plugin_dir": plugin, "version": "0.1.0", "schema_version": 1}
        }),
    );
    // The local layer holds machine settings and one preset.
    write_json(
        &paths.local_layer_dir().join("machine.json"),
        &json!({"schema_version": 1, "project_roots": ["~/src"]}),
    );
    fs::write(paths.local_layer_dir().join("presets-not-here.txt"), "ignored").unwrap();
    fs::create_dir_all(paths.local_layer_dir().join("presets")).unwrap();
    fs::write(paths.local_layer_dir().join("presets/broken.json"), "{ nope").unwrap();

    let loaded = loader::load(&paths, None);
    assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
    assert!(loaded.warnings[0].path.ends_with("presets/broken.json"));

    let input = ResolveInput {
        engine: EngineSpec {
            supported_schema_version: 1,
            default_preset: Some("builtin:neon-night".into()),
        },
        layers: loaded.layers,
        state: loaded.state,
        environment: Default::default(),
        addon: None,
    };
    let out = resolve(&input).to_value();

    assert_eq!(out["active"], json!({"requested": "builtin:dusk-calm", "id": "builtin:dusk-calm", "fell_back": false}));
    assert_eq!(out["resolved"]["parts"]["scheme"], json!({"wezterm_scheme": "Gruvbox Dark"}));
    assert_eq!(out["resolved"]["parts"]["chrome"], json!({"opacity": 0.95, "blur": 10}));
    // Local machine settings only: the built-in machine.json was not read.
    assert_eq!(out["machine"], json!({"project_roots": ["~/src"]}));
    assert_eq!(out["ignored"], json!([]));
    let ids: Vec<_> = out["catalog"].as_array().unwrap().iter().map(|e| e["id"].clone()).collect();
    assert_eq!(ids, [json!("builtin:dusk-calm"), json!("builtin:neon-night")]);
}

#[test]
fn an_explicit_checkout_beats_the_recorded_plugin_dir() {
    let home = TempDir::new().unwrap();
    let paths = Paths::from_roots(home.path().join("c"), home.path().join("d"), home.path().join("s"));
    let (recorded, checkout) = (home.path().join("recorded"), home.path().join("checkout"));
    for dir in [&recorded, &checkout] {
        fs::create_dir_all(dir.join("presets")).unwrap();
    }
    write_json(&checkout.join("presets/only-here.json"), &json!({"schema_version": 1, "id": "builtin:only-here"}));
    write_json(
        &paths.state_file(),
        &json!({
            "schema_version": 1, "active_preset": "builtin:x", "history": [],
            "engine": {"plugin_dir": recorded, "version": "0.1.0", "schema_version": 1}
        }),
    );

    let loaded = loader::load(&paths, Some(&checkout));
    let builtin = loaded.layers.builtin.expect("built-in layer present");
    assert_eq!(builtin.presets.len(), 1);

    // With nothing recorded and no checkout, there is no built-in layer.
    fs::remove_file(paths.state_file()).unwrap();
    assert!(loader::load(&paths, None).layers.builtin.is_none());
}

#[test]
fn a_state_file_from_a_newer_engine_still_locates_the_built_ins() {
    let home = TempDir::new().unwrap();
    let paths = Paths::from_roots(home.path().join("c"), home.path().join("d"), home.path().join("s"));
    let plugin = home.path().join("plugin");
    fs::create_dir_all(plugin.join("presets")).unwrap();
    write_json(
        &paths.state_file(),
        &json!({"schema_version": 9, "engine": {"plugin_dir": plugin}, "future": true}),
    );
    let loaded = loader::load(&paths, None);
    assert!(loaded.layers.builtin.is_some());
    assert!(loaded.warnings.is_empty());
}
