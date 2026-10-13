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
        sequence: Vec::new(),
        regen: false,
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

#[test]
fn a_second_generate_keeps_the_picture_unless_regen() {
    let root = std::env::temp_dir().join(format!("genos-lref-keep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("refs");
    let mut opts = tiny(dir.clone());
    opts.only_scene = Some("stress-still".into());
    opts.only_checkpoint = Some("hall".into());
    generate(&opts).expect("first trace");
    let linear = dir.join("stress-still/hall/reference.f32");
    let mut marked = std::fs::read(&linear).unwrap();
    marked[20] = marked[20].wrapping_add(1);
    std::fs::write(&linear, &marked).unwrap();
    let kept = generate(&opts).expect("second trace keeps the file");
    assert!(kept.is_empty(), "an existing picture was traced again");
    assert_eq!(std::fs::read(&linear).unwrap(), marked);
    let trace = std::fs::read_to_string(dir.join("stress-still/hall/trace.txt")).unwrap();
    assert!(trace.contains("sample_cap "), "{trace}");
    assert!(trace.contains("build testcommit"), "{trace}");
    let manifest = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
    assert!(manifest.contains("stress-still"), "{manifest}");
    opts.regen = true;
    let replaced = generate(&opts).expect("regen");
    assert_eq!(replaced.len(), 1);
    assert_ne!(std::fs::read(&linear).unwrap(), marked);
    let _ = std::fs::remove_dir_all(&root);
}
