//! OSC preview protocol between the Rust TUI and the WezTerm Lua engine.
//!
//! # Wire format
//!
//! Previews travel as WezTerm OSC 1337 SetUserVar escapes:
//!
//! ```text
//! ESC ] 1337 ; SetUserVar = <name> = <base64(utf8 json)> BEL
//! ```
//!
//! Base64 has **no line wrapping**. When `TMUX` is set the bytes are wrapped in
//! a tmux passthrough DCS (`allow-passthrough on` required on tmux 3.3+); that
//! wrapping **doubles every ESC** inside the payload.
//!
//! WezTerm decodes the base64 before delivering `user-var-changed`, so Lua
//! handlers see the JSON string, not the wire encoding.
//!
//! # User-var names
//!
//! | Name | Direction | Payload |
//! |------|-----------|---------|
//! | [`VAR_PROBE`] | TUI → engine | [`ProbePayload`] — handshake |
//! | [`VAR_PREVIEW`] | TUI → engine | [`PreviewPayload`] — candidate preset |
//! | [`VAR_HEARTBEAT`] | TUI → engine | [`PreviewPayload`] (seq + expiry only) |
//! | [`VAR_CANCEL`] | TUI → engine | [`ProbePayload`] — drop preview |
//!
//! # Acknowledgement
//!
//! WezTerm 20240203 cannot read pane user vars from outside, so the probe
//! reply is an APC-framed token written into the TUI pane with
//! `pane:send_text`:
//!
//! ```text
//! ESC _ wzt;ack=<seq> ESC \
//! ```
//!
//! The TUI reads stdin in raw mode, consumes the token via [`AckParser`], and
//! never treats those bytes as keypresses. No ack within [`ACK_TIMEOUT`] means
//! browser-mode fallback ([`crate::server::PreviewServer`]).
//!
//! # Expiry
//!
//! Only OSC-originated (TUI) previews expire. The Lua side stores `expires_at`
//! in the aggregator's preview channel (`overrides.lua`). Heartbeats renew it
//! every [`HEARTBEAT_INTERVAL`]. Lua-picker previews have no expiry.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Probe / handshake user-var name.
pub const VAR_PROBE: &str = "wzt_probe";
/// Full preview (candidate preset) user-var name.
pub const VAR_PREVIEW: &str = "wzt_preview";
/// Heartbeat renewing an active TUI preview.
pub const VAR_HEARTBEAT: &str = "wzt_hb";
/// Cancel / clear preview.
pub const VAR_CANCEL: &str = "wzt_cancel";

/// Default preview lifetime in seconds (renewed by heartbeats).
pub const DEFAULT_EXPIRY_SECS: u64 = 3;
/// How often the TUI renews expiry.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
/// How long to wait for a probe ack before falling back to browser mode.
/// WezTerm must parse the OSC, run Lua, and `send_text` the APC back.
pub const ACK_TIMEOUT: Duration = Duration::from_secs(2);

/// Minimum gap between OSC writes (rate limit).
pub const MIN_WRITE_INTERVAL: Duration = Duration::from_millis(40);

const APC_START: &[u8] = b"\x1b_";
const ST: &[u8] = b"\x1b\\";
const ACK_TAG: &[u8] = b"wzt;ack=";

/// Where the TUI is previewing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewMode {
    /// Live WezTerm window via OSC + ack handshake.
    WezTerm,
    /// Approximate browser preview via [`crate::server::PreviewServer`].
    Browser,
}

/// JSON body for probe and cancel vars.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbePayload {
    pub seq: u64,
}

/// JSON body for preview and heartbeat vars.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewPayload {
    pub seq: u64,
    #[serde(default = "default_expiry", skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<u64>,
    /// Preset id to resolve on the Lua side (`builtin:…`, `local:…`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_id: Option<String>,
    /// Full parts snapshot for theme-part editing; when set, Lua builds from
    /// these instead of (or on top of) resolving `preset_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Value>,
}

fn default_expiry() -> Option<u64> {
    Some(DEFAULT_EXPIRY_SECS)
}

