//! Operational helpers for wezterminator (install, doctor, fleet and push).

pub mod doctor;
pub mod export;
pub mod fleet;
pub mod install;
pub mod migrate;
pub mod push;
pub mod uninstall;

pub use doctor::{
    ArtAt, ArtPresence, DoctorInput, DoctorReport, Finding, FindingCode, InstallCurrency,
    MAX_THEME_LAYERS, PresetFonts, ProbeAvailability, ThemeLayers, collect, run as run_doctor,
};
pub use export::{
    Denylist, DenylistHit, ExportError, ExportReport, export_bundle, scan_value,
};
pub use fleet::{
    AttachReport, CommandOutput, CommandRunner, FleetError, PromoteReport, PullReport,
    PushFleetReport, SystemRunner, attach, fleet_preset_path, local_preset_path, promote, pull,
    push_fleet, strip_layer_prefix,
};
pub use install::{
    ConfigEnv, InstallManifest, InstallOptions, InstallReport, ResolvedConfig, install,
    install_currency, resolve_config_path,
};
pub use migrate::{MigrateReport, migrate_home_state};
pub use push::{
    HostAttempt, PushError, PushReport, StepStatus, push_to_target, push_with_target,
    resolve_push_target,
};
pub use uninstall::{UninstallReport, uninstall};
