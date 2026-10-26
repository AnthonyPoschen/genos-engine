//! View-driven mip streaming.
//!
//! Each frame estimates the mip a texture needs from the screen-space
//! derivatives of its draws (Vulkan's rho and lambda = log2(rho)). Finer mips stream
//! in nearest and most visible first. The admit follows the measured disk,
//! decode, and transfer rates and backs off when the frame runs long. A page
//! flips on the frame after the submit, never in the submitting frame, and a
//! sample reads the finest coarser resident mip or the factor colour.
//!
//! Eviction runs only when texture bytes sit past the [VRAM budget](crate::vram).
//! The order is lowest screen importance, then farthest. A mip that just
//! arrived stays for the dwell unless the heap itself is over budget.

use std::cmp::Ordering;

use crate::vram::{retry_device_alloc, AllocError, Budget, Headroom};

/// Mips kept back from eviction after they become resident. Two levels is 4×.
pub const EVICT_LEVELS: u32 = 2;
/// Evictions started in one frame while the budget is over the target.
pub const EVICT_PER_FRAME: u32 = 4;
pub const DEFAULT_DWELL: u32 = 8;

/// Screen-space derivatives of a texture coordinate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Derivatives {
    pub dudx: f32,
    pub dvdx: f32,
    pub dudy: f32,
    pub dvdy: f32,
}

/// One texture's place in the frame.
#[derive(Clone, Copy, Debug)]
pub struct TextureView {
    pub index: usize,
    pub distance: f32,
    pub visible: bool,
    pub derivatives: Option<Derivatives>,
}

/// Measured throughput. The admit is the slowest of the three.
#[derive(Clone, Copy, Debug, Default)]
pub struct BusMeter {
    disk_bytes: u64,
    disk_ms: u32,
    decode_bytes: u64,
    decode_ms: u32,
    transfer_bytes: u64,
    transfer_ms: u32,
}

impl BusMeter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe_disk(&mut self, bytes: u64, ms: u32) {
        self.disk_bytes = self.disk_bytes.saturating_add(bytes);
        self.disk_ms = self.disk_ms.saturating_add(ms);
    }

    pub fn observe_decode(&mut self, bytes: u64, ms: u32) {
        self.decode_bytes = self.decode_bytes.saturating_add(bytes);
        self.decode_ms = self.decode_ms.saturating_add(ms);
    }

    pub fn observe_transfer(&mut self, bytes: u64, ms: u32) {
        self.transfer_bytes = self.transfer_bytes.saturating_add(bytes);
        self.transfer_ms = self.transfer_ms.saturating_add(ms);
    }

    /// Bytes per millisecond of the slowest path that has a sample.
    #[must_use]
    pub fn bytes_per_ms(self) -> u64 {
        let mut slowest = u64::MAX;
        let mut any = false;
        for (bytes, ms) in [
            (self.disk_bytes, self.disk_ms),
            (self.decode_bytes, self.decode_ms),
            (self.transfer_bytes, self.transfer_ms),
        ] {
            if ms == 0 {
                continue;
            }
            any = true;
            slowest = slowest.min(bytes / u64::from(ms));
        }
        if any {
            slowest.max(1)
        } else {
            1
        }
    }
}

/// What a sample is allowed to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sampled {
    /// Finest resident mip at or coarser than the demand. 0 is the full mip.
    Mip(u32),
    /// No mip is resident. The factor colour, not black.
    Factor,
}

struct Level {
    bytes: u64,
    resident: bool,
    inflight: bool,
    loaded_at: u64,
    evicted_at: u64,
}

struct Tex {
    width: u32,
    height: u32,
    levels: Vec<Level>,
    demand: u32,
    distance: f32,
    importance: f32,
    visible: bool,
}

struct Pending {
    index: usize,
    level: u32,
    bytes: u64,
    submitted: u64,
}

/// Faults a broken reader turns on, one scenario at a time.
#[derive(Clone, Copy, Debug, Default)]
struct Faults {
    bits: u8,
}

const FAULT_HIDE: u8 = 1;
const FAULT_QUALITY: u8 = 1 << 1;
const FAULT_OVERFLOW: u8 = 1 << 2;
const FAULT_SKIP_EVICT: u8 = 1 << 3;
const FAULT_DWELL: u8 = 1 << 4;
const FAULT_STALL: u8 = 1 << 5;
const FAULT_HOLE: u8 = 1 << 6;

