//! Hall and box-step-before are one pose. A cold run gives them one seed and
//! one frame budget, so the scored blur and luminance ratio match. The test
//! does not copy one checkpoint's result onto the other.

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

fn run(root: &PathBuf) -> (String, String) {
    let out = root.join("same-pose");
    let references = root.join("references");
    let words = [
        "check",
        "--mode",
        "lavapipe-small",
        "--settle-cap",
        "1",
        "--capture-frames",
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
        "same-pose",
        "--checkpoints",
        "stress-still/hall,stress-moving/box-step-before",
    ];
    let code = genos_stress::lighting_ref::run(words.into_iter().map(str::to_string)).unwrap();
    assert_eq!(code, 1, "1 spp does not auto-pass");
    let summary = std::fs::read_to_string(out.join("summary.md")).unwrap();
    let frames = std::fs::read_to_string(out.join("frames.tsv")).unwrap();
    assert!(
        summary.contains("fresh renderer"),
        "the same-pose test is the cold path: {summary}"
    );
    (summary, frames)
}

/// Scored blur and luminance ratio from the cap frame (frame 1 at cap 1).
fn scored(frames: &str, scene: &str, checkpoint: &str) -> (f64, f64) {
    let mut blur = None;
    let mut ratio = None;
    for line in frames.lines().skip(1) {
        let cols: Vec<_> = line.split('\t').collect();
        if cols.len() < 11 || cols[0] != scene || cols[1] != checkpoint || cols[2] != "1" {
            continue;
        }
        blur = cols[8].parse().ok();
        ratio = cols[9].parse().ok();
    }
    (
        blur.unwrap_or_else(|| panic!("{scene}/{checkpoint} blur missing\n{frames}")),
        ratio.unwrap_or_else(|| panic!("{scene}/{checkpoint} ratio missing\n{frames}")),
    )
}

#[test]
fn box_step_before_matches_hall() {
    prepare_display();
    let root = std::env::temp_dir().join(format!("lref-same-pose-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (summary, frames) = run(&root);
    let hall = scored(&frames, "stress-still", "hall");
    let before = scored(&frames, "stress-moving", "box-step-before");
    let close = 1.0e-4;
    assert!(
        (hall.0 - before.0).abs() < close,
        "blur hall {} box-step-before {}\n{summary}",
        hall.0,
        before.0
    );
    assert!(
        (hall.1 - before.1).abs() < close,
        "ratio hall {} box-step-before {}\n{summary}",
        hall.1,
        before.1
    );
    assert!(hall.0.is_finite() && hall.1.is_finite());
    let _ = std::fs::remove_dir_all(&root);
}
