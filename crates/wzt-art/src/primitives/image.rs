//! `image`: a picture from disk, downscaled and dithered onto the theme's
//! palette. The work is in [`crate::import`].
//!
//! Parameters:
//! - `path` (required): the image file. Relative paths resolve against the
//!   theme's directory.
//! - `dither`: `bayer` (default) or `floyd-steinberg`.
//! - `fit`: `cover` (default), `contain` or `stretch`.
//! - `colors`: target palette, at most 255 colours (default: every distinct
//!   colour of the theme).
//! - `alpha_threshold`: source alpha below this is transparent (default 128).

use std::path::{Path, PathBuf};

use crate::dither::Dither;
use crate::error::Result;
use crate::import::{Fit, ImportOptions, import_image, load_image};
use crate::palette::Layer;
use crate::primitives::Ctx;

pub(super) fn render(ctx: &Ctx<'_>) -> Result<Layer> {
    let p = &ctx.params;
    let path = p
        .string("path")?
        .ok_or_else(|| p.error("path", "an image layer needs a `path`"))?;
    let dither = match p.choice("dither", "bayer", &["bayer", "floyd-steinberg"])? {
        "floyd-steinberg" => Dither::FloydSteinberg,
        _ => Dither::Bayer,
    };
    let fit = Fit::parse(p.choice("fit", "cover", &["cover", "contain", "stretch"])?)
        .unwrap_or(Fit::Cover);
    let alpha_threshold = p.uint("alpha_threshold", 128, 1, 255)? as u8;
    let palette = if p.raw("colors").is_some() {
        p.colour_list("colors", &[], 1, 255)?
    } else {
        let all = ctx.colors.default_palette();
        all[..all.len().min(255)].to_vec()
    };

    let given = Path::new(path);
    let resolved: PathBuf = if given.is_absolute() {
        given.to_path_buf()
    } else {
        ctx.base_dir
            .map_or_else(|| given.to_path_buf(), |dir| dir.join(given))
    };
    let img = load_image(&resolved)?;
    let options = ImportOptions {
        dither,
        fit,
        alpha_threshold,
        ..Default::default()
    };
    import_image(&img, ctx.width, ctx.height, &palette, &options)
}