impl Faults {
    fn has(self, bit: u8) -> bool {
        self.bits & bit != 0
    }

    fn set(&mut self, bit: u8) {
        self.bits |= bit;
    }
}

/// The resident set for one scene.
pub struct Streamer {
    textures: Vec<Tex>,
    pending: Vec<Pending>,
    frame: u64,
    committed: u64,
    quality_floor: u32,
    bias: f32,
    dwell: u32,
    rate: u64,
    frame_ms: f32,
    refused: u32,
    evictions: u32,
    thrash: u32,
    holes: u32,
    meter: BusMeter,
    faults: Faults,
    /// The transfer fence has signaled. The gate leaves this true.
    transfer_ready: bool,
}

impl Streamer {
    #[must_use]
    pub fn new(quality_floor: u32, bias: f32, dwell: u32) -> Self {
        Self {
            textures: Vec::new(),
            pending: Vec::new(),
            frame: 0,
            committed: 0,
            quality_floor,
            bias,
            dwell,
            rate: 0,
            frame_ms: 0.0,
            refused: 0,
            evictions: 0,
            thrash: 0,
            holes: 0,
            meter: BusMeter::new(),
            faults: Faults::default(),
            transfer_ready: true,
        }
    }

    /// The renderer clears this while a transfer fence is still pending.
    pub fn set_transfer_ready(&mut self, ready: bool) {
        self.transfer_ready = ready;
    }

    pub fn set_quality_floor(&mut self, floor: u32) {
        self.quality_floor = floor;
    }

    pub fn set_bias(&mut self, bias: f32) {
        self.bias = bias;
    }

    pub fn set_dwell(&mut self, dwell: u32) {
        self.dwell = dwell;
    }

    /// Bytes submitted and not yet flipped.
    #[must_use]
    pub fn inflight_bytes(&self) -> u64 {
        self.pending.iter().map(|pending| pending.bytes).sum()
    }

    /// `level_bytes` is finest first, down to the 1×1 mip.
    pub fn add_texture(&mut self, width: u32, height: u32, level_bytes: &[u64]) {
        let mut levels = Vec::with_capacity(level_bytes.len());
        for (index, bytes) in level_bytes.iter().copied().enumerate() {
            let last = index + 1 == level_bytes.len();
            if last {
                self.committed = self.committed.saturating_add(bytes);
            }
            levels.push(Level {
                bytes,
                resident: last,
                inflight: false,
                loaded_at: 0,
                evicted_at: u64::MAX,
            });
        }
        let demand = u32::try_from(levels.len().saturating_sub(1)).unwrap_or(0);
        self.textures.push(Tex {
            width: width.max(1),
            height: height.max(1),
            levels,
            demand,
            distance: 0.0,
            importance: 0.0,
            visible: false,
        });
    }

    pub fn observe_disk(&mut self, bytes: u64, ms: u32) {
        self.meter.observe_disk(bytes, ms);
    }

    pub fn observe_decode(&mut self, bytes: u64, ms: u32) {
        self.meter.observe_decode(bytes, ms);
    }

    pub fn observe_transfer(&mut self, bytes: u64, ms: u32) {
        self.meter.observe_transfer(bytes, ms);
    }

    #[must_use]
    pub fn committed(&self) -> u64 {
        self.committed
    }

    #[must_use]
    pub fn frame_ms(&self) -> f32 {
        self.frame_ms
    }

    #[must_use]
    pub fn rate(&self) -> u64 {
        self.rate
    }

    #[must_use]
    pub fn evictions(&self) -> u32 {
        self.evictions
    }

    #[must_use]
    pub fn thrash(&self) -> u32 {
        self.thrash
    }

    #[must_use]
    pub fn holes(&self) -> u32 {
        self.holes
    }

    #[must_use]
    pub fn refused(&self) -> u32 {
        self.refused
    }

    /// Finest resident mip, if any. Ignores a page that is still in flight.
    #[must_use]
    pub fn finest_resident(&self, index: usize) -> Option<u32> {
        let tex = self.textures.get(index)?;
        tex.levels
            .iter()
            .enumerate()
            .find(|(_, level)| level.resident)
            .map(|(level, _)| u32::try_from(level).unwrap_or(0))
    }

