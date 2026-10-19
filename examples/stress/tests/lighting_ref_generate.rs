#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::map_unwrap_or, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails"))]

//! The path-traced reference writer. One tiny checkpoint, twice, must match.

use std::path::PathBuf;

use genos_stress::lighting_ref::{generate, GenerateOptions, TraceProfile};

fn tiny(dir: PathBuf) -> GenerateOptions {
    GenerateOptions {
        reference_dir: dir,
        commit: "testcommit".into(),
        profile: TraceProfile {
            name: "test",
            width: 8,
            height: 8,
            spp: 1,
            max_spp: 1,
            max_bounces: 1,
            seconds: 15.0,
            noise_target: 1.0,
        },
        only_scene: None,
        only_checkpoint: None,
    }
}

#[test]
fn a_path_reference_is_deterministic_and_reports_its_size() {
    let root = std::env::temp_dir().join(format!(
        "genos-lref-gen-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let a = root.join("a");
    let b = root.join("b");
    let first = generate(&tiny(a.clone())).expect("first trace");
    let second = generate(&tiny(b.clone())).expect("second trace");
    assert!(first.len() > 1, "expected every available checkpoint");
    assert!(first
        .iter()
        .any(|r| r.scene == "stress-still" && r.checkpoint == "hall"));
    assert!(first.iter().any(|r| r.scene == "hell-100"));
    assert!(first.iter().any(|r| r.scene == "bounce-corner"));
    assert!(first
        .iter()
        .any(|r| r.scene == "direct-only" && r.checkpoint == "hall" && r.max_bounces == 0));
    let metrics = std::fs::read_to_string(a.join("metrics.json")).unwrap();
    assert!(metrics.contains("\"commit\": \"testcommit\""), "{metrics}");
    assert!(metrics.contains("direct-only"), "{metrics}");
    assert!(metrics.contains("luma_delta"), "{metrics}");
    assert!(first
        .iter()
        .all(|r| r.method == "path" && r.commit == "testcommit"));
    assert!(first.iter().all(|r| r.png_bytes > 0 && r.linear_bytes > 0));
    for record in &first {
        let rel = format!("{}/{}/reference.f32", record.scene, record.checkpoint);
        let linear_a = std::fs::read(a.join(&rel)).unwrap();
        let linear_b = std::fs::read(b.join(&rel)).unwrap();
        assert_eq!(linear_a, linear_b, "{rel}");
    }
    assert_eq!(second.len(), first.len());
    let manifest = std::fs::read_to_string(a.join("manifest.json")).unwrap();
    assert!(manifest.contains("testcommit"));
    assert!(manifest.contains("sponza"));
    assert!(manifest.contains(&first[0].png_bytes.to_string()));
    let sizes = std::fs::read_to_string(a.join("sizes.txt")).unwrap();
    assert!(sizes.contains("stress-still hall"));
    assert!(genos_stress::lighting_ref::scenes_cover_the_catalog());
    let _ = std::fs::remove_dir_all(&root);
}
