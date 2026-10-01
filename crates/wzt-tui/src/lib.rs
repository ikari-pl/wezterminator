//! Ratatui application for wezterminator.
//!
//! Elm-style: [`app::Model`] is updated by [`app::Msg`], and [`app::view`] is a
//! pure function of the model. Live preview goes through [`wzt_preview`] OSC
//! when a WezTerm ack is received; otherwise the app stays in browser mode
//! (U14 will host the approximate preview server).

pub mod app;
pub mod screens;

pub use app::{
    Effect, Model, Msg, PartKind, PartRow, PresetRow, PreviewTransport, RecordingTransport,
    RunOptions, Screen, StdoutTransport, update, view, render_to_string, run,
};
pub use wzt_preview::PreviewMode;
