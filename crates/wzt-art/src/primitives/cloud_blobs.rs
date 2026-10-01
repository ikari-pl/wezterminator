//! `cloud_blobs`: dithered nebula gas from a handful of soft ellipses.
//!
//! No Gaussian blur and no floating point. Each blob contributes a quartic
//! falloff `(1 - d²)²` (smooth at the rim, zero outside it), the contributions
//! add, and the sum is normalised against its own peak so overlapping blobs do
//! not saturate into one flat wash. The result is squared, which pushes thin
//! outskirts to nothing and leaves visible cores, then quantised through the
//! colour ramp with a Bayer dither.
//!
//! Parameters:
//! - `blob_count`: number of blobs (default 6).
//! - `ramp`: colours thin to dense (default `surface`, `accent`, `accent_alt`).
//! - `alpha`: alpha of the densest dots (default 120).
//! - `threshold`: percent density below which nothing is drawn (default 5).
//! - `spread`: percent scale of blob radii (default 100).
//! - `steps`: alpha steps per colour (default 8).

use crate::dither::{bayer_q10, ramp_pick};
use crate::error::Result;
use crate::palette::PaletteBuilder;
use crate::primitives::{Ctx, alpha_steps, fill_rows, finish_layer, step_of};
use crate::rng::{below, range};

struct Blob {
    cx: i64,
    cy: i64,
    rx: i64,
    ry: i64,
    weight: u64,
}

/// The summed falloff at a pixel, in Q20 times weight.
fn field(blobs: &[Blob], x: i64, y: i64) -> u64 {
    let mut acc = 0u64;
    for b in blobs {
        let (dx, dy) = (x - b.cx, y - b.cy);
        if dx.abs() >= b.rx || dy.abs() >= b.ry {
            continue;
        }
        let d2 = ((dx * dx) << 20) / (b.rx * b.rx) + ((dy * dy) << 20) / (b.ry * b.ry);
        if d2 >= 1 << 20 {
            continue;
        }
        let t = (1i64 << 20) - d2;
        acc += b.weight * ((t * t) >> 20) as u64;
    }
    acc
}

pub(super) fn render(ctx: &Ctx<'_>) -> Result<crate::palette::Layer> {
    let p = &ctx.params;
    let count = p.uint("blob_count", 6, 1, 64)?;
    let ramp = p.colour_list("ramp", &["surface", "accent", "accent_alt"], 1, 16)?;
    let alpha = p.uint("alpha", 120, 1, 255)? as u8;
    let threshold = p.uint("threshold", 5, 0, 100)? as u32;
    let spread = p.uint("spread", 100, 10, 400)?;
    let steps = p.uint("steps", 8, 1, 32)? as u32;

    let (w, h) = (ctx.width, ctx.height);
    let rng = ctx.rng;
    let blobs: Vec<Blob> = (0..count)
        .map(|i| {
            let n = |k: u64| rng.nth(i * 5 + k);
            let rx = i64::from(w) * range(n(2), 15, 40) as i64 / 100 * spread as i64 / 100;
            let ry = i64::from(h) * range(n(3), 20, 50) as i64 / 100 * spread as i64 / 100;
            Blob {
                cx: below(n(0), u64::from(w)) as i64,
                cy: below(n(1), u64::from(h)) as i64,
                rx: rx.max(2),
                ry: ry.max(2),
                weight: range(n(4), 64, 160),
            }
        })
        .collect();

    // Peak from a coarse scan, so normalisation costs a few thousand samples
    // and does not depend on how the pixel loop is split across threads.
    let step = (w.min(h) / 64).max(1) as usize;
    let mut peak = 1u64;
    for y in (0..h as usize).step_by(step) {
        for x in (0..w as usize).step_by(step) {
            peak = peak.max(field(&blobs, x as i64, y as i64));
        }
    }

    let mut pal = PaletteBuilder::new();
    // table[colour][step - 1]
    let table: Vec<Vec<u8>> = ramp
        .iter()
        .map(|c| alpha_steps(&mut pal, *c, alpha, steps))
        .collect::<Result<_>>()?;

    let threshold_q10 = threshold * 1024 / 100;
    let pixels = fill_rows(w, h, |x, y| {
        let acc = field(&blobs, i64::from(x), i64::from(y));
        if acc == 0 {
            return 0;
        }
        let v = (acc * 1024 / peak).min(1024) as u32;
        let g = (v * v) >> 10;
        if g == 0 || g < threshold_q10 {
            return 0;
        }
        let along = ((g * 1075) >> 10).min(1024); // x1.05: let cores reach the last stop
        let colour = ramp_pick(along, table.len(), bayer_q10(x, y, 2));
        let density = ((g * 1177) >> 10).min(1024); // x1.15: alpha leads colour slightly
        match step_of(density, steps) {
            0 => 0,
            s => table[colour][s as usize - 1],
        }
    });
    Ok(finish_layer(ctx, pal, pixels))
}
