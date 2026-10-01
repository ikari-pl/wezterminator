//! The `schema_version` gate.
//!
//! The engine gates every document on this integer before reading any other
//! field, so an unknown future shape can never crash resolution.

use serde_json::Value;

/// Highest document `schema_version` this build understands.
pub const SUPPORTED_SCHEMA_VERSION: u64 = 1;

/// Why a document failed the gate.
#[derive(Debug, Clone, PartialEq)]
pub enum VersionProblem {
    /// Missing, not an integer, or below 1. Carries the offending value
    /// (`null` when the key is absent or the document is not an object).
    Invalid(Value),
    /// A valid integer above what the engine supports.
    Unsupported(u64),
}

/// JSON Schema counts `1.0` as an integer, and the Lua gate accepts it too
/// (`v == math.floor(v)`), so a float with no fractional part is a version.
fn integral_u64(n: &serde_json::Number) -> Option<u64> {
    let f = n.as_f64()?;
    // 2^53 is the largest range where every integer is exactly representable.
    (f.fract() == 0.0 && (0.0..=9_007_199_254_740_992.0).contains(&f)).then_some(f as u64)
}

/// Check `document["schema_version"]` against `supported`.
pub fn check_schema_version(document: &Value, supported: u64) -> Result<u64, VersionProblem> {
    let found = document.get("schema_version").unwrap_or(&Value::Null);
    let version = match found {
        Value::Number(n) => n.as_u64().or_else(|| integral_u64(n)).filter(|v| *v >= 1),
        _ => None,
    };
    match version {
        None => Err(VersionProblem::Invalid(found.clone())),
        Some(v) if v > supported => Err(VersionProblem::Unsupported(v)),
        Some(v) => Ok(v),
    }
}
