//! Local browser preview server for when the TUI runs outside WezTerm.
//!
//! Binds to `127.0.0.1` with an OS-assigned port, serves a mock-terminal page,
//! and rejects requests whose `Host` header is not loopback. The TUI publishes
//! a [`PreviewDocument`] on selection change; the page polls `/api/version`
//! and reloads `/api/state` when the version bumps.

use std::io::{self, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Response, Server, StatusCode};

/// Embedded mock-terminal page.
const PREVIEW_HTML: &str = include_str!("../assets/preview.html");

/// Shared preview payload the HTML page renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewDocument {
    pub preset_id: String,
    pub preset_name: String,
    #[serde(default = "default_status_style")]
    pub status_style: String,
    pub colors: PreviewColors,
    /// Layer stack drawn under the mock terminal (thumbnails or CSS gradients).
    #[serde(default)]
    pub layers: Vec<PreviewLayer>,
}

fn default_status_style() -> String {
    "sparkline".into()
}

impl Default for PreviewDocument {
    fn default() -> Self {
        Self {
            preset_id: String::new(),
            preset_name: "—".into(),
            status_style: default_status_style(),
            colors: PreviewColors::default(),
            layers: vec![PreviewLayer::Color {
                color: "#0c0c18".into(),
                opacity: 1.0,
            }],
        }
    }
}

impl PreviewDocument {
    /// Minimal document from a selection; uses a deterministic palette derived
    /// from the preset id so adjacent themes look distinct without loading art.
    pub fn for_preset(preset_id: impl Into<String>, preset_name: impl Into<String>) -> Self {
        let preset_id = preset_id.into();
        let preset_name = preset_name.into();
        let colors = PreviewColors::from_seed(&preset_id);
        let layers = vec![
            PreviewLayer::Color {
                color: colors.bg.clone(),
                opacity: 1.0,
            },
            PreviewLayer::Gradient {
                angle: 135.0,
                colors: vec![colors.bg.clone(), colors.accent.clone()],
                opacity: 0.45,
            },
        ];
        Self {
            preset_id,
            preset_name,
            status_style: default_status_style(),
            colors,
            layers,
        }
    }
}

/// Terminal / chrome colours for the mock page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewColors {
    pub bg: String,
    pub fg: String,
    pub fg_dim: String,
    pub accent: String,
    pub tab_bar_bg: String,
    pub tab_active_bg: String,
    pub tab_active_fg: String,
    pub tab_inactive_fg: String,
    pub status_bg: String,
    pub status_fg: String,
}

impl Default for PreviewColors {
    fn default() -> Self {
        Self {
            bg: "#0c0c18".into(),
            fg: "#c0c0c8".into(),
            fg_dim: "#686878".into(),
            accent: "#80ffff".into(),
            tab_bar_bg: "#0c0c18".into(),
            tab_active_bg: "#000080".into(),
            tab_active_fg: "#80ffff".into(),
            tab_inactive_fg: "#686878".into(),
            status_bg: "#000040".into(),
            status_fg: "#80ffff".into(),
        }
    }
}

impl PreviewColors {
    /// Cheap distinct palette so browser mode still shows selection motion.
    pub fn from_seed(seed: &str) -> Self {
        let h = fnv1a64(seed.as_bytes());
        let hue = (h % 360) as f32;
        let bg = hsl_hex(hue, 0.35, 0.08);
        let accent = hsl_hex((hue + 40.0) % 360.0, 0.75, 0.55);
        let active = hsl_hex((hue + 200.0) % 360.0, 0.55, 0.22);
        Self {
            bg: bg.clone(),
            fg: "#d8d8e0".into(),
            fg_dim: "#707088".into(),
            accent: accent.clone(),
            tab_bar_bg: bg.clone(),
            tab_active_bg: active,
            tab_active_fg: accent.clone(),
            tab_inactive_fg: "#707088".into(),
            status_bg: hsl_hex(hue, 0.40, 0.12),
            status_fg: accent,
        }
    }
}

/// One background layer: solid, CSS gradient, or a data-URL thumbnail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewLayer {
    Color {
        color: String,
        #[serde(default = "opacity_one")]
        opacity: f32,
    },
    Gradient {
        #[serde(default = "default_angle")]
        angle: f32,
        colors: Vec<String>,
        #[serde(default = "opacity_one")]
        opacity: f32,
    },
    /// Generated art thumbnail as a `data:image/...;base64,...` URL.
    Image {
        data_url: String,
        #[serde(default = "opacity_one")]
        opacity: f32,
    },
}

