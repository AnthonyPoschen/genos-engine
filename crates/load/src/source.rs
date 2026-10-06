//! Byte sources. Decoders never open a path themselves.
//!
//! A file and a memory block both become one byte slice. A later network or disc
//! source can implement the same trait and reuse the decoders.

use std::path::{Path, PathBuf};

use crate::error::LoadError;

/// A source of asset bytes.
///
/// The filesystem and an in-memory block are the sources this crate implements.
/// A network payload or a disc payload can implement this trait later. The
/// decoders stay the same.
pub trait ByteSource {
    /// Read the whole source into memory.
    fn read_bytes(&self) -> Result<Vec<u8>, LoadError>;
}

/// Bytes that are already in memory.
pub struct MemorySource<'a> {
    bytes: &'a [u8],
}

impl<'a> MemorySource<'a> {
    /// Borrow `bytes` as a source.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}

impl ByteSource for MemorySource<'_> {
    fn read_bytes(&self) -> Result<Vec<u8>, LoadError> {
        Ok(self.bytes.to_vec())
    }
}

/// A filesystem path.
pub struct FileSource {
    path: PathBuf,
}

impl FileSource {
    /// Read `path` when a decoder asks for bytes.
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl ByteSource for FileSource {
    fn read_bytes(&self) -> Result<Vec<u8>, LoadError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => Ok(bytes),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(LoadError::NotFound {
                path: self.path.clone(),
            }),
            Err(_) => Err(LoadError::Read {
                path: self.path.clone(),
            }),
        }
    }
}
