//! Budgeted GI refresh.
//!
//! Work caps are ceilings. A controller scales them when a measured frame runs
//! past the millisecond cap. It never opens the frame to an unlimited pass.
//!
//! Refresh order (the glossary keeps "tier" for removed GI v1):
//! 1. Viewport, including the guard band, up to the quality floor.
//! 2. Out-of-view regions near the camera, up to that same floor.
//! 3. Leftover budget refines high detail by visible error.
//! A fast change stays urgent inside its order. It does not jump the order.

#![expect(clippy::as_conversions, reason = "pixel and budget counts cross the u32 boundary")]
#![expect(clippy::cast_possible_truncation, reason = "pixel and budget counts fit the picture")]
#![expect(clippy::cast_sign_loss, reason = "rounded sizes are clamped to zero first")]
#![expect(clippy::cast_precision_loss, reason = "tile counts are far below 2^24")]

/// Cell edge of cache levels L0–L4, in metres.
pub const LEVEL_METRES: [f32; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];

/// Change magnitude that counts as a fast, large change inside its order.
pub const URGENT_CHANGE: f32 = 0.5;

/// Controller scale never falls below this. The starvation slice still runs.
pub const SCALE_FLOOR: f32 = 1.0 / 64.0;

/// Per-frame work ceilings for one preset. Zero means the pass does not run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkCaps {
    pub screen_probe_rays: u32,
    pub cache_relights: u32,
    pub cache_rays: u32,
    pub spawns: u32,
    pub coarsen_ops: u32,
    pub fresh_shadow_rays: u32,
    pub denoise_items: u32,
    pub denoise_iters: u32,
    pub lighting_ms: f32,
    pub resolve_ms: f32,
}

/// Quality floor: a level this coarse or finer, and at least this convergence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QualityFloor {
    /// L0 is the finest. An item at `level <= max_level` meets the depth floor.
    pub max_level: u8,
    pub min_converged: f32,
}

/// Where an item sits relative to the camera. The viewport includes the guard band.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemZone {
    Viewport,
    Near,
    Far,
}

/// One cache entry, probe, or tile the refresh order can pick.
#[derive(Clone, Copy, Debug)]
pub struct WorkItem {
    pub index: u32,
    pub zone: ItemZone,
    pub level: u8,
    pub converged: f32,
    pub change_mag: f32,
    pub distance_m: f32,
    pub age_frames: u32,
}

/// One edge of the guard band, as a fraction of the displayed frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuardEdges {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// Lit picture and the display crop inside it. The eye does not move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LitFrame {
    pub lit_w: u32,
    pub lit_h: u32,
    pub crop_x: u32,
    pub crop_y: u32,
    pub display_w: u32,
    pub display_h: u32,
    /// Fractions of the display after the crop is snapped to the probe tile.
    /// The projection uses these, so the look direction stays on the display center.
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// What one frame's screen passes are allowed to refresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenPlan {
    pub direct_gate: bool,
    pub atrous_gate: bool,
    pub trace_gate: bool,
    pub tile_x: u32,
    pub tile_y: u32,
    pub tile_w: u32,
    pub tile_h: u32,
    pub starve_y: u32,
    pub starve_h: u32,
    /// Cache rays to write when the controller has scaled the frame. Zero keeps the settings word.
    pub cache_ray_grant: u32,
}

/// Scales the work caps after a measured lighting total.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BudgetController {
    pub scale: f32,
}

impl Default for BudgetController {
    fn default() -> Self {
        Self { scale: 1.0 }
    }
}

impl BudgetController {
    #[must_use]
    pub fn new() -> Self {
        Self { scale: 1.0 }
    }

    /// `lighting_ms` is `None` when this frame has no timestamp sample.
    /// Over the cap, scale drops by `cap / ms`. Under 75% of the cap, it creeps back toward 1.
    pub fn observe(&mut self, lighting_ms: Option<f32>, cap_ms: f32) {
        let Some(ms) = lighting_ms else {
            return;
        };
        if !ms.is_finite() || ms <= 0.0 || !cap_ms.is_finite() || cap_ms <= 0.0 {
            return;
        }
        if ms > cap_ms {
            self.scale = (self.scale * (cap_ms / ms)).clamp(SCALE_FLOOR, 1.0);
        } else if ms < cap_ms * 0.75 {
            self.scale = (self.scale * 1.05).min(1.0);
        }
    }
}

