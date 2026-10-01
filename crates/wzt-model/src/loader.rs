//! Reading the layer directories into [`LayerInput`] for [`crate::resolve`].
//!
//! Documents are loaded as raw JSON in the order the data model fixes (files
//! sorted by path) and handed to resolution untouched, so the version gate and
//! comment stripping happen in exactly one place. A file that cannot be read
//! or is not JSON is skipped and reported as a [`LoadWarning`]; it never stops
//! the other documents from loading.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::Error;
use crate::io::read_value;
use crate::paths::{
    MACHINE_FILE, OVERRIDES_FILE, PRESETS_DIR, Paths, THEME_FILE, THEMES_DIR, builtin_dir,
};
use crate::resolve::{LayerInput, LayersInput};

/// A file that was skipped, and why.
#[derive(Debug)]
pub struct LoadWarning {
    pub path: PathBuf,
    pub error: Error,
}

/// Everything resolution needs from disk.
#[derive(Debug, Default)]
pub struct Loaded {
    pub layers: LayersInput,
    /// The raw state document, or `None` when `state.json` does not exist.
    pub state: Option<Value>,
    pub warnings: Vec<LoadWarning>,
}

/// Load the built-in, fleet and local layers and the state document.
///
/// The built-in layer comes from `checkout` when given (running against a
/// local clone), otherwise from the plugin directory the Lua engine recorded
/// in `state.json`. Without either it is empty. Built-in overrides and machine
/// settings are never read, because that layer ships presets and themes only.
pub fn load(paths: &Paths, checkout: Option<&Path>) -> Loaded {
    let mut warnings = Vec::new();

    let state = read_optional(&paths.state_file(), &mut warnings);
    // Read the recorded directory from the raw document so that a state file
    // from a newer engine still tells us where the built-ins are.
    let recorded = state
        .as_ref()
        .and_then(|s| s.pointer("/engine/plugin_dir"))
        .and_then(Value::as_str);

    let builtin = builtin_dir(checkout, recorded)
        .map(|dir| load_layer(&dir, false, &mut warnings));
    let fleet = load_layer(&paths.fleet_layer_dir(), true, &mut warnings);
    let local = load_layer(paths.local_layer_dir(), true, &mut warnings);

    Loaded {
        layers: LayersInput {
            builtin,
            fleet: Some(fleet),
            local: Some(local),
        },
        state,
        warnings,
    }
}

/// Load one layer directory. A missing directory is an empty layer.
///
/// `user_documents` says whether `overrides.json` and `machine.json` are read.
pub fn load_layer(dir: &Path, user_documents: bool, warnings: &mut Vec<LoadWarning>) -> LayerInput {
    let presets = sorted_files(&dir.join(PRESETS_DIR), |p| {
        (p.extension().is_some_and(|e| e == "json")).then(|| p.to_path_buf())
    });
    let themes = sorted_files(&dir.join(THEMES_DIR), |p| {
        let file = p.join(THEME_FILE);
        file.is_file().then_some(file)
    });

    let mut read_all = |files: Vec<PathBuf>| -> Vec<Value> {
        files
            .iter()
            .filter_map(|file| read_or_warn(file, warnings))
            .collect()
    };
    let presets = read_all(presets);
    let themes = read_all(themes);

    let (overrides, machine) = if user_documents {
        (
            read_optional(&dir.join(OVERRIDES_FILE), warnings),
            read_optional(&dir.join(MACHINE_FILE), warnings),
        )
    } else {
        (None, None)
    };

    LayerInput {
        presets,
        themes,
        overrides,
        machine,
    }
}

/// Entries of `dir` mapped through `pick`, sorted by path.
fn sorted_files(dir: &Path, pick: impl Fn(&Path) -> Option<PathBuf>) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| pick(&entry.path()))
        .collect();
    files.sort();
    files
}

fn read_or_warn(path: &Path, warnings: &mut Vec<LoadWarning>) -> Option<Value> {
    match read_value(path) {
        Ok(value) => Some(value),
        Err(error) => {
            warnings.push(LoadWarning {
                path: path.to_path_buf(),
                error,
            });
            None
        }
    }
}

/// Like [`read_or_warn`], but a file that does not exist is not a warning.
fn read_optional(path: &Path, warnings: &mut Vec<LoadWarning>) -> Option<Value> {
    if !path.exists() {
        return None;
    }
    read_or_warn(path, warnings)
}
