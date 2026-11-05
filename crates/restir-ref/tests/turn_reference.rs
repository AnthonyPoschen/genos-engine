//! CPU turn reference: per-pixel error and variance against the exact light
//! sum while a camera turn keeps revealing pixels with no history.
//!
//! The live config is mirrored: light-tree proposal, 4 candidates, temporal
//! reuse with the M clamp, 4 spatial neighbours in radius 2, balance
//! weighting, one shadow ray per pixel. On a turn the reprojection keeps
//! history for every still-visible surface (`turn_reprojection.rs`), so the
//! only pixels without history are the column entering the screen. The model
//! drops one receiver column per frame, sweeping the grid, on the `shadow`
//! scene (12 lamps, a wall cutting the low ones).
//!
//! Per pixel and per seed, the estimate's luma is compared with the brute
//! luma (every lamp, visibility exact). Reported over many seeds:
//! `bias` = |mean - exact| / exact, and `rmse` = sqrt(E[(x - exact)^2]) /
//! exact, each averaged over the fresh column, the column entered one frame
//! ago, and the whole grid.

#![allow(
    clippy::pedantic,
    clippy::restriction,
    reason = "a test-only statistics harness"
)]

use genos_restir_ref::reservoir::Weighting;
use genos_restir_ref::shade::{brute, luminance};
use genos_restir_ref::{
    Boost, Config, History, Proposal, Scene, Session, TreeFault, DEFAULT_M_CLAMP,
};

const SEEDS: u64 = 600;
const WARMUP: usize = 24;
const TURN_FRAMES: usize = 9;

pub fn live_config() -> Config {
    Config {
        candidates: 4,
        spatial_neighbors: 4,
        spatial_radius: 2,
        temporal: true,
        weighting: Weighting::Balance,
        proposal: Proposal::Tree,
        history: History::Temporal,
        m_clamp: DEFAULT_M_CLAMP,
        tree_fault: TreeFault::None,
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Row {
    pub bias: f32,
    pub rmse: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct TurnStats {
    pub fresh: Row,
    pub second: Row,
    pub steady: Row,
    pub all: Row,
    pub max_shadow_rays: u32,
}

/// Per-pixel relative (bias, rmse) at the last frame over every seed, with
/// the turn's column drops (`turning`) or with the camera still.
fn run(scene: &Scene, boost: Option<Boost>, turning: bool) -> (Vec<f32>, Vec<f32>, usize, usize) {
    let scene = scene.clone();
    let (w, n) = (scene.width, scene.receivers.len());
    let exact: Vec<f32> = scene
        .receivers
        .iter()
        .map(|r| luminance(brute(r, &scene)))
        .collect();
    let (mut sum, mut sq) = (vec![0.0f64; n], vec![0.0f64; n]);
    let mut max_shadow_rays = 0;
    let last = WARMUP + TURN_FRAMES - 1;
    for seed in 0..SEEDS {
        let mut s =
            Session::new(scene.clone(), live_config(), 0x7u64 << 32 | seed).expect("session");
        s.set_boost(boost).expect("boost");
        let mut out = Vec::new();
        for f in 0..=last {
            if turning && f >= WARMUP {
                let col = (f - WARMUP) % w;
                let drop: Vec<usize> = (0..n / w).map(|row| row * w + col).collect();
                s.drop_history(&drop);
            }
            out = s.step();
            max_shadow_rays = max_shadow_rays.max(s.counters().shadow_rays as usize);
        }
        for (i, rgb) in out.iter().enumerate() {
            let x = f64::from(luminance(*rgb));
            sum[i] += x;
            sq[i] += (x - f64::from(exact[i])).powi(2);
        }
    }
    let e = |i: usize| f64::from(exact[i]).max(1e-6);
    let bias = (0..n)
        .map(|i| ((sum[i] / SEEDS as f64 - e(i)).abs() / e(i)) as f32)
        .collect();
    let rmse = (0..n)
        .map(|i| ((sq[i] / SEEDS as f64).sqrt() / e(i)) as f32)
        .collect();
    (bias, rmse, w, max_shadow_rays)
}

/// The turn against the still camera, same boost. `rmse` in each row is the
/// mean over its pixels of turn rmse / still rmse (1.0 = a turn costs
/// nothing); `bias` is the mean relative bias during the turn.
pub fn turn(scene: &Scene, boost: Option<Boost>) -> TurnStats {
    let (bias, rmse, w, rays) = run(scene, boost, true);
    let (_, still, _, _) = run(scene, None, false);
    let last = WARMUP + TURN_FRAMES - 1;
    let fresh_col = (last - WARMUP) % w;
    let second_col = (last - WARMUP - 1) % w;
    let row = |pick: &dyn Fn(usize) -> bool| {
        let idx: Vec<usize> = (0..bias.len()).filter(|&i| pick(i)).collect();
        let k = idx.len() as f32;
        Row {
            bias: idx.iter().map(|&i| bias[i]).sum::<f32>() / k,
            rmse: idx
                .iter()
                .map(|&i| rmse[i] / still[i].max(1e-6))
                .sum::<f32>()
                / k,
        }
    };
    TurnStats {
        fresh: row(&|i| i % w == fresh_col),
        second: row(&|i| i % w == second_col),
        steady: row(&|i| i % w != fresh_col && i % w != second_col),
        all: row(&|_| true),
        max_shadow_rays: rays as u32,
    }
}

#[test]
fn turn_reference_is_unbiased_with_one_shadow_ray() {
    let base = turn(&Scene::shadow(), None);
    eprintln!("turn base: {base:?}");
    let n = Scene::shadow().receivers.len() as u32;
    assert!(base.max_shadow_rays <= n, "one shadow ray per pixel");
    // 600 seeds: a pixel's mean has a standard error of about rmse / 24.
    for r in [base.fresh, base.second, base.all] {
        assert!(r.bias <= 0.05, "biased: {r:?}");
    }
}

#[test]
#[ignore = "exploration: cargo test --release --test turn_reference -- --ignored --nocapture"]
fn explore_boost() {
    for (c, nb, r) in [
        (4, 4, 2),
        (8, 4, 2),
        (16, 4, 2),
        (8, 8, 3),
        (16, 8, 4),
        (32, 8, 3),
        (16, 12, 4),
    ] {
        let b = Boost {
            thin_m: 1,
            candidates: c,
            neighbors: nb,
            radius: r,
        };
        for scene in [Scene::gate(), Scene::shadow()] {
            let t = turn(&scene, Some(b));
            eprintln!("{}occl c{c} n{nb} r{r}: fresh {:.3}/{:.3} second {:.3}/{:.3} steady {:.3}/{:.3} all {:.3}/{:.3}", if scene.occluders.is_empty() { "no-" } else { "" }, t.fresh.bias, t.fresh.rmse, t.second.bias, t.second.rmse, t.steady.bias, t.steady.rmse, t.all.bias, t.all.rmse);
        }
    }
}
