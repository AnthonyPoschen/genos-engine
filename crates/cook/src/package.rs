//! The package manifest (docs/systems/assets/plan.md step 13): the content hashes
//! a platform ships. One line per source asset, sorted, so two cooks of the
//! same bytes write the same manifest and one changed input changes one line.
//! There is no archive format; the game ships the manifest and the objects.

use std::path::Path;

use genos_load::{hex, object_path, sha256};

use crate::{cooked_path, encode, textures_path, CookError, Cooked};

/// Line `genos-package 1`, then `<asset> cook=<record sha256> inputs=<sha256>,...`,
/// ` gtex=<sha256>` when the asset has a cooked texture record, and
/// ` skin=<sha256>` when it has a `.gskin`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub lines: Vec<String>,
}

const HEADER: &str = "genos-package 1";

/// Build the manifest for a set of cooked records with no textures.
#[must_use]
pub fn manifest(cooked: &[Cooked]) -> Manifest {
    let entries: Vec<(&Cooked, Option<[u8; 32]>)> = cooked.iter().map(|c| (c, None)).collect();
    package(&entries)
}

/// One cooked asset on a manifest line.
pub struct PackageFile<'a> {
    pub cooked: &'a Cooked,
    pub gtex: Option<[u8; 32]>,
    pub skin: Option<[u8; 32]>,
}

/// Build the manifest for cooked records, each with the SHA-256 of its `.gtex`
/// file when it has one. A line built here has no `skin=` token.
#[must_use]
pub fn package(entries: &[(&Cooked, Option<[u8; 32]>)]) -> Manifest {
    let files: Vec<PackageFile<'_>> = entries
        .iter()
        .map(|(cooked, gtex)| PackageFile {
            cooked,
            gtex: *gtex,
            skin: None,
        })
        .collect();
    package_files(&files)
}

/// [`package`] plus an optional `.gskin` hash after `gtex=` when that token exists.
#[must_use]
pub fn package_files(entries: &[PackageFile<'_>]) -> Manifest {
    let mut lines: Vec<String> = entries
        .iter()
        .map(|entry| {
            let inputs: Vec<String> = entry.cooked.inputs.iter().map(|i| hex(&i.hash)).collect();
            let mut line = format!(
                "{} cook={} inputs={}",
                hex(&entry.cooked.asset_id),
                hex(&sha256(&encode(entry.cooked))),
                inputs.join(",")
            );
            if let Some(g) = entry.gtex {
                line.push_str(" gtex=");
                line.push_str(&hex(&g));
            }
            if let Some(s) = entry.skin {
                line.push_str(" skin=");
                line.push_str(&hex(&s));
            }
            line
        })
        .collect();
    lines.sort();
    lines.dedup();
    Manifest { lines }
}

impl Manifest {
    #[must_use]
    pub fn text(&self) -> String {
        let mut s = format!("{HEADER}\n");
        for line in &self.lines {
            s.push_str(line);
            s.push('\n');
        }
        s
    }

    ///
    /// # Errors
    ///
    /// A missing header is `CookError::Corrupt`.
    pub fn parse(text: &str) -> Result<Manifest, CookError> {
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return Err(CookError::Corrupt("manifest header"));
        }
        Ok(Manifest {
            lines: lines
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
        })
    }

    /// The load-time check: every record, texture record and source object
    /// under `out` is present and hashes to the value the manifest names.
    ///
    /// # Errors
    ///
    /// A missing file is `CookError::Io`; a hash that differs is `CookError::Corrupt`.
    pub fn verify(&self, out: &Path) -> Result<usize, CookError> {
        for line in &self.lines {
            let mut parts = line.split(' ');
            let asset = parts.next().ok_or(CookError::Corrupt("manifest line"))?;
            let cook = parts
                .next()
                .and_then(|p| p.strip_prefix("cook="))
                .ok_or(CookError::Corrupt("manifest line"))?;
            let inputs = parts
                .next()
                .and_then(|p| p.strip_prefix("inputs="))
                .ok_or(CookError::Corrupt("manifest line"))?;
            let id = unhex(asset)?;
            if hex(&sha256(&std::fs::read(cooked_path(out, &id))?)) != cook {
                return Err(CookError::Corrupt("record hash differs from the manifest"));
            }
            let (gtex, skin) = side_files(parts)?;
            if let Some(gtex) = gtex {
                if hex(&sha256(&std::fs::read(textures_path(out, &id))?)) != gtex {
                    return Err(CookError::Corrupt("texture hash differs from the manifest"));
                }
            }
            if let Some(skin) = skin {
                let path = genos_load::skin_path(out, &id);
                let bytes = std::fs::read(&path)?;
                if hex(&sha256(&bytes)) != skin {
                    return Err(CookError::Corrupt("skin hash differs from the manifest"));
                }
                genos_load::decode_skin(&bytes).map_err(|_| CookError::Corrupt("gskin trailer"))?;
            }
            for input in inputs.split(',') {
                let bytes = std::fs::read(object_path(out, &unhex(input)?))?;
                if hex(&sha256(&bytes)) != input {
                    return Err(CookError::Corrupt("object hash differs from its name"));
                }
            }
        }
        Ok(self.lines.len())
    }
}

fn side_files<'a>(
    parts: impl Iterator<Item = &'a str>,
) -> Result<(Option<&'a str>, Option<&'a str>), CookError> {
    let mut gtex = None;
    let mut skin = None;
    for token in parts {
        if let Some(hash) = token.strip_prefix("gtex=") {
            if gtex.replace(hash).is_some() {
                return Err(CookError::Corrupt("manifest line"));
            }
        } else if let Some(hash) = token.strip_prefix("skin=") {
            if skin.replace(hash).is_some() {
                return Err(CookError::Corrupt("manifest line"));
            }
        } else {
            return Err(CookError::Corrupt("manifest line"));
        }
    }
    Ok((gtex, skin))
}

fn unhex(s: &str) -> Result<[u8; 32], CookError> {
    let mut out = [0u8; 32];
    if s.len() != 64 {
        return Err(CookError::Corrupt("hash length"));
    }
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2).unwrap_or("zz"), 16)
            .map_err(|_| CookError::Corrupt("hash digit"))?;
    }
    Ok(out)
}
