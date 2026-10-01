//! Reading and atomic writing of document files.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tempfile::NamedTempFile;

use crate::error::{Error, Result};
use crate::version::{SUPPORTED_SCHEMA_VERSION, VersionProblem, check_schema_version};

/// Read a file as untyped JSON. Comment keys are kept.
pub fn read_value(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).map_err(|e| Error::io(path, e))?;
    serde_json::from_slice(&bytes).map_err(|source| Error::Json {
        path: path.to_path_buf(),
        source,
    })
}

/// Gate `value` on its `schema_version`, then deserialize it as `T`.
///
/// The gate runs first and reads nothing else, so a document from a newer
/// engine yields [`Error::UnsupportedSchemaVersion`] and never a shape error.
/// `origin` only labels errors.
pub fn parse_document<T: DeserializeOwned>(value: Value, origin: &Path) -> Result<T> {
    match check_schema_version(&value, SUPPORTED_SCHEMA_VERSION) {
        Ok(_) => {}
        Err(VersionProblem::Invalid(found)) => {
            return Err(Error::InvalidSchemaVersion {
                path: origin.to_path_buf(),
                found,
            });
        }
        Err(VersionProblem::Unsupported(found)) => {
            return Err(Error::UnsupportedSchemaVersion {
                path: origin.to_path_buf(),
                found,
                supported: SUPPORTED_SCHEMA_VERSION,
            });
        }
    }
    serde_json::from_value(value).map_err(|source| Error::Shape {
        path: origin.to_path_buf(),
        source,
    })
}

/// Read, gate and deserialize one document file.
pub fn read_document<T: DeserializeOwned>(path: &Path) -> Result<T> {
    parse_document(read_value(path)?, path)
}

/// Serialize as 2-space pretty JSON with a trailing newline.
pub fn to_pretty_json<T: Serialize>(document: &T) -> Vec<u8> {
    // Serializing a value tree or a derived struct to memory cannot fail.
    let mut bytes = serde_json::to_vec_pretty(document).expect("document serializes to JSON");
    bytes.push(b'\n');
    bytes
}

/// Write `document` to `path` atomically.
pub fn write_document<T: Serialize>(path: &Path, document: &T) -> Result<()> {
    write_atomic(path, &to_pretty_json(document))
}

/// Write `bytes` to `path` atomically: a temporary file in the same directory
/// is flushed to disk and then renamed over the target, so a reader (WezTerm's
/// watcher included) sees either the old file or the new one, never a partial
/// write. Missing parent directories are created.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;

    let mut tmp = NamedTempFile::new_in(parent).map_err(|e| Error::io(parent, e))?;
    tmp.write_all(bytes).map_err(|e| Error::io(tmp.path(), e))?;
    tmp.as_file()
        .sync_all()
        .map_err(|e| Error::io(tmp.path(), e))?;

    // Temp files are created 0600. Keep the target's mode when replacing it,
    // and use an ordinary 0644 for new files.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o7777)
            .unwrap_or(0o644);
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))
            .map_err(|e| Error::io(tmp.path(), e))?;
    }

    tmp.persist(path).map_err(|e| Error::io(path, e.error))?;
    Ok(())
}
