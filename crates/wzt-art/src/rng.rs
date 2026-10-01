//! Counter-based hashing, so art never depends on evaluation order.
//!
//! A sequential generator (`rand`, Python's `random.Random`) makes pixel N
//! depend on how many draws came before it, which ties the output to the order
//! rows are visited in. Here every random value is a pure function of
//! `(seed, x, y, stream)`. Rows can be computed on any number of threads, in
//! any order, and the pixels are identical.
//!
//! The mixer is the SplitMix64 finaliser, which passes the usual avalanche
//! tests and costs three multiplies. Nothing here touches floating point.

/// SplitMix64 finaliser: a bijective, well-mixed `u64 -> u64`.
pub const fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// FNV-1a over bytes. Used to turn layer ids into seed salts.
pub fn hash_str(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A seeded hash stream. `Copy`, so closures capture it freely across threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rng {
    seed: u64,
}

impl Rng {
    pub const fn new(seed: u64) -> Self {
        Rng {
            seed: mix64(seed ^ 0x9e37_79b9_7f4a_7c15),
        }
    }

    /// An independent stream derived from this one.
    pub const fn fork(self, salt: u64) -> Self {
        Rng {
            seed: mix64(self.seed ^ mix64(salt.wrapping_add(0x6a09_e667_f3bc_c909))),
        }
    }

    /// An independent stream for a layer, from its id.
    pub fn for_layer(self, id: &str) -> Self {
        self.fork(hash_str(id))
    }

    /// The hash at a pixel. Coordinates may be negative lattice positions.
    #[inline]
    pub const fn at(self, x: i64, y: i64) -> u64 {
        let key = ((y as u64) << 32) ^ (x as u64 & 0xffff_ffff);
        mix64(self.seed ^ mix64(key.wrapping_add(0xd1b5_4a32_d192_ed03)))
    }

    /// The hash at a pixel in a numbered sub-stream, for when one pixel needs
    /// several independent draws (tier, cross flag, jitter).
    #[inline]
    pub const fn at_n(self, x: i64, y: i64, stream: u64) -> u64 {
        self.fork(stream).at(x, y)
    }

    /// The `i`-th value of a plain sequence, for serial setup work.
    #[inline]
    pub const fn nth(self, i: u64) -> u64 {
        mix64(
            self.seed
                .wrapping_add(i.wrapping_mul(0x9e37_79b9_7f4a_7c15)),
        )
    }
}

/// Map a hash to `0..n` without modulo bias worth caring about (multiply-shift).
#[inline]
pub const fn below(hash: u64, n: u64) -> u64 {
    ((hash as u128 * n as u128) >> 64) as u64
}

/// A value in `lo..=hi`.
#[inline]
pub const fn range(hash: u64, lo: u64, hi: u64) -> u64 {
    lo + below(hash, hi - lo + 1)
}

/// True with probability `num / den`.
#[inline]
pub const fn chance(hash: u64, num: u64, den: u64) -> bool {
    below(hash, den) < num
}

/// Fixed-point threshold for "probability `p` per pixel", as a `u64` to
/// compare a raw hash against. `per_million` is occurrences per 1,000,000
/// pixels.
pub const fn density_threshold(per_million: u64) -> u64 {
    // per_million / 1e6 of the u64 range, in u128 to avoid overflow.
    ((per_million as u128 * (u64::MAX as u128)) / 1_000_000) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_inputs_same_hash_and_streams_differ() {
        let r = Rng::new(7);
        assert_eq!(r.at(3, 4), Rng::new(7).at(3, 4));
        assert_ne!(r.at(3, 4), r.at(4, 3));
        assert_ne!(r.at(3, 4), Rng::new(8).at(3, 4));
        assert_ne!(r.at_n(3, 4, 1), r.at_n(3, 4, 2));
        assert_ne!(r.for_layer("stars").at(0, 0), r.for_layer("grid").at(0, 0));
    }

    #[test]
    fn below_stays_in_range_and_covers_it() {
        let r = Rng::new(1);
        let mut seen = [false; 10];
        for i in 0..1000 {
            let v = below(r.nth(i), 10) as usize;
            seen[v] = true;
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn density_threshold_matches_requested_rate() {
        let r = Rng::new(99);
        let t = density_threshold(10_000); // 1%
        let hits = (0..100_000).filter(|&i| r.nth(i) < t).count();
        assert!((800..1200).contains(&hits), "hits = {hits}");
    }

    #[test]
    fn negative_coordinates_do_not_collide_with_positive() {
        let r = Rng::new(5);
        assert_ne!(r.at(-1, 0), r.at(1, 0));
        assert_ne!(r.at(0, -1), r.at(0, 1));
    }
}
