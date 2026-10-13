//! The check command without a window: self-test, tiny path traces, a report.

use std::path::PathBuf;

fn args(out: &PathBuf, references: &PathBuf, extra: &[&str]) -> Vec<String> {
    let mut words = vec![
        "check".to_string(),
        "--mode".into(),
        "lavapipe-small".into(),
        "--skip-capture".into(),
        "--out".into(),
        out.display().to_string(),
        "--reference-dir".into(),
        references.display().to_string(),
        "--commit".into(),
        "testcommit".into(),
        "--width".into(),
        "8".into(),
        "--height".into(),
        "8".into(),
        "--spp".into(),
        "1".into(),
        "--base".into(),
        "missing-baseline".into(),
    ];
    words.extend(extra.iter().map(|w| (*w).to_string()));
    words
}

#[test]
fn a_lavapipe_check_without_a_window_passes_the_self_test_and_writes_the_report() {
    let root = std::env::temp_dir().join(format!("lref-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let out = root.join("report");
    let references = root.join("references");
    let code = genos_stress::lighting_ref::run(args(&out, &references, &[]).into_iter()).unwrap();
    assert_eq!(code, 0);
    let summary = std::fs::read_to_string(out.join("summary.md")).unwrap();
    assert!(summary.contains("**pass**"), "{summary}");
    assert!(summary.contains("self-test passed"), "{summary}");
    assert!(summary.contains("capture skipped"), "{summary}");
    assert!(summary.contains("Sponza"), "{summary}");
    assert!(summary.contains("regressions were not judged"), "{summary}");
    assert!(summary.contains("Blind spots"));
    let planted = std::fs::read_to_string(out.join("self-test/summary.md")).unwrap();
    assert!(planted.contains("Self-test passed"));
    assert!(planted.contains("brightness-bias"));
    assert!(planted.contains("held-stale"));
    assert!(planted.contains("black-block"));
    assert!(planted.contains("missing-bounce"));
    assert!(planted.contains("needs human"));
    let linear = references.join("stress-still/hall/reference.f32");
    assert!(linear.exists(), "{}", linear.display());
    assert!(!references.join("metrics.json").exists());

    let again = genos_stress::lighting_ref::run(args(&out, &references, &[]).into_iter()).unwrap();
    assert_eq!(again, 0);
    let second = std::fs::read_to_string(out.join("summary.md")).unwrap();
    assert!(second.contains("Reused reference pictures"), "{second}");
    let _ = std::fs::remove_dir_all(&root);
}