impl PreviewPayload {
    pub fn preview(seq: u64, preset_id: impl Into<String>) -> Self {
        Self {
            seq,
            expires_in: Some(DEFAULT_EXPIRY_SECS),
            preset_id: Some(preset_id.into()),
            parts: None,
        }
    }

    pub fn with_parts(mut self, parts: Value) -> Self {
        self.parts = Some(parts);
        self
    }

    pub fn heartbeat(seq: u64) -> Self {
        Self {
            seq,
            expires_in: Some(DEFAULT_EXPIRY_SECS),
            preset_id: None,
            parts: None,
        }
    }
}

/// Encode a SetUserVar OSC sequence. Base64 has no newlines.
pub fn encode_set_user_var(name: &str, value_utf8: &[u8]) -> Vec<u8> {
    let encoded = B64.encode(value_utf8);
    let mut out = Vec::with_capacity(32 + name.len() + encoded.len());
    out.extend_from_slice(b"\x1b]1337;SetUserVar=");
    out.extend_from_slice(name.as_bytes());
    out.push(b'=');
    out.extend_from_slice(encoded.as_bytes());
    out.push(0x07); // BEL
    out
}

/// Wrap OSC bytes for tmux passthrough: every ESC inside becomes ESC ESC.
///
/// Produces: `ESC P tmux ; <doubled> ESC \`
pub fn wrap_for_tmux(inner: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(inner.len() * 2 + 8);
    out.extend_from_slice(b"\x1bPtmux;");
    for &b in inner {
        if b == 0x1b {
            out.push(0x1b);
        }
        out.push(b);
    }
    out.extend_from_slice(ST);
    out
}

/// Encode JSON as an OSC SetUserVar, optionally wrapped for tmux.
pub fn write_osc(name: &str, json: &impl Serialize, tmux: bool) -> Result<Vec<u8>, OscError> {
    let body = serde_json::to_vec(json)?;
    let osc = encode_set_user_var(name, &body);
    Ok(if tmux { wrap_for_tmux(&osc) } else { osc })
}

pub fn encode_probe(seq: u64, tmux: bool) -> Result<Vec<u8>, OscError> {
    write_osc(VAR_PROBE, &ProbePayload { seq }, tmux)
}

pub fn encode_preview(payload: &PreviewPayload, tmux: bool) -> Result<Vec<u8>, OscError> {
    write_osc(VAR_PREVIEW, payload, tmux)
}

pub fn encode_heartbeat(seq: u64, tmux: bool) -> Result<Vec<u8>, OscError> {
    write_osc(VAR_HEARTBEAT, &PreviewPayload::heartbeat(seq), tmux)
}

pub fn encode_cancel(seq: u64, tmux: bool) -> Result<Vec<u8>, OscError> {
    write_osc(VAR_CANCEL, &ProbePayload { seq }, tmux)
}

/// APC-framed ack token the engine sends with `pane:send_text`.
pub fn encode_ack(seq: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(24);
    out.extend_from_slice(APC_START);
    out.extend_from_slice(ACK_TAG);
    out.extend_from_slice(seq.to_string().as_bytes());
    out.extend_from_slice(ST);
    out
}

/// Scan a byte slice for a complete ack; returns `(seq, end_index_exclusive)`.
pub fn parse_ack_bytes(buf: &[u8]) -> Option<(u64, usize)> {
    find_ack(buf).map(|(_start, seq, end)| (seq, end))
}

/// Incremental stdin parser: strips ack tokens so they are not keypresses.
#[derive(Debug, Default)]
pub struct AckParser {
    buf: Vec<u8>,
}

impl AckParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed raw stdin bytes. Returns completed ack sequence numbers and the
    /// leftover bytes that are safe to treat as keyboard/crossterm input.
    pub fn push(&mut self, input: &[u8]) -> (Vec<u64>, Vec<u8>) {
        self.buf.extend_from_slice(input);
        let mut acks = Vec::new();
        let mut pass = Vec::new();
        while let Some((start, seq, end)) = find_ack(&self.buf) {
            pass.extend_from_slice(&self.buf[..start]);
            acks.push(seq);
            self.buf.drain(..end);
        }
        // Hold a trailing incomplete APC prefix; pass everything before it.
        let hold = incomplete_apc_prefix_len(&self.buf);
        let release = self.buf.len().saturating_sub(hold);
        pass.extend_from_slice(&self.buf[..release]);
        self.buf.drain(..release);
        (acks, pass)
    }
}

