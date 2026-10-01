//! Glyph coverage checks via skrifa.

use skrifa::FontRef;
use skrifa::MetadataProvider;
use thiserror::Error;

/// Polish lowercase and uppercase diacritics (R20).
pub const POLISH_DIACRITICS: &str = "ąćęłńóśżźĄĆĘŁŃÓŚŻŹ";

/// A small Nerd Font / Powerline sample the status bar expects (R20).
///
/// These are representative codepoints from Nerd Fonts Symbols / Powerline
/// sets; a face that lacks them will force WezTerm onto its built-in Symbols
/// fallback mid-glyph.
pub const NERD_FONT_SAMPLE: &str = "\u{e0b0}\u{e0b2}\u{f012}\u{f017}\u{f240}\u{f2db}\u{f1eb}";

/// Which codepoints a font covers from a requested set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageReport {
    pub present: Vec<char>,
    pub missing: Vec<char>,
}

impl CoverageReport {
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Why coverage could not be computed.
#[derive(Debug, Error)]
pub enum CoverageError {
    #[error("font data is not a valid OpenType font: {0}")]
    InvalidFont(String),
}

/// Check whether `font_bytes` maps each of `codepoints` to a glyph.
pub fn check_coverage(font_bytes: &[u8], codepoints: &[char]) -> Result<CoverageReport, CoverageError> {
    let font = FontRef::new(font_bytes).map_err(|e| CoverageError::InvalidFont(e.to_string()))?;
    let cmap = font.charmap();
    let mut present = Vec::new();
    let mut missing = Vec::new();
    for &ch in codepoints {
        if cmap.map(ch).is_some() {
            present.push(ch);
        } else {
            missing.push(ch);
        }
    }
    Ok(CoverageReport { present, missing })
}

/// Polish diacritics and the Nerd Font sample, reported separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageSets {
    pub polish: CoverageReport,
    pub nerd: CoverageReport,
}

/// Run the standard R20 coverage sets against one font.
pub fn check_coverage_sets(font_bytes: &[u8]) -> Result<CoverageSets, CoverageError> {
    let polish_chars: Vec<char> = POLISH_DIACRITICS.chars().collect();
    let nerd_chars: Vec<char> = NERD_FONT_SAMPLE.chars().collect();
    Ok(CoverageSets {
        polish: check_coverage(font_bytes, &polish_chars)?,
        nerd: check_coverage(font_bytes, &nerd_chars)?,
    })
}
