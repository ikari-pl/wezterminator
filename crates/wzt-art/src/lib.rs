//! The wezterminator art engine.
//!
//! A theme's `art.layers` is a recipe: a list of primitives with parameters
//! and an integer scale, bottom to top. This crate turns a recipe into one
//! indexed PNG per layer at a given device resolution, with no Python and no
//! GPL dependency.
//!
//! # How it stays exact
//!
//! * **Logical resolution, integer upscale.** Each layer is drawn at
//!   `device / scale` and written with nearest-neighbour upscaling, so every
//!   logical pixel is a hard-edged `scale x scale` block. A scale that does
//!   not divide the device size is rejected.
//! * **Per-pixel hash RNG** ([`rng`]). A random value is a pure function of
//!   `(seed, layer, x, y)`, never of draw order.
//! * **Palette first, pixels second.** Primitives register every colour they
//!   can emit before the parallel loop, so rows can be computed on any number
//!   of threads and the indices are identical.
//! * **Integer and fixed-point maths** in every primitive, so macOS and Linux
//!   agree to the bit.
//!
//! # Entry points
//!
//! * [`render_layer`] / [`render_theme`]: recipe to in-memory [`Layer`]s.
//! * [`generate_theme`]: render, run the legibility check, write PNGs and a
//!   manifest carrying the recipe hash.
//! * [`import::import_image`]: map a picture onto a palette in Oklab.

pub mod dither;
pub mod error;
pub mod import;
pub mod legibility;
pub mod palette;
pub mod primitives;
pub mod rng;
pub mod write;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wzt_model::{ArtLayer, Theme, ThemeArt};

pub use error::{ArtError, Result};
pub use legibility::{LegibilityInput, LegibilityReport, RenderedLayer};
pub use palette::{Layer, ThemeColors};

/// Device resolution in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Device {
    pub width: u32,
    pub height: u32,
}

impl Device {
    pub fn new(width: u64, height: u64) -> Result<Self> {
        match (u32::try_from(width), u32::try_from(height)) {
            (Ok(w), Ok(h)) if w > 0 && h > 0 => Ok(Device {
                width: w,
                height: h,
            }),
            _ => Err(ArtError::BadDevice { width, height }),
        }
    }
}

impl FromStr for Device {
    type Err = String;

    /// `WIDTHxHEIGHT`, for example `6016x3384`.
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        let bad = || format!("`{s}` is not WIDTHxHEIGHT, for example 3840x2160");
        let (w, h) = s.split_once(['x', 'X']).ok_or_else(bad)?;
        let (w, h) = (
            w.parse::<u64>().map_err(|_| bad())?,
            h.parse::<u64>().map_err(|_| bad())?,
        );
        Device::new(w, h).map_err(|e| e.to_string())
    }
}

impl std::fmt::Display for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

/// How to render and generate.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Worker threads. `None` uses rayon's global pool. The pixels are the
    /// same for every value.
    pub threads: Option<usize>,
    /// Directory relative `path` parameters (the `image` primitive) resolve
    /// against: the directory holding `theme.json`.
    pub theme_dir: Option<PathBuf>,
    /// Write art even if the legibility check fails.
    pub skip_legibility: bool,
}

fn in_pool<T: Send>(threads: Option<usize>, f: impl FnOnce() -> T + Send) -> Result<T> {
    match threads {
        None => Ok(f()),
        Some(n) => {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(n.max(1))
                .build()
                .map_err(|e| ArtError::Pool {
                    threads: n,
                    message: e.to_string(),
                })?;
            Ok(pool.install(f))
        }
    }
}

/// The logical size of a layer on `device`, or why the scale is unusable.
pub fn logical_size(layer: &ArtLayer, device: Device) -> Result<(u32, u32, u32)> {
    if layer.scale == 0 {
        return Err(ArtError::ZeroScale {
            layer: layer.id.clone(),
        });
    }
    let (w, h) = (u64::from(device.width), u64::from(device.height));
    if w % layer.scale != 0 || h % layer.scale != 0 {
        return Err(ArtError::ScaleMismatch {
            layer: layer.id.clone(),
            scale: layer.scale,
            width: w,
            height: h,
        });
    }
    // Both quotients fit in u32 because the device sides do.
    Ok((
        (w / layer.scale) as u32,
        (h / layer.scale) as u32,
        layer.scale as u32,
    ))
}

