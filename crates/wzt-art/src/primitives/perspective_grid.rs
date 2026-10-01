//! `perspective_grid`: a floor grid receding to a vanishing point, with a
//! dithered glow on the horizon.
//!
//! Vertical lines converge on the vanishing point at the middle of the
//! horizon. Horizontal lines sit at `1/z` spacing so they bunch toward the
//! horizon, with every few lines an accent. Alpha rises with proximity.
//!
//! (The Python original starts `z` at exactly 1.0, which puts the first line
//! on the bottom edge and `break`s before drawing any horizontal at all. Here
//! that first line is skipped and the loop continues.)
//!
//! Parameters:
//! - `horizon`: horizon height, percent of the layer (default 42).
//! - `columns`: vertical lines are spaced `width / columns` apart at the
//!   bottom edge (default 12).
//! - `lines`: vertical lines on each side of centre (default 18).
//! - `depth`: farthest `z` drawn (default 26).
//! - `accent_every`: every Nth horizontal is an accent, 0 for none (default 4).
//! - `glow`: percent coverage of the horizon glow, 0 for none (default 55).
//! - `colors`: vertical, horizontal, accent and glow colours (default `accent`,
//!   `accent_alt`, `fg`, `info`).

use crate::dither::bayer_q10;
use crate::error::Result;
use crate::palette::PaletteBuilder;
use crate::primitives::{Ctx, alpha_steps, fill_rows, finish_layer, isqrt};

const LEVELS: u32 = 8;
const GLOW_LEVELS: u32 = 4;
const MAX_ALPHA: u32 = 210;
/// Constant alpha of the vertical lines (`10 + 95 * 0.55^1.6` in the original).
const VERTICAL_ALPHA: u8 = 46;

/// `t^1.5` in Q10, for `t` in Q10.
fn pow15(t: u32) -> u32 {
    ((u64::from(t) * isqrt(u64::from(t) << 10)) >> 10) as u32
}

pub(super) fn render(ctx: &Ctx<'_>) -> Result<crate::palette::Layer> {
    let p = &ctx.params;
    let horizon_pct = p.uint("horizon", 42, 5, 90)? as u32;
    let columns = p.uint("columns", 12, 1, 256)? as i64;
    let lines = p.uint("lines", 18, 0, 256)? as i64;
    let depth = p.uint("depth", 26, 2, 1000)?;
    let accent_every = p.uint("accent_every", 4, 0, 1000)? as usize;
    let glow = p.uint("glow", 55, 0, 100)? as u32;
    let colors = p.colour_list("colors", &["accent", "accent_alt", "fg", "info"], 4, 4)?;

    let (w, h) = (ctx.width, ctx.height);
    let horizon = (h * horizon_pct / 100).min(h.saturating_sub(1));
    let span = i64::from(h - horizon).max(1);
    let vpx = i64::from(w) / 2;
    let step = (i64::from(w) / columns).max(1);

    let mut pal = PaletteBuilder::new();
    let vertical = pal.add(colors[0], VERTICAL_ALPHA)?;
    let normal = alpha_steps(&mut pal, colors[1], MAX_ALPHA as u8, LEVELS)?;
    let accent = alpha_steps(&mut pal, colors[2], MAX_ALPHA as u8, LEVELS)?;
    let glow_steps = alpha_steps(&mut pal, colors[3], 110, GLOW_LEVELS)?;

    // Row kinds, found serially: 0 none, 1 normal, 2 accent. Several z values
    // near the horizon can land on one row, and the strongest wins.
    let mut rows = vec![0u8; h as usize];
    let mut z: u64 = 1 << 16; // Q16
    let z_max = depth << 16;
    let mut n = 0usize;
    while z < z_max {
        let y = i64::from(horizon) + (span << 16) / z as i64;
        if y < i64::from(h) {
            let kind = if accent_every > 0 && n.is_multiple_of(accent_every) {
                2
            } else {
                1
            };
            let slot = &mut rows[y as usize];
            *slot = (*slot).max(kind);
            n += 1;
        }
        z = (z * 79_954) >> 16; // x1.22
    }

    let glow_reach = (i64::from(w) * 26 / 100).max(1);
    let glow_top = horizon.saturating_sub(8);

    let pixels = fill_rows(w, h, |x, y| {
        // Glow sits on top of everything else, as in the original.
        if glow > 0 && y >= glow_top && y <= horizon + 2 {
            let dist = (i64::from(horizon) - i64::from(y)).unsigned_abs() as i64;
            let vfall = 1024 - dist * 1024 / 9;
            let hfall = 1024 - (i64::from(x) - vpx).abs() * 1024 / glow_reach;
            if vfall > 0 && hfall > 0 {
                let strength = ((vfall * ((hfall * hfall) >> 10)) >> 10) as u32;
                if strength * glow / 100 > bayer_q10(x, y, 2) {
                    let s = (strength * GLOW_LEVELS)
                        .div_ceil(1024)
                        .clamp(1, GLOW_LEVELS);
                    return glow_steps[s as usize - 1];
                }
            }
        }
        if y <= horizon {
            return 0;
        }
        // t: 0 at the horizon, 1 at the bottom edge.
        let dy = i64::from(y - horizon);
        let kind = rows[y as usize];
        if kind != 0 {
            let t = (dy * 1024 / span) as u32;
            let peak = if kind == 2 { 190 } else { 120 };
            let a = (10 + ((peak * pow15(t)) >> 10)).min(MAX_ALPHA);
            let s = (a * LEVELS).div_ceil(MAX_ALPHA).clamp(1, LEVELS);
            return if kind == 2 {
                accent[s as usize - 1]
            } else {
                normal[s as usize - 1]
            };
        }
        // Verticals: line i is at vpx + i*step*dy/span. Test the two lines
        // either side of this pixel's exact position.
        let num = (i64::from(x) - vpx) * span;
        let den = step * dy;
        let i0 = num.div_euclid(den);
        for i in [i0, i0 + 1] {
            if i.abs() <= lines {
                let xl = vpx + (2 * i * den + span).div_euclid(2 * span);
                if xl == i64::from(x) {
                    return vertical;
                }
            }
        }
        0
    });
    Ok(finish_layer(ctx, pal, pixels))
}
