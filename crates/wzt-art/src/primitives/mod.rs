//! The drawing primitives and what they share.
//!
//! Every primitive follows one shape:
//!
//! 1. read and validate its parameters (serial);
//! 2. do any non-local setup from the seed (blob centres, grid rows, ridge
//!    lines), serially, and register every colour it can emit in the palette;
//! 3. describe the picture as a pure function `(x, y) -> palette index` and
//!    let [`fill_rows`] evaluate it across rayon threads.
//!
//! Step 3 never reads shared mutable state and uses integer or fixed-point
//! maths only, so the pixels do not depend on thread count or on the
//! platform's libm.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rayon::prelude::*;
use serde_json::Value;
use wzt_model::Primitive;

use crate::error::{ArtError, Result};
use crate::palette::{Layer, PaletteBuilder, Rgb, ThemeColors};
use crate::rng::Rng;

mod cloud_blobs;
mod dither_wash;
mod image;
mod isometric;
mod perspective_grid;
mod scanlines;
mod silhouette;
mod sprites;
mod starfield;
mod vignette;

/// Everything a primitive needs to draw one layer.
pub struct Ctx<'a> {
    pub layer_id: &'a str,
    /// Logical size: device size divided by the layer's scale.
    pub width: u32,
    pub height: u32,
    pub rng: Rng,
    pub colors: &'a ThemeColors,
    pub params: Params<'a>,
    /// Directory relative `path` parameters resolve against (the theme's).
    pub base_dir: Option<&'a Path>,
}

/// Draw one layer.
pub fn render(primitive: Primitive, ctx: &Ctx<'_>) -> Result<Layer> {
    let layer = match primitive {
        Primitive::Starfield => starfield::render(ctx)?,
        Primitive::DitherWash => dither_wash::render(ctx)?,
        Primitive::CloudBlobs => cloud_blobs::render(ctx)?,
        Primitive::PerspectiveGrid => perspective_grid::render(ctx)?,
        Primitive::Scanlines => scanlines::render(ctx)?,
        Primitive::Vignette => vignette::render(ctx)?,
        Primitive::SpriteStrip => sprites::render_strip(ctx)?,
        Primitive::ScatteredSprites => sprites::render_scattered(ctx)?,
        Primitive::IsometricTiles => isometric::render(ctx)?,
        Primitive::SilhouetteBands => silhouette::render(ctx)?,
        Primitive::Image => image::render(ctx)?,
    };
    // Unknown keys are errors: a misspelled parameter must not be a silent no-op.
    ctx.params.finish()?;
    Ok(layer)
}

/// Evaluate `f` for every pixel, one rayon task per row.
pub(crate) fn fill_rows(width: u32, height: u32, f: impl Fn(u32, u32) -> u8 + Sync) -> Vec<u8> {
    let w = width as usize;
    let mut pixels = vec![0u8; w * height as usize];
    if w == 0 {
        return pixels;
    }
    pixels.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, p) in row.iter_mut().enumerate() {
            *p = f(x as u32, y as u32);
        }
    });
    pixels
}

/// Assemble a layer from a finished builder and pixel buffer.
pub(crate) fn finish_layer(ctx: &Ctx<'_>, pal: PaletteBuilder, pixels: Vec<u8>) -> Layer {
    Layer {
        width: ctx.width,
        height: ctx.height,
        palette: pal.finish(),
        pixels,
    }
}

/// Integer square root, floor.
pub(crate) fn isqrt(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    let mut x = (n as f64).sqrt() as u64; // seed only; corrected exactly below
    while x * x > n {
        x -= 1;
    }
    while (x + 1) * (x + 1) <= n {
        x += 1;
    }
    x
}

/// Distance from the layer centre as a fraction of the centre-to-corner
/// distance, in Q10 (`0` at the centre, `1024` at the corners).
#[inline]
pub(crate) fn radial_q10(x: u32, y: u32, w: u32, h: u32) -> u32 {
    // Doubled coordinates keep the centre exact for even sizes.
    let dx = (2 * i64::from(x) + 1 - i64::from(w)).unsigned_abs();
    let dy = (2 * i64::from(y) + 1 - i64::from(h)).unsigned_abs();
    let max2 = (u64::from(w) * u64::from(w) + u64::from(h) * u64::from(h)).max(1);
    let d2 = dx * dx + dy * dy;
    isqrt((d2 << 20) / max2).min(1024) as u32
}

/// Register `levels` alpha steps of one colour: step `l` (1-based) has alpha
/// `max_alpha * l / levels`. Returns the palette indices, step 1 first.
pub(crate) fn alpha_steps(
    pal: &mut PaletteBuilder,
    rgb: Rgb,
    max_alpha: u8,
    levels: u32,
) -> Result<Vec<u8>> {
    (1..=levels)
        .map(|l| {
            let a = (u32::from(max_alpha) * l / levels).max(1) as u8;
            pal.add(rgb, a)
        })
        .collect()
}

/// The step (1-based, 0 = none) for a coverage `v` in Q10, rounding up so any
/// non-zero value shows.
#[inline]
pub(crate) fn step_of(v_q10: u32, levels: u32) -> u32 {
    (v_q10.min(1024) * levels).div_ceil(1024)
}

