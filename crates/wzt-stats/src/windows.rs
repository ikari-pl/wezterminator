//! Windows: no probes in v1.
//!
//! The collector emits no probe keys here, so every status segment that needs
//! one hides. Windows parity is deferred (origin: Windows parity deferred);
//! the Lua-native segments (clock, cwd, battery, workspace) are unaffected.

use crate::Metrics;

/// Nothing is probed on Windows in v1.
pub fn collect() -> Metrics {
    Metrics::default()
}
