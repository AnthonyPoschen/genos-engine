//! Content-addressed object cache: `<dir>/objects/<sha256 hex>` holds source
//! bytes (docs/systems/assets/plan.md, "Cache and package"). The bake cache and
//! its FNV-1a `content_key` stay where they are.

use std::path::{Path, PathBuf};

use crate::{hex, sha256, ImportDoc};

/// The object path for a hash.
#[must_use]
pub fn object_path(dir: &Path, hash: &[u8; 32]) -> PathBuf {
    dir.join("objects").join(hex(hash))
}

/// Store bytes under their SHA-256. An object already present is not rewritten.
pub fn store_object(dir: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let path = object_path(dir, &sha256(bytes));
    if path.is_file() {
        return Ok(path);
    }
    write_atomic(&path, bytes)?;
    Ok(path)
}

/// Store the source and every external file of an import. Returns the source object.
pub fn store_import(dir: &Path, doc: &ImportDoc) -> std::io::Result<PathBuf> {
    for (_, bytes) in &doc.externals {
        store_object(dir, bytes)?;
    }
    store_object(dir, &doc.source)
}

/// Write by temporary name and rename, so a reader never sees half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}
