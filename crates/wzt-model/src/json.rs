//! JSON helpers shared by resolution and the tests: comment stripping, the
//! merge rules from `docs/data-model.md`, and the fixtures' comparison rules.

use serde_json::Value;

use crate::comments::is_comment_key;

/// Remove every `_`-prefixed key from every object, recursively. This is the
/// first step of resolution and mirrors what `plugin/wzt/data.lua` does right
/// after decoding.
pub fn strip_comments(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, _| !is_comment_key(key));
            map.values_mut().for_each(strip_comments);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_comments),
        _ => {}
    }
}

/// `merge(base, over)` from the data model.
///
/// Two objects merge key by key, recursively, and a key only in `over` is
/// copied. Anything else replaces: scalars, arrays (an empty array clears the
/// list) and object-over-non-object. An empty object over an object is
/// therefore a no-op, and over nothing it creates an empty object.
pub fn merge_into(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(base), Value::Object(over)) => {
            for (key, over_value) in over {
                match base.get_mut(key) {
                    Some(base_value) => merge_into(base_value, over_value),
                    None => {
                        base.insert(key.clone(), over_value.clone());
                    }
                }
            }
        }
        (base, over) => *base = over.clone(),
    }
}

/// Merge for a `parts` object. Identical to [`merge_into`] except that
/// `scheme` is atomic: a layer that sets it replaces the whole part, so
/// `{theme}` and `{wezterm_scheme}` never combine.
pub fn merge_parts_into(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(base_map), Value::Object(over_map)) => {
            for (key, over_value) in over_map {
                match base_map.get_mut(key) {
                    Some(base_value) if key != "scheme" => merge_into(base_value, over_value),
                    _ => {
                        base_map.insert(key.clone(), over_value.clone());
                    }
                }
            }
        }
        (base, over) => merge_into(base, over),
    }
}

/// The comparison the fixtures define: object keys are unordered, array order
/// matters, an empty array never equals an empty object, and numbers compare
/// by value (so `1` equals `1.0`).
pub fn semantic_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => {
            match (a.as_i64(), b.as_i64(), a.as_u64(), b.as_u64()) {
                (Some(a), Some(b), ..) => a == b,
                (_, _, Some(a), Some(b)) => a == b,
                _ => a.as_f64() == b.as_f64(),
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| semantic_eq(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, av)| b.get(k).is_some_and(|bv| semantic_eq(av, bv)))
        }
        (a, b) => a == b,
    }
}

/// The JSON Pointer paths of every `_`-prefixed key in `value`. Empty means
/// the value is comment-free, which is what resolution output must be.
pub fn comment_key_paths(value: &Value) -> Vec<String> {
    fn walk(value: &Value, path: &mut String, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let len = path.len();
                    path.push('/');
                    path.push_str(&key.replace('~', "~0").replace('/', "~1"));
                    if is_comment_key(key) {
                        out.push(path.clone());
                    }
                    walk(child, path, out);
                    path.truncate(len);
                }
            }
            Value::Array(items) => {
                for (i, child) in items.iter().enumerate() {
                    let len = path.len();
                    path.push_str(&format!("/{i}"));
                    walk(child, path, out);
                    path.truncate(len);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(value, &mut String::new(), &mut out);
    out
}
