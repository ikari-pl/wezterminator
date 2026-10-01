//! Install / uninstall / migrate scenarios (U15, AE5).

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use wzt_model::{InstallMode, Paths, Preset, State, read_document};
use wzt_ops::{
    ConfigEnv, InstallOptions, install, migrate_home_state, resolve_config_path, uninstall,
};

struct Harness {
    _tmp: TempDir,
    home: PathBuf,
    paths: Paths,
    checkout: PathBuf,
    config_env: ConfigEnv,
}

impl Harness {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let xdg_config = home.join(".config");
        let xdg_data = home.join(".local").join("share");
        let xdg_state = home.join(".local").join("state");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&xdg_config).unwrap();
        fs::create_dir_all(home.join(".local")).unwrap();

        let paths = Paths::from_roots(xdg_config.clone(), xdg_data, xdg_state);
        let checkout = tmp.path().join("checkout");
        fs::create_dir_all(checkout.join("plugin")).unwrap();
        fs::write(
            checkout.join("plugin").join("init.lua"),
            "return { apply_to_config = function() end }\n",
        )
        .unwrap();
        fs::create_dir_all(checkout.join("presets")).unwrap();
        fs::create_dir_all(checkout.join("themes")).unwrap();

        let config_env = ConfigEnv {
            home: home.clone(),
            xdg_config,
            wezterm_config_file: None,
        };
        Self {
            _tmp: tmp,
            home,
            paths,
            checkout,
            config_env,
        }
    }

    fn opts(&self, mode: InstallMode) -> InstallOptions {
        InstallOptions {
            mode,
            paths: self.paths.clone(),
            config_env: self.config_env.clone(),
            checkout: self.checkout.clone(),
            plugin_url: None,
            skip_migrate: true,
        }
    }

    fn write_xdg_config(&self, body: &str) -> PathBuf {
        let dir = self.config_env.xdg_config.join("wezterm");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wezterm.lua");
        fs::write(&path, body).unwrap();
        path
    }

    fn write_home_dot(&self, body: &str) -> PathBuf {
        let path = self.home.join(".wezterm.lua");
        fs::write(&path, body).unwrap();
        path
    }
}

fn sample_user_config() -> &'static str {
    r#"local wezterm = require 'wezterm'
local config = wezterm.config_builder()
config.font = wezterm.font('My Fancy Font')
config.font_size = 15.0
config.color_scheme = 'Adventure'
return config
"#
}

#[test]
fn resolve_prefers_xdg_then_home_dot() {
    let h = Harness::new();
    // Nothing exists → XDG target, missing.
    let r = resolve_config_path(&h.config_env);
    assert!(r.missing);
    assert!(r.path.ends_with("wezterm/wezterm.lua"));
    assert!(!r.from_env);

    h.write_home_dot("return {}\n");
    let r = resolve_config_path(&h.config_env);
    assert!(!r.missing);
    assert_eq!(r.path, h.home.join(".wezterm.lua"));

    h.write_xdg_config("return {}\n");
    let r = resolve_config_path(&h.config_env);
    assert!(r.path.ends_with("wezterm/wezterm.lua"));
}

#[test]
fn resolve_honours_wezterm_config_file_and_warns() {
    let h = Harness::new();
    let custom = h.home.join("custom.lua");
    fs::write(&custom, "return {}\n").unwrap();
    let env = ConfigEnv {
        wezterm_config_file: Some(custom.clone()),
        ..h.config_env.clone()
    };
    let r = resolve_config_path(&env);
    assert_eq!(r.path, custom);
    assert!(r.from_env);
    assert!(r.warning.as_ref().unwrap().contains("WEZTERM_CONFIG_FILE"));
}

