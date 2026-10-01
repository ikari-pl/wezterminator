//! Operational helpers for wezterminator (install, doctor, fleet and push).

pub mod doctor;
pub mod install;
pub mod migrate;
pub mod uninstall;

pub use doctor::{
    ArtAt, ArtPresence, DoctorInput, DoctorReport, Finding, FindingCode, InstallCurrency,
    MAX_THEME_LAYERS, PresetFonts, ProbeAvailability, ThemeLayers, collect, run as run_doctor,
};
pub use install::{
    ConfigEnv, InstallManifest, InstallOptions, InstallReport, ResolvedConfig, install,
    install_currency, resolve_config_path,
};
pub use migrate::{MigrateReport, migrate_home_state};
pub use uninstall::{UninstallReport, uninstall};
