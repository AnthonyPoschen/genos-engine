//! One frame of `ReSTIR` DI on a receiver grid.
//!
//! Draw order, for a fixed seed, is the contract a later GPU stage matches:
//! 1. For each receiver in index order, for each candidate: the proposal draw
//!    (uniform rejection, one `f32` on the target CDF, or one `f32` per internal
//!    light-tree node), then one `f32` for the stream pick.
//! 2. For each receiver, when temporal reuse is on: one `f32`, then the merge
//!    with history. The draw still happens when history is empty.
//! 3. For each receiver, when `spatial_neighbors > 0`: neighbour-index draws
//!    (partial Fisher-Yates over the Chebyshev disk), then one `f32` for the
//!    spatial merge.
//!
//! History stored for the next frame is the post-temporal reservoir
//! ([`History::Temporal`]). The shaded image uses the post-spatial reservoir.
//! [`History::Final`] stores the shaded reservoir instead. The gate uses
//! temporal history so a neighbour's samples are not counted again next frame.
//!
//! [`Session::reset_temporal`] is the change record. The next frame reuses
//! neighbours only.

use genos_math::Vec3;

use crate::reservoir::{
    finalize_initial, m_as_f32, merge, stream_update, MergeInput, Reservoir, Weighting, LAMP_EMPTY,
};
use crate::rng::Rng;
use crate::shade::{target, Receiver, Scene};
use crate::tree::{LightTree, TreeFault};

/// How the initial candidates are proposed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proposal {
    /// Each lamp equally likely, including those behind the receiver.
    Uniform,
    /// Discrete distribution proportional to the unshadowed luminance.
    Target,
    /// Light-tree bound. One leaf per candidate. Not the flat target CDF.
    Tree,
}

/// Which reservoir becomes next frame's history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum History {
    /// The temporal merge, before neighbours. This is the unbiased default.
    Temporal,
    /// The shaded reservoir, after spatial reuse.
    Final,
}

/// Knobs for one reference run. The CLI writes the same fields into the dump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub candidates: u32,
    pub spatial_neighbors: u32,
    pub spatial_radius: u32,
    pub temporal: bool,
    pub weighting: Weighting,
    pub proposal: Proposal,
    pub history: History,
    /// History `M` is clamped to this multiple of the current frame's candidate
    /// count before the temporal merge. `0` leaves history `M` unclamped.
    /// The default, 20, is the usual `ReSTIR` cap (Bitterli et al., TOG 2020).
    pub m_clamp: u32,
    /// Applied only when `proposal` is [`Proposal::Tree`].
    pub tree_fault: TreeFault,
}

/// Default history-`M` multiple. `cap = m_clamp * current_candidate_count`.
pub const DEFAULT_M_CLAMP: u32 = 20;

impl Config {
    /// Target candidates and temporal reuse. Spatial neighbours stay off.
    ///
    /// A target proposal with no occlusion makes each receiver's luminance the
    /// integral, so the picture holds still and `grid_worst` sees a step under
    /// the 0.002 floor. A fresh spatial sample moves that mean by enough to
    /// score as a checkerboard. Spatial reuse is `--spatial` and the open-grid test.
    #[must_use]
    pub const fn gate() -> Self {
        Self {
            candidates: 8,
            spatial_neighbors: 0,
            spatial_radius: 2,
            temporal: true,
            weighting: Weighting::Balance,
            proposal: Proposal::Target,
            history: History::Temporal,
            m_clamp: DEFAULT_M_CLAMP,
            tree_fault: TreeFault::None,
        }
    }

    /// Validates the ranges the reference is willing to run.
    ///
    /// # Errors
    ///
    /// Returns a string when a count is outside the small-scene range.
    pub fn validate(self) -> Result<(), String> {
        if self.candidates == 0 || self.candidates > 64 {
            return Err("candidates must be from 1 to 64".to_string());
        }
        if self.spatial_neighbors > 16 {
            return Err("spatial neighbours must be at most 16".to_string());
        }
        if self.spatial_radius > 32 {
            return Err("spatial radius must be at most 32".to_string());
        }
        if self.m_clamp > 4096 {
            return Err("M clamp must be at most 4096".to_string());
        }
        Ok(())
    }
}

/// Counters for one [`Session::step`]. The dump and the CLI print these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counters {
    pub candidates: u32,
    pub temporal_merges: u32,
    pub spatial_merges: u32,
    pub shadow_rays: u32,
    pub empty: u32,
}