fn opacity_one() -> f32 {
    1.0
}

fn default_angle() -> f32 {
    90.0
}

#[derive(Debug, Clone, Serialize)]
struct StateResponse {
    version: u64,
    #[serde(flatten)]
    document: PreviewDocument,
}

#[derive(Debug, Clone, Serialize)]
struct VersionResponse {
    version: u64,
}

struct Shared {
    version: u64,
    document: PreviewDocument,
}

/// Loopback HTTP preview server.
pub struct PreviewServer {
    addr: SocketAddr,
    state: Arc<Mutex<Shared>>,
    running: Arc<AtomicBool>,
    server: Arc<Server>,
    join: Option<JoinHandle<()>>,
}

impl PreviewServer {
    /// Bind `127.0.0.1:0` and serve on a background thread.
    pub fn start() -> Result<Self, ServerError> {
        let server = Server::http("127.0.0.1:0").map_err(|e| ServerError::Bind(e.to_string()))?;
        let addr = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| ServerError::Bind("expected IPv4 listen address".into()))?;
        let server = Arc::new(server);
        let state = Arc::new(Mutex::new(Shared {
            version: 0,
            document: PreviewDocument::default(),
        }));
        let running = Arc::new(AtomicBool::new(true));

        let server_thread = Arc::clone(&server);
        let state_thread = Arc::clone(&state);
        let running_thread = Arc::clone(&running);
        let port = addr.port();
        let join = thread::Builder::new()
            .name("wzt-preview".into())
            .spawn(move || {
                while running_thread.load(Ordering::SeqCst) {
                    match server_thread.recv_timeout(Duration::from_millis(200)) {
                        Ok(Some(request)) => {
                            if let Err(err) = handle_request(request, &state_thread, port) {
                                eprintln!("wzt-preview: {err}");
                            }
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    }
                }
            })
            .map_err(ServerError::Io)?;

        Ok(Self {
            addr,
            state,
            running,
            server,
            join: Some(join),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port())
    }

    pub fn version(&self) -> u64 {
        self.state.lock().expect("preview state").version
    }

    /// Replace the published document and bump the version (page poll notices).
    pub fn set_document(&self, document: PreviewDocument) -> u64 {
        let mut shared = self.state.lock().expect("preview state");
        shared.version = shared.version.saturating_add(1);
        shared.document = document;
        shared.version
    }

    /// Open the preview URL in a browser, or print it when no display is available.
    pub fn open_or_print(&self) -> Result<OpenOutcome, ServerError> {
        open_or_print_url(&self.url())
    }
}

impl Drop for PreviewServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.server.unblock();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Result of trying to launch a browser for the preview URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenOutcome {
    Opened,
    /// No graphical display; URL was printed to stdout.
    PrintedUrl,
}

/// Open `url` with the platform opener, or print it when headless / suppressed.
///
/// Set `WZT_NO_OPEN=1` to never launch a browser (tests and CI must set this, or
/// rely on `cfg!(test)` when exercising this crate's own test binary).
pub fn open_or_print_url(url: &str) -> Result<OpenOutcome, ServerError> {
    if suppress_browser_open() {
        println!("wezterminator preview: {url}");
        return Ok(OpenOutcome::PrintedUrl);
    }
    if !display_available() {
        println!("wezterminator preview: {url}");
        return Ok(OpenOutcome::PrintedUrl);
    }
    match open_browser(url) {
        Ok(()) => Ok(OpenOutcome::Opened),
        Err(err) => {
            // Fall back to printing so the user can still open the URL.
            eprintln!("wezterminator preview: failed to open browser ({err}); URL: {url}");
            println!("wezterminator preview: {url}");
            Ok(OpenOutcome::PrintedUrl)
        }
    }
}

/// True when browser launching must not happen.
///
/// - `WZT_NO_OPEN` set (and not `0`) — tests/CI should export this
/// - `cfg!(test)` when compiling this crate's unit tests
pub fn suppress_browser_open() -> bool {
    if cfg!(test) {
        return true;
    }
    std::env::var_os("WZT_NO_OPEN").is_some_and(|v| v != "0")
}

/// Whether a graphical session looks available.
pub fn display_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
            || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
    }
    #[cfg(target_os = "macos")]
    {
        // SSH sessions without a Mac GUI still have no useful `open` target.
        std::env::var_os("SSH_CONNECTION").is_none()
            || std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
    }
    #[cfg(target_os = "windows")]
    {
        true
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
    }
}

