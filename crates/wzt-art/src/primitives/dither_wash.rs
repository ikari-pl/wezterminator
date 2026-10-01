//! `dither_wash`: a soft wash of colour, built from Bayer dither so it stays
//! pixel-crisp after the nearest-neighbour upscale.
//!
//! Parameters:
//! - `direction`: where the wash is densest: `bottom` (default), `top`,
//!   `left`, `right`, `center` or `edges`.
//! - `colors`: one colour, or a ramp dark to bright (default `surface`).
//!   With a ramp, the position along the gradient picks the stop.
//! - `alpha`: alpha of every dot (default 90).
//! - `strength`: percent coverage at the densest point (default 60).
//! - `cell`: Bayer matrix size, 4 or 8 (default 4).

use crate::dither::{bayer_q10, ramp_pick};
use crate::error::Result;
use crate::palette::PaletteBuilder;
use crate::primitives::{Ctx, fill_rows, finish_layer, radial_q10};
use crate::rng::below;

#[derive(Clone, Copy)]
enum Direction {
    Bottom,
    Top,
    Left,
    Right,
    Center,
    Edges,
}

pub(super) fn render(ctx: &Ctx<'_>) -> Result<crate::palette::Layer> {
    let p = &ctx.params;
    let direction = match p.choice(
        "direction",
        "bottom",
        &["bottom", "top", "left", "right", "center", "edges"],
    )? {
        "top" => Direction::Top,
        "left" => Direction::Left,
        "right" => Direction::Right,
        "center" => Direction::Center,
        "edges" => Direction::Edges,
        _ => Direction::Bottom,
    };
    let ramp = p.colour_list("colors", &["surface"], 1, 16)?;
    let alpha = p.uint("alpha", 90, 1, 255)? as u8;
    let strength = p.uint("strength", 60, 0, 100)? as u32;
    let k = match p.uint("cell", 4, 4, 8)? {
        4 => 2,
        8 => 3,
        other => return Err(p.error("cell", format!("{other} is not 4 or 8"))),
    };

    let mut pal = PaletteBuilder::new();
    let stops: Vec<u8> = ramp
        .iter()
        .map(|c| pal.add(*c, alpha))
        .collect::<Result<_>>()?;

    let (w, h) = (ctx.width, ctx.height);
    let rng = ctx.rng;
    let pixels = fill_rows(w, h, |x, y| {
        let v = match direction {
            Direction::Bottom => y * 1024 / h.saturating_sub(1).max(1),
            Direction::Top => 1024 - y * 1024 / h.saturating_sub(1).max(1),
            Direction::Right => x * 1024 / w.saturating_sub(1).max(1),
            Direction::Left => 1024 - x * 1024 / w.saturating_sub(1).max(1),
            Direction::Edges => radial_q10(x, y, w, h),
            Direction::Center => 1024 - radial_q10(x, y, w, h),
        }
        .min(1024);
        if v * strength / 100 <= bayer_q10(x, y, k) {
            return 0;
        }
        // Ramp stops blend by noise, so the colour mix never lines up with
        // (and so never fights) the Bayer pattern that decides coverage.
        let jitter = below(rng.at_n(i64::from(x), i64::from(y), 3), 1024) as u32;
        stops[ramp_pick(v, stops.len(), jitter)]
    });
    Ok(finish_layer(ctx, pal, pixels))
}
