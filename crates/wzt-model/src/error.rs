//! Typed errors for reading, validating and writing wezterminator documents.

use std::path::PathBuf;

use serde_json::Value;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{}: not valid JSON: {source}", path.display())]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    /// The JSON parsed but does not match the document shape: a misspelled
    /// key, a wrong type, or a field that belongs to another document.
    #[error("{}: does not match the document schema: {source}", path.display())]
    Shape {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    /// The document is newer than this build understands. The caller should
    /// ignore it and say why; nothing else in the document was read.
    #[error(
        "{}: schema_version {found} is newer than the supported version {supported}",
        path.display()
    )]
    UnsupportedSchemaVersion {
        path: PathBuf,
        found: u64,
        supported: u64,
    },

    /// `schema_version` is missing, not an integer, or below 1.
    #[error("{}: invalid schema_version {found}", path.display())]
    InvalidSchemaVersion { path: PathBuf, found: Value },

    #[error("could not determine the home directory")]
    NoHomeDir,
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
