//! Typed errors for generating, importing and writing art.

use std::path::PathBuf;

pub type Result<T, E = ArtError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum ArtError {
    /// A layer's integer scale does not divide the device size, so logical
    /// pixels would not map to whole device pixels.
    #[error(
        "layer `{layer}`: scale {scale} does not divide the {width}x{height} device size \
         (a logical pixel must map to a whole scale x scale block)"
    )]
    ScaleMismatch {
        layer: String,
        scale: u64,
        width: u64,
        height: u64,
    },

    #[error("layer `{layer}`: scale must be at least 1")]
    ZeroScale { layer: String },

    #[error("device size {width}x{height} is not usable")]
    BadDevice { width: u64, height: u64 },

    /// A recipe parameter is missing, mistyped, out of range or unknown.
    #[error("layer `{layer}`: parameter `{name}`: {message}")]
    Param {
        layer: String,
        name: String,
        message: String,
    },

    #[error("theme: {message}")]
    Theme { message: String },

    #[error("a layer needs more than 255 distinct colours")]
    PaletteFull,

    #[error("image `{}`: {message}", path.display())]
    Image { path: PathBuf, message: String },

    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("PNG encoding: {0}")]
    Png(#[from] png::EncodingError),

    #[error("could not start the {threads}-thread render pool: {message}")]
    Pool { threads: usize, message: String },

    #[error(
        "legibility check failed: {which} over the densest tile at device pixel ({x}, {y}) has \
         contrast {contrast:.2}, below the required {required:.2}"
    )]
    Illegible {
        which: &'static str,
        x: u32,
        y: u32,
        contrast: f64,
        required: f64,
    },
}

impl ArtError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        ArtError::Io {
            path: path.into(),
            source,
        }
    }
}