#[test]
fn addon_install_uninstall_restores_bytes() {
    let h = Harness::new();
    let original = sample_user_config();
    let path = h.write_xdg_config(original);
    let before = fs::read(&path).unwrap();

    let report = install(&h.opts(InstallMode::AddOn)).unwrap();
    assert!(!report.idempotent);
    assert_eq!(report.config_path, path);
    let after_install = fs::read_to_string(&path).unwrap();
    assert!(after_install.contains("My Fancy Font"));
    assert!(after_install.contains("-- BEGIN wezterminator"));
    assert!(after_install.contains("mode = 'addon'"));
    assert!(after_install.contains("return config"));

    // Double install is idempotent.
    let again = install(&h.opts(InstallMode::AddOn)).unwrap();
    assert!(again.idempotent);
    assert_eq!(fs::read(&path).unwrap(), after_install.as_bytes());

    uninstall(&h.paths).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(!wzt_ops::InstallManifest::path(&h.paths).is_file());
}

#[test]
fn replace_install_uninstall_restores_bytes() {
    let h = Harness::new();
    let original = sample_user_config();
    let path = h.write_home_dot(original);
    let before = fs::read(&path).unwrap();

    let report = install(&h.opts(InstallMode::Replace)).unwrap();
    assert_eq!(report.config_path, path);
    let shim = fs::read_to_string(&path).unwrap();
    assert!(shim.contains("mode = 'replace'"));
    assert!(shim.contains("dofile"));
    assert!(!shim.contains("My Fancy Font"));

    uninstall(&h.paths).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn replace_import_writes_local_preset_and_restores() {
    let h = Harness::new();
    let path = h.write_xdg_config(sample_user_config());
    let before = fs::read(&path).unwrap();

    let report = install(&h.opts(InstallMode::ReplaceImport)).unwrap();
    let preset_path = report.imported_preset.expect("imported preset");
    let preset: Preset = read_document(&preset_path).unwrap();
    assert_eq!(preset.id, "local:imported");
    assert_eq!(
        preset.parts.font.preferred.as_ref().unwrap(),
        &vec!["My Fancy Font".to_string()]
    );
    assert_eq!(
        preset.parts.font.size.as_ref().unwrap().as_f64().unwrap(),
        15.0
    );

    uninstall(&h.paths).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(!preset_path.is_file());
}

#[test]
fn ae5_addon_keeps_user_font_before_phosphor_block() {
    // AE5: add-on over a config that sets font keeps that assignment in the
    // file (engine snapshots owned keys; Phosphor's font is overruled).
    let h = Harness::new();
    let path = h.write_xdg_config(sample_user_config());
    install(&h.opts(InstallMode::AddOn)).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    let font_pos = text.find("config.font = wezterm.font('My Fancy Font')").unwrap();
    let begin_pos = text.find("-- BEGIN wezterminator").unwrap();
    assert!(
        font_pos < begin_pos,
        "user font must remain before the managed block"
    );
    assert!(text.contains("mode = 'addon'"));
}

#[test]
fn uninstall_after_edited_shim_warns_and_keeps_edited() {
    let h = Harness::new();
    let path = h.write_xdg_config(sample_user_config());
    install(&h.opts(InstallMode::Replace)).unwrap();

    // User edits the shim after install.
    let mut edited = fs::read_to_string(&path).unwrap();
    edited.push_str("-- user tweak\n");
    fs::write(&path, &edited).unwrap();

    let report = uninstall(&h.paths).unwrap();
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("edited after install")),
        "expected edit warning, got {:?}",
        report.warnings
    );
    assert_eq!(report.kept_edited.len(), 1);
    assert!(report.kept_edited[0].is_file());
    assert!(fs::read_to_string(&report.kept_edited[0])
        .unwrap()
        .contains("user tweak"));
    // Restored original still has the user's pre-install content.
    assert!(fs::read_to_string(&path).unwrap().contains("My Fancy Font"));
}

