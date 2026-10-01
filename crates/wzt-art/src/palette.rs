//! Palette-indexed layers and the colours a theme gives them.
//!
//! A [`Layer`] is a buffer of palette indices plus the palette. Index 0 is
//! always fully transparent, so an untouched pixel is empty. Each palette entry
//! is `(r, g, b, alpha)`: the layers carry their own soft edges and faint
//! washes as alpha steps, while WezTerm applies the per-layer opacity at
//! draw time.
//!
//! Colours in recipes are either `#rrggbb` hex or a *role* name taken from the
//! theme (`accent`, `fg_dim`, `ansi4`, ...), so one recipe re-colours itself
//! when the theme changes.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use wzt_model::Theme;

use crate::error::{ArtError, Result};

/// `[r, g, b]`.
pub type Rgb = [u8; 3];
/// `[r, g, b, a]`.
pub type Rgba = [u8; 4];

/// Parse `#rgb`, `#rrggbb` or `#rrggbbaa`. Alpha defaults to 255.
pub fn parse_hex(s: &str) -> Option<Rgba> {
    let h = s.strip_prefix('#')?;
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    let nibble = |i: usize| u8::from_str_radix(&h[i..=i], 16).ok().map(|n| n * 17);
    match h.len() {
        3 => Some([nibble(0)?, nibble(1)?, nibble(2)?, 255]),
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

/// Builds a palette up front, before any pixel is computed.
///
/// Primitives register every `(colour, alpha)` they can emit and keep the
/// returned indices. That keeps the pixel loop free of shared mutable state,
/// which is what lets it run on any number of threads and still be exact.
#[derive(Debug, Clone)]
pub struct PaletteBuilder {
    entries: Vec<Rgba>,
}

impl Default for PaletteBuilder {
    fn default() -> Self {
        PaletteBuilder {
            entries: vec![[0, 0, 0, 0]],
        }
    }
}

impl PaletteBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a colour and return its index. Repeats return the same index.
    /// A fully transparent colour is index 0.
    pub fn add(&mut self, rgb: Rgb, alpha: u8) -> Result<u8> {
        if alpha == 0 {
            return Ok(0);
        }
        let entry = [rgb[0], rgb[1], rgb[2], alpha];
        if let Some(i) = self.entries.iter().position(|e| *e == entry) {
            return Ok(i as u8);
        }
        if self.entries.len() >= 256 {
            return Err(ArtError::PaletteFull);
        }
        self.entries.push(entry);
        Ok((self.entries.len() - 1) as u8)
    }

    pub fn finish(self) -> Vec<Rgba> {
        self.entries
    }
}

/// One generated layer at logical resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    pub width: u32,
    pub height: u32,
    /// Entry 0 is transparent.
    pub palette: Vec<Rgba>,
    /// Row-major palette indices, `width * height` of them.
    pub pixels: Vec<u8>,
}

impl Layer {
    /// An empty (fully transparent) layer.
    pub fn blank(width: u32, height: u32) -> Self {
        Layer {
            width,
            height,
            palette: vec![[0, 0, 0, 0]],
            pixels: vec![0; width as usize * height as usize],
        }
    }

    pub fn index_at(&self, x: u32, y: u32) -> u8 {
        self.pixels[y as usize * self.width as usize + x as usize]
    }

    pub fn rgba_at(&self, x: u32, y: u32) -> Rgba {
        self.palette[self.index_at(x, y) as usize]
    }

    /// Number of pixels that are not transparent.
    pub fn covered(&self) -> usize {
        self.pixels
            .iter()
            .filter(|&&i| self.palette[i as usize][3] != 0)
            .count()
    }

