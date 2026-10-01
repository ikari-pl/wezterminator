//! Doctor health-check scenarios (AE6, AE8 and related).

use wzt_fonts::{CoverageReport, CoverageSets, FontCatalog};
use wzt_model::{Primitive, Screens, ThemeArt};
use wzt_ops::doctor::{
    ArtAt, ArtPresence, DoctorInput, FindingCode, InstallCurrency, PresetFonts, ThemeLayers, run,
};
use wzt_ops::MAX_THEME_LAYERS;

fn empty_coverage() -> CoverageSets {
    CoverageSets {
        polish: CoverageReport {
            present: Vec::new(),
            missing: Vec::new(),
        },
        nerd: CoverageReport {
            present: Vec::new(),
            missing: Vec::new(),
        },
    }
}

fn cpc_cool_preset() -> PresetFonts {
    PresetFonts {
        id: "builtin:cpc-cool".into(),
        name: "CPC Cool".into(),
        preferred: vec![
            "Terminess Nerd Font Mono".into(),
            "ProggyVector".into(),
        ],
        fallback: vec![
            "FiraCode Nerd Font Mono".into(),
            "JetBrains Mono".into(),
            "Menlo".into(),
        ],
        art_theme_slug: Some("cpc-cool".into()),
        wants_blur: false,
    }
}

fn base_input() -> DoctorInput {
    DoctorInput {
        install: InstallCurrency::Current,
        chrome_blur_supported: true,
        binary_version: "0.1.0".into(),
        binary_schema: 1,
        engine: Some(wzt_model::EngineInfo {
            plugin_dir: "/plugin".into(),
            version: "0.1.0".into(),
            schema_version: 1,
            comments: Default::default(),
        }),
        ..DoctorInput::default()
    }
}

#[test]
fn ae6_missing_terminess_names_it_for_cpc_cool_and_lists_fallback() {
    // Machine without Terminess; Menlo is present so fallback still renders.
    let catalog = FontCatalog::from_families(["Menlo", "JetBrains Mono"]);
    let mut input = base_input();
    input.presets = vec![cpc_cool_preset()];
    input.catalog = catalog;
    input.screens = Some(Screens {
        schema_version: 1,
        screens: vec![wzt_model::Screen {
            name: Some("main".into()),
            width: 1920,
            height: 1080,
            comments: Default::default(),
        }],
        comments: Default::default(),
    });
    // Art present so AE6 is isolated.
    input.art = vec![ArtAt {
        theme_slug: "cpc-cool".into(),
        width: 1920,
        height: 1080,
        presence: ArtPresence::Shipped {
            dir: "/shipped/cpc-cool/1920x1080".into(),
        },
    }];
    input.themes = vec![ThemeLayers {
        id: "builtin:cpc-cool".into(),
        slug: "cpc-cool".into(),
        layer_count: 4,
        art: ThemeArt {
            seed: 1,
            base_color: "#000000".into(),
            layers: Vec::new(),
            comments: Default::default(),
        },
    }];

    let report = run(&input);
    let font_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == FindingCode::MissingPreferredFont)
        .collect();
    assert_eq!(font_findings.len(), 1, "report: {}", report.render());
    let msg = &font_findings[0].message;
    assert!(
        msg.contains("Terminess Nerd Font Mono"),
        "should name Terminess: {msg}"
    );
    assert!(msg.contains("CPC Cool"), "should name the preset: {msg}");
    assert!(
        msg.contains("fallback in use:") && msg.contains("FiraCode Nerd Font Mono"),
        "should list the fallback list: {msg}"
    );
}

#[test]
fn ae8_recorded_resolution_without_art_is_reported() {
    let mut input = base_input();
    input.presets = vec![cpc_cool_preset()];
    input.catalog = FontCatalog::from_families([
        "Terminess Nerd Font Mono",
        "ProggyVector",
        "Menlo",
    ]);
    input.screens = Some(Screens {
        schema_version: 1,
        screens: vec![wzt_model::Screen {
            name: Some("ultrawide".into()),
            width: 5120,
            height: 2160,
            comments: Default::default(),
        }],
        comments: Default::default(),
    });
    input.themes = vec![ThemeLayers {
        id: "builtin:cpc-cool".into(),
        slug: "cpc-cool".into(),
        layer_count: 4,
        art: ThemeArt {
            seed: 1,
            base_color: "#000000".into(),
            layers: Vec::new(),
            comments: Default::default(),
        },
    }];
    // Art only exists at a different resolution.
    input.art = vec![ArtAt {
        theme_slug: "cpc-cool".into(),
        width: 1920,
        height: 1080,
        presence: ArtPresence::Shipped {
            dir: "/shipped/cpc-cool/1920x1080".into(),
        },
    }];

    let report = run(&input);
    let art: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == FindingCode::ArtMissingAtResolution)
        .collect();
    assert_eq!(art.len(), 1, "report: {}", report.render());
    assert!(
        art[0].message.contains("5120x2160"),
        "should name the recorded resolution: {}",
        art[0].message
    );
    assert!(
        art[0].message.contains("cpc-cool"),
        "should name the theme: {}",
        art[0].message
    );
}