impl Counters {
    const fn zero() -> Self {
        Self {
            candidates: 0,
            temporal_merges: 0,
            spatial_merges: 0,
            shadow_rays: 0,
            empty: 0,
        }
    }
}

/// A running reference. Owns the scene, the history, and one PCG stream.
#[derive(Clone, Debug)]
pub struct Session {
    scene: Scene,
    config: Config,
    rng: Rng,
    tree: LightTree,
    history: Vec<Reservoir>,
    shaded: Vec<Reservoir>,
    counters: Counters,
    frame: u32,
}

impl Session {
    /// Starts at frame 0 with empty history.
    ///
    /// # Errors
    ///
    /// Returns a string when the config or the receiver grid is inconsistent.
    pub fn new(scene: Scene, config: Config, seed: u64) -> Result<Self, String> {
        config.validate()?;
        if scene.receivers.is_empty() {
            return Err("the scene has no receivers".to_string());
        }
        if scene.width == 0 || !scene.receivers.len().is_multiple_of(scene.width) {
            return Err("receiver count must divide by width".to_string());
        }
        let count = scene.receivers.len();
        let tree = LightTree::build(&scene.lamps);
        Ok(Self {
            scene,
            config,
            rng: Rng::new(seed),
            tree,
            history: vec![Reservoir::empty(); count],
            shaded: vec![Reservoir::empty(); count],
            counters: Counters::zero(),
            frame: 0,
        })
    }

    #[must_use]
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    #[must_use]
    pub const fn config(&self) -> Config {
        self.config
    }

    #[must_use]
    pub const fn frame(&self) -> u32 {
        self.frame
    }

    #[must_use]
    pub const fn counters(&self) -> Counters {
        self.counters
    }

    #[must_use]
    pub fn shaded(&self) -> &[Reservoir] {
        &self.shaded
    }

    /// Sample count stored for the next temporal merge.
    #[must_use]
    pub fn history_m(&self, index: usize) -> Option<u32> {
        self.history.get(index).map(|reservoir| reservoir.m)
    }

    /// Moves one lamp. The next temporal merge sees the new position.
    /// History `M` is still clamped, so a moved lamp does not keep a stale weight.
    pub fn set_lamp_position(&mut self, index: usize, position: Vec3) -> bool {
        match self.scene.lamps.get_mut(index) {
            Some(lamp) => {
                lamp.position = position;
                self.tree = LightTree::build(&self.scene.lamps);
                true
            }
            None => false,
        }
    }

    /// Drops temporal history. The next frame reuses neighbours only.
    pub fn reset_temporal(&mut self) {
        for reservoir in &mut self.history {
            *reservoir = Reservoir::empty();
        }
    }

    /// One frame. Returns reflected radiance per receiver, visibility included.
    #[must_use]
    pub fn step(&mut self) -> Vec<Vec3> {
        let count = self.scene.receivers.len();
        let mut initial = vec![Reservoir::empty(); count];
        let mut counters = Counters::zero();
        for (index, slot) in initial.iter_mut().enumerate() {
            *slot = self.initial_at(index, &mut counters);
        }
        let mut temporal = vec![Reservoir::empty(); count];
        for (index, slot) in temporal.iter_mut().enumerate() {
            *slot = self.temporal_at(index, &initial, &mut counters);
        }
        if self.config.history == History::Temporal {
            self.history.clone_from(&temporal);
        }
        for index in 0..count {
            self.shaded[index] = self.spatial_at(index, &temporal, &mut counters);
        }
        if self.config.history == History::Final {
            self.history.clone_from(&self.shaded);
        }
        let radiance = self.shade_all(&mut counters);
        self.counters = counters;
        self.frame = self.frame.saturating_add(1);
        radiance
    }

    fn shade_all(&self, counters: &mut Counters) -> Vec<Vec3> {
        let mut radiance = Vec::with_capacity(self.shaded.len());
        for (index, reservoir) in self.shaded.iter().enumerate() {
            let Some(receiver) = self.scene.receivers.get(index) else {
                radiance.push(Vec3::ZERO);
                continue;
            };
            if reservoir.is_empty() {
                counters.empty = counters.empty.saturating_add(1);
                radiance.push(Vec3::ZERO);
            } else {
                counters.shadow_rays = counters.shadow_rays.saturating_add(1);
                radiance.push(crate::shade::shade(receiver, *reservoir, &self.scene));
            }
        }
        radiance
    }

