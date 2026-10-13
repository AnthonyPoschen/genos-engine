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
fn a_skipped_capture_fails_and_still_writes_the_self_test() {
    let root = std::env::temp_dir().join(format!("lref-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let out = root.join("report");
    let references = root.join("references");
    let code = genos_stress::lighting_ref::run(args(&out, &references, &[]).into_iter()).unwrap();
    assert_eq!(code, 1);
    let summary = std::fs::read_to_string(out.join("summary.md")).unwrap();
    assert!(summary.contains("**fail**"), "{summary}");
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
    let stored = references.join("metrics.json");
    let metrics = std::fs::read_to_string(&stored).unwrap();
    assert!(metrics.contains("\"checkpoint\": \"hall\""), "{metrics}");
    assert!(metrics.contains("direct-only"), "{metrics}");
    assert!(metrics.contains("luma_delta"), "{metrics}");
    assert!(metrics.contains("blur_rel"), "{metrics}");

    let again = genos_stress::lighting_ref::run(args(&out, &references, &[]).into_iter()).unwrap();
    assert_eq!(again, 1);
    let second = std::fs::read_to_string(out.join("summary.md")).unwrap();
    assert!(second.contains("Reused reference pictures"), "{second}");

    let judged = genos_stress::lighting_ref::run(
        args(
            &out,
            &references,
            &["--baseline", stored.to_str().expect("utf-8 path")],
        )
        .into_iter(),
    )
    .unwrap();
    assert_eq!(judged, 1);
    let third = std::fs::read_to_string(out.join("summary.md")).unwrap();
    assert!(third.contains("Regressions use"), "{third}");
    assert!(third.contains("**fail**"), "{third}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_mismatched_reference_is_kept_and_the_check_stops() {
    let root = std::env::temp_dir().join(format!("lref-mismatch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let references = root.join("references");
    let picture = genos_lighting_ref::Picture::solid(8, 8, [0.2, 0.1, 0.0]);
    genos_lighting_ref::write_reference(
        &references,
        genos_lighting_ref::ReferenceRecord {
            scene: "stress-still".into(),
            checkpoint: "hall".into(),
            commit: "other-build".into(),
            method: "path".into(),
            width: 8,
            height: 8,
            spp: 256,
            sample_cap: 256,
            noise: 0.08,
            seconds: 1.0,
            max_bounces: 8,
            png_bytes: 0,
            linear_bytes: 0,
            note: "planted".into(),
        },
        &picture,
    )
    .unwrap();
    let linear = references.join("stress-still/hall/reference.f32");
    let before = std::fs::read(&linear).unwrap();
    let err = genos_stress::lighting_ref::run(
        args(
            &root.join("report"),
            &references,
            &["--checkpoints", "stress-still/hall"],
        )
        .into_iter(),
    )
    .expect_err("a wrong sample cap must not be retraced");
    assert!(err.contains("--regen"), "{err}");
    assert!(err.contains("sample cap"), "{err}");
    assert!(err.contains("build"), "{err}");
    assert_eq!(std::fs::read(&linear).unwrap(), before);
    let _ = std::fs::remove_dir_all(&root);
}
