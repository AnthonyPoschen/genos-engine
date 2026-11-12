//! Stage-3 detail texels: a low-confidence cell seed beside their own samples.
//! The cache's rays read only the cache. Texel means never enter a cell.
//!
//! This deliberately models the seed/handover boundary, not stage 4 ray cuts.

use std::f64::consts::PI;

use crate::cache::{CacheConfig, Sim};
use crate::geom::{fl, uniform_hemi, EPS};
use crate::harness::{self, Bench, Prepared};
use crate::metrics;
use crate::rng::Rng;
use crate::scene::Scene;

/// Cell value used for the first fill of a texel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexelSeed {
    /// The stage-2 scalar irradiance representation.
    Scalar,
    /// The stage-2 SH-L1 representation evaluated at the texel normal.
    Sh,
}

/// Every stage-3 history knob. These become mirrored reference CLI parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TexelConfig {
    pub rays_per_texel: usize,
    pub history_sets: f64,
    pub clear_keep_sets: f64,
    pub recover_frames: usize,
    /// Seed confidence in equivalent sets, capped separately from own samples.
    pub seed_sets: f64,
    pub handover_k: f64,
    pub near_camera_m: f64,
    pub seed: TexelSeed,
    /// Must-fail control: seed becomes own mean and sample count.
    pub seed_as_samples: bool,
}

impl Default for TexelConfig {
    fn default() -> Self {
        Self {
            rays_per_texel: 1,
            history_sets: 32.0,
            clear_keep_sets: 2.0,
            recover_frames: 24,
            seed_sets: 8.0,
            handover_k: 8.0,
            near_camera_m: 1.5,
            seed: TexelSeed::Scalar,
            seed_as_samples: false,
        }
    }
}

/// One detail texel. `seed` never contributes to `own_samples`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Texel {
    pub own_mean: f64,
    pub own_samples: f64,
    pub seed_value: f64,
    pub seed_confidence: f64,
    pub since_change: usize,
    pub lamps_were_on: bool,
}

impl Texel {
    pub fn seed(&mut self, value: f64, confidence: f64, as_samples: bool, seed_sets: f64) {
        if as_samples {
            self.own_mean = value;
            self.own_samples = seed_sets;
            self.seed_value = 0.0;
            self.seed_confidence = 0.0;
        } else {
            self.seed_value = value;
            self.seed_confidence = confidence.clamp(0.0, 1.0);
        }
    }

    pub fn clear_seed(&mut self) {
        self.seed_value = 0.0;
        self.seed_confidence = 0.0;
    }

    /// Add real ray-derived samples. No seed value reaches this accumulator.
    pub fn add_sample(&mut self, sample: f64, sets: f64, cfg: &TexelConfig) {
        let frames = if self.since_change < cfg.recover_frames {
            cfg.clear_keep_sets
        } else {
            cfg.history_sets
        };
        let previous = self.own_samples.min(frames);
        self.own_mean = (previous * self.own_mean + sets * sample) / (previous + sets);
        self.own_samples = previous + sets;
    }

    #[must_use]
    pub fn shown(&self, cfg: &TexelConfig) -> f64 {
        let fade = (1.0 - self.own_samples / cfg.history_sets).clamp(0.0, 1.0);
        let k = cfg.handover_k * self.seed_confidence * fade;
        let own_weight = self.own_samples / (self.own_samples + k).max(1.0e-12);
        self.seed_value * (1.0 - own_weight) + self.own_mean * own_weight
    }
}

