//! Runs: a scene, a reference, a cache config, seeds; and the report.

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::cache::{CacheConfig, Sim};
use crate::geom::fl;
use crate::metrics;
use crate::reference;
use crate::scene::{Scene, Timeline};

/// Shared settings for a batch of runs. All are CLI arguments in the binary.
#[derive(Clone, Debug)]
pub struct Bench {
    pub pixel_size: f64,
    pub ref_spp: usize,
    pub ref_seed: u64,
    pub threads: usize,
    pub ref_dir: Option<PathBuf>,
    /// Blotch fine scale in metres.
    pub blotch_sigma: f64,
    /// Frames before measuring a static scene.
    pub warm_frames: usize,
    /// Measured frames (each one display pair).
    pub samples: usize,
    pub seeds: Vec<u64>,
}

impl Default for Bench {
    fn default() -> Self {
        Self {
            pixel_size: 0.05,
            ref_spp: 2048,
            ref_seed: 1,
            threads: std::thread::available_parallelism().map_or(4, std::num::NonZero::get),
            ref_dir: Some(PathBuf::from("genos-debug/cache-ref/reference")),
            blotch_sigma: 0.075,
            warm_frames: 160,
            samples: 8,
            seeds: vec![11, 12, 13, 14],
        }
    }
}

/// A scene with its reference.
pub struct Prepared {
    pub scene: Scene,
    pub reference: Vec<f64>,
}

impl Bench {
    /// Build `name` and its reference (cached on disk when `ref_dir` is set).
    #[must_use]
    pub fn prepare(&self, name: &str) -> Option<Prepared> {
        let scene = Scene::by_name(name, self.pixel_size)?;
        let reference = reference::indirect(
            &scene,
            self.ref_spp,
            self.ref_seed,
            self.threads,
            self.ref_dir.as_deref(),
        );
        Some(Prepared { scene, reference })
    }
}

/// Seed-averaged results of one config on one scene.
#[derive(Clone, Copy, Debug, Default)]
pub struct Metrics {
    pub energy: f64,
    pub blotch: f64,
    /// Relative bias on the scene's leak region (0 when it has none).
    pub leak: f64,
    pub mean_rel_error: f64,
    /// p99 frame-to-frame relative change, steady state.
    pub pop: f64,
    pub cells: f64,
    pub rays_per_frame: f64,
    /// Seed-to-seed standard deviation of energy and blotch.
    pub energy_sd: f64,
    pub blotch_sd: f64,
}

fn mean_sd(v: &[f64]) -> (f64, f64) {
    let n = fl(v.len().max(1));
    let m = v.iter().sum::<f64>() / n;
    let var = v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / n;
    (m, var.sqrt())
}

/// Steady-state accuracy on a static (or dolly-held) scene.
#[must_use]
pub fn run_static(b: &Bench, s: &Prepared, cfg: CacheConfig) -> Metrics {
    let mut per_seed = Vec::new();
    for &seed in &b.seeds {
        let mut sim = Sim::new(&s.scene, cfg, seed, b.threads);
        let rays_before = |sim: &Sim| sim.counters.rays;
        for _ in 0..b.warm_frames {
            sim.step();
        }
        let r0 = rays_before(&sim);
        let mut m = Metrics::default();
        for _ in 0..b.samples {
            let before = sim.display();
            sim.step();
            let after = sim.display();
            m.energy += metrics::energy(&s.scene, &after, &s.reference);
            m.blotch += metrics::blotch(&s.scene, &after, &s.reference, b.blotch_sigma);
            m.leak += metrics::region_bias(&s.scene.leak, &after, &s.reference);
            m.mean_rel_error += metrics::mean_rel_error(&s.scene, &after, &s.reference);
            m.pop += metrics::pop(&s.scene, &before, &after, &s.reference);
        }
        let k = 1.0 / fl(b.samples.max(1));
        m.energy *= k;
        m.blotch *= k;
        m.leak *= k;
        m.mean_rel_error *= k;
        m.pop *= k;
        m.cells = fl(sim.live_cells());
        let rays = f64::from(u32::try_from(sim.counters.rays - r0).unwrap_or(u32::MAX));
        m.rays_per_frame = rays * k;
        per_seed.push(m);
    }
    combine(&per_seed)
}