#[test]
fn failed_coverage_is_reported_per_family() {
    let mut input = base_input();
    input.install = InstallCurrency::Current;
    input.coverage = vec![(
        "WztCoverageTest".into(),
        CoverageSets {
            polish: CoverageReport {
                present: vec!['ó'],
                missing: vec!['ą', 'ć'],
            },
            nerd: empty_coverage().nerd,
        },
    )];
    // Avoid unrelated findings.
    input.screens = Some(Screens {
        schema_version: 1,
        screens: vec![wzt_model::Screen {
            name: None,
            width: 800,
            height: 600,
            comments: Default::default(),
        }],
        comments: Default::default(),
    });

    let report = run(&input);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == FindingCode::FailedCoverage
                && f.message.contains("WztCoverageTest")
                && f.message.contains("ą")),
        "report: {}",
        report.render()
    );
}

#[test]
fn theme_over_layer_budget_is_flagged() {
    let mut layers = Vec::new();
    for i in 0..=MAX_THEME_LAYERS {
        layers.push(wzt_model::ArtLayer {
            id: format!("l{i}"),
            primitive: Primitive::Vignette,
            params: None,
            scale: 1,
            opacity: None,
            parallax: None,
            repeat: None,
            animated: None,
            enabled: None,
            comments: Default::default(),
        });
    }
    let mut input = base_input();
    input.install = InstallCurrency::Current;
    input.themes = vec![ThemeLayers {
        id: "builtin:heavy".into(),
        slug: "heavy".into(),
        layer_count: layers.len(),
        art: ThemeArt {
            seed: 0,
            base_color: "#000".into(),
            layers,
            comments: Default::default(),
        },
    }];
    input.screens = Some(Screens {
        schema_version: 1,
        screens: vec![wzt_model::Screen {
            name: None,
            width: 100,
            height: 100,
            comments: Default::default(),
        }],
        comments: Default::default(),
    });

    let report = run(&input);
    assert!(
        report
            .codes()
            .contains(&FindingCode::ThemeLayerBudgetExceeded),
        "report: {}",
        report.render()
    );
}

#[test]
fn stale_user_art_is_flagged() {
    let mut input = base_input();
    input.presets = vec![cpc_cool_preset()];
    input.catalog = FontCatalog::from_families(["Terminess Nerd Font Mono", "ProggyVector"]);
    input.screens = Some(Screens {
        schema_version: 1,
        screens: vec![wzt_model::Screen {
            name: None,
            width: 1920,
            height: 1080,
            comments: Default::default(),
        }],
        comments: Default::default(),
    });
    input.themes = vec![ThemeLayers {
        id: "builtin:cpc-cool".into(),
        slug: "cpc-cool".into(),
        layer_count: 1,
        art: ThemeArt {
            seed: 1,
            base_color: "#000".into(),
            layers: Vec::new(),
            comments: Default::default(),
        },
    }];
    input.art = vec![ArtAt {
        theme_slug: "cpc-cool".into(),
        width: 1920,
        height: 1080,
        presence: ArtPresence::User {
            dir: "/data/art/cpc-cool/1920x1080".into(),
            recipe_ok: false,
        },
    }];

    let report = run(&input);
    assert!(
        report.codes().contains(&FindingCode::StaleUserArt),
        "report: {}",
        report.render()
    );
}

#[test]
fn schema_and_engine_version_mismatches_are_reported() {
    let mut input = base_input();
    input.install = InstallCurrency::Current;
    input.binary_version = "0.2.0".into();
    input.binary_schema = 1;
    input.engine = Some(wzt_model::EngineInfo {
        plugin_dir: "/plugin".into(),
        version: "0.1.0".into(),
        schema_version: 2,
        comments: Default::default(),
    });
    input.screens = Some(Screens {
        schema_version: 1,
        screens: vec![wzt_model::Screen {
            name: None,
            width: 1,
            height: 1,
            comments: Default::default(),
        }],
        comments: Default::default(),
    });

    let report = run(&input);
    assert!(report.codes().contains(&FindingCode::SchemaVersionMismatch));
    assert!(report.codes().contains(&FindingCode::EngineVersionMismatch));
}

#[test]
fn unknown_screens_when_file_absent() {
    let mut input = base_input();
    input.install = InstallCurrency::Current;
    input.screens = None;
    let report = run(&input);
    assert!(report.codes().contains(&FindingCode::UnknownScreenResolution));
}
