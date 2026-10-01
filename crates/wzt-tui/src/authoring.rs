//! Theme authoring helpers: palette contrast, duplicate/template, art regen.
//!
//! Art regeneration runs on a background thread. Progress updates flow through
//! [`ArtProgress`]; cancelling abandons the temp output directory so the
//! previous art on disk stays intact.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use wzt_art::legibility::{self, DEFAULT_MIN_CONTRAST};
use wzt_art::palette::parse_hex;
use wzt_art::{Device, Options, generate_theme};
use wzt_model::{Theme, Variant, write_document};

use crate::app::AppError;

/// Contrast readout for one palette pair (e.g. fg on bg).
#[derive(Debug, Clone, PartialEq)]
pub struct ContrastReadout {
    pub pair: String,
    pub ratio: f64,
    pub required: f64,
    pub ok: bool,
}

/// Check common UI pairs; returns warnings for any pair below threshold.
/// Low contrast **does not block save** — callers surface warnings only.
pub fn palette_contrast_warnings(theme: &Theme) -> Vec<ContrastReadout> {
    let ui = &theme.palette.ui;
    let required = theme
        .legibility
        .min_contrast
        .as_ref()
        .and_then(serde_json::Number::as_f64)
        .unwrap_or(DEFAULT_MIN_CONTRAST);
    let pairs = [
        ("fg/bg", ui.fg.as_str(), ui.bg.as_str()),
        ("fg_dim/bg", ui.fg_dim.as_str(), ui.bg.as_str()),
        ("accent/bg", ui.accent.as_str(), ui.bg.as_str()),
        ("tab_active_fg/tab_active_bg", ui.tab_active_fg.as_str(), ui.tab_active_bg.as_str()),
    ];
    let mut out = Vec::new();
    for (name, a, b) in pairs {
        let Some(ca) = parse_hex(a) else { continue };
        let Some(cb) = parse_hex(b) else { continue };
        let ratio = legibility::contrast([ca[0], ca[1], ca[2]], [cb[0], cb[1], cb[2]]);
        out.push(ContrastReadout {
            pair: name.into(),
            ratio,
            required,
            ok: ratio + f64::EPSILON >= required,
        });
    }
    out
}

pub fn has_contrast_warning(readouts: &[ContrastReadout]) -> bool {
    readouts.iter().any(|r| !r.ok)
}

/// Duplicate a theme under a new id/name (local layer theme dir).
pub fn duplicate_theme(source: &Theme, new_id: &str, new_name: &str) -> Theme {
    let mut t = source.clone();
    t.id = new_id.to_string();
    t.name = new_name.to_string();
    t
}

/// Minimal dark template for "create from template".
pub fn template_theme(id: &str, name: &str) -> Theme {
    let raw = serde_json::json!({
        "schema_version": 1,
        "id": id,
        "name": name,
        "variant": "dark",
        "palette": {
            "ui": {
                "bg": "#0c0c18",
                "surface": "#161628",
                "fg": "#e0e0f0",
                "fg_dim": "#686878",
                "accent": "#40c0ff",
                "ok": "#40c080",
                "warn": "#e0a040",
                "bad": "#e04040",
                "info": "#6080e0",
                "tab_bar_bg": "#0c0c18",
                "tab_active_bg": "#2030a0",
                "tab_active_fg": "#ffffff",
                "tab_inactive_bg": "#0c0c18",
                "tab_inactive_fg": "#808090"
            },
            "scheme": {
                "foreground": "#e0e0f0",
                "background": "#0c0c18",
                "cursor_bg": "#40c0ff",
                "cursor_fg": "#0c0c18",
                "selection_bg": "#2030a0",
                "selection_fg": "#ffffff",
                "ansi": ["#0c0c18","#e04040","#40c080","#e0a040","#4060e0","#c040c0","#40c0ff","#e0e0f0"],
                "brights": ["#686878","#ff6060","#60e0a0","#ffc060","#6080ff","#e060e0","#60e0ff","#ffffff"]
            }
        },
        "art": {
            "seed": 1,
            "base_color": "#0c0c18",
            "layers": []
        },
        "fallback_layers": [
            { "kind": "color", "color": "#0c0c18" }
        ],
        "legibility": {
            "text": "#e0e0f0",
            "dim_text": "#686878",
            "min_contrast": 3.0
        }
    });
    serde_json::from_value(raw).expect("template theme deserializes")
}

/// Write `theme.json` under `themes/<slug>/`.
pub fn write_theme(layer_dir: &Path, theme: &Theme) -> Result<PathBuf, AppError> {
    let slug = theme
        .id
        .rsplit(':')
        .next()
        .unwrap_or(theme.id.as_str());
    let dir = layer_dir.join("themes").join(slug);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("theme.json");
    write_document(&path, theme)?;
    Ok(path)
}

/// Progress events from a background art regen job.
#[derive(Debug, Clone)]
pub enum ArtProgress {
    Started { out_dir: PathBuf },
    Layer { index: usize, total: usize, id: String },
    Done { out_dir: PathBuf },
    Cancelled,
    Failed(String),
}

/// Handle for an in-flight art regeneration.
pub struct ArtJob {
    pub cancel: Arc<AtomicBool>,
    pub rx: Receiver<ArtProgress>,
    /// Join handle kept so the thread is not detached silently.
    _join: JoinHandle<()>,
    /// Directory that held art before this job (left untouched until success).
    pub previous_dir: PathBuf,
    /// Staging directory; deleted on cancel/failure.
    pub staging_dir: PathBuf,
}