    /// One frame. Completes last frame's uploads, applies `views`, evicts
    /// under pressure, then admits new mips. Does not wait on the transfer.
    pub fn step(
        &mut self,
        budget: &Budget,
        views: &[TextureView],
        base_ms: f32,
        frame_budget_ms: f32,
    ) {
        self.frame = self.frame.saturating_add(1);
        self.complete_uploads();
        self.apply_views(views);
        if self.faults.has(FAULT_HIDE) {
            self.evict_hidden();
        }
        if !self.faults.has(FAULT_SKIP_EVICT) {
            self.evict_pressure(budget);
        }
        let demand = self.bytes_wanted();
        let admit = self.admit(base_ms, frame_budget_ms, demand);
        self.load(budget, admit);
        self.count_sample_holes();
    }

    /// Drop the lowest-priority mip. Used when a texture allocation returns
    /// out-of-device-memory. Returns false when nothing is left to drop.
    pub fn evict_for_oom(&mut self) -> bool {
        if self.faults.has(FAULT_HOLE) {
            self.holes = self.holes.saturating_add(1);
            return false;
        }
        self.evict_one(true)
    }

    fn apply_views(&mut self, views: &[TextureView]) {
        for view in views {
            let Some(tex) = self.textures.get_mut(view.index) else {
                continue;
            };
            tex.distance = view.distance;
            tex.visible = view.visible;
            if !view.visible || view.derivatives.is_none() {
                tex.importance = 0.0;
                continue;
            }
            let Some(derivatives) = view.derivatives else {
                continue;
            };
            let floor = if self.faults.has(FAULT_QUALITY) {
                0
            } else {
                self.quality_floor
            };
            let levels = u32::try_from(tex.levels.len()).unwrap_or(0);
            tex.demand = wanted_mip(tex.width, tex.height, derivatives, self.bias, floor, levels);
            let rho = texel_rho(tex.width, tex.height, derivatives);
            tex.importance = if rho > 0.0 { 1.0 / rho } else { 1.0e6 };
        }
    }

    fn complete_uploads(&mut self) {
        if !self.transfer_ready {
            return;
        }
        let frame = self.frame;
        let dwell = u64::from(self.dwell);
        let mut done = Vec::new();
        self.pending.retain(|pending| {
            if pending.submitted < frame {
                done.push((pending.index, pending.level, pending.bytes));
                false
            } else {
                true
            }
        });
        for (index, level, bytes) in done {
            let Some(tex) = self.textures.get_mut(index) else {
                continue;
            };
            let Some(mip) = tex.levels.get_mut(usize::try_from(level).unwrap_or(0)) else {
                continue;
            };
            mip.inflight = false;
            mip.resident = true;
            mip.loaded_at = frame;
            if mip.evicted_at != u64::MAX && frame.saturating_sub(mip.evicted_at) < dwell {
                self.thrash = self.thrash.saturating_add(1);
            }
            self.meter.observe_transfer(bytes, 1);
        }
    }

    fn evict_hidden(&mut self) {
        let mut victims = Vec::new();
        for (index, tex) in self.textures.iter().enumerate() {
            if tex.visible {
                continue;
            }
            let last = tex.levels.len().saturating_sub(1);
            for (level, mip) in tex.levels.iter().enumerate() {
                if mip.resident && level != last {
                    victims.push((index, u32::try_from(level).unwrap_or(0)));
                }
            }
        }
        for (index, level) in victims {
            self.drop_level(index, level);
        }
    }

    fn evict_pressure(&mut self, budget: &Budget) {
        let mut left = EVICT_PER_FRAME;
        while self.committed > budget.texture_target() && left > 0 {
            let critical = budget.critical(self.committed);
            if !self.evict_one(critical) {
                break;
            }
            left -= 1;
        }
    }

    fn evict_one(&mut self, critical: bool) -> bool {
        let Some((index, level)) = self.pick_victim(critical) else {
            return false;
        };
        self.drop_level(index, level);
        true
    }