/// Stage-3 comparison summary; `*_noise` is max-min over repeated arms.
#[derive(Clone, Copy, Debug, Default)]
pub struct Result {
    pub energy: f64,
    pub blotch: f64,
    pub leak: f64,
    pub near_error: f64,
    pub pop: f64,
    pub fill_frame: usize,
    pub energy_noise: f64,
    pub blotch_noise: f64,
    pub leak_noise: f64,
    pub near_error_noise: f64,
    pub pop_noise: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Run {
    energy: f64,
    blotch: f64,
    leak: f64,
    near_error: f64,
    pop: f64,
    fill_frame: usize,
}

fn combine(runs: &[Run]) -> Result {
    let n = fl(runs.len().max(1));
    let pick = |f: fn(&Run) -> f64| {
        let values = runs.iter().map(f).collect::<Vec<_>>();
        let mean = values.iter().sum::<f64>() / n;
        let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (mean, hi - lo)
    };
    let (energy, energy_noise) = pick(|r| r.energy);
    let (blotch, blotch_noise) = pick(|r| r.blotch);
    let (leak, leak_noise) = pick(|r| r.leak);
    let (near_error, near_error_noise) = pick(|r| r.near_error);
    let (pop, pop_noise) = pick(|r| r.pop);
    Result {
        energy,
        blotch,
        leak,
        near_error,
        pop,
        fill_frame: runs.iter().map(|r| r.fill_frame).sum::<usize>() / runs.len().max(1),
        energy_noise,
        blotch_noise,
        leak_noise,
        near_error_noise,
        pop_noise,
    }
}

/// A cell simulation and a separate detail layer over the same scene.
pub struct TexelSim<'a> {
    pub cells: Sim<'a>,
    pub texels: Vec<Texel>,
    pub cfg: TexelConfig,
    pub seed_kind: TexelSeed,
    pub reference: Vec<f64>,
    rng: Rng,
    pub texel_rays: u64,
    pub seed_writes: u64,
}

impl<'a> TexelSim<'a> {
    #[must_use]
    pub fn new(
        scene: &'a Scene,
        cell_cfg: CacheConfig,
        cfg: TexelConfig,
        seed: u64,
        threads: usize,
    ) -> Self {
        let mut cells = Sim::new(scene, cell_cfg, seed, threads);
        // The stage-1 reference approximation has a real irradiance mean for
        // Scalar. Sh uses the directional field, not a second bounce estimate.
        cells.step();
        let texels: Vec<Texel> = scene
            .pixels
            .iter()
            .map(|p| {
                let mut t = Texel {
                    lamps_were_on: scene.lamps_on(0),
                    since_change: usize::MAX,
                    ..Texel::default()
                };
                if p.valid {
                    let (scalar, directional) = cells.seed_values(p.p, p.n);
                    let value = if matches!(cfg.seed, TexelSeed::Sh) {
                        directional
                    } else {
                        scalar
                    };
                    if value > 0.0 {
                        t.seed(value, 1.0, cfg.seed_as_samples, cfg.seed_sets);
                    }
                }
                t
            })
            .collect();
        let seed_writes = u64::try_from(
            texels
                .iter()
                .filter(|t| t.seed_confidence > 0.0 || t.own_samples > 0.0)
                .count(),
        )
        .unwrap_or(u64::MAX);
        Self {
            cells,
            texels,
            cfg,
            seed_kind: cfg.seed,
            reference: Vec::new(),
            rng: Rng::new(seed ^ 0x73a3_5eed),
            texel_rays: 0,
            seed_writes,
        }
    }

    pub fn set_reference(&mut self, reference: Vec<f64>) {
        self.reference = reference;
    }

    /// Advance cells, then texels. Only `cells.read*` is called from texels.
    pub fn step(&mut self) {
        self.cells.step();
        let frame = self.cells.frame;
        let lamps = self.cells.scene.lamps_on(frame);
        for i in 0..self.texels.len() {
            let p = self.cells.scene.pixels[i];
            if !p.valid {
                continue;
            }
            let mut texel = self.texels[i];
            if lamps != texel.lamps_were_on {
                texel.clear_seed();
                texel.own_samples = texel.own_samples.min(self.cfg.clear_keep_sets);
                texel.lamps_were_on = lamps;
                texel.since_change = 0;
                let (scalar, directional) = self.cells.seed_values(p.p, p.n);
                let value = if matches!(self.seed_kind, TexelSeed::Sh) {
                    directional
                } else {
                    scalar
                };
                // F4 deliberately keeps a stale own mean; the real path stores
                // only the new live cell seed beside the own accumulator.
                if !self.cfg.seed_as_samples && value > 0.0 {
                    texel.seed(value, 1.0, false, self.cfg.seed_sets);
                    self.seed_writes += 1;
                }
            }
            // A tiny deterministic real-sample model. Stage 3 keeps every texel
            // tracing; stage 4, not this test, decides which rays can go away.
            let sample = self.trace_texel(i);
            texel.add_sample(sample, 1.0, &self.cfg);
            texel.since_change = texel.since_change.saturating_add(1);
            self.texels[i] = texel;
        }
    }