/// Reject any layer whose scale does not divide `device`. Run before
/// rendering anything, so a bad recipe fails fast.
pub fn validate_scales(theme: &Theme, device: Device) -> Result<()> {
    theme
        .art
        .layers
        .iter()
        .try_for_each(|l| logical_size(l, device).map(|_| ()))
}

fn check_layer_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');
    if ok {
        Ok(())
    } else {
        Err(ArtError::Theme {
            message: format!(
                "layer id `{id}` cannot name a file (use letters, digits, `-`, `_`, `.`)"
            ),
        })
    }
}

/// Render one layer at its logical resolution.
pub fn render_layer(
    theme: &Theme,
    layer: &ArtLayer,
    colors: &ThemeColors,
    device: Device,
    theme_dir: Option<&Path>,
) -> Result<Layer> {
    let (width, height, _) = logical_size(layer, device)?;
    let params =
        primitives::Params::new(&layer.id, layer.params.as_ref().map(|k| &k.entries), colors);
    let ctx = primitives::Ctx {
        layer_id: &layer.id,
        width,
        height,
        rng: rng::Rng::new(theme.art.seed).for_layer(&layer.id),
        colors,
        params,
        base_dir: theme_dir,
    };
    primitives::render(layer.primitive, &ctx)
}

fn to_rendered(layer: &ArtLayer, rendered: Layer) -> RenderedLayer {
    RenderedLayer {
        id: layer.id.clone(),
        scale: layer.scale as u32,
        opacity: layer
            .opacity
            .as_ref()
            .and_then(serde_json::Number::as_f64)
            .unwrap_or(1.0),
        enabled: layer.enabled.unwrap_or(true),
        layer: rendered,
    }
}

/// Render every layer of the theme, bottom to top. Disabled layers are
/// rendered too: a layer tweak can switch one on at runtime.
pub fn render_theme(
    theme: &Theme,
    device: Device,
    options: &Options,
) -> Result<Vec<RenderedLayer>> {
    validate_scales(theme, device)?;
    let colors = ThemeColors::from_theme(theme)?;
    let dir = options.theme_dir.as_deref();
    in_pool(options.threads, || {
        theme
            .art
            .layers
            .par_iter()
            .map(|layer| {
                Ok(to_rendered(
                    layer,
                    render_layer(theme, layer, &colors, device, dir)?,
                ))
            })
            .collect::<Result<Vec<_>>>()
    })?
}

/// Run the legibility check for already-rendered layers.
pub fn check_legibility(
    theme: &Theme,
    layers: &[RenderedLayer],
    device: Device,
) -> Result<LegibilityReport> {
    let colors = ThemeColors::from_theme(theme)?;
    let hex = |name: &str, spec: &str| {
        colors.resolve(spec).ok_or_else(|| ArtError::Theme {
            message: format!("legibility.{name} is not a hex colour: `{spec}`"),
        })
    };
    let l = &theme.legibility;
    let input = LegibilityInput {
        base: colors.resolve("base").expect("base is always a role"),
        text: hex("text", &l.text)?,
        dim_text: hex("dim_text", &l.dim_text)?,
        min_contrast: l
            .min_contrast
            .as_ref()
            .and_then(serde_json::Number::as_f64)
            .unwrap_or(legibility::DEFAULT_MIN_CONTRAST),
    };
    Ok(legibility::check(
        &input,
        layers,
        (device.width, device.height),
    ))
}

// ---------------------------------------------------------------------------
// Recipe hash and manifest
// ---------------------------------------------------------------------------

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{}:", Value::String(k.clone()));
                write_canonical(&map[k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => {
            let _ = write!(out, "{other}");
        }
    }
}

/// Canonical JSON of the recipe: comment keys removed, object keys sorted,
/// no whitespace.
pub fn canonical_recipe(art: &ThemeArt) -> String {
    let mut value = serde_json::to_value(art).expect("a theme art recipe serializes");
    wzt_model::json::strip_comments(&mut value);
    let mut out = String::new();
    write_canonical(&value, &mut out);
    out
}

/// blake3 of [`canonical_recipe`], as hex. Stored in the manifest so stale
/// art (generated from an older recipe) can be told apart from current art.
pub fn recipe_hash(art: &ThemeArt) -> String {
    blake3::hash(canonical_recipe(art).as_bytes())
        .to_hex()
        .to_string()
}

