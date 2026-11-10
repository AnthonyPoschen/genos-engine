//! CPU mirror of `shaders/particles.comp`.
//!
//! Positions accumulate in f64. The shader uses the same xorshift, the same
//! spawn order, and f32 arithmetic. A GPU position may differ by at most 1e-3 m.
//! The GPU runs the walk in parallel. This mirror keeps the serial slot order
//! that the GPU matches.
//!
//! Haar and Aaltonen (SIGGRAPH 2015) update a persistent buffer from many
//! threads. This stage keeps that buffer and the dead ring. The mirror stays
//! serial so its slot order is the order the parallel scan has to reproduce.

use genos_scene::{ParticleSettings, DEFAULT_SEED};

/// Depth-fade width when the setting is zero: collision still retires, fade stays 1.
const FALLBACK_BAND: f64 = 1.0e-4;

/// Bytes of one live slot: position, age, velocity, life.
pub const SLOT_STRIDE: u64 = 32;

/// Bytes of one compacted record: position/age, velocity/life, albedo/fade.
pub const COMPACT_STRIDE: u64 = 48;

/// Bytes of the parameter block the CPU writes each frame.
pub const PARAMS_BYTES: u64 = 256;

/// Header, scan launch, and the indirect command's neighbour.
///
/// Words 16 and 17 are the block-sum scan's workgroup size and group count.
pub const STATE_BYTES: u64 = 80;

/// Bytes of the scan buffer: per-slot flags and prefixes, then two levels of
/// block sums. One thread never walks the groups.
#[must_use]
pub fn scan_buffer_bytes(max_particles: u32) -> u64 {
    let slots = u64::from(max_particles);
    let groups = u64::from(max_particles.div_ceil(64).max(1));
    let level1 = groups.div_ceil(64).max(1);
    let level2 = level1.div_ceil(64).max(1);
    slots
        .saturating_mul(2)
        .saturating_add(groups.saturating_mul(2))
        .saturating_add(level1.saturating_mul(2))
        .saturating_add(level2.saturating_mul(2))
        .saturating_mul(4)
}

const INDIRECT_BYTES: u64 = 16;

/// Low's particle buffers stay at or under this many bytes. Decimal, not MiB.
pub const LOW_VRAM_BYTES: u64 = 16_000_000;

/// Stage-1 buffer bytes for `max_particles`, decimal, not including the scene depth image.
#[must_use]
pub fn buffer_bytes(max_particles: u32) -> u64 {
    let slots = u64::from(max_particles);
    let scan = scan_buffer_bytes(max_particles);
    PARAMS_BYTES
        .saturating_add(STATE_BYTES)
        .saturating_add(INDIRECT_BYTES)
        .saturating_add(slots.saturating_mul(SLOT_STRIDE))
        .saturating_add(slots.saturating_mul(4))
        .saturating_add(slots.saturating_mul(COMPACT_STRIDE))
        .saturating_add(scan)
}

/// Decimal megabytes of [`buffer_bytes`].
#[allow(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "byte counts above 2^24 are not exact in f64; the log prints three decimals"
)]
#[must_use]
pub fn buffer_megabytes(max_particles: u32) -> f64 {
    buffer_bytes(max_particles) as f64 / 1_000_000.0
}

/// Identity the shader uses to decide a reset. Seed and budget only.
#[must_use]
pub fn sim_generation(seed: u32, budget: u32) -> u32 {
    seed ^ budget.wrapping_mul(0x9E37_79B9)
}

/// One xorshift32 step. Zero becomes [`DEFAULT_SEED`].
#[must_use]
pub fn xorshift(mut bits: u32) -> u32 {
    bits ^= bits << 13;
    bits ^= bits >> 17;
    bits ^= bits << 5;
    if bits == 0 {
        DEFAULT_SEED
    } else {
        bits
    }
}

/// Unit interval from the top 24 bits. Matches the shader's `float` divide.
#[allow(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "the shader divides the same 24-bit integer by 16777216 in f32"
)]
#[must_use]
pub fn unit(rng: &mut u32) -> f32 {
    *rng = xorshift(*rng);
    ((*rng) >> 8) as f32 / 16_777_216.0
}

