//! Runs the shared resolution fixtures in `tests/fixtures/resolution/` through
//! `wzt-model`. The Lua engine runs the same files, so a disagreement between
//! the two runtimes fails CI. Where they differ, the fixtures win.
//!
//! Also pushes every document the fixtures contain through the typed model and
//! back, which checks that `_` comments and empty arrays versus empty objects
//! survive a read and write.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use wzt_model::json::{comment_key_paths, semantic_eq, strip_comments};
use wzt_model::model::{Machine, Overrides, Preset, Screens, State, Theme};
use wzt_model::resolve::{ResolveInput, resolve};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution")
}

/// Every fixture file matching `<prefix>-*.json`, sorted, parsed.
fn fixtures(prefix: &str) -> Vec<(String, Value)> {
    let dir = fixture_dir();
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with(&format!("{prefix}-")) && n.ends_with(".json"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let path = dir.join(&name);
            let text = fs::read_to_string(&path).unwrap();
            let value = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("{}: not JSON: {e}", path.display()));
            (name, value)
        })
        .collect()
}

fn run_resolution(fixture: &Value) -> Value {
    let mut input = fixture["input"].clone();
    // Top-level `_` keys are fixture commentary; the library strips the ones
    // inside documents itself, exactly as the real loader does.
    if let Value::Object(map) = &mut input {
        map.retain(|k, _| !k.starts_with('_'));
    }
    let input: ResolveInput = serde_json::from_value(input).expect("fixture input shape");
    resolve(&input).to_value()
}

/// Check one resolution fixture and return what is wrong with it.
fn check_resolution(name: &str, fixture: &Value, output: &Value) -> Vec<String> {
    let mut failures = Vec::new();

    if fixture["name"] != name.trim_end_matches(".json") {
        failures.push(format!("`name` is {} but the file is {name}", fixture["name"]));
    }

    if let Some(expected) = fixture.get("expected").and_then(Value::as_object) {
        for (key, want) in expected {
            match output.get(key) {
                Some(got) if semantic_eq(got, want) => {}
                got => failures.push(format!(
                    "expected[{key}]\n  want: {want}\n  got:  {}",
                    got.map_or("<absent>".into(), Value::to_string)
                )),
            }
        }
    }
    if let Some(expect_at) = fixture.get("expect_at").and_then(Value::as_object) {
        for (pointer, want) in expect_at {
            match output.pointer(pointer) {
                Some(got) if semantic_eq(got, want) => {}
                got => failures.push(format!(
                    "expect_at[{pointer}]\n  want: {want}\n  got:  {}",
                    got.map_or("<absent>".into(), Value::to_string)
                )),
            }
        }
    }
    if let Some(absent) = fixture.get("expect_absent").and_then(Value::as_array) {
        for pointer in absent.iter().filter_map(Value::as_str) {
            if let Some(got) = output.pointer(pointer) {
                failures.push(format!("expect_absent[{pointer}] but found {got}"));
            }
        }
    }
    let comments = comment_key_paths(output);
    if !comments.is_empty() {
        failures.push(format!("output contains comment keys: {comments:?}"));
    }
    failures
}

#[test]
fn resolution_fixtures_match_expected_outputs() {
    let cases = fixtures("resolve");
    assert!(cases.len() >= 16, "expected the 16 resolve fixtures, found {}", cases.len());

    let mut report = Vec::new();
    for (name, fixture) in &cases {
        let output = run_resolution(fixture);
        let failures = check_resolution(name, fixture, &output);
        if !failures.is_empty() {
            report.push(format!("{name}:\n  {}", failures.join("\n  ")));
        }
    }
    assert!(report.is_empty(), "failing fixtures:\n{}", report.join("\n"));
}

/// The harness must be able to fail: wrong expectations and a stray comment
/// key in the output are both reported.
#[test]
fn harness_reports_wrong_expectations() {
    let cases = fixtures("resolve");
    let (name, fixture) = cases.iter().find(|(n, _)| n == "resolve-basic.json").unwrap();
    let output = run_resolution(fixture);
    assert_eq!(check_resolution(name, fixture, &output), Vec::<String>::new());

    let mut wrong = fixture.clone();
    wrong["expect_at"] = serde_json::json!({"/resolved/parts/chrome/opacity": 0.5});
    wrong["expect_absent"] = serde_json::json!(["/resolved/parts/status"]);
    assert_eq!(check_resolution(name, &wrong, &output).len(), 2);

    let mut leaky = output.clone();
    leaky["resolved"]["parts"]["chrome"]["_"] = "note".into();
    let failures = check_resolution(name, fixture, &leaky);
    assert!(
        failures.iter().any(|f| f.contains("comment keys")),
        "stray comment key not reported: {failures:?}"
    );
}

#[test]
fn comment_keys_twin_resolves_identically_to_basic() {
    let cases = fixtures("resolve");
    let output = |name: &str| {
        let (_, fixture) = cases.iter().find(|(n, _)| n == name).unwrap();
        run_resolution(fixture)
    };
    let basic = output("resolve-basic.json");
    let commented = output("resolve-comment-keys.json");
    assert!(
        semantic_eq(&basic, &commented),
        "comments changed the resolution\nbasic:     {basic}\ncommented: {commented}"
    );
}

#[test]
fn every_fixture_name_matches_its_file() {
    for prefix in ["resolve", "validate"] {
        for (file, fixture) in fixtures(prefix) {
            assert_eq!(fixture["name"], file.trim_end_matches(".json"), "{file}");
            let kind = if prefix == "resolve" { "resolution" } else { "validation" };
            assert_eq!(fixture["kind"], kind, "{file}");
        }
    }
}

// ---------------------------------------------------------------------------
// Round trips through the typed model
// ---------------------------------------------------------------------------