fn combine(v: &[Metrics]) -> Metrics {
    let pick = |f: fn(&Metrics) -> f64| mean_sd(&v.iter().map(f).collect::<Vec<_>>());
    let (energy, energy_sd) = pick(|m| m.energy);
    let (blotch, blotch_sd) = pick(|m| m.blotch);
    Metrics {
        energy,
        blotch,
        leak: pick(|m| m.leak).0,
        mean_rel_error: pick(|m| m.mean_rel_error).0,
        pop: pick(|m| m.pop).0,
        cells: pick(|m| m.cells).0,
        rays_per_frame: pick(|m| m.rays_per_frame).0,
        energy_sd,
        blotch_sd,
    }
}

/// Relative bias over `region` after warm-up, mean and seed sd.
#[must_use]
pub fn region_bias_seeds(
    b: &Bench,
    s: &Prepared,
    cfg: CacheConfig,
    region: &[usize],
) -> (f64, f64) {
    let mut v = Vec::new();
    for &seed in &b.seeds {
        let mut sim = Sim::new(&s.scene, cfg, seed, b.threads);
        for _ in 0..b.warm_frames {
            sim.step();
        }
        let mut acc = 0.0;
        for _ in 0..b.samples {
            sim.step();
            acc += metrics::region_bias(region, &sim.display(), &s.reference);
        }
        v.push(acc / fl(b.samples.max(1)));
    }
    mean_sd(&v)
}

/// Lamp-off timing: energy reached by the off frame (lag) and what is left
/// `after` frames later as a share of the lamps-on reference (linger).
#[derive(Clone, Copy, Debug, Default)]
pub struct Timing {
    pub lag: f64,
    pub linger: f64,
}

/// Run the `lamp-off` timeline.
#[must_use]
pub fn run_lamp_off(b: &Bench, s: &Prepared, cfg: CacheConfig, after: usize) -> Timing {
    let Timeline::LampOff { off_frame } = s.scene.timeline else {
        return Timing::default();
    };
    let mean_ref = metrics::mean_valid(&s.scene, &s.reference);
    let mut out = Vec::new();
    for &seed in &b.seeds {
        let mut sim = Sim::new(&s.scene, cfg, seed, b.threads);
        for _ in 0..off_frame {
            sim.step();
        }
        let lag = metrics::energy(&s.scene, &sim.display(), &s.reference);
        for _ in 0..after {
            sim.step();
        }
        let linger = metrics::mean_valid(&s.scene, &sim.display()) / mean_ref;
        out.push((lag, linger));
    }
    Timing {
        lag: mean_sd(&out.iter().map(|t| t.0).collect::<Vec<_>>()).0,
        linger: mean_sd(&out.iter().map(|t| t.1).collect::<Vec<_>>()).0,
    }
}

/// Stage 2c on the dolly: what splitting did.
#[derive(Clone, Copy, Debug, Default)]
pub struct DollyResult {
    /// Largest p99 frame-to-frame change over the run.
    /// `toggles_after_hold` counts splits and merges from 60 frames after the
    /// camera stops.
    pub pop_max: f64,
    /// Median p99 change over the held tail (steady state).
    pub pop_steady: f64,
    /// Blotch over split pixels at the end.
    pub blotch_split: f64,
    pub energy: f64,
    pub splits: f64,
    pub merges: f64,
    pub toggles_after_hold: f64,
    pub catch_up_rays: f64,
    pub peak_cells: f64,
    pub rays: f64,
}

