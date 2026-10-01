//! `starfield`: single-pixel stars in three brightness tiers, a few with a
//! one-pixel cross.
//!
//! Parameters:
//! - `count`: stars per megapixel of the *logical* canvas (default 400). A
//!   density rather than a total, so the same recipe looks alike on a laptop
//!   and a 6K display.
//! - `crosses`: cross-shaped stars per megapixel (default `count / 64`).
//! - `colors`: three colours, near and hot to far and faint (default `fg`,
//!   `accent`, `fg_dim`).
//! - `alpha`: alpha of the brightest tier (default 220). The other tiers sit
//!   at 150/220 and 90/220 of it, as in `gen-backgrounds.py`.
//!
//! A star is decided by hashing its own pixel, so there is no occupancy set
//! and no retry loop: stars cannot collide because each pixel decides alone.

use crate::error::Result;
use crate::palette::PaletteBuilder;
use crate::primitives::{Ctx, fill_rows, finish_layer};
use crate::rng::{below, density_threshold};

pub(super) fn render(ctx: &Ctx<'_>) -> Result<crate::palette::Layer> {
    let p = &ctx.params;
    let count = p.uint("count", 400, 0, 100_000)?;
    let crosses = p.uint("crosses", count / 64, 0, 100_000)?;
    let colors = p.colour_list("colors", &["fg", "accent", "fg_dim"], 3, 3)?;
    let alpha = p.uint("alpha", 220, 1, 255)? as u32;

    let mut pal = PaletteBuilder::new();
    let tier = [
        pal.add(colors[0], alpha as u8)?,
        pal.add(colors[1], (alpha * 150 / 220).max(1) as u8)?,
        pal.add(colors[2], (alpha * 90 / 220).max(1) as u8)?,
    ];
    let centre = pal.add(colors[0], 255)?;
    let arm = pal.add(colors[1], (alpha / 2).max(1) as u8)?;

    let (w, h) = (ctx.width, ctx.height);
    let rng = ctx.rng;
    let star_below = density_threshold(count);
    let cross_below = density_threshold(crosses);

    let is_cross = move |x: i64, y: i64| {
        cross_below > 0
            && x >= 0
            && y >= 0
            && x < i64::from(w)
            && y < i64::from(h)
            && rng.at_n(x, y, 1) < cross_below
    };

    let pixels = fill_rows(w, h, move |x, y| {
        let (xi, yi) = (i64::from(x), i64::from(y));
        if is_cross(xi, yi) {
            return centre;
        }
        if rng.at_n(xi, yi, 0) < star_below {
            // Tier shares 12 / 30 / 58 percent: few hot stars, many faint ones.
            return match below(rng.at_n(xi, yi, 2), 100) {
                0..=11 => tier[0],
                12..=41 => tier[1],
                _ => tier[2],
            };
        }
        if is_cross(xi - 1, yi)
            || is_cross(xi + 1, yi)
            || is_cross(xi, yi - 1)
            || is_cross(xi, yi + 1)
        {
            return arm;
        }
        0
    });
    Ok(finish_layer(ctx, pal, pixels))
}
