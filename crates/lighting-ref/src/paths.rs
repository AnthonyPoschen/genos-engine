//! Where a run writes. Paths are relative to the current directory.
//!
//! A check does not look at `$HOME`. Pass `--out` or `--reference-dir` to put
//! the files somewhere else.

use std::path::{Component, Path, PathBuf};

/// Root of checker output in the current directory.
pub const DEBUG_DIR: &str = "genos-debug";

/// `./genos-debug/reference/<commit>/`. Ground truth, versioned by engine commit.
pub fn reference_dir(commit: &str) -> PathBuf {
    PathBuf::from(DEBUG_DIR)
        .join("reference")
        .join(check_name(commit))
}

/// `./genos-debug/refcheck/<timestamp>/`. One audit report.
pub fn refcheck_dir(timestamp: &str) -> PathBuf {
    PathBuf::from(DEBUG_DIR)
        .join("refcheck")
        .join(check_name(timestamp))
}

/// UTC time as `YYYYMMDD-HHMMSS`. Sortable, and safe as a directory name.
pub fn utc_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_utc(secs)
}

/// Reject names that would escape the output directory.
pub fn check_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
        || trimmed.chars().any(|c| c.is_control())
    {
        return "unknown".to_string();
    }
    trimmed.to_string()
}

pub fn is_under(root: &Path, path: &Path) -> bool {
    let root = normalize(root);
    let path = normalize(path);
    path.starts_with(&root)
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn format_utc(secs: u64) -> String {
    let days = secs / 86_400;
    let tod = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    let hh = tod / 3600;
    let mm = (tod % 3600) / 60;
    let ss = tod % 60;
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

/// Howard Hinnant's civil_from_days, public domain. Days since 1970-01-01.
fn civil_from_days(days: u64) -> (i32, u32, u32) {
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + i64::from(m <= 2);
    (y as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_stay_in_the_current_directory() {
        let report = refcheck_dir("20261013-094500");
        assert!(report.is_relative());
        assert_eq!(
            report,
            PathBuf::from("genos-debug/refcheck/20261013-094500")
        );
        let reference = reference_dir("abc123");
        assert!(reference.is_relative());
        assert_eq!(reference, PathBuf::from("genos-debug/reference/abc123"));
        let text = format!("{report:?} {reference:?}");
        assert!(!text.contains("HOME"));
        assert!(!text.contains("/home"));
        assert!(!text.contains(".cache"));
    }

    #[test]
    fn names_cannot_escape() {
        assert_eq!(check_name("../x"), "unknown");
        assert_eq!(check_name("a/b"), "unknown");
        assert_eq!(check_name("abc"), "abc");
    }

    #[test]
    fn unix_epoch_is_1970() {
        assert_eq!(format_utc(0), "19700101-000000");
        assert_eq!(format_utc(1_760_000_000), "20251009-085320");
    }
}
