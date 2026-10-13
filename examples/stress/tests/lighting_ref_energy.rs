//! Seeded cold light matches the live game, and hall stays within 0.001
//! across the contexts that drifted on the owner GPU.
//!
//! The luminance tolerance is noise between a pose seed and seed 0. It is not
//! a fail band. The 0.001 contract is the same repeat as hall twice: blurred
//! relative luminance and the luminance ratio, now also across a catalog
//! slice, both checkpoint orders, and hall alone.

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

fn relative(a: f64, b: f64) -> f64 {
    (a - b).abs() / a.max(b).max(1.0e-6)
}

#[test]
fn seeded_hall_matches_live_luminance() {
    prepare_display();
    let seeded =
        genos_stress::lighting_ref::cold_mean_luminance("stress-still", "hall", None, 96, 54, 4)
            .expect("seeded hall");
    let live =
        genos_stress::lighting_ref::cold_mean_luminance("stress-still", "hall", Some(0), 96, 54, 4)
            .expect("live hall");
    let gap = relative(seeded, live);
    assert!(
        gap < 0.05,
        "seeded mean luminance {seeded} vs live {live} (relative {gap})"
    );
    assert!(seeded.is_finite() && live.is_finite() && seeded > 0.0 && live > 0.0);
}

fn run(root: &PathBuf, name: &str, checkpoints: &str) -> String {
    let out = root.join(name);
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
        "energy-hall",
        "--checkpoints",
        checkpoints,
    ];
    let code = genos_stress::lighting_ref::run(words.into_iter().map(str::to_string)).unwrap();
    assert_eq!(code, 1, "{name}: 1 spp does not auto-pass");
    std::fs::read_to_string(out.join("frames.tsv")).unwrap()
}

fn hall_cap(frames: &str) -> (f64, f64) {
    let mut found = None;
    for line in frames.lines().skip(1) {
        let cols: Vec<_> = line.split('\t').collect();
        if cols.len() < 11 || cols[0] != "stress-still" || cols[1] != "hall" || cols[2] != "4" {
            continue;
        }
        let blur: f64 = cols[8].parse().unwrap();
        let ratio: f64 = cols[9].parse().unwrap();
        found = Some((blur, ratio));
    }
    found.expect("hall cap frame")
}

#[test]
fn hall_stays_within_a_thousandth_across_contexts() {
    prepare_display();
    let root = std::env::temp_dir().join(format!("lref-energy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let contexts = [
        ("solo", "stress-still/hall"),
        (
            "hall-then-box",
            "stress-still/hall,stress-moving/box-step-before",
        ),
        (
            "box-then-hall",
            "stress-moving/box-step-before,stress-still/hall",
        ),
        (
            "catalog",
            "stress-still/room-a,stress-still/hall,bounce-corner/corner",
        ),
    ];
    let mut scores = Vec::new();
    for (name, checkpoints) in contexts {
        let frames = run(&root, name, checkpoints);
        let score = hall_cap(&frames);
        scores.push((name, score));
    }
    let close = 1.0e-3;
    let (base_name, base) = scores[0];
    for (name, score) in &scores[1..] {
        assert!(
            (score.0 - base.0).abs() < close,
            "blur {name} {} vs {base_name} {}",
            score.0,
            base.0
        );
        assert!(
            (score.1 - base.1).abs() < close,
            "ratio {name} {} vs {base_name} {}",
            score.1,
            base.1
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}
