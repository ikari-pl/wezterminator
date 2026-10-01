//! The legibility check: can the theme's dim text still be read over its own
//! art?
//!
//! The layers are composited over the base colour exactly as WezTerm will
//! (bottom to top, each layer's palette alpha times its recipe opacity), and
//! the device is divided into square tiles about the size of a few terminal
//! cells. Each tile's average background is compared with the dim text colour
//! by WCAG contrast ratio, for the theme's dim text *and* its normal text. The
//! tile where the weaker of the two is lowest is the densest one in the sense
//! that matters, and it must stay at or above `legibility.min_contrast`
//! (default 3.0).
//!
//! Both colours are needed because contrast is not monotonic in how much art
//! there is. On a dark theme a growing wash first drags dim text toward
//! unreadable, and then, once it is bright enough, lifts the *normal* text
//! into the same trap while dim text starts to look fine against it. Checking
//! only one colour passes some washes that swallow the other.
//!
//! Tiles are a multiple of every layer's scale, so each tile covers whole
//! logical pixels of every layer and nothing is resampled.

use rayon::prelude::*;

use crate::palette::{Layer, Rgb, srgb_to_linear};

/// A rendered layer with what the compositor needs to know about it.
#[derive(Debug, Clone)]
pub struct RenderedLayer {
    pub id: String,
    pub scale: u32,
    /// Recipe opacity, 0..=1.
    pub opacity: f64,
    /// Disabled layers are generated (a tweak can enable them) but do not
    /// count here.
    pub enabled: bool,
    pub layer: Layer,
}

/// What to check against.
#[derive(Debug, Clone)]
pub struct LegibilityInput {
    pub base: Rgb,
    pub text: Rgb,
    pub dim_text: Rgb,
    pub min_contrast: f64,
}

pub const DEFAULT_MIN_CONTRAST: f64 = 3.0;

#[derive(Debug, Clone, PartialEq)]
pub struct LegibilityReport {
    /// Tile edge in device pixels.
    pub tile: u32,
    /// Top-left device pixel of the lowest-contrast tile.
    pub densest: (u32, u32),
    /// Average background of that tile.
    pub background: Rgb,
    pub dim_contrast: f64,
    pub text_contrast: f64,
    /// The text colour with the lower contrast there: `"dim_text"` or `"text"`.
    pub limiting: &'static str,
    /// The lower of the two contrasts.
    pub contrast: f64,
    pub required: f64,
    pub passed: bool,
}

/// WCAG relative luminance of an sRGB colour.
pub fn luminance(c: Rgb) -> f64 {
    0.2126 * srgb_to_linear(c[0]) + 0.7152 * srgb_to_linear(c[1]) + 0.0722 * srgb_to_linear(c[2])
}

/// WCAG contrast ratio, 1..=21.
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Smallest multiple of every scale that is at least ~32 pixels.
fn tile_size(scales: impl Iterator<Item = u32>, device: (u32, u32)) -> u32 {
    let lcm = scales.fold(1, |acc, s| acc / gcd(acc, s) * s);
    let t = lcm * (32 / lcm).max(1);
    t.min(device.0.max(device.1)).max(1)
}