fn open_browser(url: &str) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("open exited with {status}")))
        }
    }
    #[cfg(target_os = "linux")]
    {
        let status = Command::new("xdg-open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("xdg-open exited with {status}")))
        }
    }
    #[cfg(target_os = "windows")]
    {
        let status = Command::new("cmd")
            .args(["/C", "start", "", url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("start exited with {status}")))
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = url;
        Err(io::Error::other("no browser opener on this platform"))
    }
}

fn handle_request(
    request: tiny_http::Request,
    state: &Mutex<Shared>,
    port: u16,
) -> Result<(), ServerError> {
    if !host_allowed(request.headers(), port) {
        let body = "forbidden host";
        let response = Response::from_data(body.as_bytes().to_vec())
            .with_status_code(StatusCode(403))
            .with_header(
                Header::from_bytes(&b"Content-Type"[..], &b"text/plain; charset=utf-8"[..])
                    .expect("header"),
            );
        request.respond(response).map_err(ServerError::Io)?;
        return Ok(());
    }

    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or(&url);

    match (request.method(), path) {
        (Method::Get, "/") | (Method::Get, "/index.html") => {
            let response = Response::from_string(PREVIEW_HTML)
                .with_header(
                    Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
                        .expect("header"),
                )
                .with_header(
                    Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).expect("header"),
                );
            request.respond(response).map_err(ServerError::Io)?;
        }
        (Method::Get, "/api/version") => {
            let version = state.lock().expect("preview state").version;
            respond_json(request, &VersionResponse { version })?;
        }
        (Method::Get, "/api/state") => {
            let shared = state.lock().expect("preview state");
            let body = StateResponse {
                version: shared.version,
                document: shared.document.clone(),
            };
            drop(shared);
            respond_json(request, &body)?;
        }
        _ => {
            let response = Response::from_string("not found").with_status_code(StatusCode(404));
            request.respond(response).map_err(ServerError::Io)?;
        }
    }
    Ok(())
}

fn respond_json(request: tiny_http::Request, value: &impl Serialize) -> Result<(), ServerError> {
    let body = serde_json::to_vec(value)?;
    let response = Response::from_data(body)
        .with_header(
            Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..])
                .expect("header"),
        )
        .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).expect("header"));
    request.respond(response).map_err(ServerError::Io)?;
    Ok(())
}

/// Accept only loopback Host values for this server's port.
pub fn host_allowed(headers: &[Header], port: u16) -> bool {
    let Some(host) = headers
        .iter()
        .find(|h| h.field.equiv("Host"))
        .map(|h| h.value.as_str().trim())
    else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    let port_s = port.to_string();
    let allowed = [
        format!("127.0.0.1:{port_s}"),
        "127.0.0.1".to_string(),
        format!("localhost:{port_s}"),
        "localhost".to_string(),
        format!("[::1]:{port_s}"),
        "[::1]".to_string(),
    ];
    allowed.iter().any(|a| a == &host)
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn hsl_hex(h: f32, s: f32, l: f32) -> String {
    let (r, g, b) = hsl_to_rgb(h, s, l);
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h_prime as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to_u8 = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    (to_u8(r1), to_u8(g1), to_u8(b1))
}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("bind: {0}")]
    Bind(String),
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Tiny HTTP client for tests (and optional tooling).
pub fn http_exchange(
    port: u16,
    path: &str,
    host: &str,
) -> Result<(u16, Vec<u8>), ServerError> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )?;
    let mut buf = Vec::new();
    io::Read::read_to_end(&mut stream, &mut buf)?;
    let header_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| ServerError::Io(io::Error::other("incomplete HTTP response")))?;
    let headers = std::str::from_utf8(&buf[..header_end])
        .map_err(|e| ServerError::Io(io::Error::other(e.to_string())))?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = buf[header_end + 4..].to_vec();
    Ok((status, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_or_print_is_suppressed_under_cfg_test() {
        assert!(suppress_browser_open());
        let server = PreviewServer::start().expect("start");
        let outcome = server.open_or_print().expect("open_or_print");
        assert_eq!(outcome, OpenOutcome::PrintedUrl);
    }
}
