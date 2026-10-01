//! Visual snapshots for every TUI screen (U12 + U13).

use ratatui::widgets::ListState;
use wzt_model::resolve::{Layer, Overruled};
use wzt_preview::PreviewMode;
use wzt_tui::app::{
    AuthorState, AuthorTab, ChromeState, FontsState, KeysState, MachineState, Model, MotionState,
    PartKind, PartRow, PresetRow, Screen, StatusState,
};
use wzt_tui::save::SaveLayer;
use wzt_tui::app::render_to_string;

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
    let mut keys = KeysState::default();
    // Surface one conflict so the keys snapshot shows the warning pane.
    if let Some(palette) = keys.bindings.iter_mut().find(|b| b.id == "palette") {
        palette.key = "p".into();
        palette.mods = "PRIMARY".into();
    }
    keys.refresh_conflicts();

    let author = AuthorState {
        tab: AuthorTab::Palette,
        contrast_warning: String::new(),
        ..AuthorState::default()
    };

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
        draft_parts: None,
        draft_based_on: Some("builtin:ember".into()),
        save_layer: SaveLayer::LocalPreset,
        chrome: ChromeState::default(),
        status_ed: StatusState::default(),
        motion: MotionState::default(),
        fonts: FontsState::default(),
        keys,
        machine: MachineState::default(),
        author,
    }
}

macro_rules! snap {
    ($name:ident, $screen:expr, $w:expr, $h:expr) => {
        #[test]
        fn $name() {
            let mut model = sample_model($screen);
            let shot = render_to_string(&mut model, $w, $h);
            insta::assert_snapshot!(shot);
        }
    };
}

snap!(presets_80x24, Screen::Presets, 80, 24);
snap!(presets_120x40, Screen::Presets, 120, 40);
snap!(parts_80x24, Screen::Parts, 80, 24);
snap!(parts_120x40, Screen::Parts, 120, 40);
snap!(chrome_80x24, Screen::Chrome, 80, 24);
snap!(status_80x24, Screen::Status, 80, 24);
snap!(motion_80x24, Screen::Motion, 80, 24);
snap!(fonts_80x24, Screen::Fonts, 80, 24);
snap!(keys_80x24, Screen::Keys, 80, 24);
snap!(machine_80x24, Screen::Machine, 80, 24);
snap!(author_80x24, Screen::Author, 80, 24);