    fn initial_at(&mut self, index: usize, counters: &mut Counters) -> Reservoir {
        let weights = lamp_weights(&self.scene, index);
        let mut reservoir = Reservoir::empty();
        let count = usize::try_from(self.config.candidates).unwrap_or(1);
        for _ in 0..count {
            counters.candidates = counters.candidates.saturating_add(1);
            let Some((lamp, pdf)) = self.propose(index, &weights) else {
                stream_update(&mut reservoir, LAMP_EMPTY, 0.0, &mut self.rng);
                continue;
            };
            let phat = weights.get(lamp).copied().unwrap_or(0.0);
            let stream = if pdf > 0.0 { phat / pdf } else { 0.0 };
            let lamp_id = u32::try_from(lamp).unwrap_or(LAMP_EMPTY);
            stream_update(&mut reservoir, lamp_id, stream, &mut self.rng);
        }
        let selected = usize::try_from(reservoir.lamp).unwrap_or(usize::MAX);
        finalize_initial(
            &mut reservoir,
            weights.get(selected).copied().unwrap_or(0.0),
        );
        reservoir
    }

    fn propose(&mut self, index: usize, weights: &[f32]) -> Option<(usize, f32)> {
        match self.config.proposal {
            Proposal::Uniform => {
                if weights.is_empty() {
                    return None;
                }
                let index = self.rng.uniform_usize(weights.len());
                Some((index, 1.0 / len_as_f32(weights.len())))
            }
            Proposal::Target => sample_cdf(weights, &mut self.rng),
            Proposal::Tree => {
                let receiver = self.scene.receivers.get(index).copied()?;
                let fault = self.config.tree_fault;
                self.tree.sample(&receiver, fault, &mut self.rng)
            }
        }
    }

    fn temporal_at(
        &mut self,
        index: usize,
        initial: &[Reservoir],
        counters: &mut Counters,
    ) -> Reservoir {
        let current = initial.get(index).copied().unwrap_or_else(Reservoir::empty);
        if !self.config.temporal {
            return current;
        }
        let unit = self.rng.next_f32();
        let mut previous = self
            .history
            .get(index)
            .copied()
            .unwrap_or_else(Reservoir::empty);
        clamp_history(&mut previous, self.config.m_clamp, current.m);
        if previous.is_empty() {
            return current;
        }
        counters.temporal_merges = counters.temporal_merges.saturating_add(1);
        let slots = [(index, current), (index, previous)];
        self.merge_slots(index, &slots, unit)
    }

    fn spatial_at(
        &mut self,
        index: usize,
        temporal: &[Reservoir],
        counters: &mut Counters,
    ) -> Reservoir {
        let current = temporal
            .get(index)
            .copied()
            .unwrap_or_else(Reservoir::empty);
        if self.config.spatial_neighbors == 0 || self.config.spatial_radius == 0 {
            return current;
        }
        let height = self.scene.receivers.len() / self.scene.width;
        let neighbours = neighbor_indices(
            index,
            self.scene.width,
            height,
            self.config.spatial_radius,
            self.config.spatial_neighbors,
            &mut self.rng,
        );
        let unit = self.rng.next_f32();
        if neighbours.is_empty() {
            return current;
        }
        let mut slots = Vec::with_capacity(neighbours.len().saturating_add(1));
        slots.push((index, current));
        for neighbour in neighbours {
            let reservoir = temporal
                .get(neighbour)
                .copied()
                .unwrap_or_else(Reservoir::empty);
            slots.push((neighbour, reservoir));
        }
        counters.spatial_merges = counters.spatial_merges.saturating_add(1);
        self.merge_slots(index, &slots, unit)
    }

    fn merge_slots(&self, dest: usize, slots: &[(usize, Reservoir)], unit: f32) -> Reservoir {
        let Some(destination) = self.scene.receivers.get(dest) else {
            return Reservoir::empty();
        };
        let inputs: Vec<MergeInput> = slots
            .iter()
            .map(|(_, reservoir)| MergeInput {
                reservoir: *reservoir,
                target_q: lamp_luminance(destination, reservoir.lamp, &self.scene),
            })
            .collect();
        let scene = &self.scene;
        merge(
            &inputs,
            self.config.weighting,
            |slot, lamp| {
                let source = slots.get(slot).map_or(dest, |(index, _)| *index);
                scene
                    .receivers
                    .get(source)
                    .map_or(0.0, |receiver| lamp_luminance(receiver, lamp, scene))
            },
            unit,
        )
    }
}

