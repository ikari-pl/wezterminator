//! The shipped built-in presets and themes go through the typed model and back
//! unchanged (every `_` comment, empty arrays versus empty objects, number
//! forms), and resolve with no fleet or local layer present.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use wzt_model::json::{comment_key_paths, semantic_eq};
use wzt_model::model::{Preset, Theme};
use wzt_model::resolve::{EngineInput, LayerInput, Layers, ResolveInput, resolve};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn sorted(dir: &Path, pick: impl Fn(PathBuf) -> Option<PathBuf>) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| pick(e.ok()?.path()))
        .collect();
    files.sort();
    files
}

fn preset_files() -> Vec<PathBuf> {
    sorted(&repo_root().join("presets"), |p| {
        (p.extension().is_some_and(|e| e == "json")).then_some(p)
    })
}

fn theme_files() -> Vec<PathBuf> {
    sorted(&repo_root().join("themes"), |p| {
        let f = p.join("theme.json");
        f.is_file().then_some(f)
    })
}

/// Problems with one file, empty when it round-trips.
fn round_trip_problems<T: DeserializeOwned + Serialize>(path: &Path) -> Vec<String> {
    let original = read(path);
    let label = path.strip_prefix(repo_root()).unwrap_or(path).display();
    let typed: T = match serde_json::from_value(original.clone()) {
        Ok(t) => t,
        Err(e) => return vec![format!("{label}: does not parse: {e}")],
    };
    let back = serde_json::to_value(&typed).unwrap();
    let mut problems = Vec::new();
    if !semantic_eq(&original, &back) {
        problems.push(format!("{label}: changed in the round trip"));
    }
    let (mut before, mut after) = (comment_key_paths(&original), comment_key_paths(&back));
    before.sort();
    after.sort();
    if before != after {
        problems.push(format!("{label}: comment keys moved"));
    }
    problems
}

fn assert_all_round_trip<T: DeserializeOwned + Serialize>(files: &[PathBuf]) {
    let problems: Vec<String> = files.iter().flat_map(|f| round_trip_problems::<T>(f)).collect();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn built_in_presets_round_trip() {
    let files = preset_files();
    assert!(!files.is_empty(), "no built-in presets found");
    assert_all_round_trip::<Preset>(&files);
}

#[test]
fn built_in_themes_round_trip() {
    let files = theme_files();
    assert!(!files.is_empty(), "no built-in themes found");
    assert_all_round_trip::<Theme>(&files);
}

#[test]
fn every_built_in_preset_resolves_with_no_other_layer() {
    let presets: Vec<Value> = preset_files().iter().map(|f| read(f)).collect();
    let themes: Vec<Value> = theme_files().iter().map(|f| read(f)).collect();
    let ids: Vec<String> = presets
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_owned())
        .collect();

    for id in &ids {
        let input = ResolveInput {
            engine: EngineInput {
                supported_schema_version: 1,
                default_preset: Some(ids[0].clone()),
            },
            layers: Layers {
                builtin: Some(LayerInput {
                    presets: presets.clone(),
                    themes: themes.clone(),
                    overrides: None,
                    machine: None,
                }),
                fleet: None,
                local: None,
            },
            state: Some(serde_json::json!({
                "schema_version": 1, "active_preset": id, "history": []
            })),
            ..Default::default()
        };
        let out = resolve(&input).to_value();
        assert_eq!(out["error"], Value::Null, "{id}: {out}");
        assert_eq!(out["ignored"], serde_json::json!([]), "{id}");
        assert_eq!(out["notices"], serde_json::json!([]), "{id}");
        assert_eq!(out["resolved"]["id"], id.as_str(), "{id}");
        assert!(out["resolved"]["parts"]["art"]["fallback_layers"].is_array(), "{id}");
        assert!(comment_key_paths(&out).is_empty(), "{id}: comments leaked");
    }
}