    fn pick_victim(&self, critical: bool) -> Option<(usize, u32)> {
        let mut best: Option<(usize, u32, f32, f32)> = None;
        for (index, tex) in self.textures.iter().enumerate() {
            let last = tex.levels.len().saturating_sub(1);
            for (level, mip) in tex.levels.iter().enumerate() {
                if !mip.resident || mip.inflight {
                    continue;
                }
                let level_u = u32::try_from(level).unwrap_or(0);
                let coarsest = level == last;
                if coarsest && !critical {
                    continue;
                }
                if !critical && !self.faults.has(FAULT_DWELL) {
                    let age = self.frame.saturating_sub(mip.loaded_at);
                    if age < u64::from(self.dwell) {
                        continue;
                    }
                    if tex.demand < level_u.saturating_add(EVICT_LEVELS) {
                        continue;
                    }
                }
                let replace = match best {
                    None => true,
                    Some((_, best_level, importance, distance)) => {
                        let importance_cmp = tex.importance.total_cmp(&importance);
                        let distance_cmp = tex.distance.total_cmp(&distance);
                        importance_cmp == Ordering::Less
                            || (importance_cmp == Ordering::Equal
                                && distance_cmp == Ordering::Greater)
                            || (importance_cmp == Ordering::Equal
                                && distance_cmp == Ordering::Equal
                                && level_u < best_level)
                    }
                };
                if replace {
                    best = Some((index, level_u, tex.importance, tex.distance));
                }
            }
        }
        best.map(|(index, level, _, _)| (index, level))
    }

    fn drop_level(&mut self, index: usize, level: u32) {
        let Some(tex) = self.textures.get_mut(index) else {
            return;
        };
        let Some(mip) = tex.levels.get_mut(usize::try_from(level).unwrap_or(0)) else {
            return;
        };
        if !mip.resident {
            return;
        }
        mip.resident = false;
        mip.evicted_at = self.frame;
        self.committed = self.committed.saturating_sub(mip.bytes);
        self.evictions = self.evictions.saturating_add(1);
    }

    fn bytes_wanted(&self) -> u64 {
        let mut sum = 0u64;
        for tex in &self.textures {
            let have = committed_finest(tex);
            if have <= tex.demand {
                continue;
            }
            let next = usize::try_from(have - 1).unwrap_or(0);
            if let Some(level) = tex.levels.get(next) {
                if !level.resident && !level.inflight {
                    sum = sum.saturating_add(level.bytes);
                }
            }
        }
        sum
    }

    fn admit(&mut self, base_ms: f32, frame_budget_ms: f32, demand: u64) -> u64 {
        let per = self.meter.bytes_per_ms().max(1);
        let leftover = frame_budget_ms - base_ms;
        let leftover_ms = floor_nonneg(if leftover > 0.0 { leftover } else { 0.0 });
        let sustainable = per.saturating_mul(u64::from(leftover_ms));
        if self.faults.has(FAULT_STALL) {
            let extra = demand / per;
            let extra_ms = u32::try_from(extra).unwrap_or(u32::MAX);
            self.frame_ms = base_ms + u32_to_f32(extra_ms);
            self.rate = demand;
            return demand;
        }
        self.frame_ms = base_ms;
        if base_ms > frame_budget_ms {
            self.rate = (self.rate / 2).max(1);
        } else if self.rate == 0 {
            self.rate = (sustainable / 4).max(1);
        } else if base_ms * 4.0 < frame_budget_ms * 3.0 {
            let grown = self.rate.saturating_mul(3) / 2;
            let cap = sustainable.max(1);
            self.rate = grown.min(cap).max(1);
        }
        self.rate.min(sustainable)
    }

    fn load(&mut self, budget: &Budget, mut admit: u64) {
        if !self.transfer_ready && !self.pending.is_empty() {
            return;
        }
        loop {
            let mut order: Vec<usize> = (0..self.textures.len()).collect();
            order.sort_by(|&a, &b| {
                let left = &self.textures[a];
                let right = &self.textures[b];
                left.distance
                    .total_cmp(&right.distance)
                    .then(right.importance.total_cmp(&left.importance))
            });
            let mut progress = false;
            for index in order {
                let have = committed_finest(&self.textures[index]);
                let demand = self.textures[index].demand;
                if have <= demand {
                    continue;
                }
                let next = have - 1;
                let bytes = self.textures[index].levels[usize::try_from(next).unwrap_or(0)].bytes;
                if bytes > admit {
                    continue;
                }
                if !budget.admits(self.committed, bytes) && !self.faults.has(FAULT_OVERFLOW) {
                    self.refused = self.refused.saturating_add(1);
                    continue;
                }
                self.textures[index].levels[usize::try_from(next).unwrap_or(0)].inflight = true;
                self.committed = self.committed.saturating_add(bytes);
                self.pending.push(Pending {
                    index,
                    level: next,
                    bytes,
                    submitted: self.frame,
                });
                admit = admit.saturating_sub(bytes);
                progress = true;
            }
            if !progress {
                break;
            }
        }
    }

