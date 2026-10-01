//! Ratatui application for wezterminator.
//!
//! Elm-style: [`app::Model`] is updated by [`app::Msg`], and [`app::view`] is a
//! pure function of the model. Live preview goes through [`wzt_preview`] OSC
//! when a WezTerm ack is received; otherwise the app stays in browser mode
//! (U14 will host the approximate preview server).
//!
//! U13 adds chrome / status / motion / fonts / keys / machine / author screens
//! on top of the U12 presets / parts shell.

pub mod app;
pub mod authoring;
pub mod keys_data;
pub mod save;
pub mod screens;

pub use app::{
    AuthorState, AuthorTab, ChromeState, Effect, FontRow, FontsState, KeysState, MachineState,
    Model, MotionState, Msg, PartKind, PartRow, PresetRow, PreviewTransport, RecordingTransport,
    RunOptions, Screen, StatusState, StdoutTransport, update, view, render_to_string, run,
};
pub use save::SaveLayer;
pub use wzt_preview::PreviewMode;