/// A stage-1 mutation. The compute shader and the CPU mirror both honour it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ParticleFault {
    /// The step as specified.
    #[default]
    None,
    /// Do not return a dead slot to the ring.
    SkipDeadPush,
    /// Leave the indirect instance count at zero.
    SkipIndirect,
    /// Record the step without the cross-submit compute barrier.
    ///
    /// The canary fill keeps its transfer barrier. Synchronization validation
    /// should report the compute read of the touch fill, which this barrier
    /// orders. The shipping path does not set this.
    SkipFlightBarrier,
}

impl ParticleFault {
    /// Flag bits in the compute uniform. Freeze and depth are separate.
    #[must_use]
    pub fn bits(self) -> u32 {
        match self {
            Self::None | Self::SkipFlightBarrier => 0,
            Self::SkipDeadPush => 2,
            Self::SkipIndirect => 4,
        }
    }
}

/// What one step reads. The GPU uniform is the same numbers.
#[derive(Clone, Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "freeze, depth collision, and the two mutations are independent flags"
)]
pub struct StepInput {
    /// Emitter origin, metres.
    pub origin: [f32; 3],
    /// Births per second.
    pub rate: f32,
    /// Lifetime, seconds.
    pub life: f32,
    /// Upward speed, metres per second.
    pub rise: f32,
    /// Spawn radius and sideways speed.
    pub spread: f32,
    /// Card width, metres. The mirror does not expand cards.
    pub size: f32,
    /// Unlit colour.
    pub albedo: [f32; 3],
    /// Seed after the zero substitution.
    pub seed: u32,
    /// Buffer budget.
    pub budget: u32,
    /// Live cap, already clamped to the budget.
    pub count: u32,
    /// Seconds. Non-positive or non-finite leaves the step still.
    pub dt: f32,
    /// Skip integrate and emit.
    pub freeze: bool,
    /// Retire particles behind the depth samples.
    pub depth_collision: bool,
    /// Mutation: do not return a dead slot to the ring.
    pub skip_dead: bool,
    /// Mutation: leave the indirect instance count at zero.
    pub skip_indirect: bool,
    /// NDC width of the depth fade.
    pub soft_band: f32,
    /// Column-major view-projection. Used only for the depth test.
    pub view_proj: [f32; 16],
    /// Depth buffer width. Zero skips collision and fade.
    pub depth_w: u32,
    /// Depth buffer height.
    pub depth_h: u32,
    /// Row-major depths, 0 near and 1 far. Empty when `depth_w` is zero.
    pub depth: Vec<f32>,
}

