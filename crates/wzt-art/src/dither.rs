//! Ordered (Bayer) and error-diffusion (Floyd–Steinberg) dithering.
//!
//! Bayer is a pure function of the pixel position, so it composes with the
//! per-pixel renderer and is exact on any thread count. Floyd–Steinberg walks
//! the image serially (each pixel's error feeds its neighbours), which is fine
//! because it only runs on imported images at logical resolution.

/// Which dither import uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dither {
    Bayer,
    FloydSteinberg,
}

impl Dither {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bayer" => Some(Dither::Bayer),
            "floyd-steinberg" | "floyd_steinberg" | "fs" => Some(Dither::FloydSteinberg),
            _ => None,
        }
    }
}

/// Value of the `2^k x 2^k` Bayer matrix at `(x, y)`, in `0..4^k`.
///
/// Built from the recursion `B(2n) = 4*B(n) + B2` with `B2 = [[0,2],[3,1]]`,
/// which for `k = 2` reproduces the 4x4 matrix `gen-backgrounds.py` used.
pub const fn bayer(x: u32, y: u32, k: u32) -> u32 {
    const B2: [[u32; 2]; 2] = [[0, 2], [3, 1]];
    let mut v = 0;
    let mut l = 0;
    while l < k {
        let cell = B2[((y >> l) & 1) as usize][((x >> l) & 1) as usize];
        v += cell * 4u32.pow(k - 1 - l);
        l += 1;
    }
    v
}

/// The Bayer value as a threshold in Q10 (`0..1024`), centred in its step so
/// a coverage of exactly 0 never lights a pixel and 1024 always does.
#[inline]
pub const fn bayer_q10(x: u32, y: u32, k: u32) -> u32 {
    let n = 4u32.pow(k);
    ((2 * bayer(x, y, k) + 1) * 1024) / (2 * n)
}

/// Pick between the two neighbouring ramp stops for a value `v` in Q10
/// (`0..=1024`, dark to bright). The Bayer threshold decides which side of the
/// boundary each pixel lands on, so the stops blend by dot density.
#[inline]
pub fn ramp_pick(v_q10: u32, stops: usize, threshold_q10: u32) -> usize {
    debug_assert!(stops >= 1);
    if stops == 1 {
        return 0;
    }
    let t = v_q10.min(1024) * (stops as u32 - 1);
    let i = (t >> 10) as usize;
    let frac = t & 1023;
    if i >= stops - 1 {
        stops - 1
    } else if threshold_q10 < frac {
        i + 1
    } else {
        i
    }
}

/// Floyd–Steinberg error diffusion over a 3-channel image, left to right.
///
/// `quantize(pixel_index, value)` returns the chosen palette index and the
/// value that index stands for, or `None` for a pixel that takes no part
/// (transparent): it is skipped and passes no error on.
pub fn floyd_steinberg(
    width: usize,
    height: usize,
    pixels: &mut [[f64; 3]],
    mut quantize: impl FnMut(usize, [f64; 3]) -> Option<(u8, [f64; 3])>,
) -> Vec<u8> {
    let mut out = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            let old = pixels[i];
            let Some((index, chosen)) = quantize(i, old) else {
                continue;
            };
            out[i] = index;
            let err = [old[0] - chosen[0], old[1] - chosen[1], old[2] - chosen[2]];
            let mut spread = |dx: isize, dy: usize, weight: f64| {
                let nx = x as isize + dx;
                if nx < 0 || nx >= width as isize || y + dy >= height {
                    return;
                }
                let p = &mut pixels[(y + dy) * width + nx as usize];
                for c in 0..3 {
                    p[c] += err[c] * weight;
                }
            };
            spread(1, 0, 7.0 / 16.0);
            spread(-1, 1, 3.0 / 16.0);
            spread(0, 1, 5.0 / 16.0);
            spread(1, 1, 1.0 / 16.0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bayer4_matches_the_original_matrix() {
        let original = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
        for (y, row) in original.iter().enumerate() {
            for (x, &want) in row.iter().enumerate() {
                assert_eq!(bayer(x as u32, y as u32, 2), want, "({x},{y})");
            }
        }
    }

    #[test]
    fn bayer_is_a_permutation_at_every_order() {
        for k in 1..=3 {
            let n = 1u32 << k;
            let mut seen = vec![false; (n * n) as usize];
            for y in 0..n {
                for x in 0..n {
                    seen[bayer(x, y, k) as usize] = true;
                }
            }
            assert!(seen.iter().all(|&s| s), "order {k}");
        }
    }

    #[test]
    fn threshold_extremes() {
        for y in 0..8 {
            for x in 0..8 {
                let t = bayer_q10(x, y, 3);
                assert!(t > 0 && t < 1024);
            }
        }
    }

    #[test]
    fn ramp_pick_coverage_matches_value() {
        // Halfway between two stops: about half the 4x4 cell takes the upper one.
        let upper = (0..16)
            .filter(|&i| ramp_pick(512, 2, bayer_q10(i % 4, i / 4, 2)) == 1)
            .count();
        assert_eq!(upper, 8);
        assert_eq!(ramp_pick(0, 3, 500), 0);
        assert_eq!(ramp_pick(1024, 3, 0), 2);
        assert_eq!(ramp_pick(700, 1, 0), 0);
    }

    #[test]
    fn floyd_steinberg_flat_grey_gives_half_and_half() {
        let (w, h) = (8, 8);
        let mut px = vec![[0.5, 0.5, 0.5]; w * h];
        let out = floyd_steinberg(w, h, &mut px, |_, v| {
            if v[0] >= 0.5 {
                Some((1, [1.0; 3]))
            } else {
                Some((0, [0.0; 3]))
            }
        });
        let ones = out.iter().filter(|&&i| i == 1).count();
        assert!((28..=36).contains(&ones), "ones = {ones}");
    }
}