    /// blake3 over dimensions, palette and indices. Equal hashes mean equal
    /// pixels, which is what the thread-count determinism test compares.
    pub fn content_hash(&self) -> String {
        let mut h = blake3::Hasher::new();
        h.update(&self.width.to_le_bytes());
        h.update(&self.height.to_le_bytes());
        h.update(&(self.palette.len() as u32).to_le_bytes());
        for e in &self.palette {
            h.update(e);
        }
        h.update(&self.pixels);
        h.finalize().to_hex().to_string()
    }
}

/// The colours a theme offers art, addressable by role.
#[derive(Debug, Clone)]
pub struct ThemeColors {
    roles: BTreeMap<String, Rgb>,
    /// Roles in a stable order, for the default import palette.
    default_palette: Vec<Rgb>,
}

impl ThemeColors {
    pub fn from_theme(theme: &Theme) -> Result<Self> {
        let mut roles = BTreeMap::new();
        let mut order: Vec<Rgb> = Vec::new();
        let mut put = |name: &str, spec: &str, order: &mut Vec<Rgb>| -> Result<()> {
            let c = parse_hex(spec).ok_or_else(|| ArtError::Theme {
                message: format!("colour `{name}` is not a hex colour: `{spec}`"),
            })?;
            let rgb = [c[0], c[1], c[2]];
            roles.insert(name.to_string(), rgb);
            if !order.contains(&rgb) {
                order.push(rgb);
            }
            Ok(())
        };

        let ui = &theme.palette.ui;
        put("base", &theme.art.base_color, &mut order)?;
        put("bg", &ui.bg, &mut order)?;
        put("surface", &ui.surface, &mut order)?;
        put("fg", &ui.fg, &mut order)?;
        put("fg_dim", &ui.fg_dim, &mut order)?;
        put("accent", &ui.accent, &mut order)?;
        put(
            "accent_alt",
            ui.accent_alt.as_deref().unwrap_or(&ui.accent),
            &mut order,
        )?;
        put("ok", &ui.ok, &mut order)?;
        put("warn", &ui.warn, &mut order)?;
        put("bad", &ui.bad, &mut order)?;
        put("info", &ui.info, &mut order)?;
        put("text", &theme.palette.scheme.foreground, &mut order)?;
        for (i, c) in theme.palette.scheme.ansi.iter().enumerate() {
            put(&format!("ansi{i}"), c, &mut order)?;
        }
        for (i, c) in theme.palette.scheme.brights.iter().enumerate() {
            put(&format!("bright{i}"), c, &mut order)?;
        }
        roles.insert("black".into(), [0, 0, 0]);
        roles.insert("white".into(), [255, 255, 255]);
        Ok(ThemeColors {
            roles,
            default_palette: order,
        })
    }

    /// A role name or a hex colour. Alpha in a hex colour is ignored here;
    /// primitives take alpha as its own parameter.
    pub fn resolve(&self, spec: &str) -> Option<Rgb> {
        if let Some(c) = self.roles.get(spec) {
            return Some(*c);
        }
        parse_hex(spec).map(|c| [c[0], c[1], c[2]])
    }

    /// Every distinct theme colour, in role order. The default target palette
    /// for image import.
    pub fn default_palette(&self) -> &[Rgb] {
        &self.default_palette
    }
}

// ---------------------------------------------------------------------------
// Colour science, without libm
// ---------------------------------------------------------------------------
//
// `powf` and `cbrt` come from the platform's libm and may differ in the last
// bit between macOS and Linux. Import and the legibility check compare
// distances and thresholds, so a last-bit difference could flip a pixel. The
// helpers below use only `+ - * /`, which IEEE 754 fixes exactly, so the same
// image maps to the same indices everywhere.

/// `a^(1/5)` for `0 < a <= 1`, by Newton's method.
fn fifth_root(a: f64) -> f64 {
    let mut r = 1.0;
    for _ in 0..80 {
        let r2 = r * r;
        r = (4.0 * r + a / (r2 * r2)) / 5.0;
    }
    r
}