#[must_use]
pub fn caps_for(preset: &str) -> WorkCaps {
    match preset {
        "potato" | "day-cycle" => WorkCaps {
            screen_probe_rays: 0,
            cache_relights: 0,
            cache_rays: 0,
            spawns: 0,
            coarsen_ops: 0,
            fresh_shadow_rays: 500_000,
            denoise_items: 0,
            denoise_iters: 0,
            lighting_ms: 1.5,
            resolve_ms: 0.8,
        },
        "low" | "gi-1bounce" => WorkCaps {
            screen_probe_rays: 33_000,
            cache_relights: 16_000,
            cache_rays: 64_000,
            spawns: 2_000,
            coarsen_ops: 2_000,
            fresh_shadow_rays: 500_000,
            denoise_items: 4_000,
            denoise_iters: 1,
            lighting_ms: 4.0,
            resolve_ms: 1.5,
        },
        "medium" => WorkCaps {
            screen_probe_rays: 261_000,
            cache_relights: 32_000,
            cache_rays: 128_000,
            spawns: 4_000,
            coarsen_ops: 4_000,
            fresh_shadow_rays: 1_000_000,
            denoise_items: 8_000,
            denoise_iters: 3,
            lighting_ms: 6.0,
            resolve_ms: 2.0,
        },
        _ => WorkCaps {
            screen_probe_rays: 522_000,
            cache_relights: 64_000,
            cache_rays: 256_000,
            spawns: 8_000,
            coarsen_ops: 8_000,
            fresh_shadow_rays: 2_000_000,
            denoise_items: 16_000,
            denoise_iters: 3,
            lighting_ms: 8.0,
            resolve_ms: 2.5,
        },
    }
}

#[must_use]
pub fn floor_for(preset: &str) -> QualityFloor {
    match preset {
        "medium" => QualityFloor {
            max_level: 2,
            min_converged: 0.35,
        },
        "low" | "gi-1bounce" => QualityFloor {
            max_level: 3,
            min_converged: 0.35,
        },
        "potato" | "day-cycle" => QualityFloor {
            max_level: 4,
            min_converged: 0.35,
        },
        _ => QualityFloor {
            max_level: 1,
            min_converged: 0.35,
        },
    }
}

#[must_use]
pub fn loosen_for(preset: &str) -> f32 {
    match preset {
        "medium" => 3.0,
        "low" | "gi-1bounce" => 2.5,
        "potato" | "day-cycle" => 2.0,
        _ => 4.0,
    }
}

#[must_use]
pub fn near_radius_for(preset: &str) -> f32 {
    match preset {
        "medium" => 8.0,
        "low" | "gi-1bounce" => 6.0,
        "potato" | "day-cycle" => 4.0,
        _ => 12.0,
    }
}

#[must_use]
pub fn guard_edge_for(preset: &str) -> f32 {
    match preset {
        "medium" => 0.12,
        "high" => 0.15,
        _ => 0.10,
    }
}

/// Coarse layer cap in bytes. High 24 MB, Medium 16 MB, Low 8 MB.
#[must_use]
pub fn coarse_cap_bytes(preset: &str) -> u64 {
    match preset {
        "medium" => 16 * 1024 * 1024,
        "low" | "gi-1bounce" | "potato" | "day-cycle" => 8 * 1024 * 1024,
        _ => 24 * 1024 * 1024,
    }
}

#[must_use]
pub fn scale_u32(value: u32, scale: f32) -> u32 {
    if value == 0 || !(scale > 0.0) {
        return 0;
    }
    let scaled = (f64::from(value) * f64::from(scale.clamp(SCALE_FLOOR, 1.0))).floor();
    if scaled <= 1.0 {
        1
    } else if scaled >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        scaled as u32
    }
}

#[must_use]
pub fn below_floor(item: &WorkItem, floor: QualityFloor) -> bool {
    item.level > floor.max_level || item.converged < floor.min_converged
}

#[must_use]
pub fn visible_error(change_mag: f32, distance_m: f32) -> f32 {
    let closeness = 1.0 / (distance_m.max(0.25));
    change_mag.max(0.0) * closeness
}