impl StepInput {
    /// The stage-1 emitter, one step.
    #[must_use]
    pub fn from_settings(
        settings: &ParticleSettings,
        dt: f32,
        view_proj: [f32; 16],
        depth_w: u32,
        depth_h: u32,
        depth: Vec<f32>,
    ) -> Self {
        Self {
            origin: settings.origin,
            rate: settings.rate,
            life: settings.life,
            rise: settings.rise,
            spread: settings.spread,
            size: settings.size,
            albedo: settings.albedo,
            seed: settings.rng_seed(),
            budget: settings.max_particles,
            count: settings.live_cap(),
            dt,
            freeze: settings.freeze,
            depth_collision: settings.depth_collision,
            skip_dead: false,
            skip_indirect: false,
            soft_band: settings.soft_band,
            view_proj,
            depth_w,
            depth_h,
            depth,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    /// Metres. The f64 integral. The shader stores the f32 value of the same steps.
    position: [f64; 3],
    velocity: [f64; 3],
    /// Seconds, f32, so a retirement lands on the same frame as the shader.
    age: f32,
    life: f32,
}

/// Serial mirror of the compute step.
#[derive(Clone, Debug)]
pub struct CpuParticles {
    generation: u32,
    rng: u32,
    head: u32,
    dead_count: u32,
    alive: u32,
    spawned: u32,
    died: u32,
    spawned_frame: u32,
    /// Emit residual, f32, matching the shader's `float` clock.
    clock: f32,
    budget: u32,
    slots: Vec<Slot>,
    dead: Vec<u32>,
    positions: Vec<[f64; 3]>,
    fades: Vec<f64>,
    draw_instances: u32,
}

impl CpuParticles {
    /// Empty until the first step sees a budget.
    #[must_use]
    pub fn new() -> Self {
        Self {
            generation: 0,
            rng: 0,
            head: 0,
            dead_count: 0,
            alive: 0,
            spawned: 0,
            died: 0,
            spawned_frame: 0,
            clock: 0.0,
            budget: 0,
            slots: Vec::new(),
            dead: Vec::new(),
            positions: Vec::new(),
            fades: Vec::new(),
            draw_instances: 0,
        }
    }

    /// Alive particles after the last step.
    #[must_use]
    pub fn alive(&self) -> u32 {
        self.alive
    }

    /// Births since the last reset.
    #[must_use]
    pub fn spawned(&self) -> u32 {
        self.spawned
    }

    /// Retirements since the last reset.
    #[must_use]
    pub fn died(&self) -> u32 {
        self.died
    }

    /// Births during the last step.
    #[must_use]
    pub fn spawned_frame(&self) -> u32 {
        self.spawned_frame
    }

    /// Indirect instance count. Zero when the skip-indirect mutation is on.
    #[must_use]
    pub fn draw_instances(&self) -> u32 {
        self.draw_instances
    }

    /// Compacted positions, slot order, metres.
    #[must_use]
    pub fn positions(&self) -> &[[f64; 3]] {
        &self.positions
    }

    /// Bit mix of the alive count and the compacted positions. Replay and checkpoint compare it.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut hash = u64::from(self.alive).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for position in &self.positions {
            for axis in position {
                hash = hash
                    .wrapping_mul(0x0000_0100_0000_01B3)
                    .wrapping_add(axis.to_bits());
            }
        }
        hash
    }

    /// Depth fade of each compacted particle, 0 to 1.
    #[must_use]
    pub fn fades(&self) -> &[f64] {
        &self.fades
    }

    /// Integrate, emit, then compact.
    pub fn step(&mut self, input: &StepInput) {
        let generation = sim_generation(input.seed, input.budget);
        if input.budget == 0 {
            self.clear();
            return;
        }
        if self.generation != generation || self.budget != input.budget {
            self.reset(input.seed, input.budget, generation);
        }
        self.spawned_frame = 0;
        let moving = input.dt.is_finite() && input.dt > 0.0 && !input.freeze;
        if moving {
            self.integrate(input);
            self.emit(input);
        }
        self.compact(input);
    }

    fn clear(&mut self) {
        *self = Self::new();
    }

    fn reset(&mut self, seed: u32, budget: u32, generation: u32) {
        let width = usize::try_from(budget).unwrap_or(0);
        self.generation = generation;
        self.rng = seed;
        self.head = 0;
        self.dead_count = budget;
        self.alive = 0;
        self.spawned = 0;
        self.died = 0;
        self.spawned_frame = 0;
        self.clock = 0.0;
        self.budget = budget;
        self.slots = vec![
            Slot {
                position: [0.0; 3],
                velocity: [0.0; 3],
                age: 0.0,
                life: 0.0,
            };
            width
        ];
        self.dead = (0..width).map(|index| u32::try_from(index).unwrap_or(0)).collect();
        self.positions.clear();
        self.fades.clear();
        self.draw_instances = 0;
    }

    fn integrate(&mut self, input: &StepInput) {
        let dt = f64::from(input.dt);
        let budget = self.budget;
        let mut retiring = Vec::new();
        for index in 0..budget {
            let Some(slot) = self.slots.get_mut(usize::try_from(index).unwrap_or(0)) else {
                continue;
            };
            if slot.life <= 0.0 {
                continue;
            }
            slot.age += input.dt;
            for axis in 0..3 {
                slot.position[axis] += slot.velocity[axis] * dt;
            }
            let behind = depth_fade(input, slot.position) < 0.0;
            if slot.age >= slot.life || (input.depth_collision && behind) {
                slot.life = 0.0;
                retiring.push(index);
            }
        }
        for index in retiring {
            self.alive = self.alive.saturating_sub(1);
            self.died = self.died.saturating_add(1);
            if !input.skip_dead {
                push_dead(
                    &mut self.dead,
                    budget,
                    &mut self.head,
                    &mut self.dead_count,
                    index,
                );
            }
        }
    }