pub const MANIFEST_FILE: &str = "manifest.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestLayer {
    pub id: String,
    pub file: String,
    pub scale: u32,
}

/// `manifest.json`, next to the layer PNGs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u64,
    pub recipe_hash: String,
    pub width: u32,
    pub height: u32,
    pub layers: Vec<ManifestLayer>,
}

/// What [`generate_theme`] did.
#[derive(Debug)]
pub struct GenerateReport {
    pub dir: PathBuf,
    pub manifest: Manifest,
    /// `None` when the check was skipped.
    pub legibility: Option<LegibilityReport>,
    /// Things worth telling the user that did not stop generation.
    pub warnings: Vec<String>,
}

/// Render `theme` for `device`, check legibility, and write
/// `<out_dir>/<layer id>.png` for every layer plus `manifest.json`.
///
/// Nothing is written if a scale is invalid, a primitive rejects its
/// parameters, or the legibility check fails (unless
/// [`Options::skip_legibility`]). The manifest goes last, so an interrupted
/// run leaves no manifest and the art counts as absent.
pub fn generate_theme(
    theme: &Theme,
    device: Device,
    out_dir: &Path,
    options: &Options,
) -> Result<GenerateReport> {
    for layer in &theme.art.layers {
        check_layer_id(&layer.id)?;
    }
    let rendered = render_theme(theme, device, options)?;

    let mut warnings = Vec::new();
    for layer in &theme.art.layers {
        if layer.animated == Some(true) {
            warnings.push(format!(
                "layer `{}`: animated layers are written as static PNGs for now",
                layer.id
            ));
        }
    }

    let legibility = if options.skip_legibility {
        None
    } else {
        let report = check_legibility(theme, &rendered, device)?;
        if !report.passed {
            return Err(ArtError::Illegible {
                which: report.limiting,
                x: report.densest.0,
                y: report.densest.1,
                contrast: report.contrast,
                required: report.required,
            });
        }
        Some(report)
    };

    in_pool(options.threads, || {
        rendered.par_iter().try_for_each(|r| {
            write::write_png_file(&out_dir.join(format!("{}.png", r.id)), &r.layer, r.scale)
        })
    })??;

    let manifest = Manifest {
        schema_version: 1,
        recipe_hash: recipe_hash(&theme.art),
        width: device.width,
        height: device.height,
        layers: rendered
            .iter()
            .map(|r| ManifestLayer {
                id: r.id.clone(),
                file: format!("{}.png", r.id),
                scale: r.scale,
            })
            .collect(),
    };
    let mut bytes = serde_json::to_vec_pretty(&manifest).expect("manifest serializes");
    bytes.push(b'\n');
    let manifest_path = out_dir.join(MANIFEST_FILE);
    wzt_model::write_atomic(&manifest_path, &bytes).map_err(|e| ArtError::Theme {
        message: e.to_string(),
    })?;

    Ok(GenerateReport {
        dir: out_dir.to_path_buf(),
        manifest,
        legibility,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_parses_and_rejects() {
        assert_eq!(
            "6016x3384".parse::<Device>().unwrap(),
            Device {
                width: 6016,
                height: 3384
            }
        );
        assert!("6016".parse::<Device>().is_err());
        assert!("0x10".parse::<Device>().is_err());
        assert!("axb".parse::<Device>().is_err());
        assert_eq!(
            Device {
                width: 3,
                height: 4
            }
            .to_string(),
            "3x4"
        );
    }

    #[test]
    fn canonical_json_sorts_keys_and_is_compact() {
        let v: Value = serde_json::json!({"b": [1, {"z": 1, "a": 2}], "a": "x"});
        let mut s = String::new();
        write_canonical(&v, &mut s);
        assert_eq!(s, r#"{"a":"x","b":[1,{"a":2,"z":1}]}"#);
    }

    #[test]
    fn layer_ids_must_be_safe_file_names() {
        assert!(check_layer_id("stars").is_ok());
        assert!(check_layer_id("grid-2").is_ok());
        assert!(check_layer_id("../x").is_err());
        assert!(check_layer_id("a/b").is_err());
        assert!(check_layer_id("").is_err());
        assert!(check_layer_id(".hidden").is_err());
    }
}