impl ArtJob {
    pub fn poll(&self) -> Vec<ArtProgress> {
        let mut out = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            out.push(msg);
        }
        out
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// Start regenerating `theme` into a staging directory beside `art_dir`.
///
/// On success the caller replaces `art_dir` with the staging contents. On
/// cancel/failure the staging dir is removed and `art_dir` is unchanged.
pub fn start_art_regen(
    theme: Theme,
    art_dir: PathBuf,
    device: Device,
) -> Result<ArtJob, AppError> {
    let staging = art_dir.with_extension("staging");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_t = Arc::clone(&cancel);
    let (tx, rx): (Sender<ArtProgress>, Receiver<ArtProgress>) = mpsc::channel();
    let staging_t = staging.clone();
    let previous = art_dir.clone();
    let join = thread::spawn(move || {
        let _ = tx.send(ArtProgress::Started {
            out_dir: staging_t.clone(),
        });
        if cancel_t.load(Ordering::SeqCst) {
            let _ = std::fs::remove_dir_all(&staging_t);
            let _ = tx.send(ArtProgress::Cancelled);
            return;
        }
        let total = theme.art.layers.len().max(1);
        for (index, layer) in theme.art.layers.iter().enumerate() {
            if cancel_t.load(Ordering::SeqCst) {
                let _ = std::fs::remove_dir_all(&staging_t);
                let _ = tx.send(ArtProgress::Cancelled);
                return;
            }
            let _ = tx.send(ArtProgress::Layer {
                index,
                total,
                id: layer.id.clone(),
            });
        }
        if cancel_t.load(Ordering::SeqCst) {
            let _ = std::fs::remove_dir_all(&staging_t);
            let _ = tx.send(ArtProgress::Cancelled);
            return;
        }
        let opts = Options {
            skip_legibility: true,
            ..Options::default()
        };
        match generate_theme(&theme, device, &staging_t, &opts) {
            Ok(_) => {
                if cancel_t.load(Ordering::SeqCst) {
                    let _ = std::fs::remove_dir_all(&staging_t);
                    let _ = tx.send(ArtProgress::Cancelled);
                } else {
                    let _ = tx.send(ArtProgress::Done {
                        out_dir: staging_t,
                    });
                }
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staging_t);
                let _ = tx.send(ArtProgress::Failed(e.to_string()));
            }
        }
    });
    Ok(ArtJob {
        cancel,
        rx,
        _join: join,
        previous_dir: previous,
        staging_dir: staging,
    })
}

/// Promote staging into `art_dir` after a successful [`ArtProgress::Done`].
pub fn commit_art_staging(art_dir: &Path, staging: &Path) -> Result<(), AppError> {
    if art_dir.exists() {
        let bak = art_dir.with_extension("bak");
        if bak.exists() {
            std::fs::remove_dir_all(&bak)?;
        }
        std::fs::rename(art_dir, &bak)?;
        match std::fs::rename(staging, art_dir) {
            Ok(()) => {
                let _ = std::fs::remove_dir_all(&bak);
                Ok(())
            }
            Err(e) => {
                let _ = std::fs::rename(&bak, art_dir);
                Err(e.into())
            }
        }
    } else {
        std::fs::rename(staging, art_dir)?;
        Ok(())
    }
}

/// Whether `previous` still has its content after a cancel (for tests).
pub fn previous_art_intact(previous: &Path, marker_file: &str) -> bool {
    previous.join(marker_file).is_file()
}

pub fn variant_label(v: Variant) -> &'static str {
    match v {
        Variant::Dark => "dark",
        Variant::Light => "light",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    #[test]
    fn low_contrast_warns_but_does_not_block_conceptually() {
        let mut theme = template_theme("local:dim", "Dim");
        // Near-identical fg/bg → well below 3.0.
        theme.palette.ui.fg = "#101018".into();
        theme.palette.ui.bg = "#0c0c18".into();
        theme.palette.ui.fg_dim = "#0e0e18".into();
        let warnings = palette_contrast_warnings(&theme);
        assert!(has_contrast_warning(&warnings));
        // Saving is still allowed: write_theme succeeds.
        let dir = tempfile::tempdir().unwrap();
        write_theme(dir.path(), &theme).unwrap();
        assert!(dir.path().join("themes/dim/theme.json").is_file());
    }

    #[test]
    fn art_regen_cancel_leaves_previous_intact() {
        let dir = tempfile::tempdir().unwrap();
        let art_dir = dir.path().join("art");
        fs::create_dir_all(&art_dir).unwrap();
        let marker = "old-layer.png";
        fs::write(art_dir.join(marker), b"previous-bytes").unwrap();

        let theme = template_theme("local:t", "T");
        let device = Device::new(64, 64).unwrap();
        let job = start_art_regen(theme, art_dir.clone(), device).unwrap();
        job.request_cancel();

        // Drain until cancelled or failed/done.
        let mut saw_cancel = false;
        for _ in 0..200 {
            for msg in job.poll() {
                if matches!(msg, ArtProgress::Cancelled) {
                    saw_cancel = true;
                }
            }
            if saw_cancel {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        // Even if the job finished before cancel landed, previous must remain
        // unless Done was committed — we never call commit_art_staging here.
        assert!(previous_art_intact(&art_dir, marker));
        let contents = fs::read(art_dir.join(marker)).unwrap();
        assert_eq!(contents, b"previous-bytes");
    }
}
