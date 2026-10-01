//! Image import: map a picture onto a theme's palette.
//!
//! Order matters. The image is first **downscaled** to the layer's logical
//! size with an exact area-averaging box filter, and only then dithered. Dithering a
//! full-size photo and shrinking it afterwards would average the dither
//! pattern away into banded mush, and a dither pattern on source pixels would
//! land at a different scale than the final blocks.
//!
//! Colour matching is in Oklab, where straight distance tracks perceived
//! difference, so "nearest palette colour" is the colour that looks nearest.
//! Ordered dithering perturbs lightness by the Bayer threshold; Floyd–Steinberg
//! diffuses the full Oklab error.

use std::path::Path;

use image::RgbaImage;

use crate::dither::{Dither, bayer_q10, floyd_steinberg};
use crate::error::{ArtError, Result};
use crate::palette::{Layer, Rgb, oklab, oklab_distance2};
use crate::primitives::fill_rows;

/// How the picture fits the layer when the aspect ratios differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Fill the layer, cropping the overflow (centred).
    Cover,
    /// Fit inside, leaving the rest transparent (centred).
    Contain,
    /// Distort to the layer's shape.
    Stretch,
}

impl Fit {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cover" => Some(Fit::Cover),
            "contain" => Some(Fit::Contain),
            "stretch" => Some(Fit::Stretch),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ImportOptions {
    pub dither: Dither,
    pub fit: Fit,
    /// Source pixels with alpha below this are transparent.
    pub alpha_threshold: u8,
    /// Bayer only: the span the threshold moves lightness over, in Oklab L.
    /// `None` picks the mean lightness gap between palette colours, which is
    /// what makes coverage track lightness: 1.0 for black and white (a 57%
    /// grey gets 57% white), finer for a rich palette.
    pub bayer_spread: Option<f64>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions {
            dither: Dither::Bayer,
            fit: Fit::Cover,
            alpha_threshold: 128,
            bayer_spread: None,
        }
    }
}

