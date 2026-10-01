//! Operational helpers for wezterminator (health checks and related tooling).

pub mod doctor;

pub use doctor::{
    ArtAt, ArtPresence, DoctorInput, DoctorReport, Finding, FindingCode, InstallCurrency,
    MAX_THEME_LAYERS, PresetFonts, ProbeAvailability, ThemeLayers, collect, run as run_doctor,
};