    fn emit(&mut self, input: &StepInput) {
            self.clock += input.dt;
        let interval = if input.rate > 0.0 {
            1.0 / input.rate
        } else {
            1.0e9
        };
        let mut guard = 0u32;
        while self.clock >= interval
            && self.alive < input.count
            && self.dead_count > 0
            && guard < input.count
        {
            guard += 1;
            self.clock -= interval;
            let Some(index) =
                pop_dead(&self.dead, self.budget, &mut self.head, &mut self.dead_count)
            else {
                break;
            };
            let birth = spawn_slot(input, &mut self.rng);
            let Some(slot) = self.slots.get_mut(usize::try_from(index).unwrap_or(0)) else {
                break;
            };
            *slot = birth;
            if slot.life <= 0.0 {
                push_dead(
                    &mut self.dead,
                    self.budget,
                    &mut self.head,
                    &mut self.dead_count,
                    index,
                );
                continue;
            }
            self.alive = self.alive.saturating_add(1);
            self.spawned = self.spawned.saturating_add(1);
            self.spawned_frame = self.spawned_frame.saturating_add(1);
        }
    }

    fn compact(&mut self, input: &StepInput) {
        self.positions.clear();
        self.fades.clear();
        for slot in &self.slots {
            if slot.life <= 0.0 {
                continue;
            }
            let fade = depth_fade(input, slot.position).max(0.0);
            self.positions.push(slot.position);
            self.fades.push(fade);
        }
        self.draw_instances = if input.skip_indirect {
            0
        } else {
            u32::try_from(self.positions.len()).unwrap_or(u32::MAX)
        };
    }
}

impl Default for CpuParticles {
    fn default() -> Self {
        Self::new()
    }
}

fn spawn_slot(input: &StepInput, rng: &mut u32) -> Slot {
    let u = f64::from(unit(rng));
    let v = f64::from(unit(rng));
    let angle = u * std::f64::consts::TAU;
    let radius = v.sqrt() * f64::from(input.spread);
    let dx = f64::from(unit(rng));
    let dz = f64::from(unit(rng));
    Slot {
        position: [
            f64::from(input.origin[0]) + angle.cos() * radius,
            f64::from(input.origin[1]),
            f64::from(input.origin[2]) + angle.sin() * radius,
        ],
        velocity: [
            (dx - 0.5) * f64::from(input.spread),
            f64::from(input.rise),
            (dz - 0.5) * f64::from(input.spread),
        ],
        age: 0.0,
        life: input.life.max(0.0),
    }
}

fn push_dead(dead: &mut [u32], budget: u32, head: &mut u32, dead_count: &mut u32, index: u32) {
    // A full ring has nowhere to write. Growing past the budget would alias a live index.
    if budget == 0 || *dead_count >= budget {
        return;
    }
    let tail = head.wrapping_add(*dead_count) % budget;
    if let Some(slot) = dead.get_mut(usize::try_from(tail).unwrap_or(0)) {
        *slot = index;
    }
    *dead_count = dead_count.saturating_add(1);
}

fn pop_dead(dead: &[u32], budget: u32, head: &mut u32, dead_count: &mut u32) -> Option<u32> {
    if budget == 0 || *dead_count == 0 {
        return None;
    }
    let index = *dead.get(usize::try_from(*head % budget).unwrap_or(0))?;
    *head = head.wrapping_add(1) % budget;
    *dead_count = dead_count.saturating_sub(1);
    Some(index)
}

