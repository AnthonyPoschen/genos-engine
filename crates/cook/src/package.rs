//! The package manifest (docs/systems/assets/plan.md step 13): the content hashes
//! a platform ships. One line per source asset, sorted, so two cooks of the
//! same bytes write the same manifest and one changed input changes one line.
//! There is no archive format; the game ships the manifest and the objects.

use std::path::Path;

use genos_load::{hex, object_path, sha256};

use crate::{cooked_path, encode, textures_path, CookError, Cooked};

/// Line `genos-package 1`, then `<asset> cook=<record sha256> inputs=<sha256>,...`,
/// and ` gtex=<sha256>` when the asset has a cooked texture record.
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

/// Build the manifest for cooked records, each with the SHA-256 of its `.gtex`
/// file when it has one.
#[must_use]
pub fn package(entries: &[(&Cooked, Option<[u8; 32]>)]) -> Manifest {
    let mut lines: Vec<String> = entries
        .iter()
        .map(|(c, gtex)| {
            let inputs: Vec<String> = c.inputs.iter().map(|i| hex(&i.hash)).collect();
            let mut line = format!(
                "{} cook={} inputs={}",
                hex(&c.asset_id),
                hex(&sha256(&encode(c))),
                inputs.join(",")
            );
            if let Some(g) = gtex {
                line.push_str(" gtex=");
                line.push_str(&hex(g));
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
            if let Some(gtex) = parts.next() {
                let gtex = gtex
                    .strip_prefix("gtex=")
                    .ok_or(CookError::Corrupt("manifest line"))?;
                if hex(&sha256(&std::fs::read(textures_path(out, &id))?)) != gtex {
                    return Err(CookError::Corrupt("texture hash differs from the manifest"));
                }
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