/// Viewport below the floor, then near out-of-view below the floor, then visible error.
/// Ten percent of `budget` is held for the oldest viewport items.
/// Far items below the floor are not floor catch-up.
#[must_use]
pub fn select_refresh(items: &[WorkItem], budget: u32, floor: QualityFloor) -> Vec<u32> {
    if budget == 0 || items.is_empty() {
        return Vec::new();
    }
    let reserve = budget / 10;
    let main_budget = budget - reserve;
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        let ia = &items[a];
        let ib = &items[b];
        refresh_rank(ia, floor)
            .cmp(&refresh_rank(ib, floor))
            .then_with(|| {
                u8::from(ia.change_mag < URGENT_CHANGE).cmp(&u8::from(ib.change_mag < URGENT_CHANGE))
            })
            .then_with(|| within_order(ia, ib, floor))
            .then_with(|| ia.index.cmp(&ib.index))
    });
    let mut chosen = Vec::new();
    let mut taken = vec![false; items.len()];
    for slot in order {
        if chosen.len() >= main_budget as usize {
            break;
        }
        if refresh_rank(&items[slot], floor) > 2 {
            break;
        }
        taken[slot] = true;
        chosen.push(items[slot].index);
    }
    let mut ages: Vec<usize> = (0..items.len())
        .filter(|&slot| !taken[slot] && items[slot].zone == ItemZone::Viewport)
        .collect();
    ages.sort_by(|&a, &b| {
        items[b]
            .age_frames
            .cmp(&items[a].age_frames)
            .then_with(|| items[a].index.cmp(&items[b].index))
    });
    for slot in ages.into_iter().take(reserve as usize) {
        chosen.push(items[slot].index);
    }
    chosen
}

fn refresh_rank(item: &WorkItem, floor: QualityFloor) -> u8 {
    let below = below_floor(item, floor);
    match (item.zone, below) {
        (ItemZone::Viewport, true) => 0,
        (ItemZone::Near, true) => 1,
        (_, false) => 2,
        (ItemZone::Far, true) => 3,
    }
}

fn within_order(left: &WorkItem, right: &WorkItem, floor: QualityFloor) -> std::cmp::Ordering {
    let rank = refresh_rank(left, floor);
    if rank <= 1 {
        left.distance_m
            .total_cmp(&right.distance_m)
            .then_with(|| right.change_mag.total_cmp(&left.change_mag))
    } else {
        visible_error(right.change_mag, right.distance_m)
            .total_cmp(&visible_error(left.change_mag, left.distance_m))
    }
}

/// Cone in front of the camera is the viewport. The guard widens that cone.
/// Outside it, `near_radius` is the catch-up shell. The rest is far.
#[must_use]
pub fn zone_of(forward_dot: f32, distance_m: f32, near_radius: f32, guard_edge: f32) -> ItemZone {
    let widen = (1.0 + 2.0 * guard_edge.max(0.0)).max(1.0);
    // 60° vertical fov. The guard grows the half-angle. A facing point is in view.
    let half = (30.0_f32.to_radians() * widen).min(89.0_f32.to_radians());
    let limit = half.cos();
    if distance_m <= 0.05 {
        return ItemZone::Viewport;
    }
    if forward_dot >= limit {
        ItemZone::Viewport
    } else if distance_m <= near_radius.max(0.0) {
        ItemZone::Near
    } else {
        ItemZone::Far
    }
}

/// Projected size in pixels of a cell `cell_m` metres across at `distance_m`.
/// Off-screen distance still produces a size. Leaving the view does not zero it.
#[must_use]
pub fn projected_px(cell_m: f32, distance_m: f32, focal_px: f32) -> f32 {
    if cell_m <= 0.0 || focal_px <= 0.0 {
        return 0.0;
    }
    cell_m * focal_px / distance_m.max(0.05)
}

#[must_use]
pub fn should_add_level(projected: f32, target_px: f32) -> bool {
    target_px > 0.0 && projected > 1.5 * target_px
}

