//! Failures from a byte source or a decoder.
//!
//! A failure does not return a usable asset.

use std::path::PathBuf;

/// Why a load did not return an asset.
#[derive(Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The path does not name a file.
    NotFound { path: PathBuf },
    /// The source exists, and the read failed.
    Read { path: PathBuf },
    /// The bytes ended before the asset was complete.
    Truncated,
    /// The bytes are not a file of the requested kind.
    Unrecognized,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { path } => write!(f, "file not found: {}", path.display()),
            Self::Read { path } => write!(f, "could not read {}", path.display()),
            Self::Truncated => f.write_str("truncated asset"),
            Self::Unrecognized => f.write_str("unrecognized asset"),
        }
    }
}

impl std::error::Error for LoadError {}
