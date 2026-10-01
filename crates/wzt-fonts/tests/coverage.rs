//! Coverage and ls-fonts parsing tests for wzt-fonts.

use std::path::PathBuf;

use wzt_fonts::{
    NERD_FONT_SAMPLE, POLISH_DIACRITICS, check_coverage, check_coverage_sets, parse_list_system,
};

fn fixture_font() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/coverage-test.ttf")
}

#[test]
fn bundled_font_reports_exact_polish_hits_and_misses() {
    let bytes = std::fs::read(fixture_font()).expect("fixture font");
    let polish: Vec<char> = POLISH_DIACRITICS.chars().collect();
    let report = check_coverage(&bytes, &polish).expect("coverage");

    // The fixture only includes ó (U+00F3) from the Polish set.
    assert_eq!(report.present, vec!['ó']);
    let expected_missing: Vec<char> = POLISH_DIACRITICS.chars().filter(|&c| c != 'ó').collect();
    assert_eq!(report.missing, expected_missing);
    assert!(!report.is_complete());
}

#[test]
fn bundled_font_misses_the_nerd_font_sample() {
    let bytes = std::fs::read(fixture_font()).expect("fixture font");
    let sets = check_coverage_sets(&bytes).expect("coverage sets");
    assert!(sets.nerd.present.is_empty());
    assert_eq!(
        sets.nerd.missing,
        NERD_FONT_SAMPLE.chars().collect::<Vec<_>>()
    );
}

#[test]
fn bundled_font_covers_basic_ascii_it_embeds() {
    let bytes = std::fs::read(fixture_font()).expect("fixture font");
    let report = check_coverage(&bytes, &['A', 'z', '0', 'ó', 'ą']).expect("coverage");
    assert_eq!(report.present, vec!['A', 'z', '0', 'ó']);
    assert_eq!(report.missing, vec!['ą']);
}

#[test]
fn parse_list_system_yields_expected_families() {
    let fixture = r#"
Primary font:
wezterm.font_with_fallback({
  {family="Operator Mono SSm Lig", weight="DemiLight"},
  "JetBrains Mono",
})

112 fonts found in your font_dirs + built-in fonts:
wezterm.font("Cascadia Code", {weight="Regular", stretch="Normal", italic=false}) -- /home/wez/.fonts/CascadiaCode.ttf index=0 variation=4, FontDirs
wezterm.font("Fira Code", {weight="Regular", stretch="Normal", italic=false}) -- /home/wez/.fonts/FiraCode-Regular.otf, FontDirs
wezterm.font("Fira Code", {weight="Bold", stretch="Normal", italic=false}) -- /home/wez/.fonts/FiraCode-Bold.otf, FontDirs
wezterm.font("Terminess Nerd Font Mono", {weight="Regular", stretch="Normal", italic=false}) -- /usr/share/fonts/TerminessNerdFontMono-Regular.ttf, FontConfig

690 system fonts found using FontConfig:
wezterm.font("Abyssinica SIL", {weight="Regular", stretch="Normal", italic=false}) -- /usr/share/fonts/sil-abyssinica-fonts/AbyssinicaSIL-R.ttf, FontConfig
wezterm.font("JetBrains Mono", {weight="Regular", stretch="Normal", italic=false}) -- /usr/share/fonts/jetbrains-mono-fonts/JetBrainsMono-Regular.ttf, FontConfig
"#;

    let families = parse_list_system(fixture);
    assert_eq!(
        families,
        vec![
            "Abyssinica SIL".to_string(),
            "Cascadia Code".to_string(),
            "Fira Code".to_string(),
            "JetBrains Mono".to_string(),
            "Terminess Nerd Font Mono".to_string(),
        ]
    );
}
