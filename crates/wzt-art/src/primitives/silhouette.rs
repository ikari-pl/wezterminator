//! `silhouette_bands`: layered ridgelines (hills, mountains, a skyline) from
//! the bottom of the layer, far bands first.
//!
//! Each band's top edge is 1D value noise: a hashed value at each lattice
//! point, blended with a smoothstep, summed over a few octaves. The edges are
//! computed per column up front and the pixel loop is a comparison against
//! them.
//!
//! Parameters:
//! - `bands`: number of bands (default 3).
//! - `height`: percent of the layer the whole stack may reach (default 35).
//! - `roughness`: noise octaves (default 3).
//! - `feature`: wavelength of the coarsest octave in pixels (default a sixth
//!   of the width).
//! - `colors`: far to near. Bands pick evenly along the list (default
//!   `surface`, `bg`).
//! - `alpha`: band alpha (default 220).

use crate::error::Result;
use crate::palette::PaletteBuilder;
use crate::primitives::{Ctx, fill_rows, finish_layer};
use crate::rng::{Rng, below};

/// 1D value noise at `x`, Q10, with octave wavelength `wl`.
fn value_noise(rng: Rng, octave: u64, x: u32, wl: u32) -> u32 {
    let (cell, t) = (x / wl, (x % wl) * 1024 / wl);
    let at = |c: u32| below(rng.at_n(i64::from(c), 0, octave), 1025) as u32;
    let s = (t * t * (3 * 1024 - 2 * t)) >> 20; // smoothstep, Q10
    let (a, b) = (at(cell), at(cell + 1));
    (a * (1024 - s) + b * s) >> 10
}

pub(super) fn render(ctx: &Ctx<'_>) -> Result<crate::palette::Layer> {
    let p = &ctx.params;
    let bands = p.uint("bands", 3, 1, 8)? as usize;
    let height_pct = p.uint("height", 35, 5, 95)? as i64;
    let octaves = p.uint("roughness", 3, 1, 6)? as u32;
    let (w, h) = (ctx.width, ctx.height);
    let feature = p.uint("feature", u64::from((w / 6).max(4)), 2, 65_535)? as u32;
    let colors = p.colour_list("colors", &["surface", "bg"], 1, 8)?;
    let alpha = p.uint("alpha", 220, 1, 255)? as u8;

    let mut pal = PaletteBuilder::new();
    let ids: Vec<u8> = (0..bands)
        .map(|i| {
            let pick = if bands == 1 {
                colors.len() - 1
            } else {
                i * (colors.len() - 1) / (bands - 1)
            };
            pal.add(colors[pick], alpha)
        })
        .collect::<Result<_>>()?;

    let reach = i64::from(h) * height_pct / 100;
    let amp = (reach * 3 / (2 * bands as i64)).max(1);
    let tops: Vec<Vec<i64>> = (0..bands)
        .map(|i| {
            // Far bands start higher; near bands start lower and overlap them.
            let base = i64::from(h) - reach * (bands - i) as i64 / bands as i64;
            let rng = ctx.rng.fork(i as u64);
            (0..w)
                .map(|x| {
                    let (mut sum, mut weight) = (0u64, 0u64);
                    for o in 0..octaves {
                        let wl = (feature >> o).max(2);
                        let wt = 1u64 << (octaves - 1 - o);
                        sum += wt * u64::from(value_noise(rng, u64::from(o), x, wl));
                        weight += wt;
                    }
                    let n = (sum / weight) as i64; // Q10
                    base - ((amp * n) >> 10)
                })
                .collect()
        })
        .collect();

    let pixels = fill_rows(w, h, |x, y| {
        for i in (0..bands).rev() {
            if i64::from(y) >= tops[i][x as usize] {
                return ids[i];
            }
        }
        0
    });
    Ok(finish_layer(ctx, pal, pixels))
}