    fn count_sample_holes(&mut self) {
        let count = self.textures.len();
        for index in 0..count {
            let demand = self.textures[index].demand;
            if let Sampled::Mip(level) = self.sample(index, demand) {
                let resident =
                    self.textures[index].levels[usize::try_from(level).unwrap_or(0)].resident;
                let inflight =
                    self.textures[index].levels[usize::try_from(level).unwrap_or(0)].inflight;
                if !resident || inflight {
                    self.holes = self.holes.saturating_add(1);
                }
            }
        }
    }

    /// The miss rule. An in-flight page is not visible yet.
    #[must_use]
    pub fn sample(&self, index: usize, wanted: u32) -> Sampled {
        let Some(tex) = self.textures.get(index) else {
            return Sampled::Factor;
        };
        let start = usize::try_from(wanted).unwrap_or(0);
        for (level, mip) in tex.levels.iter().enumerate().skip(start) {
            if mip.resident && !mip.inflight {
                return Sampled::Mip(u32::try_from(level).unwrap_or(0));
            }
        }
        Sampled::Factor
    }
}

fn committed_finest(tex: &Tex) -> u32 {
    tex.levels
        .iter()
        .enumerate()
        .find(|(_, level)| level.resident || level.inflight)
        .map_or(
            u32::try_from(tex.levels.len().saturating_sub(1)).unwrap_or(0),
            |(level, _)| u32::try_from(level).unwrap_or(0),
        )
}

/// ρ from the Vulkan scale-factor operation: the longer screen-space edge of a texel.
#[must_use]
pub fn texel_rho(width: u32, height: u32, derivatives: Derivatives) -> f32 {
    let w = u32_to_f32(width.max(1));
    let h = u32_to_f32(height.max(1));
    let along_x = (derivatives.dudx * w).hypot(derivatives.dvdx * h);
    let along_y = (derivatives.dudy * w).hypot(derivatives.dvdy * h);
    along_x.max(along_y)
}

/// Wanted mip. `floor` is the sharpest mip `texture_quality` allows.
#[must_use]
pub fn wanted_mip(
    width: u32,
    height: u32,
    derivatives: Derivatives,
    bias: f32,
    floor: u32,
    levels: u32,
) -> u32 {
    if levels == 0 {
        return 0;
    }
    let rho = texel_rho(width, height, derivatives);
    let lambda = if rho > 0.0 { rho.log2() + bias } else { 0.0 };
    let estimated = floor_nonneg(lambda);
    let last = levels.saturating_sub(1);
    estimated.max(floor).min(last)
}

fn u32_to_f32(value: u32) -> f32 {
    let mut out = 0.0f32;
    let mut bit = 1.0f32;
    let mut left = value;
    while left > 0 {
        if left & 1 == 1 {
            out += bit;
        }
        left >>= 1;
        bit *= 2.0;
    }
    out
}

fn floor_nonneg(value: f32) -> u32 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    let mut level = 0u32;
    while level < 31 && u32_to_f32(level) + 1.0 <= value {
        level += 1;
    }
    level
}

fn mb(n: u64) -> u64 {
    n * 1024 * 1024
}

fn budget_of(heap_mb: u64) -> Budget {
    Budget::from_heaps(
        &[crate::vram::HeapSlice {
            budget: mb(heap_mb),
            usage: 0,
            device_local: true,
        }],
        0,
        Headroom::default(),
        true,
    )
}

fn fast(stream: &mut Streamer) {
    let bytes = mb(64);
    stream.observe_disk(bytes, 1);
    stream.observe_decode(bytes, 1);
    stream.observe_transfer(bytes, 1);
}

/// Enough throughput to fill every mip before a simulated shrink.
fn flood(stream: &mut Streamer) {
    let bytes = mb(4096);
    stream.observe_disk(bytes, 1);
    stream.observe_decode(bytes, 1);
    stream.observe_transfer(bytes, 1);
}

fn near_derivatives(width: u32) -> Derivatives {
    let step = 0.25 / u32_to_f32(width.max(1));
    Derivatives {
        dudx: step,
        dvdx: 0.0,
        dudy: 0.0,
        dvdy: 0.0,
    }
}