/// Composite `layers` over `input.base` and test the densest tile.
pub fn check(
    input: &LegibilityInput,
    layers: &[RenderedLayer],
    device: (u32, u32),
) -> LegibilityReport {
    let active: Vec<&RenderedLayer> = layers
        .iter()
        .filter(|l| l.enabled && l.opacity > 0.0)
        .collect();
    let tile = tile_size(active.iter().map(|l| l.scale), device);
    let (dw, dh) = device;
    let (tx, ty) = (dw.div_ceil(tile), dh.div_ceil(tile));

    // Per layer, per device column and row: the logical coordinate and the
    // opacity in Q8. Lookups in the hot loop are then plain indexing.
    let col_maps: Vec<Vec<u32>> = active
        .iter()
        .map(|l| {
            (0..dw)
                .map(|x| (x / l.scale).min(l.layer.width - 1))
                .collect()
        })
        .collect();
    let row_maps: Vec<Vec<u32>> = active
        .iter()
        .map(|l| {
            (0..dh)
                .map(|y| (y / l.scale).min(l.layer.height - 1))
                .collect()
        })
        .collect();
    let opacity_q8: Vec<u32> = active
        .iter()
        .map(|l| (l.opacity.clamp(0.0, 1.0) * 256.0).round() as u32)
        .collect();

    // Mean composite colour for every tile, in tile order. Parallel by tile
    // row; each tile is a pure function of the layers, so order is irrelevant.
    let means: Vec<[u64; 4]> = (0..ty)
        .into_par_iter()
        .flat_map_iter(|row| (0..tx).map(move |col| (row, col)))
        .map(|(row, col)| {
            let (x0, y0) = (col * tile, row * tile);
            let (x1, y1) = ((x0 + tile).min(dw), (y0 + tile).min(dh));
            let mut sum = [0u64; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    let mut px = [
                        i64::from(input.base[0]),
                        i64::from(input.base[1]),
                        i64::from(input.base[2]),
                    ];
                    for (li, l) in active.iter().enumerate() {
                        let (lx, ly) = (col_maps[li][x as usize], row_maps[li][y as usize]);
                        let e = l.layer.rgba_at(lx, ly);
                        let a = (i64::from(e[3]) * i64::from(opacity_q8[li])) >> 8; // 0..=255
                        if a == 0 {
                            continue;
                        }
                        for c in 0..3 {
                            px[c] += ((i64::from(e[c]) - px[c]) * a) / 255;
                        }
                    }
                    for c in 0..3 {
                        sum[c] += px[c] as u64;
                    }
                }
            }
            [sum[0], sum[1], sum[2], u64::from((x1 - x0) * (y1 - y0))]
        })
        .collect();

    // (tile index, dim contrast, text contrast, background)
    let mut worst: Option<(usize, f64, f64, Rgb)> = None;
    for (i, m) in means.iter().enumerate() {
        if m[3] == 0 {
            continue;
        }
        let bg = [
            (m[0] / m[3]) as u8,
            (m[1] / m[3]) as u8,
            (m[2] / m[3]) as u8,
        ];
        let (cd, ct) = (contrast(input.dim_text, bg), contrast(input.text, bg));
        // Strictly lower wins, so ties keep the earliest tile.
        if worst.is_none_or(|(_, d, t, _)| cd.min(ct) < d.min(t)) {
            worst = Some((i, cd, ct, bg));
        }
    }
    let (index, dim_contrast, text_contrast, background) =
        worst.unwrap_or((0, f64::INFINITY, f64::INFINITY, input.base));
    let (col, row) = ((index as u32) % tx.max(1), (index as u32) / tx.max(1));
    let lowest = dim_contrast.min(text_contrast);
    LegibilityReport {
        tile,
        densest: (col * tile, row * tile),
        background,
        dim_contrast,
        text_contrast,
        limiting: if dim_contrast <= text_contrast {
            "dim_text"
        } else {
            "text"
        },
        contrast: lowest,
        required: input.min_contrast,
        passed: lowest >= input.min_contrast,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(
        id: &str,
        scale: u32,
        opacity: f64,
        rgb: Rgb,
        alpha: u8,
        logical: (u32, u32),
    ) -> RenderedLayer {
        RenderedLayer {
            id: id.into(),
            scale,
            opacity,
            enabled: true,
            layer: Layer {
                width: logical.0,
                height: logical.1,
                palette: vec![[0, 0, 0, 0], [rgb[0], rgb[1], rgb[2], alpha]],
                pixels: vec![1; (logical.0 * logical.1) as usize],
            },
        }
    }

    fn input() -> LegibilityInput {
        LegibilityInput {
            base: [12, 12, 24],
            text: [192, 192, 200],
            dim_text: [96, 96, 112],
            min_contrast: 3.0,
        }
    }

    #[test]
    fn wcag_extremes() {
        assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
        assert!((contrast([7, 7, 7], [7, 7, 7]) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn empty_stack_is_the_base_colour() {
        let r = check(&input(), &[], (96, 96));
        assert_eq!(r.background, [12, 12, 24]);
        assert!(r.passed, "{r:?}");
    }

    #[test]
    fn dense_bright_layer_fails_and_names_a_tile() {
        let layers = [solid("wash", 2, 1.0, [200, 200, 220], 255, (48, 48))];
        let r = check(&input(), &layers, (96, 96));
        assert!(!r.passed, "{r:?}");
        // Dim text is still fine against a bright wash; normal text is what drowns.
        assert_eq!(r.limiting, "text", "{r:?}");
        assert!(r.contrast < 3.0);
        assert!(r.densest.0 < 96 && r.densest.1 < 96);
    }

    #[test]
    fn opacity_and_enabled_scale_the_effect() {
        let bright = [200, 200, 220];
        let faint = [solid("wash", 2, 0.01, bright, 255, (48, 48))];
        assert!(check(&input(), &faint, (96, 96)).passed);
        let mut off = [solid("wash", 2, 1.0, bright, 255, (48, 48))];
        off[0].enabled = false;
        assert!(check(&input(), &off, (96, 96)).passed);
    }

    #[test]
    fn tiles_cover_whole_logical_pixels_for_every_scale() {
        for scales in [vec![2, 3, 4], vec![5], vec![7, 11], vec![1]] {
            let t = tile_size(scales.iter().copied(), (4000, 4000));
            assert!(scales.iter().all(|s| t % s == 0), "{scales:?} -> {t}");
        }
    }

    #[test]
    fn worst_tile_is_found_where_the_dense_part_is() {
        // Bright only in the bottom-right logical quadrant.
        let (w, h) = (48u32, 48u32);
        let mut layer = solid("wash", 2, 1.0, [200, 200, 220], 255, (w, h));
        for y in 0..h {
            for x in 0..w {
                layer.layer.pixels[(y * w + x) as usize] = u8::from(x >= 24 && y >= 24);
            }
        }
        let r = check(&input(), &[layer], (96, 96));
        assert!(!r.passed);
        assert!(r.densest.0 >= 48 && r.densest.1 >= 48, "{:?}", r.densest);
    }
}
