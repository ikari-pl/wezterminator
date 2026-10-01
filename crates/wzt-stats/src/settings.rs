//! The slice of machine settings the collector needs: `vpn_probes`.
//!
//! Resolution merges fleet then local `machine.json`, and arrays replace, so
//! a layer that sets `vpn_probes` replaces the other layer's list wholesale.
//! Reading just this one field keeps the collector independent of the rest of
//! the resolution pipeline (no preset or theme is loaded on a 1 Hz path), and
//! a broken `machine.json` costs the VPN keys, never the load and memory ones.

use std::path::PathBuf;

use serde_json::Value;
use wzt_model::io::{parse_document, read_value};
use wzt_model::paths::MACHINE_FILE;
use wzt_model::{Paths, VpnProbe};

/// Probe definitions plus a note for everything that was skipped.
#[derive(Debug, Default)]
pub struct Settings {
    pub probes: Vec<VpnProbe>,
    pub warnings: Vec<String>,
}

/// Read the machine settings for the current user (fleet, then local).
pub fn load(paths: &Paths) -> Settings {
    load_layers(&[
        paths.fleet_layer_dir(),
        paths.local_layer_dir().to_path_buf(),
    ])
}

/// Read `machine.json` from each layer directory, lowest precedence first.
pub fn load_layers(layer_dirs: &[PathBuf]) -> Settings {
    let mut settings = Settings::default();
    let mut probes: Option<Value> = None;

    for dir in layer_dirs {
        let path = dir.join(MACHINE_FILE);
        if !path.exists() {
            continue;
        }
        let document = read_value(&path).and_then(|v| parse_document::<Value>(v, &path));
        match document {
            Ok(document) => {
                if let Some(list) = document.get("vpn_probes") {
                    probes = Some(list.clone());
                }
            }
            Err(error) => settings.warnings.push(error.to_string()),
        }
    }

    match probes {
        None => {}
        Some(Value::Array(items)) => {
            for item in items {
                match serde_json::from_value::<VpnProbe>(item) {
                    Ok(probe) => settings.probes.push(probe),
                    Err(error) => settings
                        .warnings
                        .push(format!("vpn_probes: skipped one probe: {error}")),
                }
            }
        }
        Some(_) => settings
            .warnings
            .push("vpn_probes: expected a list".to_owned()),
    }
    settings
}
