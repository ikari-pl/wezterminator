//! `vignette`: darkened corners, as dithered dots that thicken and strengthen
//! toward the edges.
//!
//! Parameters:
//! - `color`: edge colour (default `black`).
//! - `alpha`: alpha at the corners (default 110).
//! - `strength`: percent dot coverage at the corners (default 100).
//! - `steps`: alpha steps between none and `alpha` (default 8).

use crate::dither::bayer_q10;
use crate::error::Result;
use crate::palette::PaletteBuilder;
use crate::primitives::{Ctx, alpha_steps, fill_rows, finish_layer, isqrt, radial_q10, step_of};

pub(super) fn render(ctx: &Ctx<'_>) -> Result<crate::palette::Layer> {
    let p = &ctx.params;
    let color = p.colour("color", "black")?;
    let alpha = p.uint("alpha", 110, 1, 255)? as u8;
    let strength = p.uint("strength", 100, 0, 100)? as u32;
    let steps = p.uint("steps", 8, 1, 64)? as u32;

    let mut pal = PaletteBuilder::new();
    let ramp = alpha_steps(&mut pal, color, alpha, steps)?;

    let (w, h) = (ctx.width, ctx.height);
    let pixels = fill_rows(w, h, |x, y| {
        // v = r^2.5: nothing near the middle, a firm edge at the corners.
        let r = radial_q10(x, y, w, h);
        let v = (((r * r) >> 10) * isqrt(u64::from(r) << 10) as u32) >> 10;
        if v * strength / 100 <= bayer_q10(x, y, 2) {
            return 0;
        }
        match step_of(v, steps) {
            0 => 0,
            s => ramp[s as usize - 1],
        }
    });
    Ok(finish_layer(ctx, pal, pixels))
}