/// Negative when the particle is behind the depth sample. Otherwise the fade, 0 to 1.
fn depth_fade(input: &StepInput, position: [f64; 3]) -> f64 {
    if input.depth_w == 0 || input.depth_h == 0 || input.depth.is_empty() {
        return 1.0;
    }
    let Some(ndc) = project(&input.view_proj, position) else {
        return 1.0;
    };
    let uv_x = ndc[0] * 0.5 + 0.5;
    let uv_y = ndc[1] * 0.5 + 0.5;
    if !(0.0..1.0).contains(&uv_x) || !(0.0..1.0).contains(&uv_y) {
        return 1.0;
    }
    let x = floor_index(uv_x * f64::from(input.depth_w), input.depth_w);
    let y = floor_index(uv_y * f64::from(input.depth_h), input.depth_h);
    let texel = usize::try_from(y)
        .unwrap_or(0)
        .saturating_mul(usize::try_from(input.depth_w).unwrap_or(0))
        .saturating_add(usize::try_from(x).unwrap_or(0));
    let Some(sample) = input.depth.get(texel).copied() else {
        return 1.0;
    };
    let delta = f64::from(sample) - ndc[2];
    if delta < 0.0 {
        return -1.0;
    }
    let band = if input.soft_band > 0.0 {
        f64::from(input.soft_band)
    } else {
        FALLBACK_BAND
    };
    (delta / band).clamp(0.0, 1.0)
}

fn floor_index(value: f64, limit: u32) -> u32 {
    if !value.is_finite() || value <= 0.0 || limit == 0 {
        return 0;
    }
    let last = limit - 1;
    let mut index = 0u32;
    while index < last && f64::from(index) + 1.0 <= value {
        index += 1;
    }
    index
}

fn project(matrix: &[f32; 16], position: [f64; 3]) -> Option<[f64; 3]> {
    if matrix.len() < 16 {
        return None;
    }
    let x = position[0];
    let y = position[1];
    let z = position[2];
    let clip_x = f64::from(matrix[0]) * x
        + f64::from(matrix[4]) * y
        + f64::from(matrix[8]) * z
        + f64::from(matrix[12]);
    let clip_y = f64::from(matrix[1]) * x
        + f64::from(matrix[5]) * y
        + f64::from(matrix[9]) * z
        + f64::from(matrix[13]);
    let clip_z = f64::from(matrix[2]) * x
        + f64::from(matrix[6]) * y
        + f64::from(matrix[10]) * z
        + f64::from(matrix[14]);
    let clip_w = f64::from(matrix[3]) * x
        + f64::from(matrix[7]) * y
        + f64::from(matrix[11]) * z
        + f64::from(matrix[15]);
    if clip_w.abs() < 1.0e-12 {
        return None;
    }
    Some([clip_x / clip_w, clip_y / clip_w, clip_z / clip_w])
}

/// Counters the debug page, the log, and `fx.stats` share.
///
/// `sort_ms` and `overdraw_px` stay 0: stage 1 does not sort and does not
/// count covered pixels. `sim_ms` and `draw_ms` are lavapipe timestamps when
/// the device wrote them, otherwise 0. They are not a 4070 measurement.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleStats {
    /// Compute integrate, emit, and compact, milliseconds.
    pub sim_ms: f64,
    /// Init, inside `sim_ms`.
    pub sim_init_ms: f64,
    /// Integrate and the dead-list scan, inside `sim_ms`.
    pub sim_death_ms: f64,
    /// Spawn, inside `sim_ms`.
    pub sim_emit_ms: f64,
    /// Alive scan and compact, inside `sim_ms`.
    pub sim_compact_ms: f64,
    /// Sort. Stage 1 does not dispatch it.
    pub sort_ms: f64,
    /// Indirect draw, milliseconds.
    pub draw_ms: f64,
    /// Alive after the step.
    pub alive: u32,
    /// Births since reset.
    pub spawned: u32,
    /// Covered pixels. Stage 1 leaves this 0.
    pub overdraw_px: u32,
    /// Particle buffers, bytes.
    pub vram_bytes: u64,
    /// [`vram_bytes`] in decimal megabytes.
    pub vram_mb: f64,
    /// Budget the buffers were sized for.
    pub max_particles: u32,
    /// Live cap.
    pub count: u32,
    /// Indirect instances. Zero when the draw was suppressed.
    pub draw_instances: u32,
}

