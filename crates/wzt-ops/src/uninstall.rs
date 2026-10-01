//! Undo an install from its manifest.

use std::fs;
use std::path::{Path, PathBuf};

use wzt_model::{Paths, State, write_atomic, write_document};

use crate::install::{
    InstallError, InstallManifest, Result, blake3_hex, hash_file, read_manifest,
};

/// Report from a successful uninstall.
#[derive(Debug, Clone)]
pub struct UninstallReport {
    pub config_path: PathBuf,
    pub warnings: Vec<String>,
    /// Paths where an edited post-install file was kept beside the restore.
    pub kept_edited: Vec<PathBuf>,
}

/// Restore files from the install manifest and remove the manifest.
pub fn uninstall(paths: &Paths) -> Result<UninstallReport> {
    let manifest_path = InstallManifest::path(paths);
    if !manifest_path.is_file() {
        return Err(InstallError::msg(format!(
            "no install manifest at {}",
            manifest_path.display()
        )));
    }
    let manifest = read_manifest(&manifest_path)?;
    let mut warnings = Vec::new();
    let mut kept_edited = Vec::new();

    for entry in &manifest.files {
        restore_file(entry, &manifest.backup_dir, &mut warnings, &mut kept_edited)?;
    }

    for created in &manifest.created {
        if created.is_file() {
            fs::remove_file(created).map_err(|e| InstallError::Io {
                path: created.clone(),
                source: e,
            })?;
        }
    }

    // Clear install_mode from state but keep the rest.
    clear_install_mode(paths)?;

    fs::remove_file(&manifest_path).map_err(|e| InstallError::Io {
        path: manifest_path.clone(),
        source: e,
    })?;

    Ok(UninstallReport {
        config_path: manifest.config_path,
        warnings,
        kept_edited,
    })
}

fn restore_file(
    entry: &crate::install::ManifestFile,
    backup_dir: &Path,
    warnings: &mut Vec<String>,
    kept_edited: &mut Vec<PathBuf>,
) -> Result<()> {
    let current_hash = if entry.path.is_file() {
        Some(hash_file(&entry.path)?)
    } else {
        None
    };

    let edited = current_hash
        .as_ref()
        .is_some_and(|h| h != &entry.post_hash);

    if edited {
        let kept = sibling_edited_path(&entry.path);
        fs::copy(&entry.path, &kept).map_err(|e| InstallError::Io {
            path: entry.path.clone(),
            source: e,
        })?;
        warnings.push(format!(
            "{} was edited after install; keeping the edited copy at {}",
            entry.path.display(),
            kept.display()
        ));
        kept_edited.push(kept);
    }

    match (&entry.backup_name, &entry.pre_hash) {
        (Some(name), Some(_)) => {
            let backup = backup_dir.join(name);
            let bytes = fs::read(&backup).map_err(|e| InstallError::Io {
                path: backup.clone(),
                source: e,
            })?;
            // Verify backup integrity.
            let h = blake3_hex(&bytes);
            if let Some(pre) = &entry.pre_hash
                && h != *pre
            {
                warnings.push(format!(
                    "backup hash mismatch for {}; restoring anyway",
                    backup.display()
                ));
            }
            if let Some(parent) = entry.path.parent() {
                fs::create_dir_all(parent).map_err(|e| InstallError::Io {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
            write_atomic(&entry.path, &bytes)?;
        }
        _ => {
            // File was created by install: remove it.
            if entry.path.is_file() {
                fs::remove_file(&entry.path).map_err(|e| InstallError::Io {
                    path: entry.path.clone(),
                    source: e,
                })?;
            }
        }
    }
    Ok(())
}

fn sibling_edited_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("config");
    path.with_file_name(format!("{file_name}.edited-after-install"))
}

fn clear_install_mode(paths: &Paths) -> Result<()> {
    let state_path = paths.state_file();
    if !state_path.is_file() {
        return Ok(());
    }
    let mut state: State = wzt_model::read_document(&state_path)?;
    state.install_mode = None;
    write_document(&state_path, &state)?;
    Ok(())
}
