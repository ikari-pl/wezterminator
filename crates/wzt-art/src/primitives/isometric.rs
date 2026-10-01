//! `isometric_tiles`: a diamond floor lattice, some tiles filled.
//!
//! The 2:1 isometric lattice is the set of lines `x/hx + y/hy = k` and
//! `x/hx - y/hy = k` with `hx = tile / 2` and `hy = tile / 4`. Scaling by
//! `hx * hy` keeps everything in integers: with `u = x*hy + y*hx` and
//! `v = x*hy - y*hx`, a pixel's tile is `(u / S, v / S)` and it lies on an
//! edge when `u mod S` or `v mod S` falls in the first `hx` units (one pixel
//! of thickness along a column).
//!
//! Parameters:
//! - `tile`: tile width in pixels, a multiple of 4 (default 16).
//! - `fill`: percent of tiles filled (default 35).
//! - `colors`: fill ramp; each filled tile takes one at random (default
//!   `surface`, `accent`, `accent_alt`).
//! - `edge`: edge colour (default `accent`).
//! - `edge_alpha` and `fill_alpha`: alphas (defaults 70 and 60).

use crate::error::Result;
use crate::palette::PaletteBuilder;
use crate::primitives::{Ctx, fill_rows, finish_layer};
use crate::rng::{below, chance};

pub(super) fn render(ctx: &Ctx<'_>) -> Result<crate::palette::Layer> {
    let p = &ctx.params;
    let tile = p.uint("tile", 16, 4, 512)? as i64;
    if tile % 4 != 0 {
        return Err(p.error("tile", format!("{tile} is not a multiple of 4")));
    }
    let fill = p.uint("fill", 35, 0, 100)?;
    let ramp = p.colour_list("colors", &["surface", "accent", "accent_alt"], 1, 16)?;
    let edge_color = p.colour("edge", "accent")?;
    let edge_alpha = p.uint("edge_alpha", 70, 1, 255)? as u8;
    let fill_alpha = p.uint("fill_alpha", 60, 1, 255)? as u8;

    let mut pal = PaletteBuilder::new();
    let edge = pal.add(edge_color, edge_alpha)?;
    let fills: Vec<u8> = ramp
        .iter()
        .map(|c| pal.add(*c, fill_alpha))
        .collect::<Result<_>>()?;

    let (hx, hy) = (tile / 2, tile / 4);
    let s = hx * hy;
    let rng = ctx.rng;
    let pixels = fill_rows(ctx.width, ctx.height, |x, y| {
        let (x, y) = (i64::from(x), i64::from(y));
        let u = x * hy + y * hx;
        let v = x * hy - y * hx;
        if u.rem_euclid(s) < hx || v.rem_euclid(s) < hx {
            return edge;
        }
        let (iu, iv) = (u.div_euclid(s), v.div_euclid(s));
        if chance(rng.at_n(iu, iv, 0), fill, 100) {
            fills[below(rng.at_n(iu, iv, 1), fills.len() as u64) as usize]
        } else {
            0
        }
    });
    Ok(finish_layer(ctx, pal, pixels))
}