fn far_derivatives(width: u32) -> Derivatives {
    let step = 8.0 / u32_to_f32(width.max(1));
    Derivatives {
        dudx: step,
        dvdx: 0.0,
        dudy: 0.0,
        dvdy: 0.0,
    }
}

const BASE_MS: f32 = 8.0;
const FRAME_BUDGET_MS: f32 = 16.0;

/// Frames until a shrink sits under the target. The broken reader does not evict.
#[must_use]
pub fn budget_recover(live: bool) -> f32 {
    let mut stream = Streamer::new(0, 0.0, DEFAULT_DWELL);
    if !live {
        stream.faults.set(FAULT_SKIP_EVICT);
    }
    for _ in 0..4 {
        stream.add_texture(1024, 1024, &[mb(512), mb(128), mb(32)]);
    }
    flood(&mut stream);
    let wide = budget_of(4096);
    let views: Vec<TextureView> = (0..4)
        .map(|index| TextureView {
            index,
            distance: u32_to_f32(u32::try_from(index).unwrap_or(0)),
            visible: true,
            derivatives: Some(near_derivatives(1024)),
        })
        .collect();
    stream.step(&wide, &views, BASE_MS, FRAME_BUDGET_MS);
    stream.step(&wide, &views, BASE_MS, FRAME_BUDGET_MS);
    let tight = budget_of(600);
    let mut frames = 0u32;
    while frames < 6 {
        frames += 1;
        stream.step(&tight, &views, BASE_MS, FRAME_BUDGET_MS);
        if stream.committed() <= tight.texture_target() {
            return u32_to_f32(frames);
        }
    }
    u32_to_f32(frames.saturating_add(1))
}

/// 0 when an overflowing upload is refused.
#[must_use]
pub fn upload_refused(live: bool) -> f32 {
    let mut stream = Streamer::new(0, 0.0, DEFAULT_DWELL);
    if !live {
        stream.faults.set(FAULT_OVERFLOW);
    }
    stream.add_texture(1024, 1024, &[mb(200), mb(10)]);
    flood(&mut stream);
    let budget = budget_of(400);
    let views = [TextureView {
        index: 0,
        distance: 1.0,
        visible: true,
        derivatives: Some(near_derivatives(1024)),
    }];
    stream.step(&budget, &views, BASE_MS, FRAME_BUDGET_MS);
    let over = stream.committed() > budget.texture_target();
    if live {
        if stream.refused() > 0 && !over {
            0.0
        } else {
            1.0
        }
    } else if over {
        1.0
    } else {
        0.0
    }
}

/// 0 when a device OOM evicts, retries, and leaves no hole.
#[must_use]
pub fn oom_recover(live: bool) -> f32 {
    let mut stream = Streamer::new(0, 0.0, DEFAULT_DWELL);
    stream.add_texture(64, 64, &[100, 10]);
    stream.textures[0].levels[0].resident = true;
    stream.textures[0].levels[0].loaded_at = 0;
    stream.committed = stream.committed.saturating_add(100);
    stream.frame = u64::from(DEFAULT_DWELL) + 1;
    if !live {
        stream.faults.set(FAULT_HOLE);
    }
    let mut attempts = 0u32;
    let result = retry_device_alloc(
        || {
            attempts += 1;
            if attempts == 1 {
                Err(AllocError::Oom)
            } else {
                Ok(())
            }
        },
        || stream.evict_for_oom(),
        4,
    );
    let sampled = stream.sample(0, 0);
    let hole = stream.holes() > 0
        || matches!(sampled, Sampled::Mip(level) if stream.textures[0].levels[usize::try_from(level).unwrap_or(0)].inflight);
    if live && result.is_ok() && !hole && matches!(sampled, Sampled::Mip(_)) {
        0.0
    } else {
        1.0
    }
}

/// Evictions during a spin while the budget has room.
#[must_use]
pub fn spin_evict(live: bool) -> f32 {
    let mut stream = Streamer::new(0, 0.0, DEFAULT_DWELL);
    if !live {
        stream.faults.set(FAULT_HIDE);
    }
    stream.add_texture(256, 256, &[1000, 100]);
    stream.add_texture(256, 256, &[1000, 100]);
    fast(&mut stream);
    let budget = budget_of(8192);
    let both = |visible_first: bool| {
        [
            TextureView {
                index: 0,
                distance: 2.0,
                visible: visible_first,
                derivatives: visible_first.then(|| near_derivatives(256)),
            },
            TextureView {
                index: 1,
                distance: 2.0,
                visible: !visible_first,
                derivatives: (!visible_first).then(|| near_derivatives(256)),
            },
        ]
    };
    stream.step(&budget, &both(true), BASE_MS, FRAME_BUDGET_MS);
    stream.step(&budget, &both(true), BASE_MS, FRAME_BUDGET_MS);
    let before = stream.evictions();
    for turn in 0..4 {
        stream.step(&budget, &both(turn % 2 == 1), BASE_MS, FRAME_BUDGET_MS);
    }
    u32_to_f32(stream.evictions().saturating_sub(before))
}

