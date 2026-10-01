//! Font discovery and glyph coverage for wezterminator.
//!
//! - [`discovery`] enumerates installed families through fontique.
//! - [`coverage`] checks Polish diacritics and a Nerd Font sample with skrifa.
//! - [`ls_fonts`] parses `wezterm ls-fonts` output so doctor can confirm what
//!   WezTerm itself resolves (its built-in fallbacks change what actually
//!   renders).

pub mod coverage;
pub mod discovery;
pub mod ls_fonts;

pub use coverage::{
    CoverageError, CoverageReport, CoverageSets, NERD_FONT_SAMPLE, POLISH_DIACRITICS,
    check_coverage, check_coverage_sets,
};
pub use discovery::{
    FontCatalog, catalog_from_font_bytes, catalog_from_font_file, family_present,
    family_source_path, load_family_bytes, system_catalog,
};
pub use ls_fonts::{TextGlyphRow, parse_list_system, parse_text_report};
