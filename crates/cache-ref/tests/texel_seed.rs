//! ADR 0039 stage 3: cells seed texels, never the reverse.
//!
//! Fast tests pin the history contract and the must-fail controls. The ignored
//! release gate compares scalar-cell and SH-cell seeds against the CPU path
//! trace with interleaved seeds, the stage-2 Step 9 noise rule.

use genos_cache_ref::cache::{CacheConfig, TexelFeedback};
use genos_cache_ref::harness::{self, Bench};
use genos_cache_ref::texel::{Texel, TexelConfig, TexelSeed, TexelSim};

#[test]
fn a_seed_is_not_an_own_sample() {
    let cfg = TexelConfig::default();
    let mut texel = Texel::default();
    texel.seed(0.75, 0.5, false, cfg.seed_sets);
    assert_eq!(texel.own_samples, 0.0);
    assert!((texel.shown(&cfg) - 0.75).abs() < 1.0e-12);
    texel.add_sample(0.25, 1.0, &cfg);
    assert_eq!(texel.own_samples, 1.0);
    assert_eq!(texel.own_mean, 0.25);
    let fade = 1.0 - 1.0 / cfg.history_sets;
    let expected =
        0.25 + (0.75 - 0.25) * cfg.handover_k * 0.5 * fade / (1.0 + cfg.handover_k * 0.5 * fade);
    assert!((texel.shown(&cfg) - expected).abs() < 1.0e-12);
}

#[test]
fn a_lamp_change_drops_the_seed() {
    let mut texel = Texel::default();
    texel.seed(0.75, 1.0, false, TexelConfig::default().seed_sets);
    texel.clear_seed();
    assert_eq!(texel.shown(&TexelConfig::default()), 0.0);
    assert_eq!(texel.own_samples, 0.0);
}

#[test]
fn seed_as_samples_survives_the_event_clear() {
    let good = TexelConfig::default();
    let broken = TexelConfig {
        seed_as_samples: true,
        ..good
    };
    for cfg in [good, broken] {
        let mut texel = Texel::default();
        texel.seed(0.75, 1.0, cfg.seed_as_samples, cfg.seed_sets);
        texel.clear_seed();
        if cfg.seed_as_samples {
            assert_eq!(
                texel.own_samples, cfg.seed_sets,
                "must-fail fixture was not installed"
            );
            assert!(
                texel.shown(&cfg) > 0.05,
                "fault did not retain the stale seed"
            );
        } else {
            assert_eq!(texel.shown(&cfg), 0.0);
        }
    }
}

#[test]
fn scalar_and_sh_seed_runs_are_finite() -> Result<(), String> {
    let b = Bench {
        pixel_size: 0.2,
        ref_spp: 32,
        threads: 2,
        ref_dir: None,
        warm_frames: 8,
        samples: 2,
        seeds: vec![11],
        ..Bench::default()
    };
    let prepared = b.prepare("contact").ok_or("missing contact scene")?;
    let cells =
        genos_cache_ref::cache::Sim::new(&prepared.scene, CacheConfig::plain(0.25, 16), 11, 2);
    let p = prepared
        .scene
        .pixels
        .iter()
        .find(|p| p.valid)
        .ok_or("no receiver")?;
    let (scalar_a, sh_a) = cells.seed_values(p.p, p.n);
    let (scalar_b, sh_b) = cells.seed_values(p.p, p.n);
    assert_eq!(
        (scalar_a, sh_a),
        (scalar_b, sh_b),
        "seed arms must share cell state"
    );
    for seed in [TexelSeed::Scalar, TexelSeed::Sh] {
        let result = TexelSim::run_static(&b, &prepared, seed, TexelConfig::default());
        assert!(
            result.energy.is_finite() && result.energy > 0.0,
            "{seed:?}: {result:?}"
        );
        assert!(result.blotch.is_finite(), "{seed:?}: {result:?}");
        assert!(result.fill_frame > 0, "{seed:?}: {result:?}");
    }
    Ok(())
}

/// F3 is the existing cache-ref feedback fault. It must still fail with the
/// stage-3 texel layer present; a green feedback arm would invalidate the gate.
#[test]
fn texel_feedback_still_fails_linger() -> Result<(), String> {
    let b = Bench {
        pixel_size: 0.2,
        ref_spp: 32,
        threads: 2,
        ref_dir: None,
        warm_frames: 48,
        samples: 2,
        seeds: vec![11],
        ..Bench::default()
    };
    let prepared = b.prepare("lamp-off").ok_or("missing lamp-off scene")?;
    let cells = CacheConfig {
        texel: Some(TexelFeedback {
            alpha: 0.99,
            beta: 0.5,
        }),
        ..CacheConfig::plain(0.25, 4)
    };
    let timing = harness::run_lamp_off(&b, &prepared, cells, 120);
    assert!(
        timing.linger > 0.05,
        "must-fail feedback arm passed: {}",
        timing.linger
    );
    Ok(())
}