/// Parse `doc` as `T`, write it back, and require a semantically equal
/// document. Returns what is wrong, if anything.
fn round_trip<T: DeserializeOwned + Serialize>(label: &str, doc: &Value) -> Option<String> {
    let typed: T = match serde_json::from_value(doc.clone()) {
        Ok(t) => t,
        Err(e) => return Some(format!("{label}: does not parse: {e}")),
    };
    let back = serde_json::to_value(&typed).unwrap();
    if !semantic_eq(doc, &back) {
        return Some(format!("{label}: changed in the round trip\n  in:  {doc}\n  out: {back}"));
    }
    // Every comment key must survive at the same path.
    let (mut before, mut after) = (comment_key_paths(doc), comment_key_paths(&back));
    before.sort();
    after.sort();
    (before != after).then(|| format!("{label}: comment keys moved\n  in:  {before:?}\n  out: {after:?}"))
}

/// Round-trips one document and reports what is wrong, if anything.
type RoundTrip = fn(&str, &Value) -> Option<String>;

fn has_valid_version(doc: &Value) -> bool {
    doc.get("schema_version") == Some(&Value::from(1))
}

#[test]
fn documents_in_resolution_fixtures_round_trip_with_comments_and_empties() {
    let mut problems = Vec::new();
    let mut count = 0;
    for (name, fixture) in fixtures("resolve") {
        let layers = &fixture["input"]["layers"];
        for layer in ["builtin", "fleet", "local"] {
            let Some(docs) = layers[layer].as_object() else { continue };
            let kinds: [(&str, RoundTrip); 2] = [
                ("presets", |l, d| round_trip::<Preset>(l, d)),
                ("themes", |l, d| round_trip::<Theme>(l, d)),
            ];
            for (key, check) in kinds {
                for (i, doc) in docs[key].as_array().into_iter().flatten().enumerate() {
                    if has_valid_version(doc) {
                        count += 1;
                        problems.extend(check(&format!("{name} {layer}.{key}[{i}]"), doc));
                    }
                }
            }
            let singles: [(&str, RoundTrip); 2] = [
                ("overrides", |l, d| round_trip::<Overrides>(l, d)),
                ("machine", |l, d| round_trip::<Machine>(l, d)),
            ];
            for (key, check) in singles {
                if has_valid_version(&docs[key]) {
                    count += 1;
                    problems.extend(check(&format!("{name} {layer}.{key}"), &docs[key]));
                }
            }
        }
        let state = &fixture["input"]["state"];
        if has_valid_version(state) {
            count += 1;
            problems.extend(round_trip::<State>(&format!("{name} state"), state));
        }
    }
    assert!(count > 50, "round-tripped only {count} documents");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The typed model parses the validation fixtures the schemas accept, and
/// keeps them intact.
#[test]
fn valid_validation_fixtures_round_trip() {
    let mut checked = 0;
    for (name, fixture) in fixtures("validate") {
        if fixture["valid"] != true {
            continue;
        }
        let doc = &fixture["document"];
        let def = fixture.get("def").and_then(Value::as_str);
        let label = name.as_str();
        let problem = match (fixture["schema"].as_str().unwrap(), def) {
            ("schema/preset.schema.json", None) => round_trip::<Preset>(label, doc),
            ("schema/preset.schema.json", Some("overrides_document")) => round_trip::<Overrides>(label, doc),
            ("schema/theme.schema.json", None) => round_trip::<Theme>(label, doc),
            ("schema/machine.schema.json", None) => round_trip::<Machine>(label, doc),
            ("schema/state.schema.json", None) => round_trip::<State>(label, doc),
            ("schema/state.schema.json", Some("screens_document")) => round_trip::<Screens>(label, doc),
            other => panic!("{name}: unhandled schema {other:?}"),
        };
        assert_eq!(problem, None);
        checked += 1;
    }
    assert!(checked >= 6, "only {checked} valid fixtures checked");
}

/// The typed model rejects the invalid documents whose fault is structural.
/// The remaining invalid fixtures (id pattern, history cap, issue URL pattern,
/// required font fallback) are constraints only a schema validator checks.
#[test]
fn structurally_invalid_validation_fixtures_are_rejected() {
    let rejected = [
        "validate-theme-misspelled-key.json",
        "validate-theme-comment-without-underscore.json",
        "validate-preset-machine-key-leak.json",
        "validate-overrides-scheme-both-selectors.json",
    ];
    let cases = fixtures("validate");
    for name in rejected {
        let (_, fixture) = cases.iter().find(|(n, _)| n == name).expect(name);
        assert_eq!(fixture["valid"], false);
        let doc = fixture["document"].clone();
        let result = match fixture["schema"].as_str().unwrap() {
            "schema/theme.schema.json" => serde_json::from_value::<Theme>(doc).map(drop),
            "schema/preset.schema.json" if fixture.get("def").is_some() => {
                serde_json::from_value::<Overrides>(doc).map(drop)
            }
            "schema/preset.schema.json" => serde_json::from_value::<Preset>(doc).map(drop),
            other => panic!("{name}: unhandled schema {other}"),
        };
        assert!(result.is_err(), "{name} should not parse as a typed document");
    }
}

#[test]
fn stripping_comments_from_a_commented_fixture_leaves_valid_json() {
    let cases = fixtures("validate");
    let (_, fixture) = cases
        .iter()
        .find(|(n, _)| n == "validate-theme-comments-valid.json")
        .unwrap();
    let mut doc = fixture["document"].clone();
    assert!(!comment_key_paths(&doc).is_empty());
    strip_comments(&mut doc);
    assert!(comment_key_paths(&doc).is_empty());
    // And what remains is still a theme.
    serde_json::from_value::<Theme>(doc).expect("stripped theme parses");
}