/// Run the dolly timeline for `frames` frames. Returns the result and the
/// split-pixel mask at the end of the first seed's run (for a static
/// comparison over the same pixels).
#[must_use]
pub fn run_dolly(
    b: &Bench,
    s: &Prepared,
    cfg: CacheConfig,
    frames: usize,
) -> (DollyResult, Vec<bool>) {
    let hold_from = match s.scene.timeline {
        Timeline::Dolly { frames, .. } => frames,
        Timeline::Static | Timeline::LampOff { .. } => 0,
    };
    let mut all = Vec::new();
    let mut mask_out = Vec::new();
    for &seed in &b.seeds {
        let mut sim = Sim::new(&s.scene, cfg, seed, b.threads);
        let mut prev = sim.display();
        let mut pops = Vec::new();
        let mut tail = Vec::new();
        let mut toggles_at_hold = 0;
        for f in 0..frames {
            sim.step();
            if f + 1 == hold_from + 60 {
                toggles_at_hold = sim.counters.splits + sim.counters.merges;
            }
            let cur = sim.display();
            let p = metrics::pop(&s.scene, &prev, &cur, &s.reference);
            pops.push(p);
            if f > hold_from + 60 {
                tail.push(p);
            }
            prev = cur;
        }
        tail.sort_by(f64::total_cmp);
        let mask = split_mask(&sim);
        let r = DollyResult {
            pop_max: pops.iter().copied().fold(0.0, f64::max),
            pop_steady: tail.get(tail.len() / 2).copied().unwrap_or(0.0),
            blotch_split: metrics::blotch_in(
                &s.scene,
                &prev,
                &s.reference,
                b.blotch_sigma,
                Some(&mask),
            ),
            energy: metrics::energy(&s.scene, &prev, &s.reference),
            splits: count(sim.counters.splits),
            merges: count(sim.counters.merges),
            toggles_after_hold: count(
                (sim.counters.splits + sim.counters.merges).saturating_sub(toggles_at_hold),
            ),
            catch_up_rays: count(sim.counters.catch_up_rays),
            peak_cells: fl(sim.counters.peak_cells),
            rays: count(sim.counters.rays),
        };
        if mask_out.is_empty() {
            mask_out = mask;
        }
        all.push(r);
    }
    let avg = |f: fn(&DollyResult) -> f64| mean_sd(&all.iter().map(f).collect::<Vec<_>>()).0;
    (
        DollyResult {
            pop_max: avg(|r| r.pop_max),
            pop_steady: avg(|r| r.pop_steady),
            blotch_split: avg(|r| r.blotch_split),
            energy: avg(|r| r.energy),
            splits: avg(|r| r.splits),
            merges: avg(|r| r.merges),
            toggles_after_hold: avg(|r| r.toggles_after_hold),
            catch_up_rays: avg(|r| r.catch_up_rays),
            peak_cells: avg(|r| r.peak_cells),
            rays: avg(|r| r.rays),
        },
        mask_out,
    )
}

fn count(n: u64) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Pixels whose level-0 cell is split.
fn split_mask(sim: &Sim) -> Vec<bool> {
    let mut mask = vec![false; sim.scene.pixels.len()];
    for (i, px) in sim.scene.pixels.iter().enumerate() {
        if !px.valid {
            continue;
        }
        let side = crate::cache::side_of(px.n, &sim.cfg);
        let key = crate::cache::Key {
            side,
            k: sim.lat.nearest(px.p),
            child: None,
        };
        mask[i] = sim
            .keys
            .iter()
            .zip(&sim.cells)
            .any(|(k, c)| *k == key && c.alive && c.split);
    }
    mask
}

/// Static blotch over `mask` for a config (the 2c comparison point).
#[must_use]
pub fn static_blotch_in(b: &Bench, s: &Prepared, cfg: CacheConfig, mask: &[bool]) -> f64 {
    let mut v = Vec::new();
    for &seed in &b.seeds {
        let mut sim = Sim::new(&s.scene, cfg, seed, b.threads);
        for _ in 0..b.warm_frames {
            sim.step();
        }
        let d = sim.display();
        v.push(metrics::blotch_in(
            &s.scene,
            &d,
            &s.reference,
            b.blotch_sigma,
            Some(mask),
        ));
    }
    mean_sd(&v).0
}

/// One row of a results table.
pub fn row(out: &mut String, label: &str, m: &Metrics) {
    let _ = writeln!(
        out,
        "| {label} | {:.3} ± {:.3} | {:.4} ± {:.4} | {:+.3} | {:.3} | {:.4} | {:.0} | {:.0} |",
        m.energy,
        m.energy_sd,
        m.blotch,
        m.blotch_sd,
        m.leak,
        m.mean_rel_error,
        m.pop,
        m.cells,
        m.rays_per_frame
    );
}

/// Header matching [`row`].
pub const HEADER: &str = "| config | energy | blotch | leak | mean rel err | pop p99 | cells | rays/frame |\n|---|---|---|---|---|---|---|---|";