/// Removal is `loosen` times further from the add threshold. Unseen frames are not an input.
#[must_use]
pub fn should_remove_level(level: u8, projected: f32, target_px: f32, loosen: f32) -> bool {
    if level >= 3 || target_px <= 0.0 {
        return false;
    }
    let loosen = loosen.max(1.0);
    projected < (1.5 / loosen) * target_px
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MsmeSample {
    pub mean: f32,
    pub variance: f32,
    pub area: f32,
}

/// Area-weighted merge of a retiring fine entry into its coarser parent.
#[must_use]
pub fn merge_into_parent(child: MsmeSample, parent: MsmeSample) -> MsmeSample {
    let area = (child.area.max(0.0) + parent.area.max(0.0)).max(0.0);
    if area <= 1.0e-8 {
        return parent;
    }
    let mean = (child.mean * child.area.max(0.0) + parent.mean * parent.area.max(0.0)) / area;
    let child_second = child.variance.max(0.0) + child.mean * child.mean;
    let parent_second = parent.variance.max(0.0) + parent.mean * parent.mean;
    let second = (child_second * child.area.max(0.0) + parent_second * parent.area.max(0.0)) / area;
    let variance = (second - mean * mean).max(0.0);
    MsmeSample { mean, variance, area }
}

#[derive(Clone, Copy, Debug)]
pub struct CoarseEntry {
    pub index: u32,
    pub distance_m: f32,
    pub seen_frame: u32,
    pub bytes: u64,
}

/// Memory pressure on L3/L4. Farthest first. A distance tie drops the least recently seen.
/// This is not an unseen-frame eviction.
#[must_use]
pub fn evict_coarse(entries: &[CoarseEntry], cap_bytes: u64) -> Vec<u32> {
    let mut kept: Vec<usize> = (0..entries.len()).collect();
    let mut total: u64 = entries.iter().map(|entry| entry.bytes).sum();
    let mut dropped = Vec::new();
    if total <= cap_bytes {
        return dropped;
    }
    kept.sort_by(|&a, &b| {
        entries[b]
            .distance_m
            .total_cmp(&entries[a].distance_m)
            .then_with(|| entries[a].seen_frame.cmp(&entries[b].seen_frame))
            .then_with(|| entries[a].index.cmp(&entries[b].index))
    });
    for slot in kept {
        if total <= cap_bytes {
            break;
        }
        total = total.saturating_sub(entries[slot].bytes);
        dropped.push(entries[slot].index);
    }
    dropped
}

/// Bias the guard toward the turn. `yaw_delta` > 0 turns toward +X (screen right).
/// `pitch_delta` > 0 looks up. Left+right and top+bottom stay `2 * base`.
#[must_use]
pub fn bias_guard(base: f32, yaw_delta: f32, pitch_delta: f32) -> GuardEdges {
    let base = if base.is_finite() { base.clamp(0.0, 0.5) } else { 0.0 };
    if base <= 0.0 {
        return GuardEdges {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        };
    }
    let yaw = if yaw_delta.is_finite() { yaw_delta } else { 0.0 };
    let pitch = if pitch_delta.is_finite() { pitch_delta } else { 0.0 };
    let shift_x = (yaw / 0.05).clamp(-1.0, 1.0) * base * 0.5;
    let shift_y = (pitch / 0.05).clamp(-1.0, 1.0) * base * 0.5;
    let (left, right) = renormalize_pair(base - shift_x, base + shift_x, base * 2.0);
    let (bottom, top) = renormalize_pair(base - shift_y, base + shift_y, base * 2.0);
    GuardEdges { left, right, top, bottom }
}

fn renormalize_pair(a: f32, b: f32, sum: f32) -> (f32, f32) {
    let mut a = a.clamp(0.05, 0.35);
    let mut b = b.clamp(0.05, 0.35);
    let scale = a + b;
    if scale > 0.0 && sum > 0.0 {
        a *= sum / scale;
        b *= sum / scale;
    }
    (a, b)
}

#[must_use]
pub fn lit_frame(display_w: u32, display_h: u32, edges: GuardEdges, tile: u32) -> LitFrame {
    let display_w = display_w.max(1);
    let display_h = display_h.max(1);
    let tile = tile.max(1);
    let sum_x = (edges.left + edges.right).max(0.0);
    let sum_y = (edges.top + edges.bottom).max(0.0);
    if sum_x <= 1.0e-6 && sum_y <= 1.0e-6 {
        return LitFrame {
            lit_w: display_w,
            lit_h: display_h,
            crop_x: 0,
            crop_y: 0,
            display_w,
            display_h,
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        };
    }
    let extra_w = (f64::from(display_w) * f64::from(sum_x)).ceil() as u32;
    let extra_h = (f64::from(display_h) * f64::from(sum_y)).ceil() as u32;
    let lit_w = (display_w.saturating_add(extra_w)).div_ceil(tile) * tile;
    let lit_h = (display_h.saturating_add(extra_h)).div_ceil(tile) * tile;
    let mut crop_x = (f64::from(display_w) * f64::from(edges.left.max(0.0))).round() as u32;
    let mut crop_y = (f64::from(display_h) * f64::from(edges.top.max(0.0))).round() as u32;
    crop_x = (crop_x / tile) * tile;
    crop_y = (crop_y / tile) * tile;
    crop_x = crop_x.min(lit_w.saturating_sub(display_w));
    crop_y = crop_y.min(lit_h.saturating_sub(display_h));
    let left = crop_x as f32 / display_w as f32;
    let right = lit_w.saturating_sub(display_w).saturating_sub(crop_x) as f32 / display_w as f32;
    let top = crop_y as f32 / display_h as f32;
    let bottom = lit_h.saturating_sub(display_h).saturating_sub(crop_y) as f32 / display_h as f32;
    LitFrame {
        lit_w,
        lit_h,
        crop_x,
        crop_y,
        display_w,
        display_h,
        left,
        right,
        top,
        bottom,
    }
}

#[must_use]
pub fn screen_plan(
    tiles_x: u32,
    tiles_y: u32,
    shadow_grant: u32,
    shadow_demand: u32,
    denoise_grant: u32,
    denoise_demand: u32,
    trace_grant: u32,
    trace_demand: u32,
    changes: u32,
    scaled: bool,
    frame: u32,
    cache_rays: u32,
) -> ScreenPlan {
    let tiles_x = tiles_x.max(1);
    let tiles_y = tiles_y.max(1);
    let tiles = tiles_x.saturating_mul(tiles_y);
    let direct = budget_mode_gates(shadow_grant, shadow_demand, true, scaled);
    let atrous = changes > 0 && denoise_grant < denoise_demand;
    let trace = budget_mode_gates(trace_grant, trace_demand, changes > 0, scaled);
    let cache_ray_grant = if scaled && cache_rays > 0 { cache_rays } else { 0 };
    if !direct && !atrous && !trace {
        return ScreenPlan {
            direct_gate: false,
            atrous_gate: false,
            trace_gate: false,
            tile_x: 0,
            tile_y: 0,
            tile_w: 0,
            tile_h: 0,
            starve_y: 0,
            starve_h: 0,
            cache_ray_grant,
        };
    }
    let grant_tiles = if direct {
        tiles_for_grant(tiles, shadow_grant, shadow_demand)
    } else if atrous {
        tiles_for_grant(tiles, denoise_grant, denoise_demand)
    } else {
        tiles_for_grant(tiles, trace_grant, trace_demand)
    };
    let (tile_x, tile_y, tile_w, tile_h, starve_y, starve_h) =
        centered_and_band(tiles_x, tiles_y, grant_tiles, frame);
    ScreenPlan {
        direct_gate: direct,
        atrous_gate: atrous,
        trace_gate: trace,
        tile_x,
        tile_y,
        tile_w,
        tile_h,
        starve_y,
        starve_h,
        cache_ray_grant,
    }
}

fn budget_mode_gates(grant: u32, demand: u32, changes: bool, scaled: bool) -> bool {
    grant < demand && (changes || scaled)
}

fn tiles_for_grant(tiles: u32, grant: u32, demand: u32) -> u32 {
    if demand == 0 || grant >= demand {
        return tiles;
    }
    let share = (u64::from(tiles) * u64::from(grant)) / u64::from(demand);
    share.max(1).min(u64::from(tiles)) as u32
}

fn centered_and_band(tiles_x: u32, tiles_y: u32, grant_tiles: u32, frame: u32) -> (u32, u32, u32, u32, u32, u32) {
    let reserve = grant_tiles / 10;
    let main = grant_tiles.saturating_sub(reserve).max(1).min(tiles_x.saturating_mul(tiles_y));
    let (tile_w, tile_h) = rect_wh(tiles_x, tiles_y, main);
    let tile_x = tiles_x.saturating_sub(tile_w) / 2;
    let tile_y = tiles_y.saturating_sub(tile_h) / 2;
    let starve_h = if reserve == 0 {
        0
    } else {
        reserve.div_ceil(tiles_x).clamp(1, tiles_y)
    };
    let starve_y = if starve_h == 0 {
        0
    } else {
        (frame % tiles_y) % tiles_y.saturating_sub(starve_h).max(1)
    };
    (tile_x, tile_y, tile_w, tile_h, starve_y, starve_h)
}

fn rect_wh(tiles_x: u32, tiles_y: u32, count: u32) -> (u32, u32) {
    if count >= tiles_x.saturating_mul(tiles_y) {
        return (tiles_x, tiles_y);
    }
    let aspect = f64::from(tiles_x) / f64::from(tiles_y.max(1));
    let mut h = ((f64::from(count) / aspect).sqrt()).floor().max(1.0) as u32;
    h = h.clamp(1, tiles_y);
    let mut w = (count / h).clamp(1, tiles_x);
    while w.saturating_mul(h) > count && h > 1 {
        h -= 1;
        w = (count / h).clamp(1, tiles_x);
    }
    if w.saturating_mul(h) > count {
        w = count.min(tiles_x).max(1);
        h = 1;
    }
    (w, h)
}

impl ScreenPlan {
    /// Push-constant words. Flags, one tile rect, the starvation band, the crop.
    #[must_use]
    pub fn words(self, crop_x: u32, crop_y: u32) -> [u32; 4] {
        let flags = u32::from(self.direct_gate)
            | (u32::from(self.atrous_gate) << 1)
            | (u32::from(self.trace_gate) << 2);
        let pack = |a, b, c, d| {
            (a & 255) | ((b & 255) << 8) | ((c & 255) << 16) | ((d & 255) << 24)
        };
        let rect = pack(self.tile_x, self.tile_y, self.tile_w, self.tile_h);
        let band = pack(self.starve_y, self.starve_h, 0, 0);
        let crop = (crop_x & 0xffff) | ((crop_y & 0xffff) << 16);
        [flags, rect, band, crop]
    }

    #[must_use]
    pub fn gates_off(self) -> bool {
        !self.direct_gate && !self.atrous_gate && !self.trace_gate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(
        index: u32,
        zone: ItemZone,
        level: u8,
        converged: f32,
        change: f32,
        distance: f32,
        age: u32,
    ) -> WorkItem {
        WorkItem {
            index,
            zone,
            level,
            converged,
            change_mag: change,
            distance_m: distance,
            age_frames: age,
        }
    }

    #[test]
    fn viewport_floor_runs_before_near_catch_up_and_refinement() {
        let floor = floor_for("high");
        let items = [
            item(1, ItemZone::Near, 4, 0.0, 0.9, 1.0, 0),
            item(2, ItemZone::Viewport, 0, 1.0, 0.9, 1.0, 0),
            item(3, ItemZone::Viewport, 4, 0.0, 0.1, 8.0, 0),
        ];
        let picked = select_refresh(&items, 2, floor);
        assert_eq!(picked, vec![3, 1]);
    }

    #[test]
    fn a_fast_change_stays_inside_its_order() {
        let floor = floor_for("high");
        let items = [
            item(1, ItemZone::Far, 4, 0.0, 1.0, 2.0, 0),
            item(2, ItemZone::Viewport, 4, 0.0, 0.1, 3.0, 0),
            item(3, ItemZone::Viewport, 4, 0.0, 0.8, 9.0, 0),
        ];
        let picked = select_refresh(&items, 2, floor);
        assert_eq!(picked[0], 3);
        assert_eq!(picked[1], 2);
        assert!(!picked.contains(&1));
    }

    #[test]
    fn the_reserve_refreshes_an_old_viewport_item() {
        let floor = QualityFloor {
            max_level: 4,
            min_converged: 0.0,
        };
        let mut items = Vec::new();
        for index in 0..10 {
            items.push(item(index, ItemZone::Viewport, 0, 1.0, 0.0, index as f32, index));
        }
        let picked = select_refresh(&items, 10, floor);
        assert_eq!(picked.len(), 10);
        assert_eq!(*picked.last().unwrap_or(&0), 9);
    }

    #[test]
    fn far_below_the_floor_is_not_catch_up() {
        let floor = floor_for("high");
        let items = [
            item(1, ItemZone::Far, 4, 0.0, 0.2, 30.0, 0),
            item(2, ItemZone::Viewport, 0, 1.0, 0.2, 4.0, 0),
        ];
        let picked = select_refresh(&items, 1, floor);
        assert_eq!(picked, vec![2]);
    }

    #[test]
    fn removal_is_looser_than_adding_and_ignores_being_unseen() {
        let target = 8.0;
        assert!(should_add_level(13.0, target));
        assert!(!should_remove_level(1, 13.0, target, 4.0));
        assert!(!should_remove_level(1, 4.0, target, 4.0));
        assert!(should_remove_level(1, 2.0, target, 4.0));
        assert!(!should_remove_level(3, 0.0, target, 4.0));
        assert!(!should_remove_level(4, 0.0, target, 4.0));
        let off_screen = projected_px(0.25, 2.0, 800.0);
        assert!(off_screen > 1.0);
    }

    #[test]
    fn a_fine_entry_merges_into_its_parent() {
        let merged = merge_into_parent(
            MsmeSample {
                mean: 4.0,
                variance: 0.0,
                area: 1.0,
            },
            MsmeSample {
                mean: 0.0,
                variance: 0.0,
                area: 1.0,
            },
        );
        assert!((merged.mean - 2.0).abs() < 1.0e-5);
        assert!((merged.area - 2.0).abs() < 1.0e-5);
    }

    #[test]
    fn coarse_pressure_drops_the_farthest_then_the_older_tie() {
        let entries = [
            CoarseEntry {
                index: 1,
                distance_m: 10.0,
                seen_frame: 5,
                bytes: 8,
            },
            CoarseEntry {
                index: 2,
                distance_m: 10.0,
                seen_frame: 1,
                bytes: 8,
            },
            CoarseEntry {
                index: 3,
                distance_m: 2.0,
                seen_frame: 0,
                bytes: 8,
            },
        ];
        assert_eq!(evict_coarse(&entries, 8), vec![2, 1]);
    }

    #[test]
    fn guard_bias_keeps_the_margin_sum_and_leads_the_turn() {
        let edges = bias_guard(0.15, 0.2, 0.0);
        assert!((edges.left + edges.right - 0.30).abs() < 1.0e-4);
        assert!(edges.right > edges.left);
        let still = bias_guard(0.15, 0.0, 0.0);
        assert!((still.left - still.right).abs() < 1.0e-4);
        assert!((still.top + still.bottom - 0.30).abs() < 1.0e-4);
    }

    #[test]
    fn a_turn_does_not_change_the_lit_size() {
        let tile = 16;
        let still = lit_frame(320, 180, bias_guard(0.15, 0.0, 0.0), tile);
        let turning = lit_frame(320, 180, bias_guard(0.15, 0.2, 0.0), tile);
        assert_eq!(still.lit_w, turning.lit_w);
        assert_eq!(still.lit_h, turning.lit_h);
        assert_ne!(still.crop_x, turning.crop_x);
        assert_eq!(still.crop_x % tile, 0);
    }

    #[test]
    fn the_controller_scales_down_and_does_not_open_the_frame() {
        let mut controller = BudgetController::new();
        controller.observe(None, 8.0);
        assert!((controller.scale - 1.0).abs() < 1.0e-6);
        controller.observe(Some(16.0), 8.0);
        assert!(controller.scale < 0.6);
        assert!(controller.scale >= SCALE_FLOOR);
        controller.scale = SCALE_FLOOR / 2.0;
        controller.observe(Some(1000.0), 8.0);
        assert!((controller.scale - SCALE_FLOOR).abs() < 1.0e-6);
        controller.observe(Some(1.0), 8.0);
        assert!(controller.scale <= 1.0);
    }

    #[test]
    fn a_covered_frame_does_not_gate() {
        let plan = screen_plan(20, 12, 2_000_000, 57_600, 16_000, 57_600, 522_000, 15_360, 0, false, 0, 256_000);
        assert!(plan.gates_off());
        assert_eq!(plan.words(0, 0)[0], 0);
    }

    #[test]
    fn a_mover_gates_denoise_to_a_centered_rect() {
        let plan = screen_plan(20, 12, 2_000_000, 57_600, 16_000, 57_600, 522_000, 15_360, 1, false, 3, 256_000);
        assert!(plan.atrous_gate);
        assert!(!plan.direct_gate);
        assert!(plan.tile_w > 0 && plan.tile_h > 0);
        let center_x = plan.tile_x + plan.tile_w / 2;
        assert!((center_x as i32 - 10).abs() <= 1);
        assert!(plan.tile_w * plan.tile_h < 20 * 12);
    }

    #[test]
    fn high_caps_match_the_design_table() {
        let caps = caps_for("high");
        assert_eq!(caps.fresh_shadow_rays, 2_000_000);
        assert_eq!(caps.denoise_items, 16_000);
        assert!((caps.lighting_ms - 8.0).abs() < 1.0e-6);
        assert_eq!(caps_for("potato").lighting_ms.to_bits(), 1.5_f32.to_bits());
    }
}