/// `(start, seq, end)` of the first complete ack in `buf`.
fn find_ack(buf: &[u8]) -> Option<(usize, u64, usize)> {
    let mut i = 0;
    while i + APC_START.len() + ACK_TAG.len() + ST.len() <= buf.len() {
        if buf[i..].starts_with(APC_START) && buf[i + APC_START.len()..].starts_with(ACK_TAG) {
            let num_start = i + APC_START.len() + ACK_TAG.len();
            let mut num_end = num_start;
            while num_end < buf.len() && buf[num_end].is_ascii_digit() {
                num_end += 1;
            }
            if num_end > num_start
                && num_end + ST.len() <= buf.len()
                && &buf[num_end..num_end + ST.len()] == ST
            {
                let digits = std::str::from_utf8(&buf[num_start..num_end]).ok()?;
                let seq = digits.parse().ok()?;
                return Some((i, seq, num_end + ST.len()));
            }
        }
        i += 1;
    }
    None
}

fn incomplete_apc_prefix_len(buf: &[u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    // Longest suffix that is a proper prefix of an ack frame, or an open APC
    // that has not yet seen ST.
    let max = buf.len().min(64);
    for n in (1..=max).rev() {
        let suffix = &buf[buf.len() - n..];
        if is_ack_prefix(suffix) {
            return n;
        }
    }
    0
}

fn is_ack_prefix(suffix: &[u8]) -> bool {
    // Pure prefix of "ESC _ wzt;ack="
    let head = [APC_START, ACK_TAG].concat();
    if head.starts_with(suffix) {
        return true;
    }
    // Started the APC + tag, digits maybe, no ST yet.
    if suffix.starts_with(APC_START) && suffix[APC_START.len()..].starts_with(ACK_TAG) {
        let rest = &suffix[APC_START.len() + ACK_TAG.len()..];
        return rest.iter().all(u8::is_ascii_digit);
    }
    // ESC _ alone, or ESC _ + partial tag
    if suffix.starts_with(APC_START) {
        let rest = &suffix[APC_START.len()..];
        return ACK_TAG.starts_with(rest);
    }
    false
}

/// Clock injection point for rate limiting and timeouts in tests.
pub trait Clock {
    fn now(&self) -> Instant;
}

/// Wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Rate-limits OSC writes so rapid selection motion cannot flood WezTerm.
pub struct RateLimitedWriter<W, C = SystemClock> {
    inner: W,
    clock: C,
    min_interval: Duration,
    last: Option<Instant>,
    tmux: bool,
}

impl<W: Write> RateLimitedWriter<W, SystemClock> {
    pub fn new(inner: W, tmux: bool) -> Self {
        Self {
            inner,
            clock: SystemClock,
            min_interval: MIN_WRITE_INTERVAL,
            last: None,
            tmux,
        }
    }
}

impl<W: Write, C: Clock> RateLimitedWriter<W, C> {
    pub fn with_clock(inner: W, tmux: bool, clock: C, min_interval: Duration) -> Self {
        Self {
            inner,
            clock,
            min_interval,
            last: None,
            tmux,
        }
    }

    pub fn tmux(&self) -> bool {
        self.tmux
    }

    pub fn set_tmux(&mut self, tmux: bool) {
        self.tmux = tmux;
    }

    /// Write immediately if the interval has elapsed; otherwise skip.
    /// Returns whether bytes were written.
    pub fn try_write(&mut self, bytes: &[u8]) -> io::Result<bool> {
        let now = self.clock.now();
        if let Some(last) = self.last
            && now.duration_since(last) < self.min_interval
        {
            return Ok(false);
        }
        self.inner.write_all(bytes)?;
        self.inner.flush()?;
        self.last = Some(now);
        Ok(true)
    }

    pub fn write_osc(&mut self, name: &str, json: &impl Serialize) -> Result<bool, OscError> {
        let bytes = write_osc(name, json, self.tmux)?;
        Ok(self.try_write(&bytes)?)
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
}

/// Detect whether we should attempt WezTerm OSC mode (`WEZTERM_PANE` set).
pub fn wezterm_pane_set() -> bool {
    std::env::var_os("WEZTERM_PANE").is_some_and(|v| !v.is_empty())
}

/// Detect tmux (`TMUX` set).
pub fn tmux_set() -> bool {
    std::env::var_os("TMUX").is_some_and(|v| !v.is_empty())
}

#[derive(Debug, thiserror::Error)]
pub enum OscError {
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn base64_has_no_newlines() {
        let big = PreviewPayload {
            seq: 1,
            expires_in: Some(3),
            preset_id: Some("builtin:cpc-cool".into()),
            parts: Some(serde_json::json!({"font":{"preferred":["A".repeat(200)]}})),
        };
        let bytes = encode_preview(&big, false).unwrap();
        let s = String::from_utf8(bytes).unwrap();
        assert!(!s.contains('\n'), "OSC base64 must not wrap");
        assert!(s.starts_with("\u{1b}]1337;SetUserVar=wzt_preview="));
        assert!(s.ends_with('\u{7}'));
    }

    #[test]
    fn tmux_wrapping_doubles_inner_escapes() {
        let inner = encode_set_user_var("wzt_probe", b"{\"seq\":1}");
        let esc_count_inner = inner.iter().filter(|&&b| b == 0x1b).count();
        let wrapped = wrap_for_tmux(&inner);
        assert!(wrapped.starts_with(b"\x1bPtmux;"));
        assert!(wrapped.ends_with(b"\x1b\\"));
        // Payload between prefix and final ST: every original ESC doubled,
        // plus the opening ESC of the DCS itself is outside.
        let payload = &wrapped[7..wrapped.len() - 2]; // after "\x1bPtmux;" before ST
        let esc_count_payload = payload.iter().filter(|&&b| b == 0x1b).count();
        assert_eq!(
            esc_count_payload,
            esc_count_inner * 2,
            "each inner ESC must be doubled"
        );
    }

    #[test]
    fn ack_round_trip() {
        let token = encode_ack(42);
        assert_eq!(parse_ack_bytes(&token), Some((42, token.len())));
    }

    #[test]
    fn ack_parser_consumes_token_not_as_keypress() {
        let mut parser = AckParser::new();
        let mut stream = b"hello".to_vec();
        stream.extend_from_slice(&encode_ack(7));
        stream.extend_from_slice(b"world");
        let (acks, pass) = parser.push(&stream);
        assert_eq!(acks, vec![7]);
        assert_eq!(pass, b"helloworld");
    }

    #[test]
    fn ack_parser_handles_split_frames() {
        let token = encode_ack(99);
        let mid = token.len() / 2;
        let mut parser = AckParser::new();
        let (a1, p1) = parser.push(&token[..mid]);
        assert!(a1.is_empty());
        assert!(p1.is_empty(), "incomplete APC held, not passed as keys");
        let (a2, p2) = parser.push(&token[mid..]);
        assert_eq!(a2, vec![99]);
        assert!(p2.is_empty());
    }

    #[derive(Clone)]
    struct FakeClock {
        now: Arc<Mutex<Instant>>,
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            *self.now.lock().unwrap()
        }
    }

    #[test]
    fn rate_limiter_skips_bursts() {
        let start = Instant::now();
        let clock = FakeClock {
            now: Arc::new(Mutex::new(start)),
        };
        let mut w = RateLimitedWriter::with_clock(
            Vec::new(),
            false,
            clock.clone(),
            Duration::from_millis(50),
        );
        assert!(w.try_write(b"a").unwrap());
        assert!(!w.try_write(b"b").unwrap());
        *clock.now.lock().unwrap() = start + Duration::from_millis(60);
        assert!(w.try_write(b"c").unwrap());
        assert_eq!(w.into_inner(), b"ac");
    }

    #[test]
    fn preview_payload_json_shape() {
        let p = PreviewPayload::preview(3, "builtin:ember");
        let v: Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["seq"], 3);
        assert_eq!(v["preset_id"], "builtin:ember");
        assert_eq!(v["expires_in"], 3);
    }
}