fn lamp_weights(scene: &Scene, index: usize) -> Vec<f32> {
    let Some(receiver) = scene.receivers.get(index) else {
        return Vec::new();
    };
    scene
        .lamps
        .iter()
        .map(|lamp| target(receiver, lamp))
        .collect()
}

fn lamp_luminance(receiver: &Receiver, lamp_id: u32, scene: &Scene) -> f32 {
    if lamp_id == LAMP_EMPTY {
        return 0.0;
    }
    let Some(index) = usize::try_from(lamp_id).ok() else {
        return 0.0;
    };
    scene
        .lamps
        .get(index)
        .map_or(0.0, |lamp| target(receiver, lamp))
}

fn len_as_f32(len: usize) -> f32 {
    crate::reservoir::m_as_f32(u32::try_from(len).unwrap_or(1))
}

fn sample_cdf(weights: &[f32], rng: &mut Rng) -> Option<(usize, f32)> {
    let sum: f32 = weights.iter().copied().sum();
    if sum <= 0.0 {
        let _ = rng.next_f32();
        return None;
    }
    let mark = rng.next_f32() * sum;
    let mut acc = 0.0;
    for (index, weight) in weights.iter().enumerate() {
        if *weight <= 0.0 {
            continue;
        }
        acc += *weight;
        if mark < acc {
            return Some((index, *weight / sum));
        }
    }
    weights
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, weight)| {
            if *weight > 0.0 {
                Some((index, *weight / sum))
            } else {
                None
            }
        })
}

/// Scales history so its `M` is at most `factor * current_m`.
///
/// `current_m` is this frame's candidate count. The stored reservoir can
/// still be one candidate above the cap, because the clamp runs on the
/// history input and the merge then adds the new sample.
fn clamp_history(previous: &mut Reservoir, factor: u32, current_m: u32) {
    if factor == 0 || current_m == 0 || previous.m == 0 {
        return;
    }
    let cap = factor.saturating_mul(current_m);
    if previous.m <= cap {
        return;
    }
    let scale = m_as_f32(cap) / m_as_f32(previous.m);
    previous.w_sum *= scale;
    previous.m = cap;
}

fn neighbor_indices(
    index: usize,
    width: usize,
    height: usize,
    radius: u32,
    count: u32,
    rng: &mut Rng,
) -> Vec<usize> {
    if count == 0 || radius == 0 || width == 0 || height == 0 {
        return Vec::new();
    }
    let x = index % width;
    let y = index / width;
    let reach = usize::try_from(radius).unwrap_or(0);
    let x0 = x.saturating_sub(reach);
    let y0 = y.saturating_sub(reach);
    let x1 = x.saturating_add(reach).min(width.saturating_sub(1));
    let y1 = y.saturating_add(reach).min(height.saturating_sub(1));
    let mut pool = Vec::new();
    for row in y0..=y1 {
        for col in x0..=x1 {
            if col == x && row == y {
                continue;
            }
            pool.push(row.saturating_mul(width).saturating_add(col));
        }
    }
    let take = usize::try_from(count).unwrap_or(pool.len()).min(pool.len());
    for slot in 0..take {
        let pick = slot + rng.uniform_usize(pool.len() - slot);
        if pick < pool.len() {
            pool.swap(slot, pick);
        }
    }
    pool.truncate(take);
    pool
}

#[cfg(test)]
mod tests {
    use super::{Config, History, Proposal, Session};
    use crate::reservoir::Weighting;
    use crate::shade::{brute, luminance, Scene};

    fn reuse(candidates: u32, spatial: u32, temporal: bool, proposal: Proposal) -> Config {
        Config {
            candidates,
            spatial_neighbors: spatial,
            spatial_radius: 1,
            temporal,
            weighting: Weighting::Balance,
            proposal,
            history: History::Temporal,
            m_clamp: super::DEFAULT_M_CLAMP,
            tree_fault: crate::tree::TreeFault::None,
        }
    }

    fn rel(est: f32, reference: f32) -> f32 {
        (est - reference).abs() / reference.abs().max(1.0e-6)
    }

    #[test]
    fn target_proposal_matches_brute_luminance() -> Result<(), String> {
        let scene = Scene::open_row();
        let brute_luma = luminance(brute(&scene.receivers[0], &scene));
        let config = reuse(4, 0, true, Proposal::Target);
        let mut session = Session::new(scene, config, 11)?;
        let mut estimate = 0.0;
        for _ in 0..6 {
            let frame = session.step();
            estimate = luminance(*frame.first().ok_or("no receiver")?);
        }
        let error = rel(estimate, brute_luma);
        assert!(
            error < 1.0e-3,
            "luminance error {error} estimate {estimate} brute {brute_luma}"
        );
        Ok(())
    }

