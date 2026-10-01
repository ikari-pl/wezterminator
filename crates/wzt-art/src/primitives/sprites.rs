//! `sprite_strip` and `scattered_sprites`: small hand-drawn sprites from the
//! recipe itself, either laid in a row or sprinkled over the layer.
//!
//! A sprite is ASCII art in the recipe:
//!
//! ```json
//! { "rows": [".#.", "###", ".#."], "colors": { "#": "accent" } }
//! ```
//!
//! `.` and space are transparent, and every other character needs an entry in
//! `colors` (a hex colour or theme role). `sprite` takes one sprite and
//! `sprites` takes a list. With neither, a small built-in diamond in `accent`
//! is used so the primitive always draws something.
//!
//! `sprite_strip` parameters:
//! - `y`: baseline as a percent of the layer height (default 85). Sprites sit
//!   on it, bottoms aligned.
//! - `gap`: pixels between sprites (default 2).
//! - `offset`: x of the first sprite (default 0).
//! - `order`: `cycle` through the sprites or `random` (default `random`).
//!
//! `scattered_sprites` parameters:
//! - `cell`: the layer is divided into square cells (default four times the
//!   largest sprite). A cell holds at most one sprite, placed at a hashed
//!   position inside it, so sprites never overlap.
//! - `chance`: percent of cells that hold a sprite (default 25).
//!
//! Both read only their own pixel's cell, so they stay per-pixel pure.

use serde_json::Value;

use crate::error::Result;
use crate::palette::{Layer, PaletteBuilder};
use crate::primitives::{Ctx, fill_rows, finish_layer};
use crate::rng::{below, chance};

struct Sprite {
    w: u32,
    h: u32,
    /// Palette indices, row-major. 0 is transparent.
    cells: Vec<u8>,
}

impl Sprite {
    fn at(&self, x: u32, y: u32) -> u8 {
        self.cells[(y * self.w + x) as usize]
    }
}

const BUILTIN_ROWS: [&str; 5] = ["..#..", ".###.", "#####", ".###.", "..#.."];

fn parse_sprite(
    ctx: &Ctx<'_>,
    pal: &mut PaletteBuilder,
    name: &str,
    def: &Value,
) -> Result<Sprite> {
    let p = &ctx.params;
    let obj = def
        .as_object()
        .ok_or_else(|| p.error(name, "a sprite is an object with `rows` and `colors`"))?;
    let rows: Vec<&str> = obj
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| p.error(name, "a sprite needs a `rows` array of strings"))?
        .iter()
        .map(|r| {
            r.as_str()
                .ok_or_else(|| p.error(name, "`rows` entries must be strings"))
        })
        .collect::<Result<_>>()?;
    let w = rows.first().map_or(0, |r| r.chars().count());
    if rows.is_empty() || w == 0 || w > 256 || rows.len() > 256 {
        return Err(p.error(name, "a sprite is 1 to 256 rows of 1 to 256 characters"));
    }
    if rows.iter().any(|r| r.chars().count() != w) {
        return Err(p.error(name, "every row of a sprite must have the same width"));
    }
    for key in obj.keys().filter(|k| !k.starts_with('_')) {
        if key != "rows" && key != "colors" {
            return Err(p.error(name, format!("unknown sprite key `{key}`")));
        }
    }
    let colors = obj.get("colors").and_then(Value::as_object);

    let mut cells = Vec::with_capacity(w * rows.len());
    for row in &rows {
        for ch in row.chars() {
            if ch == '.' || ch == ' ' {
                cells.push(0);
                continue;
            }
            let spec = colors
                .and_then(|c| c.get(&ch.to_string()))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    p.error(name, format!("character `{ch}` has no entry in `colors`"))
                })?;
            let rgb = p.spec_to_rgb(name, spec)?;
            cells.push(pal.add(rgb, 255)?);
        }
    }
    Ok(Sprite {
        w: w as u32,
        h: rows.len() as u32,
        cells,
    })
}