/// Resident mips finer than `texture_quality`.
#[must_use]
pub fn quality_cap(live: bool) -> f32 {
    let mut stream = Streamer::new(2, 0.0, DEFAULT_DWELL);
    if !live {
        stream.faults.set(FAULT_QUALITY);
    }
    stream.add_texture(512, 512, &[8, 4, 2, 1, 1]);
    fast(&mut stream);
    let budget = budget_of(8192);
    let views = [TextureView {
        index: 0,
        distance: 1.0,
        visible: true,
        derivatives: Some(near_derivatives(512)),
    }];
    for _ in 0..4 {
        stream.step(&budget, &views, BASE_MS, FRAME_BUDGET_MS);
    }
    let over = stream.textures[0]
        .levels
        .iter()
        .enumerate()
        .filter(|(level, mip)| *level < 2 && (mip.resident || mip.inflight))
        .count();
    u32::try_from(over).map_or(1.0, u32_to_f32)
}

/// Ultra on a small heap and a fast bus: the near mip is full, the far one is not.
#[must_use]
pub fn ultra_near(live: bool) -> f32 {
    let mut stream = Streamer::new(0, 0.0, DEFAULT_DWELL);
    if !live {
        stream.quality_floor = 2;
    }
    stream.add_texture(1024, 1024, &[mb(32), mb(8), mb(2)]);
    stream.add_texture(1024, 1024, &[mb(32), mb(8), mb(2)]);
    fast(&mut stream);
    let budget = budget_of(300);
    let views = [
        TextureView {
            index: 0,
            distance: 1.0,
            visible: true,
            derivatives: Some(near_derivatives(1024)),
        },
        TextureView {
            index: 1,
            distance: 40.0,
            visible: true,
            derivatives: Some(far_derivatives(1024)),
        },
    ];
    for _ in 0..4 {
        stream.step(&budget, &views, BASE_MS, FRAME_BUDGET_MS);
    }
    let near = stream.finest_resident(0);
    let far = stream.finest_resident(1);
    let inside = stream.committed() <= budget.texture_target();
    if near == Some(0) && far.is_some_and(|level| level >= 2) && inside {
        0.0
    } else {
        1.0
    }
}

/// A slow bus lowers the admit. The frame time stays on the graphics budget.
#[must_use]
pub fn slow_bus(live: bool) -> f32 {
    let mut slow = Streamer::new(0, 0.0, DEFAULT_DWELL);
    let mut fast_stream = Streamer::new(0, 0.0, DEFAULT_DWELL);
    if !live {
        slow.faults.set(FAULT_STALL);
    }
    slow.add_texture(4096, 4096, &[1_000_000, 1]);
    fast_stream.add_texture(4096, 4096, &[1_000_000, 1]);
    slow.observe_disk(1_000, 1);
    slow.observe_decode(1_000, 1);
    slow.observe_transfer(1_000, 1);
    let fast_bytes = 1_000_000u64;
    fast_stream.observe_disk(fast_bytes, 1);
    fast_stream.observe_decode(fast_bytes, 1);
    fast_stream.observe_transfer(fast_bytes, 1);
    let budget = budget_of(8192);
    let views = [TextureView {
        index: 0,
        distance: 1.0,
        visible: true,
        derivatives: Some(near_derivatives(4096)),
    }];
    slow.step(&budget, &views, BASE_MS, FRAME_BUDGET_MS);
    fast_stream.step(&budget, &views, BASE_MS, FRAME_BUDGET_MS);
    if slow.rate() < fast_stream.rate() && slow.frame_ms() <= FRAME_BUDGET_MS {
        0.0
    } else {
        slow.frame_ms()
    }
}

