//! Where wezterminator keeps things.
//!
//! XDG on macOS and Linux (macOS included, deliberately: `~/.config`, not
//! `~/Library`), Known Folders on Windows, both through `etcetera`.
//!
//! ```text
//! <config>/wezterminator/            local layer: presets/, themes/, overrides.json, machine.json
//! <data>/wezterminator/fleet/        fleet layer: a git clone, same layout
//! <data>/wezterminator/art/<theme>/<W>x<H>/   user-generated art
//! <state>/wezterminator/state.json   engine state (watched by WezTerm)
//! <state>/wezterminator/screens.json recorded screens (not watched)
//! <state>/wezterminator/stats        one-line stats cache
//! ```
//!
//! State and the stats cache stay out of the config tree on purpose: the
//! replace-mode shim lives in `~/.config/wezterm/`, which WezTerm watches
//! wholesale, so anything churning there would reload every window.
//!
//! The built-in layer is not here. Its presets and themes ship inside the
//! plugin directory the Lua engine records in `state.json`; see
//! [`builtin_dir`].

use std::path::{Path, PathBuf};

use etcetera::BaseStrategy;

use crate::error::{Error, Result};

/// Directory name used under every XDG root.
pub const APP_DIR: &str = "wezterminator";

/// Relative layout shared by every layer (built-in, fleet and local).
pub const PRESETS_DIR: &str = "presets";
pub const THEMES_DIR: &str = "themes";
pub const THEME_FILE: &str = "theme.json";
pub const OVERRIDES_FILE: &str = "overrides.json";
pub const MACHINE_FILE: &str = "machine.json";

pub const STATE_FILE: &str = "state.json";
pub const SCREENS_FILE: &str = "screens.json";
pub const STATS_FILE: &str = "stats";

/// The three per-user roots, each already ending in `wezterminator`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    config: PathBuf,
    data: PathBuf,
    state: PathBuf,
}

impl Paths {
    /// Resolve the roots for the current user from the platform conventions
    /// and the `XDG_*` environment variables.
    pub fn discover() -> Result<Self> {
        let strategy = etcetera::choose_base_strategy().map_err(|_| Error::NoHomeDir)?;
        let data = strategy.data_dir();
        // Windows has no state directory; keep state beside the data.
        let state = strategy.state_dir().unwrap_or_else(|| data.join("state"));
        Ok(Self::from_roots(strategy.config_dir(), data, state))
    }

    /// Build from explicit base directories (the `wezterminator` component is
    /// appended). Tests use this with a temporary home.
    pub fn from_roots(config: PathBuf, data: PathBuf, state: PathBuf) -> Self {
        Paths {
            config: config.join(APP_DIR),
            data: data.join(APP_DIR),
            state: state.join(APP_DIR),
        }
    }

    /// The local layer: the user's own presets, themes, overrides and machine
    /// settings.
    pub fn local_layer_dir(&self) -> &Path {
        &self.config
    }

    /// The fleet layer, a git clone managed by `wezterminator fleet`.
    pub fn fleet_layer_dir(&self) -> PathBuf {
        self.data.join("fleet")
    }

    /// Engine state directory.
    pub fn state_dir(&self) -> &Path {
        &self.state
    }

    pub fn state_file(&self) -> PathBuf {
        self.state.join(STATE_FILE)
    }

    pub fn screens_file(&self) -> PathBuf {
        self.state.join(SCREENS_FILE)
    }

    pub fn stats_file(&self) -> PathBuf {
        self.state.join(STATS_FILE)
    }

    /// Root of user-generated art.
    pub fn art_root(&self) -> PathBuf {
        self.data.join("art")
    }

    /// Art for one theme at one device resolution.
    pub fn art_dir(&self, theme_slug: &str, width: u64, height: u64) -> PathBuf {
        self.art_root()
            .join(theme_slug)
            .join(format!("{width}x{height}"))
    }
}

/// Where the built-in layer's presets and themes are read from.
///
/// An explicit `checkout` (running against a local clone) wins. Otherwise it
/// is the plugin directory the Lua engine recorded in `state.json` (see
/// [`crate::model::State::plugin_dir`]), which is the directory the running engine reads its
/// built-ins from. `None` means neither is known yet, for example before
/// WezTerm has ever loaded the engine.
pub fn builtin_dir(checkout: Option<&Path>, recorded_plugin_dir: Option<&str>) -> Option<PathBuf> {
    checkout
        .map(Path::to_path_buf)
        .or_else(|| recorded_plugin_dir.map(PathBuf::from))
}

/// Shipped (pre-generated) art inside a layer directory:
/// `<layer>/themes/<theme>/art/<W>x<H>/`.
pub fn shipped_art_dir(layer_dir: &Path, theme_slug: &str, width: u64, height: u64) -> PathBuf {
    layer_dir
        .join(THEMES_DIR)
        .join(theme_slug)
        .join("art")
        .join(format!("{width}x{height}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    #[test]
    fn layout_is_rooted_under_wezterminator() {
        let p = Paths::from_roots("/c".into(), "/d".into(), "/s".into());
        assert_eq!(p.local_layer_dir(), Path::new("/c/wezterminator"));
        assert_eq!(p.fleet_layer_dir(), Path::new("/d/wezterminator/fleet"));
        assert_eq!(p.state_file(), Path::new("/s/wezterminator/state.json"));
        assert_eq!(p.screens_file(), Path::new("/s/wezterminator/screens.json"));
        assert_eq!(
            p.art_dir("abyssal", 6016, 3384),
            Path::new("/d/wezterminator/art/abyssal/6016x3384")
        );
    }

    #[test]
    fn state_and_stats_stay_out_of_the_config_tree() {
        let p = Paths::from_roots("/c".into(), "/d".into(), "/s".into());
        assert!(!p.state_file().starts_with("/c"));
        assert!(!p.stats_file().starts_with("/c"));
    }

    #[test]
    fn checkout_beats_the_recorded_plugin_dir() {
        let state: State = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "active_preset": "builtin:x",
            "history": [],
            "engine": {"plugin_dir": "/plugin", "version": "0.1.0", "schema_version": 1}
        }))
        .unwrap();
        assert_eq!(
            builtin_dir(Some(Path::new("/clone")), state.plugin_dir()),
            Some(PathBuf::from("/clone"))
        );
        assert_eq!(
            builtin_dir(None, state.plugin_dir()),
            Some(PathBuf::from("/plugin"))
        );
        assert_eq!(builtin_dir(None, None), None);
    }
}