/// A layer's `params` object, read with validation.
///
/// Reads are recorded so [`Params::finish`] can reject keys no primitive
/// asked for.
pub struct Params<'a> {
    layer: &'a str,
    map: Option<&'a BTreeMap<String, Value>>,
    colors: &'a ThemeColors,
    used: RefCell<BTreeSet<String>>,
}

impl<'a> Params<'a> {
    pub fn new(
        layer: &'a str,
        map: Option<&'a BTreeMap<String, Value>>,
        colors: &'a ThemeColors,
    ) -> Self {
        Params {
            layer,
            map,
            colors,
            used: RefCell::new(BTreeSet::new()),
        }
    }

    pub fn error(&self, name: &str, message: impl Into<String>) -> ArtError {
        ArtError::Param {
            layer: self.layer.to_string(),
            name: name.to_string(),
            message: message.into(),
        }
    }

    pub fn raw(&self, name: &str) -> Option<&'a Value> {
        self.used.borrow_mut().insert(name.to_string());
        self.map.and_then(|m| m.get(name))
    }

    /// An integer in `min..=max`, or `default` when absent.
    pub fn uint(&self, name: &str, default: u64, min: u64, max: u64) -> Result<u64> {
        debug_assert!(
            (min..=max).contains(&default),
            "default of `{name}` out of range"
        );
        let Some(v) = self.raw(name) else {
            return Ok(default);
        };
        let n = v
            .as_u64()
            .ok_or_else(|| self.error(name, format!("expected a whole number, found {v}")))?;
        if !(min..=max).contains(&n) {
            return Err(self.error(name, format!("{n} is outside {min}..={max}")));
        }
        Ok(n)
    }

    /// A string from a fixed set, or `default` when absent.
    pub fn choice<'c>(&self, name: &str, default: &'c str, allowed: &[&'c str]) -> Result<&'c str> {
        let Some(v) = self.raw(name) else {
            return Ok(default);
        };
        let s = v
            .as_str()
            .ok_or_else(|| self.error(name, format!("expected a string, found {v}")))?;
        allowed
            .iter()
            .find(|a| **a == s)
            .copied()
            .ok_or_else(|| self.error(name, format!("`{s}` is not one of {}", allowed.join(", "))))
    }

    pub fn string(&self, name: &str) -> Result<Option<&'a str>> {
        match self.raw(name) {
            None => Ok(None),
            Some(v) => v
                .as_str()
                .map(Some)
                .ok_or_else(|| self.error(name, format!("expected a string, found {v}"))),
        }
    }

    /// Resolve a colour spec (hex or theme role).
    pub fn spec_to_rgb(&self, name: &str, spec: &str) -> Result<Rgb> {
        self.colors.resolve(spec).ok_or_else(|| {
            self.error(
                name,
                format!("`{spec}` is neither a #rrggbb colour nor a theme role"),
            )
        })
    }

    /// One colour, or `default` (a spec) when absent.
    pub fn colour(&self, name: &str, default: &str) -> Result<Rgb> {
        match self.string(name)? {
            Some(spec) => self.spec_to_rgb(name, spec),
            None => self.spec_to_rgb(name, default),
        }
    }

    /// A list of colours with `min_len..=max_len` entries, or `defaults`.
    pub fn colour_list(
        &self,
        name: &str,
        defaults: &[&str],
        min_len: usize,
        max_len: usize,
    ) -> Result<Vec<Rgb>> {
        let specs: Vec<String> = match self.raw(name) {
            None => defaults.iter().map(|s| (*s).to_string()).collect(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|v| {
                    v.as_str().map(str::to_string).ok_or_else(|| {
                        self.error(name, format!("expected colour strings, found {v}"))
                    })
                })
                .collect::<Result<_>>()?,
            Some(v) => return Err(self.error(name, format!("expected an array, found {v}"))),
        };
        if !(min_len..=max_len).contains(&specs.len()) {
            return Err(self.error(
                name,
                format!(
                    "needs {min_len} to {max_len} colours, found {}",
                    specs.len()
                ),
            ));
        }
        specs.iter().map(|s| self.spec_to_rgb(name, s)).collect()
    }

    /// Reject keys that were never read.
    pub fn finish(&self) -> Result<()> {
        let used = self.used.borrow();
        if let Some(map) = self.map {
            for key in map.keys() {
                if !used.contains(key) {
                    return Err(self.error(key, "unknown parameter for this primitive"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_is_exact() {
        for n in [
            0u64,
            1,
            2,
            3,
            4,
            15,
            16,
            17,
            1 << 40,
            (1 << 40) - 1,
            1 << 60,
        ] {
            let r = isqrt(n);
            assert!(r * r <= n && (r + 1) * (r + 1) > n, "n = {n}");
        }
    }

    #[test]
    fn radial_is_zero_at_centre_and_full_at_corner() {
        assert!(radial_q10(50, 50, 101, 101) < 8);
        assert!(radial_q10(0, 0, 100, 100) >= 1000);
        assert!(radial_q10(99, 99, 100, 100) >= 1000);
    }

    #[test]
    fn step_rounds_up_so_small_values_show() {
        assert_eq!(step_of(0, 8), 0);
        assert_eq!(step_of(1, 8), 1);
        assert_eq!(step_of(1024, 8), 8);
    }
}
