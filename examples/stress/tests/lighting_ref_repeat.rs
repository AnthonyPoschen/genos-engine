//! Hall, twice, from a fresh renderer each time. The 32×18 one-frame run only
//! checks that the pose seed is wired. This one is 96×54 for four frames, so
//! the surface cache has to fill. The scored blur and the luminance ratio stay
//! within 0.001. The contract is that repeat, not a wider fail band.

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
    let out = root.join("repeat");
    let references = root.join("references");
    let words = [
        "check",
        "--mode",
        "lavapipe-small",
        "--settle-cap",
        "4",
        "--capture-frames",
        "1",
        "--width",
        "96",
        "--height",
        "54",
        "--out",
        &out.display().to_string(),
        "--reference-dir",
        &references.display().to_string(),
        "--commit",
        "repeat-hall",
        "--checkpoints",
        "stress-still/hall,stress-still/hall",
    ];
    let code = genos_stress::lighting_ref::run(words.into_iter().map(str::to_string)).unwrap();
    assert_eq!(code, 1, "1 spp does not auto-pass");
    let summary = std::fs::read_to_string(out.join("summary.md")).unwrap();
    let frames = std::fs::read_to_string(out.join("frames.tsv")).unwrap();
    assert!(
        summary.contains("fresh renderer"),
        "the repeat is the cold path: {summary}"
    );
    (summary, frames)
}

/// Cap-frame blur and luminance ratio, in checkpoint order.
fn cap_scores(frames: &str) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for line in frames.lines().skip(1) {
        let cols: Vec<_> = line.split('\t').collect();
        if cols.len() < 11 || cols[0] != "stress-still" || cols[1] != "hall" || cols[2] != "4" {
            continue;
        }
        let blur: f64 = cols[8]
            .parse()
            .unwrap_or_else(|_| panic!("blur {}\n{frames}", cols[8]));
        let ratio: f64 = cols[9]
            .parse()
            .unwrap_or_else(|_| panic!("ratio {}\n{frames}", cols[9]));
        out.push((blur, ratio));
    }
    out
}

#[test]
fn hall_twice_matches_within_a_thousandth() {
    prepare_display();
    let root = std::env::temp_dir().join(format!("lref-repeat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (summary, frames) = run(&root);
    let scores = cap_scores(&frames);
    assert_eq!(
        scores.len(),
        2,
        "hall should be scored twice at frame 4\n{frames}"
    );
    let close = 1.0e-3;
    assert!(
        (scores[0].0 - scores[1].0).abs() < close,
        "blur {} then {}\n{summary}",
        scores[0].0,
        scores[1].0
    );
    assert!(
        (scores[0].1 - scores[1].1).abs() < close,
        "ratio {} then {}\n{summary}",
        scores[0].1,
        scores[1].1
    );
    assert!(scores[0].0.is_finite() && scores[0].1.is_finite());
    let _ = std::fs::remove_dir_all(&root);
}
