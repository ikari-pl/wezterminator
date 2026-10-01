//! Preview transport for wezterminator.
//!
//! - [`osc`]: WezTerm OSC 1337 SetUserVar encoding, tmux passthrough wrapping,
//!   APC ack framing, and rate-limited writers.
//! - [`server`]: loopback HTTP preview for [`PreviewMode::Browser`] (U14).

pub mod osc;
pub mod server;

pub use osc::{
    ACK_TIMEOUT, AckParser, DEFAULT_EXPIRY_SECS, HEARTBEAT_INTERVAL, OscError, PreviewMode,
    PreviewPayload, ProbePayload, RateLimitedWriter, VAR_CANCEL, VAR_HEARTBEAT, VAR_PREVIEW,
    VAR_PROBE, encode_ack, encode_cancel, encode_heartbeat, encode_preview, encode_probe,
    parse_ack_bytes, tmux_set, wezterm_pane_set, wrap_for_tmux, write_osc,
};
pub use server::{
    OpenOutcome, PreviewColors, PreviewDocument, PreviewLayer, PreviewServer, ServerError,
    display_available, host_allowed, http_exchange, open_or_print_url,
};