/// Decode an image file (PNG, JPEG or GIF's first frame) to RGBA.
pub fn load_image(path: &Path) -> Result<RgbaImage> {
    let img = image::open(path).map_err(|e| ArtError::Image {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    Ok(img.to_rgba8())
}

/// For each destination index, the source pixels it covers and how much.
///
/// The destination pixel `i` spans `[start + i*step, start + (i+1)*step)` in
/// source coordinates, where `step = len / dst`. Each source pixel's weight is
/// its overlap with that span, so shrinking averages every source pixel that
/// falls inside (an exact box filter) and enlarging repeats pixels.
///
/// Only `+ - * /`, `floor` and `ceil` are used, which IEEE 754 makes exact
/// everywhere, so the result does not depend on the platform (the `image`
/// crate's own resizer makes no such promise).
fn axis_weights(src: usize, dst: usize, start: f64, len: f64) -> Vec<Vec<(usize, f64)>> {
    let step = len / dst as f64;
    (0..dst)
        .map(|i| {
            let lo = start + i as f64 * step;
            let hi = lo + step;
            let first = lo.floor().max(0.0) as usize;
            let last = (hi.ceil() as usize).min(src);
            (first..last)
                .map(|p| (p, (hi.min(p as f64 + 1.0) - lo.max(p as f64)).max(0.0)))
                .filter(|&(_, w)| w > 0.0)
                .collect()
        })
        .collect()
}

/// Area-average the crop `(x, y, w, h)` of `img` (source pixels, fractional)
/// down to `tw` x `th`. Colour is averaged with alpha as weight, so
/// transparent pixels do not drag the colour of their neighbours toward black.
fn area_resample(img: &RgbaImage, crop: (f64, f64, f64, f64), tw: u32, th: u32) -> RgbaImage {
    let (sw, sh) = (img.width() as usize, img.height() as usize);
    let xs = axis_weights(sw, tw as usize, crop.0, crop.2);
    let ys = axis_weights(sh, th as usize, crop.1, crop.3);

    // Horizontal pass: per source row, premultiplied [r*a, g*a, b*a, a] sums.
    let mut rows = vec![[0.0f64; 4]; tw as usize * sh];
    for y in 0..sh {
        for (x, taps) in xs.iter().enumerate() {
            let mut acc = [0.0f64; 4];
            for &(sx, w) in taps {
                let p = img.get_pixel(sx as u32, y as u32);
                let a = f64::from(p[3]) / 255.0;
                acc[0] += w * a * f64::from(p[0]);
                acc[1] += w * a * f64::from(p[1]);
                acc[2] += w * a * f64::from(p[2]);
                acc[3] += w * a;
            }
            rows[y * tw as usize + x] = acc;
        }
    }

    let mut out = RgbaImage::new(tw, th);
    let area = (crop.2 / tw as f64) * (crop.3 / th as f64); // source pixels per output pixel
    for (y, taps) in ys.iter().enumerate() {
        for x in 0..tw as usize {
            let mut acc = [0.0f64; 4];
            for &(sy, w) in taps {
                let r = rows[sy * tw as usize + x];
                for c in 0..4 {
                    acc[c] += w * r[c];
                }
            }
            let alpha = acc[3] / area; // 0..=1
            let px = if acc[3] <= 0.0 {
                [0, 0, 0, 0]
            } else {
                let q = |v: f64| (v / acc[3] + 0.5).floor().clamp(0.0, 255.0) as u8;
                [
                    q(acc[0]),
                    q(acc[1]),
                    q(acc[2]),
                    (alpha * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8,
                ]
            };
            out.put_pixel(x as u32, y as u32, image::Rgba(px));
        }
    }
    out
}

/// Downscale (or enlarge) `img` to exactly `width` x `height` per `fit`.
/// Uncovered area, for [`Fit::Contain`], is transparent.
fn fit_to(img: &RgbaImage, width: u32, height: u32, fit: Fit) -> RgbaImage {
    let (sw, sh) = (img.width() as f64, img.height() as f64);
    let (tw, th) = (f64::from(width), f64::from(height));
    match fit {
        Fit::Stretch => area_resample(img, (0.0, 0.0, sw, sh), width, height),
        Fit::Cover => {
            // Crop the source to the target's aspect ratio, centred.
            let crop = if sw * th > sh * tw {
                let cw = sh * tw / th;
                ((sw - cw) / 2.0, 0.0, cw, sh)
            } else {
                let ch = sw * th / tw;
                (0.0, (sh - ch) / 2.0, sw, ch)
            };
            area_resample(img, crop, width, height)
        }
        Fit::Contain => {
            let (nw, nh) = if sw * th > sh * tw {
                (width, ((sh * tw / sw).floor() as u32).clamp(1, height))
            } else {
                (((sw * th / sh).floor() as u32).clamp(1, width), height)
            };
            let placed = area_resample(img, (0.0, 0.0, sw, sh), nw, nh);
            let mut canvas = RgbaImage::new(width, height);
            let (ox, oy) = ((width - nw) / 2, (height - nh) / 2);
            for y in 0..nh {
                for x in 0..nw {
                    canvas.put_pixel(ox + x, oy + y, *placed.get_pixel(x, y));
                }
            }
            canvas
        }
    }
}

/// Mean gap between consecutive palette lightnesses, clamped to a useful range.
fn mean_lightness_gap(lab: &[[f64; 3]]) -> f64 {
    if lab.len() < 2 {
        return 1.0;
    }
    let lo = lab.iter().map(|c| c[0]).fold(f64::INFINITY, f64::min);
    let hi = lab.iter().map(|c| c[0]).fold(f64::NEG_INFINITY, f64::max);
    ((hi - lo) / (lab.len() - 1) as f64).clamp(0.04, 1.0)
}

/// Map `img` onto `palette` at `width` x `height`.
///
/// The returned layer's palette is transparent plus `palette` in order, so
/// colour `i` of the input is index `i + 1`. At most 255 colours.
pub fn import_image(
    img: &RgbaImage,
    width: u32,
    height: u32,
    palette: &[Rgb],
    options: &ImportOptions,
) -> Result<Layer> {
    if palette.is_empty() || palette.len() > 255 {
        return Err(ArtError::Theme {
            message: format!(
                "import needs 1 to 255 palette colours, found {}",
                palette.len()
            ),
        });
    }
    if width == 0 || height == 0 || img.width() == 0 || img.height() == 0 {
        return Err(ArtError::BadDevice {
            width: u64::from(width),
            height: u64::from(height),
        });
    }

    // 1. Downscale to logical size. 2. Only then dither.
    let small = fit_to(img, width, height, options.fit);
    let lab: Vec<[f64; 3]> = palette.iter().map(|c| oklab(*c)).collect();
    let nearest = |v: [f64; 3]| -> usize {
        let mut best = (0, f64::INFINITY);
        for (i, p) in lab.iter().enumerate() {
            let d = oklab_distance2(v, *p);
            if d < best.1 {
                best = (i, d);
            }
        }
        best.0
    };
    let opaque = |px: &image::Rgba<u8>| px[3] >= options.alpha_threshold;
    let pixel_lab = |px: &image::Rgba<u8>| oklab([px[0], px[1], px[2]]);

    let pixels = match options.dither {
        Dither::Bayer => {
            let spread = options
                .bayer_spread
                .unwrap_or_else(|| mean_lightness_gap(&lab));
            fill_rows(width, height, |x, y| {
                let px = small.get_pixel(x, y);
                if !opaque(px) {
                    return 0;
                }
                let mut v = pixel_lab(px);
                let t = f64::from(bayer_q10(x, y, 3)) / 1024.0 - 0.5;
                v[0] += t * spread;
                (nearest(v) + 1) as u8
            })
        }
        Dither::FloydSteinberg => {
            let mut work: Vec<[f64; 3]> = small.pixels().map(pixel_lab).collect();
            floyd_steinberg(width as usize, height as usize, &mut work, |i, v| {
                let x = (i % width as usize) as u32;
                let y = (i / width as usize) as u32;
                if !opaque(small.get_pixel(x, y)) {
                    return None;
                }
                let n = nearest(v);
                Some(((n + 1) as u8, lab[n]))
            })
        }
    };

    let mut full = vec![[0, 0, 0, 0]];
    full.extend(palette.iter().map(|c| [c[0], c[1], c[2], 255]));
    Ok(Layer {
        width,
        height,
        palette: full,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn gradient(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, _| {
            let v = (x * 255 / (w - 1)) as u8;
            Rgba([v, v, v, 255])
        })
    }

    #[test]
    fn gradient_maps_only_to_palette_indices() {
        let palette = [[0, 0, 0], [0, 0, 128], [128, 255, 255], [255, 255, 255]];
        for dither in [Dither::Bayer, Dither::FloydSteinberg] {
            let opts = ImportOptions {
                dither,
                ..Default::default()
            };
            let layer = import_image(&gradient(256, 64), 32, 8, &palette, &opts).unwrap();
            assert_eq!((layer.width, layer.height), (32, 8));
            assert_eq!(layer.pixels.len(), 32 * 8);
            assert!(
                layer.pixels.iter().all(|&i| (1..=4).contains(&i)),
                "{dither:?}"
            );
            assert_eq!(layer.palette.len(), 5);
            assert_eq!(layer.palette[0][3], 0);
        }
    }

    #[test]
    fn downscaling_happens_before_dithering() {
        // A flat mid-grey source, huge compared with the target. If dithering ran
        // on source pixels, the 8x8 result would be a crop of a fine pattern;
        // dithered at logical size it is a balanced mix of the two colours.
        let src = RgbaImage::from_pixel(512, 512, Rgba([119, 119, 119, 255]));
        let palette = [[0, 0, 0], [255, 255, 255]];
        for dither in [Dither::Bayer, Dither::FloydSteinberg] {
            let opts = ImportOptions {
                dither,
                ..Default::default()
            };
            let layer = import_image(&src, 8, 8, &palette, &opts).unwrap();
            let white = layer.pixels.iter().filter(|&&i| i == 2).count();
            let black = layer.pixels.iter().filter(|&&i| i == 1).count();
            assert_eq!(white + black, 64);
            // Oklab L of #777 is about 0.57, so a little over half the pixels light.
            assert!((24..=48).contains(&white), "{dither:?}: white = {white}");
            assert!(black >= 16, "{dither:?}: black = {black}");
        }
    }

    #[test]
    fn import_is_deterministic() {
        let palette = [[12, 12, 24], [0, 0, 128], [128, 255, 255]];
        let a = import_image(
            &gradient(100, 40),
            25,
            10,
            &palette,
            &ImportOptions::default(),
        )
        .unwrap();
        let b = import_image(
            &gradient(100, 40),
            25,
            10,
            &palette,
            &ImportOptions::default(),
        )
        .unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn transparent_source_pixels_stay_transparent() {
        let mut src = RgbaImage::from_pixel(4, 4, Rgba([255, 255, 255, 255]));
        src.put_pixel(0, 0, Rgba([255, 255, 255, 0]));
        let opts = ImportOptions {
            fit: Fit::Stretch,
            ..Default::default()
        };
        let layer = import_image(&src, 4, 4, &[[255, 255, 255]], &opts).unwrap();
        assert_eq!(layer.index_at(0, 0), 0);
        assert_eq!(layer.index_at(3, 3), 1);
    }

    #[test]
    fn contain_letterboxes_and_cover_fills() {
        let wide = RgbaImage::from_pixel(40, 10, Rgba([255, 255, 255, 255]));
        let pal = [[255, 255, 255]];
        let contain = ImportOptions {
            fit: Fit::Contain,
            ..Default::default()
        };
        let l = import_image(&wide, 8, 8, &pal, &contain).unwrap();
        assert_eq!(l.index_at(4, 0), 0, "top row is letterbox");
        assert_eq!(l.index_at(4, 4), 1);
        let cover = ImportOptions {
            fit: Fit::Cover,
            ..Default::default()
        };
        let l = import_image(&wide, 8, 8, &pal, &cover).unwrap();
        assert!(l.pixels.iter().all(|&i| i == 1));
    }

    #[test]
    fn rejects_bad_palettes() {
        let img = gradient(4, 4);
        assert!(import_image(&img, 4, 4, &[], &ImportOptions::default()).is_err());
        let big = vec![[1, 2, 3]; 256];
        assert!(import_image(&img, 4, 4, &big, &ImportOptions::default()).is_err());
    }
}

#[cfg(test)]
mod resample_tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn shrinking_averages_whole_blocks() {
        // 4x4 source, 2x2 target: each target pixel is the mean of a 2x2 block.
        let src = RgbaImage::from_fn(4, 4, |x, _| {
            if x < 2 {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([200, 100, 50, 255])
            }
        });
        let out = area_resample(&src, (0.0, 0.0, 4.0, 4.0), 2, 2);
        assert_eq!(*out.get_pixel(0, 0), Rgba([0, 0, 0, 255]));
        assert_eq!(*out.get_pixel(1, 1), Rgba([200, 100, 50, 255]));
        // 4x1 -> 1 pixel averages all four.
        let row = RgbaImage::from_fn(4, 1, |x, _| Rgba([x as u8 * 40, 0, 0, 255]));
        let one = area_resample(&row, (0.0, 0.0, 4.0, 1.0), 1, 1);
        assert_eq!(one.get_pixel(0, 0)[0], 60);
    }

    #[test]
    fn fractional_spans_weight_by_overlap() {
        // 3 source pixels into 2 target pixels: target 0 = src0 + half of src1.
        let src = RgbaImage::from_fn(3, 1, |x, _| Rgba([[0u8, 90, 240][x as usize], 0, 0, 255]));
        let out = area_resample(&src, (0.0, 0.0, 3.0, 1.0), 2, 1);
        assert_eq!(out.get_pixel(0, 0)[0], 30); // (0*1 + 90*0.5) / 1.5
        assert_eq!(out.get_pixel(1, 0)[0], 190); // (90*0.5 + 240*1) / 1.5
    }

    #[test]
    fn enlarging_repeats_pixels() {
        let src = RgbaImage::from_fn(2, 1, |x, _| Rgba([x as u8 * 255, 0, 0, 255]));
        let out = area_resample(&src, (0.0, 0.0, 2.0, 1.0), 6, 1);
        let reds: Vec<u8> = (0..6).map(|x| out.get_pixel(x, 0)[0]).collect();
        assert_eq!(reds, [0, 0, 0, 255, 255, 255]);
    }

    #[test]
    fn transparent_pixels_do_not_darken_their_neighbours() {
        let mut src = RgbaImage::from_pixel(2, 1, Rgba([255, 255, 255, 255]));
        src.put_pixel(1, 0, Rgba([0, 0, 0, 0]));
        let out = area_resample(&src, (0.0, 0.0, 2.0, 1.0), 1, 1);
        let p = out.get_pixel(0, 0);
        assert_eq!((p[0], p[1], p[2]), (255, 255, 255));
        assert_eq!(p[3], 128);
    }
}
