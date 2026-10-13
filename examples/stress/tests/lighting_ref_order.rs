//! Two checkpoints, both orders, on lavapipe. A fresh renderer makes the
//! verdicts match. One frame is enough: room-a after hall used to start warm.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn prepare_display() {
    if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", "/tmp/genos-runtime") }
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        unsafe { std::env::set_var("WAYLAND_DISPLAY", "genos-way") }
    }
    if std::env::var_os("VK_ICD_FILENAMES").is_none() {
        unsafe { std::env::set_var("VK_ICD_FILENAMES", "/usr/share/vulkan/icd.d/lvp_icd.json") }
    }
}

fn run(root: &PathBuf, out_name: &str, order: &str) -> (String, String) {
    let out = root.join(out_name);
    let references = root.join("references");
    let words = [
        "check",
        "--mode",
        "lavapipe-small",
        "--settle-frames",
        "1",
        "--width",
        "32",
        "--height",
        "18",
        "--out",
        &out.display().to_string(),
        "--reference-dir",
        &references.display().to_string(),
        "--commit",
        "order-test",
        "--checkpoints",
        order,
    ];
    let code = genos_stress::lighting_ref::run(words.into_iter().map(str::to_string)).unwrap();
    assert_eq!(code, 1, "{order} should score and not auto-pass at 1 spp");
    let summary = std::fs::read_to_string(out.join("summary.md")).unwrap();
    let frames = std::fs::read_to_string(out.join("frames.tsv")).unwrap();
    assert!(
        summary.contains("fresh renderer"),
        "the order test is the cold path: {summary}"
    );
    (summary, frames)
}

/// Verdict, and the first settle frame's blurred error and luminance ratio.
fn scores(summary: &str, frames: &str) -> BTreeMap<String, (String, f64, f64)> {
    let mut out = BTreeMap::new();
    for line in summary.lines() {
        let Some(rest) = line.strip_prefix("## ") else {
            continue;
        };
        let Some((name, verdict)) = rest.split_once(" — ") else {
            continue;
        };
        out.insert(
            name.to_string(),
            (verdict.trim().to_string(), f64::NAN, f64::NAN),
        );
    }
    for line in frames.lines().skip(1) {
        let cols: Vec<_> = line.split('\t').collect();
        if cols.len() < 11 || cols[2] != "1" {
            continue;
        }
        let key = format!("{} / {}", cols[0], cols[1]);
        let Some(row) = out.get_mut(&key) else {
            continue;
        };
        row.1 = cols[8].parse().unwrap_or(f64::NAN);
        row.2 = cols[9].parse().unwrap_or(f64::NAN);
    }
    out
}

#[test]
fn two_checkpoints_have_the_same_verdict_in_either_order() {
    prepare_display();
    let root = std::env::temp_dir().join(format!("lref-order-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let forward = run(
        &root,
        "hall-then-room",
        "stress-still/hall,stress-still/room-a",
    );
    let reverse = run(
        &root,
        "room-then-hall",
        "stress-still/room-a,stress-still/hall",
    );
    let left = scores(&forward.0, &forward.1);
    let right = scores(&reverse.0, &reverse.1);
    assert_eq!(left.len(), 2, "forward {left:?}\n{}", forward.0);
    assert_eq!(right.len(), 2, "reverse {right:?}\n{}", reverse.0);
    // A warm chain moved room-a from about 0.46 to 1.05. Lavapipe's last
    // digit can move by a few hundred-thousandths on a fresh device.
    let close = 0.002;
    for (name, forward_row) in &left {
        let reverse_row = right.get(name).unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(forward_row.0, reverse_row.0, "{name} verdict");
        assert!(
            (forward_row.1 - reverse_row.1).abs() < close,
            "{name} blur {} vs {}",
            forward_row.1,
            reverse_row.1
        );
        assert!(
            (forward_row.2 - reverse_row.2).abs() < close,
            "{name} ratio {} vs {}",
            forward_row.2,
            reverse_row.2
        );
        assert!(forward_row.1.is_finite(), "{name} blur was missing");
        assert!(forward_row.2.is_finite(), "{name} ratio was missing");
    }
    let _ = std::fs::remove_dir_all(&root);
}