    #[test]
    fn coloured_lights_match_in_rgb_on_average() -> Result<(), String> {
        let scene = Scene::open_row();
        let reference = brute(&scene.receivers[0], &scene);
        let config = reuse(4, 0, true, Proposal::Target);
        let mut sum = genos_math::Vec3::ZERO;
        let seeds = 96u32;
        for seed in 1..=seeds {
            let mut session = Session::new(scene.clone(), config, u64::from(seed))?;
            let mut frame = genos_math::Vec3::ZERO;
            for _ in 0..4 {
                frame = *session.step().first().ok_or("no receiver")?;
            }
            sum += frame;
        }
        let mean = sum / crate::reservoir::m_as_f32(seeds);
        for (got, expect) in
            [mean.x, mean.y, mean.z]
                .into_iter()
                .zip([reference.x, reference.y, reference.z])
        {
            let error = rel(got, expect);
            assert!(
                error < 0.12,
                "channel error {error} got {got} expect {expect}"
            );
        }
        Ok(())
    }

    #[test]
    fn variance_falls_with_temporal_reuse() -> Result<(), String> {
        let scene = Scene::variance_point();
        let early_config = reuse(1, 0, false, Proposal::Uniform);
        let late_config = reuse(1, 0, true, Proposal::Uniform);
        let mut early = Vec::new();
        let mut late = Vec::new();
        for seed in 1..=64u64 {
            let mut once = Session::new(scene.clone(), early_config, seed)?;
            early.push(luminance(*once.step().first().ok_or("no receiver")?));
            let mut many = Session::new(scene.clone(), late_config, seed)?;
            let mut value = 0.0;
            for _ in 0..24 {
                value = luminance(*many.step().first().ok_or("no receiver")?);
            }
            late.push(value);
        }
        let early_var = variance(&early);
        let late_var = variance(&late);
        assert!(
            late_var < early_var * 0.35,
            "early {early_var} late {late_var}"
        );
        Ok(())
    }

    #[test]
    fn open_grid_mean_stays_near_the_brute_sum() -> Result<(), String> {
        let scene = Scene::open_grid();
        let config = reuse(2, 3, true, Proposal::Uniform);
        let mut acc = vec![genos_math::Vec3::ZERO; scene.receivers.len()];
        let seeds = 36u32;
        for seed in 1..=seeds {
            let mut session = Session::new(scene.clone(), config, u64::from(seed))?;
            let mut frame = Vec::new();
            for _ in 0..10 {
                frame = session.step();
            }
            for (sum, sample) in acc.iter_mut().zip(frame) {
                *sum += sample;
            }
        }
        let scale = crate::reservoir::m_as_f32(seeds);
        let mut worst = 0.0f32;
        for (index, sum) in acc.iter().enumerate() {
            let Some(receiver) = scene.receivers.get(index) else {
                continue;
            };
            let reference = luminance(brute(receiver, &scene));
            worst = worst.max(rel(luminance(*sum / scale), reference));
        }
        assert!(worst < 0.06, "worst relative error {worst}");
        Ok(())
    }

    #[test]
    fn balance_beats_section3_when_neighbours_disagree() -> Result<(), String> {
        let scene = Scene::domain_pair();
        let balance = domain_error(&scene, Weighting::Balance)?;
        assert!(balance < 0.08, "balance error {balance}");
        Ok(())
    }

    fn domain_error(scene: &Scene, weighting: Weighting) -> Result<f32, String> {
        let config = Config {
            candidates: 1,
            spatial_neighbors: 1,
            spatial_radius: 1,
            temporal: false,
            weighting,
            proposal: Proposal::Uniform,
            history: History::Temporal,
            m_clamp: super::DEFAULT_M_CLAMP,
            tree_fault: crate::tree::TreeFault::None,
        };
        let mut acc = vec![genos_math::Vec3::ZERO; scene.receivers.len()];
        let seeds = 2000u32;
        for seed in 1..=seeds {
            let mut session = Session::new(scene.clone(), config, u64::from(seed))?;
            for (sum, sample) in acc.iter_mut().zip(session.step()) {
                *sum += sample;
            }
        }
        let scale = crate::reservoir::m_as_f32(seeds);
        let mut worst = 0.0f32;
        for (index, sum) in acc.iter().enumerate() {
            let Some(receiver) = scene.receivers.get(index) else {
                continue;
            };
            let reference = luminance(brute(receiver, scene));
            worst = worst.max(rel(luminance(*sum / scale), reference));
        }
        Ok(worst)
    }