    fn trace_texel(&mut self, index: usize) -> f64 {
        let p = self.cells.scene.pixels[index];
        let cells = &self.cells;
        let mut total = 0.0;
        for _ in 0..self.cfg.rays_per_texel {
            let d = uniform_hemi(p.n, &mut self.rng);
            let o = p.p.add(p.n.scale(EPS * 4.0));
            if let Some(hit) = cells.scene.world.trace(o, d, 1.0e9) {
                if hit.n.dot(d) < 0.0 {
                    let lamps = cells.scene.lamps_on(cells.frame);
                    total += cells.scene.world.albedo / PI
                        * (cells.scene.world.direct(hit.p, hit.n, lamps)
                            + cells.read(hit.p, hit.n));
                }
            }
            self.texel_rays += 1;
        }
        total / fl(self.cfg.rays_per_texel.max(1))
    }

    #[must_use]
    pub fn display(&self) -> Vec<f64> {
        self.texels
            .iter()
            .zip(&self.cells.scene.pixels)
            .map(|(t, p)| if p.valid { t.shown(&self.cfg) } else { 0.0 })
            .collect()
    }

    /// Interleave baseline/candidate per seed so paired runs see the same load.
    #[must_use]
    pub fn run_static(b: &Bench, s: &Prepared, kind: TexelSeed, cfg: TexelConfig) -> Result {
        let runs = b
            .seeds
            .iter()
            .map(|&seed| run_one(b, s, kind, cfg, seed))
            .collect::<Vec<_>>();
        combine(&runs)
    }

    /// Paired Step-9 run. This is the method the full gate uses, not sequential
    /// all-baseline then all-candidate batches.
    #[must_use]
    pub fn compare_step_9(
        b: &Bench,
        s: &Prepared,
        baseline_cfg: TexelConfig,
        candidate_cfg: TexelConfig,
    ) -> (Result, Result) {
        let mut baseline = Vec::new();
        let mut candidate = Vec::new();
        for &seed in &b.seeds {
            baseline.push(run_one(b, s, TexelSeed::Scalar, baseline_cfg, seed));
            candidate.push(run_one(b, s, TexelSeed::Sh, candidate_cfg, seed));
        }
        (combine(&baseline), combine(&candidate))
    }

    #[must_use]
    pub fn run_lamp_timing(&mut self, after: usize) -> harness::Timing {
        let lit = metrics::mean_valid(self.cells.scene, &self.reference);
        let lag = metrics::energy(self.cells.scene, &self.display(), &self.reference);
        for _ in 0..after {
            self.step();
        }
        let linger = metrics::mean_valid(self.cells.scene, &self.display()) / lit;
        harness::Timing { lag, linger }
    }
}

fn run_one(b: &Bench, s: &Prepared, kind: TexelSeed, mut cfg: TexelConfig, seed: u64) -> Run {
    cfg.seed = kind;
    let mut sim = TexelSim::new(&s.scene, CacheConfig::plain(0.25, 4), cfg, seed, b.threads);
    sim.set_reference(s.reference.clone());
    let mut fill_frame = b.warm_frames;
    for frame in 0..b.warm_frames {
        let display = sim.display();
        if fill_frame == b.warm_frames && metrics::energy(&s.scene, &display, &s.reference) >= 0.9 {
            fill_frame = frame;
        }
        sim.step();
    }
    let mut run = Run {
        fill_frame,
        ..Run::default()
    };
    let mut previous = sim.display();
    for _ in 0..b.samples {
        let display = sim.display();
        run.energy += metrics::energy(&s.scene, &display, &s.reference);
        run.blotch += metrics::blotch(&s.scene, &display, &s.reference, b.blotch_sigma);
        run.leak += metrics::region_bias(&s.scene.leak, &display, &s.reference);
        let near: Vec<usize> = s
            .scene
            .pixels
            .iter()
            .enumerate()
            .filter(|(_, p)| p.valid && p.p.sub(s.scene.camera).length() <= cfg.near_camera_m)
            .map(|(i, _)| i)
            .collect();
        run.near_error += metrics::mean_rel_error_in(&near, &display, &s.reference);
        run.pop += metrics::pop(&s.scene, &previous, &display, &s.reference);
        previous = display;
        sim.step();
    }
    let k = 1.0 / fl(b.samples.max(1));
    run.energy *= k;
    run.blotch *= k;
    run.leak *= k;
    run.near_error *= k;
    run.pop *= k;
    run
}