/// Mips evicted and loaded again inside the dwell, during a spin.
#[must_use]
pub fn spin_thrash(live: bool) -> f32 {
    let mut stream = Streamer::new(0, 0.0, DEFAULT_DWELL);
    if !live {
        stream.faults.set(FAULT_HIDE);
        stream.faults.set(FAULT_DWELL);
    }
    stream.add_texture(1024, 1024, &[mb(30), mb(2)]);
    stream.add_texture(1024, 1024, &[mb(30), mb(2)]);
    fast(&mut stream);
    let budget = budget_of(300);
    let look = |at_first: bool| {
        [
            TextureView {
                index: 0,
                distance: if at_first { 1.0 } else { 30.0 },
                visible: at_first,
                derivatives: at_first.then(|| near_derivatives(1024)),
            },
            TextureView {
                index: 1,
                distance: if at_first { 30.0 } else { 1.0 },
                visible: !at_first,
                derivatives: if at_first {
                    Some(far_derivatives(1024))
                } else {
                    Some(near_derivatives(1024))
                },
            },
        ]
    };
    stream.step(&budget, &look(true), BASE_MS, FRAME_BUDGET_MS);
    stream.step(&budget, &look(true), BASE_MS, FRAME_BUDGET_MS);
    stream.step(&budget, &look(false), BASE_MS, FRAME_BUDGET_MS);
    stream.step(&budget, &look(false), BASE_MS, FRAME_BUDGET_MS);
    stream.step(&budget, &look(true), BASE_MS, FRAME_BUDGET_MS);
    stream.step(&budget, &look(true), BASE_MS, FRAME_BUDGET_MS);
    u32_to_f32(stream.thrash())
}

/// The eight gate rows. `live` is the engine. The other side is the broken reader.
#[derive(Clone, Copy, Debug)]
pub struct GateRows {
    pub budget_recover: f32,
    pub upload_refused: f32,
    pub oom_recover: f32,
    pub spin_evict: f32,
    pub quality_cap: f32,
    pub ultra_near: f32,
    pub slow_bus: f32,
    pub thrash: f32,
}

#[must_use]
pub fn gate_rows(live: bool) -> GateRows {
    GateRows {
        budget_recover: budget_recover(live),
        upload_refused: upload_refused(live),
        oom_recover: oom_recover(live),
        spin_evict: spin_evict(live),
        quality_cap: quality_cap(live),
        ultra_near: ultra_near(live),
        slow_bus: slow_bus(live),
        thrash: spin_thrash(live),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_rows_pass_live_and_fail_broken() {
        let live = gate_rows(true);
        let broken = gate_rows(false);
        assert!(
            live.budget_recover.to_bits() <= 6.0f32.to_bits(),
            "{}",
            live.budget_recover
        );
        assert!(
            broken.budget_recover.to_bits() > 6.0f32.to_bits(),
            "{}",
            broken.budget_recover
        );
        assert_eq!(live.upload_refused.to_bits(), 0.0f32.to_bits());
        assert!(broken.upload_refused.to_bits() > 0);
        assert_eq!(live.oom_recover.to_bits(), 0.0f32.to_bits());
        assert!(broken.oom_recover.to_bits() > 0);
        assert_eq!(live.spin_evict.to_bits(), 0.0f32.to_bits());
        assert!(broken.spin_evict.to_bits() > 0);
        assert_eq!(live.quality_cap.to_bits(), 0.0f32.to_bits());
        assert!(broken.quality_cap.to_bits() > 0);
        assert_eq!(live.ultra_near.to_bits(), 0.0f32.to_bits());
        assert!(broken.ultra_near.to_bits() > 0);
        assert_eq!(live.slow_bus.to_bits(), 0.0f32.to_bits());
        assert!(broken.slow_bus.to_bits() > 0);
        assert_eq!(live.thrash.to_bits(), 0.0f32.to_bits());
        assert!(broken.thrash.to_bits() > 0);
    }

    #[test]
    fn a_magnified_texel_wants_mip_zero_and_a_tiny_one_does_not() {
        let near = wanted_mip(1024, 1024, near_derivatives(1024), 0.0, 0, 8);
        let far = wanted_mip(1024, 1024, far_derivatives(1024), 0.0, 0, 8);
        assert_eq!(near, 0);
        assert!(far >= 3, "{far}");
        let capped = wanted_mip(1024, 1024, near_derivatives(1024), 0.0, 2, 8);
        assert_eq!(capped, 2);
    }
}
