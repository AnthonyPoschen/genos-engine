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

/// One mip the transfer queue should copy this submit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingCopy {
    pub index: usize,
    pub level: u32,
    pub bytes: u64,
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
    /// Bytes `admit` handed to `load` on the last step.
    last_admit: u64,
    frame_ms: f32,
    refused: u32,
    evictions: u32,
    thrash: u32,
    holes: u32,
    /// Victims in eviction order: texture index, then mip level.
    evicted: Vec<(usize, u32)>,
    meter: BusMeter,
    /// The transfer fence has signaled. Completions wait on this.
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
            last_admit: 0,
            frame_ms: 0.0,
            refused: 0,
            evictions: 0,
            thrash: 0,
            holes: 0,
            evicted: Vec::new(),
            meter: BusMeter::new(),
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

    /// Drop every texture. The meter and the counters stay.
    pub fn clear(&mut self) {
        self.textures.clear();
        self.pending.clear();
        self.committed = 0;
        self.evicted.clear();
    }

    /// Bytes per millisecond of the slowest measured path.
    #[must_use]
    pub fn bytes_per_ms(&self) -> u64 {
        self.meter.bytes_per_ms()
    }

    /// Bytes admitted on the last step, after the sustainable cap.
    #[must_use]
    pub fn last_admit(&self) -> u64 {
        self.last_admit
    }

    /// Eviction victims in the order they were dropped.
    #[must_use]
    pub fn evicted(&self) -> &[(usize, u32)] {
        &self.evicted
    }

    #[must_use]
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    #[must_use]
    pub fn level_count(&self, index: usize) -> u32 {
        self.textures
            .get(index)
            .map_or(0, |tex| u32::try_from(tex.levels.len()).unwrap_or(0))
    }

    /// True when the mip is sampled. An in-flight page is not.
    #[must_use]
    pub fn is_resident(&self, index: usize, level: u32) -> bool {
        let Some(slot) = usize::try_from(level).ok() else {
            return false;
        };
        self.textures
            .get(index)
            .and_then(|tex| tex.levels.get(slot))
            .is_some_and(|mip| mip.resident && !mip.inflight)
    }

    /// True when the mip occupies device memory: resident or in flight.
    #[must_use]
    pub fn holds_memory(&self, index: usize, level: u32) -> bool {
        let Some(slot) = usize::try_from(level).ok() else {
            return false;
        };
        self.textures
            .get(index)
            .and_then(|tex| tex.levels.get(slot))
            .is_some_and(|mip| mip.resident || mip.inflight)
    }

    /// Pages the transfer queue still has to copy.
    #[must_use]
    pub fn pending_copies(&self) -> Vec<PendingCopy> {
        self.pending
            .iter()
            .map(|pending| PendingCopy {
                index: pending.index,
                level: pending.level,
                bytes: pending.bytes,
            })
            .collect()
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
        self.evict_pressure(budget);
        let admit = self.admit(base_ms, frame_budget_ms);
        self.load(budget, admit);
        self.count_sample_holes();
    }

    /// Drop the lowest-priority mip. Used when a texture allocation returns
    /// out-of-device-memory. Returns false when nothing is left to drop.
    pub fn evict_for_oom(&mut self) -> bool {
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
            let levels = u32::try_from(tex.levels.len()).unwrap_or(0);
            tex.demand = wanted_mip(
                tex.width,
                tex.height,
                derivatives,
                self.bias,
                self.quality_floor,
                levels,
            );
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
        for (index, level, _bytes) in done {
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
                if !critical {
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
        self.evicted.push((index, level));
    }

    fn admit(&mut self, base_ms: f32, frame_budget_ms: f32) -> u64 {
        let per = self.meter.bytes_per_ms().max(1);
        let leftover = frame_budget_ms - base_ms;
        let leftover_ms = floor_nonneg(if leftover > 0.0 { leftover } else { 0.0 });
        let sustainable = per.saturating_mul(u64::from(leftover_ms));
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
        let admit = self.rate.min(sustainable);
        self.last_admit = admit;
        admit
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
                if !budget.admits(self.committed, bytes) {
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

/// Frames until a shrink sits under the target. Staying over the line is a miss.
#[must_use]
pub fn budget_recover() -> f32 {
    let mut stream = Streamer::new(0, 0.0, DEFAULT_DWELL);
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
pub fn upload_refused() -> f32 {
    let mut stream = Streamer::new(0, 0.0, DEFAULT_DWELL);
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
    if stream.refused() > 0 && !over {
        0.0
    } else {
        1.0
    }
}

/// 0 when a device OOM evicts, retries, and leaves no hole.
#[must_use]
pub fn oom_recover() -> f32 {
    let mut stream = Streamer::new(0, 0.0, DEFAULT_DWELL);
    stream.add_texture(64, 64, &[100, 10]);
    stream.textures[0].levels[0].resident = true;
    stream.textures[0].levels[0].loaded_at = 0;
    stream.committed = stream.committed.saturating_add(100);
    stream.frame = u64::from(DEFAULT_DWELL) + 1;
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
    if result.is_ok() && !hole && matches!(sampled, Sampled::Mip(_)) {
        0.0
    } else {
        1.0
    }
}

/// Evictions during a spin while the budget has room.
#[must_use]
pub fn spin_evict() -> f32 {
    let mut stream = Streamer::new(0, 0.0, DEFAULT_DWELL);
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

/// Resident or in-flight mips finer than `texture_quality`.
#[must_use]
pub fn quality_cap() -> f32 {
    let floor = 2u32;
    let mut stream = Streamer::new(floor, 0.0, DEFAULT_DWELL);
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
        .filter(|(level, mip)| {
            u32::try_from(*level).unwrap_or(floor) < floor && (mip.resident || mip.inflight)
        })
        .count();
    u32::try_from(over).map_or(1.0, u32_to_f32)
}

/// Ultra on a small heap and a fast bus: the near mip is full, the far one is not.
#[must_use]
pub fn ultra_near() -> f32 {
    let mut stream = Streamer::new(0, 0.0, DEFAULT_DWELL);
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

/// Overrun halves the rate, headroom grows it, and the admit stays under
/// `bytes_per_ms * leftover_ms` computed from the meter, not from `rate`.
#[must_use]
pub fn slow_bus() -> f32 {
    let mut stream = Streamer::new(0, 0.0, 0);
    stream.add_texture(64, 64, &[4_000, 10]);
    let per = 1_000u64;
    stream.observe_disk(per, 1);
    stream.observe_decode(per, 1);
    stream.observe_transfer(per, 1);
    let budget = budget_of(8192);
    let views = [view_at(0, 1.0, near_derivatives(64))];
    let mut faults = 0u32;
    stream.step(&budget, &views, BASE_MS, FRAME_BUDGET_MS);
    note_admit(&stream, BASE_MS, &mut faults);
    stream.step(&budget, &views, BASE_MS, FRAME_BUDGET_MS);
    note_admit(&stream, BASE_MS, &mut faults);
    let established = stream.rate();
    if established <= 1 {
        return 4.0;
    }
    let overrun = FRAME_BUDGET_MS + 4.0;
    stream.step(&budget, &views, overrun, FRAME_BUDGET_MS);
    note_admit(&stream, overrun, &mut faults);
    let once = stream.rate();
    if once != established / 2 {
        faults = faults.saturating_add(1);
    }
    stream.step(&budget, &views, overrun, FRAME_BUDGET_MS);
    note_admit(&stream, overrun, &mut faults);
    let halved = stream.rate();
    if halved != once / 2 {
        faults = faults.saturating_add(1);
    }
    stream.step(&budget, &views, BASE_MS, FRAME_BUDGET_MS);
    note_admit(&stream, BASE_MS, &mut faults);
    let recovered = stream.rate();
    if recovered <= halved || recovered >= established {
        faults = faults.saturating_add(1);
    }
    u32_to_f32(faults)
}

/// Thrash count for an A/B spin that only the dwell can hold still.
///
/// Texture 0's finest mip is the only page the far view can drop, and it is
/// smaller than the overshoot, so that drop leaves the heap over the target.
/// Looking back makes texture 1's larger finest the victim, which frees enough
/// room for texture 0's finest to be admitted again inside the dwell.
#[must_use]
pub fn spin_thrash() -> f32 {
    let mut stream = Streamer::new(0, 0.0, DEFAULT_DWELL);
    stream.add_texture(64, 64, &[8, 4, 1]);
    stream.add_texture(64, 64, &[40, 4, 1]);
    flood(&mut stream);
    let wide = byte_budget(1_000, 0);
    let near = [
        view_at(0, 1.0, near_derivatives(64)),
        view_at(1, 1.0, near_derivatives(64)),
    ];
    stream.step(&wide, &near, BASE_MS, FRAME_BUDGET_MS);
    stream.step(&wide, &near, BASE_MS, FRAME_BUDGET_MS);
    let tight = byte_budget(58, 18);
    if stream.committed() <= tight.texture_target() || tight.critical(stream.committed()) {
        return 1_000.0;
    }
    let away = [
        view_at(0, 30.0, far_derivatives(64)),
        view_at(1, 1.0, near_derivatives(64)),
    ];
    let back = [
        view_at(0, 1.0, near_derivatives(64)),
        view_at(1, 30.0, far_derivatives(64)),
    ];
    stream.step(&tight, &away, BASE_MS, FRAME_BUDGET_MS);
    stream.step(&tight, &back, BASE_MS, FRAME_BUDGET_MS);
    stream.step(&tight, &away, BASE_MS, FRAME_BUDGET_MS);
    u32_to_f32(stream.thrash())
}

/// Mismatches against importance, then distance, then finer level.
#[must_use]
pub fn evict_order() -> f32 {
    let mut stream = Streamer::new(0, 0.0, 0);
    let levels = [32, 16, 8, 4, 2];
    for _ in 0..3 {
        stream.add_texture(64, 64, &levels);
    }
    flood(&mut stream);
    let wide = byte_budget(10_000, 0);
    let load = [
        view_at(0, 1.0, near_derivatives(64)),
        view_at(1, 1.0, near_derivatives(64)),
        view_at(2, 1.0, near_derivatives(64)),
    ];
    stream.step(&wide, &load, BASE_MS, FRAME_BUDGET_MS);
    stream.step(&wide, &load, BASE_MS, FRAME_BUDGET_MS);
    let tight = byte_budget(400, 320);
    if stream.committed() <= tight.texture_target() || tight.critical(stream.committed()) {
        return 1_000.0;
    }
    let pressure = [
        view_at(0, 100.0, rho_derivatives(64, 1.0)),
        view_at(1, 5.0, rho_derivatives(64, 8.0)),
        view_at(2, 1.0, rho_derivatives(64, 16.0)),
    ];
    for _ in 0..3 {
        stream.step(&tight, &pressure, BASE_MS, FRAME_BUDGET_MS);
        if stream.committed() <= tight.texture_target() {
            break;
        }
    }
    let expected = [(2usize, 0u32), (2, 1), (2, 2), (1, 0), (1, 1)];
    u32_to_f32(sequence_mismatches(stream.evicted(), &expected))
}

/// Mips that became resident while the transfer fence was still pending.
#[must_use]
pub fn fence() -> f32 {
    let mut stream = Streamer::new(0, 0.0, 0);
    stream.add_texture(32, 32, &[1_000, 10]);
    flood(&mut stream);
    let budget = budget_of(8192);
    let views = [view_at(0, 1.0, near_derivatives(32))];
    stream.step(&budget, &views, BASE_MS, FRAME_BUDGET_MS);
    if stream.is_resident(0, 0) {
        return 1.0;
    }
    stream.set_transfer_ready(false);
    let mut completed = 0u32;
    for _ in 0..3 {
        let before = stream.is_resident(0, 0);
        stream.step(&budget, &views, BASE_MS, FRAME_BUDGET_MS);
        if !before && stream.is_resident(0, 0) {
            completed = completed.saturating_add(1);
        }
    }
    u32_to_f32(completed)
}

fn view_at(index: usize, distance: f32, derivatives: Derivatives) -> TextureView {
    TextureView {
        index,
        distance,
        visible: true,
        derivatives: Some(derivatives),
    }
}

fn rho_derivatives(width: u32, rho: f32) -> Derivatives {
    Derivatives {
        dudx: rho / u32_to_f32(width.max(1)),
        dvdx: 0.0,
        dudy: 0.0,
        dvdy: 0.0,
    }
}

fn byte_budget(heap: u64, headroom: u64) -> Budget {
    Budget::from_heaps(
        &[crate::vram::HeapSlice {
            budget: heap,
            usage: 0,
            device_local: true,
        }],
        0,
        Headroom {
            percent: 0,
            floor_bytes: headroom,
        },
        true,
    )
}

fn note_admit(stream: &Streamer, base_ms: f32, faults: &mut u32) {
    let leftover = if base_ms >= FRAME_BUDGET_MS {
        0
    } else {
        floor_nonneg(FRAME_BUDGET_MS - base_ms)
    };
    let cap = stream.bytes_per_ms().saturating_mul(u64::from(leftover));
    if stream.last_admit() > cap {
        *faults = faults.saturating_add(1);
    }
}

fn sequence_mismatches(got: &[(usize, u32)], expected: &[(usize, u32)]) -> u32 {
    let mut count = 0u32;
    let len = got.len().max(expected.len());
    for index in 0..len {
        if got.get(index).copied() != expected.get(index).copied() {
            count = count.saturating_add(1);
        }
    }
    count
}

/// Admit stays under a positive meter cap. A cap of 0 is a miss.
///
/// Eight frames at `base_ms` 1 let the rate climb to the wide sustainable
/// cap. The scored frame uses `base_ms` 12, still under 16, so the leftover
/// shrinks and the cap sits strictly below the stored rate. Dropping the
/// sustainable min admits the stored rate and scores 1.
#[must_use]
pub fn admit_cap() -> f32 {
    let mut stream = Streamer::new(0, 0.0, 0);
    stream.add_texture(64, 64, &[4_000, 10]);
    let per = 1_000u64;
    stream.observe_disk(per, 1);
    stream.observe_decode(per, 1);
    stream.observe_transfer(per, 1);
    let budget = budget_of(8192);
    let views = [view_at(0, 1.0, near_derivatives(64))];
    for _ in 0..8 {
        stream.step(&budget, &views, 1.0, FRAME_BUDGET_MS);
    }
    let base_ms = 12.0;
    stream.step(&budget, &views, base_ms, FRAME_BUDGET_MS);
    let leftover = floor_nonneg(FRAME_BUDGET_MS - base_ms);
    let cap = stream.bytes_per_ms().saturating_mul(u64::from(leftover));
    if cap == 0 || stream.rate() <= cap || stream.last_admit() > cap {
        1.0
    } else {
        0.0
    }
}

/// Admit stays at or under the stored rate when that rate is below the
/// sustainable cap. Scoring the full sustainable value instead of the rate
/// scores 1.
#[must_use]
pub fn admit_rate() -> f32 {
    let mut stream = Streamer::new(0, 0.0, 0);
    stream.add_texture(64, 64, &[4_000, 10]);
    let per = 1_000u64;
    stream.observe_disk(per, 1);
    stream.observe_decode(per, 1);
    stream.observe_transfer(per, 1);
    let budget = budget_of(8192);
    let views = [view_at(0, 1.0, near_derivatives(64))];
    stream.step(&budget, &views, BASE_MS, FRAME_BUDGET_MS);
    let leftover = floor_nonneg(FRAME_BUDGET_MS - BASE_MS);
    let sustainable = stream.bytes_per_ms().saturating_mul(u64::from(leftover));
    let rate = stream.rate();
    if rate == 0 || sustainable == 0 || rate >= sustainable || stream.last_admit() > rate {
        1.0
    } else {
        0.0
    }
}

/// Streaming rows. They grade the streamer, not a shade.
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
    pub evict_order: f32,
    pub fence: f32,
    pub admit_cap: f32,
    pub admit_rate: f32,
}

#[must_use]
pub fn gate_rows() -> GateRows {
    GateRows {
        budget_recover: budget_recover(),
        upload_refused: upload_refused(),
        oom_recover: oom_recover(),
        spin_evict: spin_evict(),
        quality_cap: quality_cap(),
        ultra_near: ultra_near(),
        slow_bus: slow_bus(),
        thrash: spin_thrash(),
        evict_order: evict_order(),
        fence: fence(),
        admit_cap: admit_cap(),
        admit_rate: admit_rate(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero(value: f32) -> bool {
        value.to_bits() == 0.0f32.to_bits()
    }

    #[test]
    fn the_gate_rows_pass() {
        let rows = gate_rows();
        assert!(
            rows.budget_recover.to_bits() <= 6.0f32.to_bits(),
            "{}",
            rows.budget_recover
        );
        assert!(zero(rows.upload_refused), "{}", rows.upload_refused);
        assert!(zero(rows.oom_recover), "{}", rows.oom_recover);
        assert!(zero(rows.spin_evict), "{}", rows.spin_evict);
        assert!(zero(rows.quality_cap), "{}", rows.quality_cap);
        assert!(zero(rows.ultra_near), "{}", rows.ultra_near);
        assert!(zero(rows.slow_bus), "{}", rows.slow_bus);
        assert!(zero(rows.thrash), "{}", rows.thrash);
        assert!(zero(rows.evict_order), "{}", rows.evict_order);
        assert!(zero(rows.fence), "{}", rows.fence);
        assert!(zero(rows.admit_cap), "{}", rows.admit_cap);
        assert!(zero(rows.admit_rate), "{}", rows.admit_rate);
    }

    #[test]
    fn an_oom_eviction_releases_the_mip_the_device_holds() {
        let mut stream = Streamer::new(0, 0.0, 0);
        stream.add_texture(8, 8, &[64, 16]);
        assert!(stream.holds_memory(0, 1));
        assert!(stream.evict_for_oom());
        assert!(!stream.holds_memory(0, 1));
        assert!(!stream.evict_for_oom());
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