impl ParticleStats {
    /// Probe before a frame has run. Times are 0. Bytes follow the budget when the emitter is on.
    #[must_use]
    pub fn from_settings(settings: &ParticleSettings) -> Self {
        let vram_bytes = if settings.dispatches() {
            buffer_bytes(settings.max_particles)
        } else {
            0
        };
        Self {
            sim_ms: 0.0,
            sim_init_ms: 0.0,
            sim_death_ms: 0.0,
            sim_emit_ms: 0.0,
            sim_compact_ms: 0.0,
            sort_ms: 0.0,
            draw_ms: 0.0,
            alive: 0,
            spawned: 0,
            overdraw_px: 0,
            vram_bytes,
            vram_mb: if settings.dispatches() {
                buffer_megabytes(settings.max_particles)
            } else {
                0.0
            },
            max_particles: settings.max_particles,
            count: settings.live_cap(),
            draw_instances: 0,
        }
    }

    /// One line. Names are the design's `fx.*` counters.
    #[must_use]
    pub fn line(self) -> String {
        format!(
            "fx.sim_ms={:.3} fx.sort_ms={:.3} fx.draw_ms={:.3} fx.alive={} fx.spawned={} fx.overdraw_px={} fx.vram_mb={:.3} fx.sim_init_ms={:.3} fx.sim_death_ms={:.3} fx.sim_emit_ms={:.3} fx.sim_compact_ms={:.3}",
            self.sim_ms,
            self.sort_ms,
            self.draw_ms,
            self.alive,
            self.spawned,
            self.overdraw_px,
            self.vram_mb,
            self.sim_init_ms,
            self.sim_death_ms,
            self.sim_emit_ms,
            self.sim_compact_ms,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn still(budget: u32, count: u32, life: f32) -> StepInput {
        StepInput {
            origin: [0.0, 1.0, 0.0],
            rate: 60.0,
            life,
            rise: 1.0,
            spread: 0.0,
            size: 0.4,
            albedo: [1.0, 0.2, 0.1],
            seed: DEFAULT_SEED,
            budget,
            count,
            dt: 1.0 / 60.0,
            freeze: false,
            depth_collision: false,
            skip_dead: false,
            skip_indirect: false,
            soft_band: 0.08,
            view_proj: identity(),
            depth_w: 0,
            depth_h: 0,
            depth: Vec::new(),
        }
    }

    fn identity() -> [f32; 16] {
        let mut matrix = [0.0; 16];
        matrix[0] = 1.0;
        matrix[5] = 1.0;
        matrix[10] = 1.0;
        matrix[15] = 1.0;
        matrix
    }

    #[test]
    fn the_low_budget_stays_under_sixteen_megabytes() {
        assert!(buffer_bytes(65_536) <= LOW_VRAM_BYTES);
        assert!(buffer_bytes(genos_scene::LOW_MAX_PARTICLES) <= LOW_VRAM_BYTES);
    }

    #[test]
    fn the_high_scan_tree_fits_the_buffer() {
        let budget = genos_scene::HIGH_MAX_PARTICLES;
        let wide = budget.div_ceil(64);
        let level1 = wide.div_ceil(64).max(1);
        let level2 = level1.div_ceil(64).max(1);
        assert!(level2 > 1, "the high budget must use the third scan level");
        let sum = budget.saturating_mul(2);
        let excl = sum.saturating_add(wide);
        let level1_sum = excl.saturating_add(wide);
        let level1_excl = level1_sum.saturating_add(level1);
        let level2_sum = level1_excl.saturating_add(level1);
        let level2_excl = level2_sum.saturating_add(level2);
        let end = u64::from(level2_excl.saturating_add(level2)).saturating_mul(4);
        assert!(
            end <= scan_buffer_bytes(budget),
            "scan writes {end} bytes past a {} byte buffer",
            scan_buffer_bytes(budget)
        );
    }

    #[test]
    fn the_same_seed_matches_and_the_cap_holds() {
        let mut a = CpuParticles::new();
        let mut b = CpuParticles::new();
        let mut input = still(8, 100, 1.0);
        input.spread = 0.4;
        for _ in 0..30 {
            a.step(&input);
            b.step(&input);
        }
        assert_eq!(a.positions(), b.positions());
        assert!(a.alive() <= 8, "{}", a.alive());
        assert_eq!(a.alive(), a.spawned() - a.died());
        let mut other = CpuParticles::new();
        let mut shifted = input.clone();
        shifted.seed = shifted.seed.wrapping_add(1);
        for _ in 0..30 {
            other.step(&shifted);
        }
        assert_ne!(a.positions(), other.positions());
    }

    #[test]
    fn dropping_dt_does_not_match_a_real_step() {
        let mut moved = CpuParticles::new();
        let mut frozen = CpuParticles::new();
        let input = still(4, 4, 2.0);
        let mut held = input.clone();
        held.dt = 0.0;
        for _ in 0..10 {
            moved.step(&input);
            frozen.step(&held);
        }
        assert!(moved.alive() > 0);
        assert_eq!(frozen.alive(), 0);
        assert_ne!(moved.positions(), frozen.positions());
        let mut nan = CpuParticles::new();
        let mut inf = CpuParticles::new();
        let mut bad = input.clone();
        bad.dt = f32::NAN;
        let mut huge = input.clone();
        huge.dt = f32::INFINITY;
        for _ in 0..10 {
            nan.step(&bad);
            inf.step(&huge);
        }
        assert_eq!(nan.alive(), 0, "NaN dt spawned");
        assert_eq!(inf.alive(), 0, "infinite dt spawned");
    }

    #[test]
    fn a_full_dead_ring_refuses_another_push() {
        let mut dead = [0u32, 1, 2, 3];
        let mut head = 0u32;
        let mut count = 4u32;
        push_dead(&mut dead, 4, &mut head, &mut count, 9);
        assert_eq!(count, 4);
        assert_eq!(dead, [0, 1, 2, 3]);
        assert_eq!(head, 0);
    }

    #[test]
    fn a_hundred_thousand_frames_do_not_leak_slots() {
        let mut sim = CpuParticles::new();
        let input = still(4, 4, 0.05);
        for _ in 0..100_000 {
            sim.step(&input);
            assert_eq!(sim.spawned() - sim.died(), sim.alive());
            assert!(sim.alive() <= 4);
        }
        assert!(sim.spawned() > 4);
    }

    #[test]
    fn skipping_the_dead_ring_stops_respawning() {
        let mut healthy = CpuParticles::new();
        let mut leaked = CpuParticles::new();
        let input = still(2, 2, 0.05);
        let mut broken = input.clone();
        broken.skip_dead = true;
        for _ in 0..40 {
            healthy.step(&input);
            leaked.step(&broken);
        }
        assert!(healthy.spawned() > leaked.spawned());
        assert_eq!(leaked.spawned(), 2);
    }

    #[test]
    fn depth_behind_the_sample_retires_and_a_near_one_fades() {
        let mut sim = CpuParticles::new();
        let mut input = still(1, 1, 10.0);
        input.spread = 0.0;
        input.rise = 0.0;
        input.origin = [0.0, 0.0, 0.9];
        input.depth_collision = true;
        input.depth_w = 1;
        input.depth_h = 1;
        input.depth = vec![0.2];
        input.soft_band = 0.1;
        sim.step(&input);
        assert_eq!(sim.alive(), 1);
        input.rate = 0.0;
        sim.step(&input);
        assert_eq!(sim.alive(), 0, "a particle at z 0.9 is behind depth 0.2");

        let mut near = CpuParticles::new();
        input.origin = [0.0, 0.0, 0.15];
        input.rate = 60.0;
        near.step(&input);
        input.rate = 0.0;
        near.step(&input);
        assert_eq!(near.alive(), 1);
        let fade = near.fades().first().copied().unwrap_or(0.0);
        assert!((fade - 0.5).abs() < 1.0e-6, "{fade}");
    }
}
