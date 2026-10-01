//! Visual snapshots for the presets and parts screens.

use ratatui::widgets::ListState;
use wzt_model::resolve::{Layer, Overruled};
use wzt_preview::PreviewMode;
use wzt_tui::app::{Model, PartKind, PartRow, PresetRow, Screen, render_to_string};

fn sample_model(screen: Screen) -> Model {
    let mut preset_state = ListState::default();
    preset_state.select(Some(1));
    let mut part_state = ListState::default();
    part_state.select(Some(0));
    let presets = vec![
        PresetRow {
            id: "builtin:cpc-cool".into(),
            name: "CPC Cool".into(),
            layer: Layer::Builtin,
            shadowed: false,
        },
        PresetRow {
            id: "builtin:ember".into(),
            name: "Ember".into(),
            layer: Layer::Builtin,
            shadowed: false,
        },
        PresetRow {
            id: "builtin:soft-nebula".into(),
            name: "Soft Nebula".into(),
            layer: Layer::Builtin,
            shadowed: false,
        },
        PresetRow {
            id: "local:mine".into(),
            name: "Mine".into(),
            layer: Layer::Local,
            shadowed: false,
        },
    ];
    let parts = PartKind::ALL
        .iter()
        .map(|kind| PartRow {
            kind: *kind,
            summary: format!("{} · Ember", kind.label()),
            source: "builtin".into(),
            overruled: *kind == PartKind::Font,
        })
        .collect();
    Model {
        screen,
        presets,
        preset_state,
        parts,
        part_state,
        active_id: Some("builtin:cpc-cool".into()),
        mode: PreviewMode::WezTerm,
        seq: 2,
        status: "wezterm preview".into(),
        addon_mode: true,
        overruled: vec![Overruled {
            path: "font".into(),
            config_key: "font".into(),
        }],
        quit: false,
        checkout: None,
    }
}

#[test]
fn presets_80x24() {
    let mut model = sample_model(Screen::Presets);
    let shot = render_to_string(&mut model, 80, 24);
    insta::assert_snapshot!(shot);
}

#[test]
fn presets_120x40() {
    let mut model = sample_model(Screen::Presets);
    let shot = render_to_string(&mut model, 120, 40);
    insta::assert_snapshot!(shot);
}

#[test]
fn parts_80x24() {
    let mut model = sample_model(Screen::Parts);
    let shot = render_to_string(&mut model, 80, 24);
    insta::assert_snapshot!(shot);
}

#[test]
fn parts_120x40() {
    let mut model = sample_model(Screen::Parts);
    let shot = render_to_string(&mut model, 120, 40);
    insta::assert_snapshot!(shot);
}