#[test]
fn migrate_metis_like_state_to_local_preset_and_font_offsets() {
    let h = Harness::new();
    // Mirror sources/metis/state/
    fs::write(h.home.join(".wezterm-bgtheme"), "cool\n").unwrap();
    fs::write(h.home.join(".wezterm-font"), "Kode Mono\n").unwrap();
    fs::write(
        h.home.join(".wezterm-font-offsets"),
        "\
3270 Nerd Font Mono=1.0
BlexMono Nerd Font Mono=-0.5
FiraCode Nerd Font Mono=-0.5
Kode Mono=-0.5
ProggyVector=-1.5
Terminess Nerd Font Mono=1.5
",
    )
    .unwrap();

    let report = migrate_home_state(&h.paths, &h.home).unwrap();
    assert_eq!(report.theme_id.as_deref(), Some("builtin:cpc-cool"));
    assert_eq!(report.font_preferred.as_deref(), Some("Kode Mono"));
    assert_eq!(report.font_corrections.len(), 6);
    assert!(
        report
            .font_corrections
            .iter()
            .any(|(n, v)| n == "Terminess Nerd Font Mono" && (*v - 1.5).abs() < f64::EPSILON)
    );

    let preset_path = h
        .paths
        .local_layer_dir()
        .join("presets")
        .join("migrated.json");
    assert!(preset_path.is_file());
    let preset: Preset = read_document(&preset_path).unwrap();
    assert_eq!(preset.id, "local:migrated");
    assert_eq!(preset.parts.art.theme.as_deref(), Some("builtin:cpc-cool"));
    assert_eq!(
        preset.parts.font.preferred.as_ref().unwrap()[0],
        "Kode Mono"
    );
    let corr = preset.parts.font.corrections.as_ref().unwrap();
    assert_eq!(
        corr.get("Kode Mono").and_then(|n| n.as_f64()),
        Some(-0.5)
    );
    assert_eq!(
        corr.get("Terminess Nerd Font Mono")
            .and_then(|n| n.as_f64()),
        Some(1.5)
    );

    // Originals kept.
    assert!(h.home.join(".wezterm-bgtheme").is_file());
    assert!(h.home.join(".wezterm-font").is_file());

    let state: State = read_document(&h.paths.state_file()).unwrap();
    assert_eq!(state.active_preset, "local:migrated");
}

#[test]
fn install_with_wezterm_config_file_targets_that_file() {
    let h = Harness::new();
    let custom = h.home.join("elsewhere.lua");
    fs::write(&custom, sample_user_config()).unwrap();
    let mut opts = h.opts(InstallMode::AddOn);
    opts.config_env.wezterm_config_file = Some(custom.clone());

    let report = install(&opts).unwrap();
    assert_eq!(report.config_path, custom);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("WEZTERM_CONFIG_FILE")),
        "{:?}",
        report.warnings
    );
    assert!(fs::read_to_string(&custom).unwrap().contains("BEGIN wezterminator"));
}

#[test]
fn addon_install_runs_migrate_when_enabled() {
    let h = Harness::new();
    h.write_xdg_config(sample_user_config());
    fs::write(h.home.join(".wezterm-bgtheme"), "warm\n").unwrap();
    fs::write(h.home.join(".wezterm-font"), "ProggyVector\n").unwrap();

    let mut opts = h.opts(InstallMode::AddOn);
    opts.skip_migrate = false;
    let report = install(&opts).unwrap();
    let migrate = report.migrate.expect("migrate ran");
    assert_eq!(migrate.theme_id.as_deref(), Some("builtin:ember"));
    assert!(
        h.paths
            .local_layer_dir()
            .join("presets")
            .join("migrated.json")
            .is_file()
    );
}

#[test]
fn checkout_without_plugin_fails() {
    let h = Harness::new();
    h.write_xdg_config(sample_user_config());
    let mut opts = h.opts(InstallMode::AddOn);
    opts.checkout = h.home.join("not-a-checkout");
    assert!(install(&opts).is_err());
}

#[test]
fn created_config_removed_on_uninstall_when_no_prior_file() {
    let h = Harness::new();
    // No prior config.
    let report = install(&h.opts(InstallMode::Replace)).unwrap();
    assert!(report.config_path.is_file());
    uninstall(&h.paths).unwrap();
    assert!(
        !report.config_path.is_file(),
        "shim created by install should be removed"
    );
}
