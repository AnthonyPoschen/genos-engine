#![expect(
    clippy::many_single_char_names,
    reason = "same short names as the crate's harness (b bench, k knobs, s scene)"
)]

//! ADR 0039 addendum A gates. The smoke test runs in debug in seconds; the
//! `#[ignore]` gates need `--release` and minutes:
//!
//! ```text
//! cargo test --release -p genos-cache-ref -- --ignored
//! ```

use genos_cache_ref::cache::{CacheConfig, Selection, TexelFeedback};
use genos_cache_ref::harness::{self, Bench};
use genos_cache_ref::report::Knobs;

fn small() -> Bench {
    Bench {
        pixel_size: 0.1,
        ref_spp: 32,
        threads: 2,
        ref_dir: None,
        warm_frames: 24,
        samples: 2,
        seeds: vec![11],
        ..Bench::default()
    }
}

fn gate() -> Bench {
    Bench {
        ref_spp: 512,
        ref_dir: None,
        seeds: vec![11, 12],
        ..Bench::default()
    }
}

#[test]
fn smoke_contact_runs_and_is_finite() -> Result<(), String> {
    let b = small();
    let k = Knobs::default();
    let s = b.prepare("contact").ok_or("no contact scene")?;
    for cfg in [k.plain(), k.stage_2b()] {
        let m = harness::run_static(&b, &s, cfg);
        assert!(
            m.energy.is_finite() && m.energy > 0.3 && m.energy < 1.7,
            "energy {}",
            m.energy
        );
        assert!(m.blotch.is_finite(), "blotch {}", m.blotch);
        assert!(m.cells > 0.0, "no cells");
    }
    Ok(())
}

/// F3 must fail: texel results copied into cells hold light after the lamp
/// goes off.
#[test]
#[ignore = "release, minutes"]
fn f3_texel_feedback_fails_linger() -> Result<(), String> {
    let b = gate();
    let k = Knobs::default();
    let s = b.prepare("lamp-off").ok_or("no lamp-off scene")?;
    let f3 = CacheConfig {
        texel: Some(TexelFeedback {
            alpha: 0.99,
            beta: 0.5,
        }),
        ..k.plain()
    };
    let t = harness::run_lamp_off(&b, &s, f3, k.linger_after);
    assert!(
        t.linger > 0.05,
        "F3 passed linger ({}) so the test is too weak",
        t.linger
    );
    Ok(())
}

/// F2 must fail: skewed ray origins without 1/q weights bias hard spots
/// measurably (more than 3 seed sd beyond uniform), and the weights remove
/// most of it.
#[test]
#[ignore = "release, minutes"]
fn f2_unweighted_skew_biases_hard_spots() -> Result<(), String> {
    let b = gate();
    let k = Knobs::default();
    let s = b.prepare("contact").ok_or("no contact scene")?;
    let hard: Vec<usize> = genos_cache_ref::cache::hardness(&s.scene)
        .iter()
        .enumerate()
        .filter(|(i, h)| s.scene.pixels[*i].valid && **h >= 0.5)
        .map(|(i, _)| i)
        .collect();
    let run = |selection| {
        harness::region_bias_seeds(
            &b,
            &s,
            CacheConfig {
                selection,
                skew: 64.0,
                ..k.plain()
            },
            &hard,
        )
    };
    let u = run(Selection::Uniform);
    let f2 = run(Selection::SkewUnweighted);
    let w = run(Selection::SkewWeighted);
    let sd = u.1.max(f2.1).max(1e-4);
    let d = (f2.0 - u.0).abs();
    assert!(
        d > 3.0 * sd,
        "F2 bias {d} not measurable (sd {sd}) so the test is too weak"
    );
    assert!((w.0 - u.0).abs() < d, "1/q weights did not reduce the bias");
    Ok(())
}

/// Stage 2b against plain on thin-wall and lamp-off: the gates that hold.
#[test]
#[ignore = "release, minutes"]
fn stage_2b_gates() -> Result<(), String> {
    let b = gate();
    let k = Knobs::default();
    for name in ["contact", "near-wall", "thin-wall", "shadow-edge"] {
        let s = b.prepare(name).ok_or("missing scene")?;
        let p = harness::run_static(&b, &s, k.plain());
        let r = harness::run_static(&b, &s, k.stage_2b());
        assert!(r.energy >= 0.9, "{name}: 2b energy {}", r.energy);
        assert!(
            r.blotch < p.blotch,
            "{name}: 2b blotch {} not below plain {}",
            r.blotch,
            p.blotch
        );
    }
    let s = b.prepare("lamp-off").ok_or("no lamp-off scene")?;
    let timing = harness::run_lamp_off(&b, &s, k.stage_2b(), k.linger_after);
    assert!(timing.linger <= 0.05, "2b linger {}", timing.linger);
    assert!(timing.lag >= 0.9, "2b lag {}", timing.lag);
    Ok(())
}

/// The lighting-memory mapping is self-consistent: the cell size picked for a
/// pool needs that pool back.
#[test]
fn pool_mapping_round_trips() {
    use genos_cache_ref::report::{cell_for_pool, pool_mb, WORLD_CELLS_AT_25CM};
    for mb in [64.0, 128.0, 256.0, 512.0] {
        let h = cell_for_pool(mb, WORLD_CELLS_AT_25CM, 34, 0.25);
        let ratio = (0.25 / h) * (0.25 / h);
        let back = pool_mb(ratio * 1000.0, 1000.0, WORLD_CELLS_AT_25CM, 34);
        assert!(
            (back - mb).abs() < 1e-6 * mb,
            "{mb} MiB -> {h} m -> {back} MiB"
        );
    }
}
