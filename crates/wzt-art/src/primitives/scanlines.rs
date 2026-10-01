//! `scanlines`: evenly spaced horizontal lines, like a CRT.
//!
//! Parameters:
//! - `period`: rows from one line to the next (default 2).
//! - `thickness`: rows per line, at most `period` (default 1).
//! - `phase`: row offset of the first line (default 0).
//! - `alpha`: line alpha (default 40).
//! - `color`: line colour (default `black`).

use crate::error::Result;
use crate::palette::PaletteBuilder;
use crate::primitives::{Ctx, fill_rows, finish_layer};

pub(super) fn render(ctx: &Ctx<'_>) -> Result<crate::palette::Layer> {
    let p = &ctx.params;
    let period = p.uint("period", 2, 1, 256)? as u32;
    let thickness = p.uint("thickness", 1, 1, 256)? as u32;
    if thickness > period {
        return Err(p.error(
            "thickness",
            format!("{thickness} is larger than the period {period}"),
        ));
    }
    let phase = p.uint("phase", 0, 0, 255)? as u32 % period;
    let alpha = p.uint("alpha", 40, 1, 255)? as u8;
    let color = p.colour("color", "black")?;

    let mut pal = PaletteBuilder::new();
    let line = pal.add(color, alpha)?;

    let pixels = fill_rows(ctx.width, ctx.height, |_, y| {
        if (y + period - phase) % period < thickness {
            line
        } else {
            0
        }
    });
    Ok(finish_layer(ctx, pal, pixels))
}