/// `a^(1/3)` for `a >= 0`, by Newton's method.
fn cube_root(a: f64) -> f64 {
    if a <= 0.0 {
        return 0.0;
    }
    let mut r = if a > 1.0 { a } else { 1.0 };
    for _ in 0..100 {
        r = (2.0 * r + a / (r * r)) / 3.0;
    }
    r
}

/// sRGB channel (0..=255) to linear light (0..=1).
pub fn srgb_to_linear(c: u8) -> f64 {
    static LUT: OnceLock<[f64; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut lut = [0.0; 256];
        for (i, slot) in lut.iter_mut().enumerate() {
            let s = i as f64 / 255.0;
            *slot = if s <= 0.04045 {
                s / 12.92
            } else {
                // ((s + 0.055) / 1.055)^2.4 = y^2 * (y^2)^(1/5)
                let y = (s + 0.055) / 1.055;
                let y2 = y * y;
                y2 * fifth_root(y2)
            };
        }
        lut
    })[c as usize]
}

/// sRGB to Oklab `[L, a, b]` (Björn Ottosson's matrices).
pub fn oklab(rgb: Rgb) -> [f64; 3] {
    let (r, g, b) = (
        srgb_to_linear(rgb[0]),
        srgb_to_linear(rgb[1]),
        srgb_to_linear(rgb[2]),
    );
    let l = cube_root(0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b);
    let m = cube_root(0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b);
    let s = cube_root(0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b);
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    ]
}

/// Squared Euclidean distance in Oklab, which is perceptual distance.
pub fn oklab_distance2(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("#0c0c18"), Some([12, 12, 24, 255]));
        assert_eq!(parse_hex("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(parse_hex("#40ffA080"), Some([64, 255, 160, 128]));
        assert_eq!(parse_hex("0c0c18"), None);
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("#gggggg"), None);
    }

    #[test]
    fn oklab_anchors() {
        let white = oklab([255, 255, 255]);
        assert!((white[0] - 1.0).abs() < 1e-3 && white[1].abs() < 1e-3 && white[2].abs() < 1e-3);
        let black = oklab([0, 0, 0]);
        assert!(black[0].abs() < 1e-9);
        // Known value: sRGB red is about L 0.628, a 0.225, b 0.126.
        let red = oklab([255, 0, 0]);
        assert!((red[0] - 0.628).abs() < 2e-3, "{red:?}");
        assert!((red[1] - 0.2249).abs() < 2e-3, "{red:?}");
        assert!((red[2] - 0.1258).abs() < 2e-3, "{red:?}");
    }

    #[test]
    fn srgb_lut_is_monotonic_and_ends_at_one() {
        assert_eq!(srgb_to_linear(0), 0.0);
        assert!((srgb_to_linear(255) - 1.0).abs() < 1e-9);
        for c in 0..255u8 {
            assert!(srgb_to_linear(c) < srgb_to_linear(c + 1));
        }
        // 128 -> 0.2158605 (reference value)
        assert!((srgb_to_linear(128) - 0.215_860_5).abs() < 1e-6);
    }

    #[test]
    fn builder_dedupes_and_reserves_transparent() {
        let mut b = PaletteBuilder::new();
        assert_eq!(b.add([1, 2, 3], 0).unwrap(), 0);
        let a = b.add([1, 2, 3], 100).unwrap();
        assert_eq!(a, 1);
        assert_eq!(b.add([1, 2, 3], 100).unwrap(), 1);
        assert_eq!(b.add([1, 2, 3], 101).unwrap(), 2);
        assert_eq!(b.finish()[0], [0, 0, 0, 0]);
    }

    #[test]
    fn builder_rejects_the_257th_entry() {
        let mut b = PaletteBuilder::new();
        for i in 0..255u32 {
            b.add([i as u8, 0, 0], 255).unwrap();
        }
        assert!(matches!(
            b.add([0, 255, 0], 255),
            Err(ArtError::PaletteFull)
        ));
    }
}