/// Full CPU reference gate. Run after stage 2 and #168 merge, one process:
/// `cargo test --release -p genos-cache-ref --test texel_seed -- --ignored --nocapture --test-threads=1`
#[test]
#[ignore = "release path traces; run on an otherwise idle box"]
fn stage_3_step_9_reference_gates() -> Result<(), String> {
    let b = Bench {
        pixel_size: 0.1,
        ref_spp: 256,
        threads: 2,
        ref_dir: None,
        warm_frames: 160,
        samples: 4,
        seeds: vec![11, 12],
        ..Bench::default()
    };
    for scene in ["contact", "near-wall", "thin-wall", "shadow-edge"] {
        let prepared = b.prepare(scene).ok_or("missing scene")?;
        let (baseline, candidate) = TexelSim::compare_step_9(
            &b,
            &prepared,
            TexelConfig::default(),
            TexelConfig::default(),
        );
        assert!(
            candidate.energy >= 0.9,
            "{scene}: energy {}",
            candidate.energy
        );
        assert!(
            candidate.energy >= baseline.energy - baseline.energy_noise,
            "{scene}: energy dropped {} -> {} (noise {})",
            baseline.energy,
            candidate.energy,
            baseline.energy_noise
        );
        assert!(
            candidate.blotch <= 0.014,
            "{scene}: blotch {}",
            candidate.blotch
        );
        assert!(
            candidate.blotch <= baseline.blotch + baseline.blotch_noise,
            "{scene}: blotch regression {} -> {}",
            baseline.blotch,
            candidate.blotch
        );
        assert!(
            candidate.leak <= baseline.leak + baseline.leak_noise,
            "{scene}: leak regression {} -> {}",
            baseline.leak,
            candidate.leak
        );
        assert!(
            candidate.near_error <= baseline.near_error + baseline.near_error_noise,
            "{scene}: near-camera regression {} -> {}",
            baseline.near_error,
            candidate.near_error
        );
        assert!(
            candidate.pop <= baseline.pop + baseline.pop_noise,
            "{scene}: new pops {} -> {}",
            baseline.pop,
            candidate.pop
        );
        assert!(
            candidate.fill_frame <= baseline.fill_frame,
            "{scene}: fill regressed {} -> {}",
            baseline.fill_frame,
            candidate.fill_frame
        );
    }
    Ok(())
}

/// ADR 0037/0039 timing: 300 dark -> 300 lit, and lamp-off -> black at 120.
#[test]
#[ignore = "release path traces; run on an otherwise idle box"]
fn stage_3_lag_linger_and_quiet_drain() -> Result<(), String> {
    let b = Bench {
        pixel_size: 0.1,
        ref_spp: 256,
        threads: 2,
        ref_dir: None,
        warm_frames: 300,
        samples: 2,
        seeds: vec![11, 12],
        ..Bench::default()
    };
    for scene_name in ["lamp-on", "lamp-off"] {
        let prepared = b.prepare(scene_name).ok_or("missing timing scene")?;
        for &seed in &b.seeds {
            let mut sim = TexelSim::new(
                &prepared.scene,
                CacheConfig::plain(0.25, 16),
                TexelConfig {
                    seed: TexelSeed::Sh,
                    ..TexelConfig::default()
                },
                seed,
                2,
            );
            sim.set_reference(prepared.reference.clone());
            for _ in 0..300 {
                sim.step();
            }
            let before = sim.texel_rays;
            if scene_name == "lamp-on" {
                for _ in 0..300 {
                    sim.step();
                }
                let energy = genos_cache_ref::metrics::energy(
                    &prepared.scene,
                    &sim.display(),
                    &prepared.reference,
                );
                assert!(energy >= 0.9, "lag@300 energy {energy}");
            } else {
                let timing = sim.run_lamp_timing(120);
                assert!(timing.linger <= 0.05, "linger@120 {}", timing.linger);
            }
            let seed_writes = sim.seed_writes;
            for _ in 0..120 {
                sim.step();
            }
            assert_eq!(
                sim.seed_writes, seed_writes,
                "quiet scene reseeded repeatedly"
            );
            assert!(
                sim.texel_rays > before,
                "drain control never traced its checks"
            );
        }
    }
    Ok(())
}