    #[test]
    fn reset_temporal_drops_the_sample_count() -> Result<(), String> {
        let config = reuse(2, 0, true, Proposal::Uniform);
        let mut session = Session::new(Scene::open_row(), config, 3)?;
        for _ in 0..10 {
            let _ = session.step();
        }
        let grown = session.history_m(0).ok_or("missing history")?;
        assert!(grown > 2, "history M {grown}");
        session.reset_temporal();
        let _ = session.step();
        let fresh = session.history_m(0).ok_or("missing history")?;
        assert_eq!(fresh, 2, "reset left M {fresh}");
        Ok(())
    }

    /// A lamp that moves keeps a huge unclamped `M`, so the shade stays on the
    /// old integral. The 20× cap lets the new integral win inside a fixed frame
    /// budget (Bitterli et al. 2020, temporal reuse).
    #[test]
    fn a_moved_lamp_stops_being_stale_inside_the_m_clamp() -> Result<(), String> {
        let scene = Scene {
            lamps: vec![
                crate::shade::Lamp {
                    position: genos_math::Vec3::new(0.0, 1.0, 0.15),
                    intensity: genos_math::Vec3::new(30.0, 28.0, 24.0),
                },
                crate::shade::Lamp {
                    position: genos_math::Vec3::new(2.4, 1.1, 0.2),
                    intensity: genos_math::Vec3::new(8.0, 8.0, 8.0),
                },
            ],
            receivers: vec![crate::shade::Receiver {
                position: genos_math::Vec3::ZERO,
                normal: genos_math::Vec3::Y,
                albedo: genos_math::Vec3::new(0.8, 0.8, 0.8),
            }],
            occluders: Vec::new(),
            width: 1,
            camera: genos_math::Vec3::new(0.0, 1.6, -2.0),
        };
        let mut held = Session::new(scene.clone(), mover(0), 9)?;
        let mut fresh = Session::new(scene.clone(), mover(super::DEFAULT_M_CLAMP), 9)?;
        for _ in 0..48 {
            let _ = held.step();
            let _ = fresh.step();
        }
        let grown = held.history_m(0).ok_or("missing history")?;
        let limited = fresh.history_m(0).ok_or("missing history")?;
        assert!(grown > 40, "unclamped M {grown}");
        assert!(
            limited <= super::DEFAULT_M_CLAMP + 1,
            "clamped M {limited}"
        );
        let far = genos_math::Vec3::new(7.5, 1.0, 0.15);
        assert!(held.set_lamp_position(0, far));
        assert!(fresh.set_lamp_position(0, far));
        let bound = 24u32;
        let mut settled = None;
        for frame in 1..=bound {
            let held_luma = luminance(*held.step().first().ok_or("no receiver")?);
            let fresh_luma = luminance(*fresh.step().first().ok_or("no receiver")?);
            let truth = luminance(brute(&fresh.scene().receivers[0], fresh.scene()));
            let held_err = rel(held_luma, truth);
            let fresh_err = rel(fresh_luma, truth);
            if fresh_err < 0.15 {
                assert!(
                    held_err > 0.5,
                    "unclamped path already matched at frame {frame}: error {held_err}"
                );
                settled = Some(frame);
                break;
            }
        }
        let settled = settled.ok_or("clamped path still stale after 24 frames")?;
        assert!(settled <= bound, "settled at {settled}");
        Ok(())
    }

    fn mover(m_clamp: u32) -> Config {
        Config {
            candidates: 1,
            spatial_neighbors: 0,
            spatial_radius: 1,
            temporal: true,
            weighting: Weighting::Balance,
            proposal: Proposal::Uniform,
            history: History::Temporal,
            m_clamp,
            tree_fault: crate::tree::TreeFault::None,
        }
    }

    fn variance(values: &[f32]) -> f32 {
        let n = crate::reservoir::m_as_f32(u32::try_from(values.len()).unwrap_or(1));
        let mean = values.iter().sum::<f32>() / n;
        values
            .iter()
            .map(|value| (value - mean) * (value - mean))
            .sum::<f32>()
            / n
    }
}