fn load_sprites(ctx: &Ctx<'_>, pal: &mut PaletteBuilder) -> Result<Vec<Sprite>> {
    let p = &ctx.params;
    let list = p.raw("sprites");
    let single = p.raw("sprite");
    let defs: Vec<&Value> = match (list, single) {
        (Some(_), Some(_)) => return Err(p.error("sprite", "use `sprite` or `sprites`, not both")),
        (Some(Value::Array(items)), None) if !items.is_empty() && items.len() <= 32 => {
            items.iter().collect()
        }
        (Some(_), None) => return Err(p.error("sprites", "expected an array of 1 to 32 sprites")),
        (None, Some(one)) => vec![one],
        (None, None) => {
            let rgb = p.spec_to_rgb("sprite", "accent")?;
            let idx = pal.add(rgb, 255)?;
            let w = BUILTIN_ROWS[0].len() as u32;
            let cells = BUILTIN_ROWS
                .iter()
                .flat_map(|r| r.bytes().map(|b| if b == b'#' { idx } else { 0 }))
                .collect();
            return Ok(vec![Sprite {
                w,
                h: BUILTIN_ROWS.len() as u32,
                cells,
            }]);
        }
    };
    let name = if list.is_some() { "sprites" } else { "sprite" };
    defs.into_iter()
        .map(|d| parse_sprite(ctx, pal, name, d))
        .collect()
}

pub(super) fn render_strip(ctx: &Ctx<'_>) -> Result<Layer> {
    let p = &ctx.params;
    let y_pct = p.uint("y", 85, 0, 100)? as u32;
    let gap = p.uint("gap", 2, 0, 1024)? as u32;
    let offset = p.uint("offset", 0, 0, 65_535)? as u32;
    let order = p.choice("order", "random", &["random", "cycle"])?;

    let mut pal = PaletteBuilder::new();
    let sprites = load_sprites(ctx, &mut pal)?;
    let (w, h) = (ctx.width, ctx.height);
    let baseline = (h * y_pct / 100).min(h);

    // Lay the strip out serially: for each column, which sprite and which of
    // its columns, or none in the gaps.
    let mut columns: Vec<Option<(u16, u16)>> = vec![None; w as usize];
    let mut x = offset;
    let mut n = 0u64;
    while x < w {
        let which = match order {
            "cycle" => (n % sprites.len() as u64) as usize,
            _ => below(ctx.rng.nth(n), sprites.len() as u64) as usize,
        };
        let s = &sprites[which];
        for dx in 0..s.w {
            if x + dx < w {
                columns[(x + dx) as usize] = Some((which as u16, dx as u16));
            }
        }
        x += s.w + gap;
        n += 1;
    }

    let pixels = fill_rows(w, h, |x, y| {
        let Some((which, dx)) = columns[x as usize] else {
            return 0;
        };
        let s = &sprites[which as usize];
        if baseline < s.h {
            return 0;
        }
        let top = baseline - s.h;
        if y < top || y >= baseline {
            return 0;
        }
        s.at(u32::from(dx), y - top)
    });
    Ok(finish_layer(ctx, pal, pixels))
}

pub(super) fn render_scattered(ctx: &Ctx<'_>) -> Result<Layer> {
    let p = &ctx.params;
    let mut pal = PaletteBuilder::new();
    let sprites = load_sprites(ctx, &mut pal)?;
    let largest = sprites.iter().map(|s| s.w.max(s.h)).max().unwrap_or(1);
    let cell = (p.uint("cell", u64::from(largest) * 4, 1, 65_535)? as u32).max(largest);
    let chance_pct = p.uint("chance", 25, 0, 100)?;

    let rng = ctx.rng;
    let (w, h) = (ctx.width, ctx.height);
    let pixels = fill_rows(w, h, |x, y| {
        let (cx, cy) = (i64::from(x / cell), i64::from(y / cell));
        if !chance(rng.at_n(cx, cy, 0), chance_pct, 100) {
            return 0;
        }
        let which = below(rng.at_n(cx, cy, 1), sprites.len() as u64) as usize;
        let s = &sprites[which];
        // The sprite lives wholly inside its cell, so no neighbour reaches here.
        let ox = below(rng.at_n(cx, cy, 2), u64::from(cell - s.w + 1)) as u32;
        let oy = below(rng.at_n(cx, cy, 3), u64::from(cell - s.h + 1)) as u32;
        let (lx, ly) = (x % cell, y % cell);
        if lx < ox || ly < oy || lx - ox >= s.w || ly - oy >= s.h {
            return 0;
        }
        s.at(lx - ox, ly - oy)
    });
    Ok(finish_layer(ctx, pal, pixels))
}
