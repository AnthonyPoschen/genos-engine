//! Budgeted GI refresh.
//!
//! Work caps are ceilings. A controller scales them when a measured frame runs
//! past the millisecond cap. It never opens the frame to an unlimited pass.
//!
//! Refresh order (the glossary keeps "tier" for removed GI v1):
//! 1. The display, up to the quality floor.
//! 2. The guard band toward the turn, up to that same floor.
//! 3. Leftover budget refines the rest of the lit frame by visible error.
//!
//! A fast change stays urgent inside its order. It does not jump the order.

#![expect(clippy::as_conversions, reason = "guard fractions are display pixels over the picture")]
#![expect(clippy::cast_precision_loss, reason = "tile counts are far below 2^24")]

/// Cell edge of cache levels L0–L4, in metres.
pub const LEVEL_METRES: [f32; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];

/// Change magnitude that counts as a fast, large change inside its order.
pub const URGENT_CHANGE: f32 = 0.5;

/// Controller scale never falls below this. The starvation slice still runs.
pub const SCALE_FLOOR: f32 = 1.0 / 64.0;

/// Non-negative `value` truncated toward zero. Out of range clamps.
///
/// The integer comes from the IEEE bits, so this file does not narrow with `as`.
fn trunc_u32(value: f64) -> u32 {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    // 2^32 is the first integer a `u32` cannot hold. f64 rounds `u32::MAX` to that.
    if value.is_infinite() || value >= 4_294_967_296.0 {
        return u32::MAX;
    }
    let bits = value.to_bits();
    let exp = (bits >> 52) & 0x7ff;
    if exp < 1023 {
        return 0;
    }
    let shift = exp - 1023;
    let mant = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
    let whole = mant >> (52 - shift);
    u32::try_from(whole).unwrap_or(u32::MAX)
}

fn clamp_scale(value: f64, limit: f64) -> f64 {
    let floor = f64::from(SCALE_FLOOR);
    if value.is_finite() {
        value.clamp(floor, limit)
    } else {
        floor
    }
}

fn usize_from_u32(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

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

/// Where an item sits relative to the camera.
///
/// On the screen gate the display is the viewport, the turn-side guard is near,
/// and the rest of the lit frame is far. `zone_of` is the cone used for that split.
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

/// Grants and demands for the three screen passes, plus the cache-ray cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenBudget {
    pub tiles_x: u32,
    pub tiles_y: u32,
    pub shadow_grant: u32,
    pub shadow_demand: u32,
    pub denoise_grant: u32,
    pub denoise_demand: u32,
    pub trace_grant: u32,
    pub trace_demand: u32,
    pub changes: u32,
    pub scaled: bool,
    pub frame: u32,
    pub cache_rays: u32,
    /// Display window inside the lit grid, in tiles. Zero width means the whole grid.
    pub display_x: u32,
    pub display_y: u32,
    pub display_tiles_w: u32,
    pub display_tiles_h: u32,
    /// Sign of the turn. Positive grows the rect toward larger tile x or y.
    pub turn_x: i32,
    pub turn_y: i32,
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
    /// 1 keeps the preset caps. The floor is 1/64. It never rises above 1.
    pub scale: f64,
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
    pub fn observe(&mut self, lighting_ms: Option<f64>, cap_ms: f32) {
        let Some(ms) = lighting_ms else {
            return;
        };
        let cap = f64::from(cap_ms);
        if !ms.is_finite() || ms <= 0.0 || !cap.is_finite() || cap <= 0.0 {
            return;
        }
        if ms > cap {
            self.scale = clamp_scale(self.scale * (cap / ms), 1.0);
        } else if ms < cap * 0.75 {
            self.scale = clamp_scale(self.scale * 1.05, 1.0);
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
    if value == 0 || !scale.is_finite() || scale <= 0.0 {
        return 0;
    }
    let clamped = f64::from(scale).clamp(f64::from(SCALE_FLOOR), 1.0);
    let scaled = (f64::from(value) * clamped).floor();
    if scaled <= 1.0 {
        1
    } else {
        trunc_u32(scaled)
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
        if chosen.len() >= usize_from_u32(main_budget) {
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
    for slot in ages.into_iter().take(usize_from_u32(reserve)) {
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

/// Cone in front of the camera is the viewport. `guard_edge` is the extra on
/// the axis, so the cone grows by that fraction. Outside it, `near_radius` is
/// the catch-up shell. The rest is far.
#[must_use]
pub fn zone_of(forward_dot: f32, distance_m: f32, near_radius: f32, guard_edge: f32) -> ItemZone {
    let widen = (1.0 + guard_edge.max(0.0)).max(1.0);
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

/// Half-turn per frame that takes the whole leading-edge extra.
const GUARD_TURN_FULL_RAD: f32 = std::f32::consts::PI;
/// Extra fraction of the display added on the leading edge at that rate.
const GUARD_TURN_EXTRA: f32 = 0.10;

/// `base` is the symmetric extra on each axis (High 0.15), split across the
/// two edges. A turn adds up to [`GUARD_TURN_EXTRA`] on the leading edge.
/// The share is `yaw / π`, so 0.05 rad does not saturate. `yaw_delta` > 0
/// turns toward +X (screen right). `pitch_delta` > 0 looks up.
#[must_use]
pub fn bias_guard(base: f32, yaw_delta: f32, pitch_delta: f32) -> GuardEdges {
    let base = if base.is_finite() { base.clamp(0.0, 0.5) } else { 0.0 };
    let half = base * 0.5;
    let yaw = if yaw_delta.is_finite() { yaw_delta } else { 0.0 };
    let pitch = if pitch_delta.is_finite() { pitch_delta } else { 0.0 };
    let turn_x = (yaw / GUARD_TURN_FULL_RAD).clamp(-1.0, 1.0) * GUARD_TURN_EXTRA;
    let turn_y = (pitch / GUARD_TURN_FULL_RAD).clamp(-1.0, 1.0) * GUARD_TURN_EXTRA;
    GuardEdges {
        left: half + (-turn_x).max(0.0),
        right: half + turn_x.max(0.0),
        bottom: half + (-turn_y).max(0.0),
        top: half + turn_y.max(0.0),
    }
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
    let extra_w = trunc_u32((f64::from(display_w) * f64::from(sum_x)).ceil());
    let extra_h = trunc_u32((f64::from(display_h) * f64::from(sum_y)).ceil());
    let lit_w = (display_w.saturating_add(extra_w)).div_ceil(tile) * tile;
    let lit_h = (display_h.saturating_add(extra_h)).div_ceil(tile) * tile;
    let mut crop_x = trunc_u32((f64::from(display_w) * f64::from(edges.left.max(0.0))).round());
    let mut crop_y = trunc_u32((f64::from(display_h) * f64::from(edges.top.max(0.0))).round());
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
pub fn screen_plan(input: ScreenBudget) -> ScreenPlan {
    let tiles_x = input.tiles_x.max(1);
    let tiles_y = input.tiles_y.max(1);
    let tiles = tiles_x.saturating_mul(tiles_y);
    let direct = budget_mode_gates(input.shadow_grant, input.shadow_demand, true, input.scaled);
    let atrous = input.changes > 0 && input.denoise_grant < input.denoise_demand;
    let trace = budget_mode_gates(input.trace_grant, input.trace_demand, input.changes > 0, input.scaled);
    let cache_ray_grant = if input.scaled && input.cache_rays > 0 {
        input.cache_rays
    } else {
        0
    };
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
        tiles_for_grant(tiles, input.shadow_grant, input.shadow_demand)
    } else if atrous {
        tiles_for_grant(tiles, input.denoise_grant, input.denoise_demand)
    } else {
        tiles_for_grant(tiles, input.trace_grant, input.trace_demand)
    };
    let (origin_x, origin_y, span_w, span_h, band_y, band_h) =
        refresh_rect(&input, tiles_x, tiles_y, grant_tiles);
    ScreenPlan {
        direct_gate: direct,
        atrous_gate: atrous,
        trace_gate: trace,
        tile_x: origin_x,
        tile_y: origin_y,
        tile_w: span_w,
        tile_h: span_h,
        starve_y: band_y,
        starve_h: band_h,
        cache_ray_grant,
    }
}

fn turn_sign(toward: f32, away: f32) -> i32 {
    const EDGE: f32 = 0.001;
    if toward > away + EDGE {
        1
    } else if away > toward + EDGE {
        -1
    } else {
        0
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
    u32::try_from(share.max(1).min(u64::from(tiles))).unwrap_or(u32::MAX)
}

/// Screen tiles are below this floor when they still need a first refresh.
const SCREEN_FLOOR: QualityFloor = QualityFloor {
    max_level: 0,
    min_converged: 0.35,
};

/// Off-display distance `zone_of` treats as the turn-side shell.
const TURN_NEAR_M: f32 = 4.0;

fn refresh_rect(input: &ScreenBudget, tiles_x: u32, tiles_y: u32, grant_tiles: u32) -> (u32, u32, u32, u32, u32, u32) {
    let display = display_window(input, tiles_x, tiles_y);
    let items = screen_items(tiles_x, tiles_y, display, input.turn_x, input.turn_y);
    let picks = select_refresh(&items, grant_tiles, SCREEN_FLOOR);
    let mut viewport_n = 0u32;
    let mut near_n = 0u32;
    for index in &picks {
        match items.get(usize_from_u32(*index)).map(|item| item.zone) {
            Some(ItemZone::Viewport) => viewport_n = viewport_n.saturating_add(1),
            Some(ItemZone::Near) => near_n = near_n.saturating_add(1),
            _ => {}
        }
    }
    let reserve = grant_tiles / 10;
    let tiles = tiles_x.saturating_mul(tiles_y);
    let picked = u32::try_from(picks.len()).unwrap_or(u32::MAX);
    let refine_n = picked.saturating_sub(viewport_n).saturating_sub(near_n);
    // The shader band is the oldest-viewport reserve. Near picks grow the turn
    // edge instead of sitting inside this centered rect.
    let base = viewport_n
        .saturating_add(refine_n)
        .saturating_sub(reserve)
        .max(1)
        .min(tiles);
    let (mut span_w, mut span_h) = rect_wh(tiles_x, tiles_y, base);
    let (dx, dy, dw, dh) = display;
    let mut origin_x = center_origin(dx, dw, span_w, tiles_x);
    let mut origin_y = center_origin(dy, dh, span_h, tiles_y);
    if near_n > 0 {
        let mut rect = TileRect {
            x: origin_x,
            y: origin_y,
            w: span_w,
            h: span_h,
        };
        extend_turn(&mut rect, tiles_x, tiles_y, (input.turn_x, input.turn_y), near_n);
        origin_x = rect.x;
        origin_y = rect.y;
        span_w = rect.w;
        span_h = rect.h;
    }
    let band_h = if reserve == 0 {
        0
    } else {
        reserve.div_ceil(tiles_x).clamp(1, tiles_y)
    };
    let band_y = if band_h == 0 {
        0
    } else {
        (input.frame % tiles_y) % tiles_y.saturating_sub(band_h).max(1)
    };
    (origin_x, origin_y, span_w, span_h, band_y, band_h)
}

fn display_window(input: &ScreenBudget, tiles_x: u32, tiles_y: u32) -> (u32, u32, u32, u32) {
    if input.display_tiles_w == 0 || input.display_tiles_h == 0 {
        return (0, 0, tiles_x, tiles_y);
    }
    let w = input.display_tiles_w.min(tiles_x).max(1);
    let h = input.display_tiles_h.min(tiles_y).max(1);
    let x = input.display_x.min(tiles_x.saturating_sub(w));
    let y = input.display_y.min(tiles_y.saturating_sub(h));
    (x, y, w, h)
}

fn center_origin(window: u32, window_span: u32, rect_span: u32, tiles: u32) -> u32 {
    let raw = if rect_span >= window_span {
        window.saturating_sub((rect_span - window_span) / 2)
    } else {
        window + (window_span - rect_span) / 2
    };
    raw.min(tiles.saturating_sub(rect_span))
}

fn screen_items(tiles_x: u32, tiles_y: u32, display: (u32, u32, u32, u32), turn_x: i32, turn_y: i32) -> Vec<WorkItem> {
    let (dx, dy, dw, dh) = display;
    let cx = dx + dw / 2;
    let cy = dy + dh / 2;
    let mut items = Vec::with_capacity(usize_from_u32(tiles_x.saturating_mul(tiles_y)));
    let mut index = 0u32;
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let in_display = tx >= dx && tx < dx.saturating_add(dw) && ty >= dy && ty < dy.saturating_add(dh);
            let zone = if in_display {
                ItemZone::Viewport
            } else {
                let on_turn = tile_on_turn(tx, ty, display, (turn_x, turn_y));
                let distance = if on_turn { 1.0 } else { TURN_NEAR_M + 1.0 };
                zone_of(0.0, distance, TURN_NEAR_M, 0.0)
            };
            let below = zone != ItemZone::Far;
            let dist_x = tx.abs_diff(cx);
            let dist_y = ty.abs_diff(cy);
            items.push(WorkItem {
                index,
                zone,
                level: 0,
                converged: if below { 0.0 } else { 1.0 },
                change_mag: screen_change_mag(tx, ty, display, turn_x, turn_y, in_display),
                distance_m: tile_f32(dist_x.max(dist_y)),
                age_frames: dist_x.max(dist_y),
            });
            index = index.saturating_add(1);
        }
    }
    items
}

fn tile_f32(value: u32) -> f32 {
    f32::from(u16::try_from(value).unwrap_or(u16::MAX))
}

/// Turn exposure for one screen tile. A still camera is 0. A tile outside the
/// display on the turn side is 1 (newly revealed). Inside the display, the
/// value is the fraction toward the leading edge. Off the turn it stays 0.
/// This is not an irradiance delta. There is no per-tile light on the CPU.
fn screen_change_mag(
    tx: u32,
    ty: u32,
    display: (u32, u32, u32, u32),
    turn_x: i32,
    turn_y: i32,
    in_display: bool,
) -> f32 {
    if turn_x == 0 && turn_y == 0 {
        return 0.0;
    }
    if !in_display {
        return if tile_on_turn(tx, ty, display, (turn_x, turn_y)) {
            1.0
        } else {
            0.0
        };
    }
    let (dx, dy, dw, dh) = display;
    let mut mag = 0.0f32;
    if turn_x != 0 {
        mag = mag.max(edge_fraction(tx, dx, dw, turn_x > 0));
    }
    if turn_y != 0 {
        mag = mag.max(edge_fraction(ty, dy, dh, turn_y > 0));
    }
    mag.clamp(0.0, 1.0)
}

fn edge_fraction(tile: u32, origin: u32, span: u32, toward_high: bool) -> f32 {
    let span = span.max(1);
    if span == 1 {
        return 1.0;
    }
    let along = if toward_high {
        tile.saturating_sub(origin)
    } else {
        origin.saturating_add(span - 1).saturating_sub(tile)
    };
    (tile_f32(along) / tile_f32(span - 1)).clamp(0.0, 1.0)
}

fn tile_on_turn(tx: u32, ty: u32, display: (u32, u32, u32, u32), turn: (i32, i32)) -> bool {
    let (dx, dy, dw, dh) = display;
    let (turn_x, turn_y) = turn;
    if turn_x == 0 && turn_y == 0 {
        return false;
    }
    let right = tx >= dx.saturating_add(dw);
    let left = tx < dx;
    let up = ty >= dy.saturating_add(dh);
    let down = ty < dy;
    let x_hit = (turn_x > 0 && right) || (turn_x < 0 && left);
    let y_hit = (turn_y > 0 && up) || (turn_y < 0 && down);
    x_hit || y_hit
}

struct TileRect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

fn extend_turn(rect: &mut TileRect, tiles_x: u32, tiles_y: u32, turn: (i32, i32), near_n: u32) {
    let (turn_x, turn_y) = turn;
    if turn_x != 0 {
        let add = near_n.div_ceil(rect.h.max(1)).max(1);
        if turn_x > 0 {
            let room = tiles_x.saturating_sub(rect.x.saturating_add(rect.w));
            rect.w = rect.w.saturating_add(add.min(room));
        } else {
            let shift = add.min(rect.x);
            rect.x = rect.x.saturating_sub(shift);
            rect.w = rect.w.saturating_add(shift);
        }
    }
    if turn_y != 0 {
        let add = near_n.div_ceil(rect.w.max(1)).max(1);
        if turn_y > 0 {
            let room = tiles_y.saturating_sub(rect.y.saturating_add(rect.h));
            rect.h = rect.h.saturating_add(add.min(room));
        } else {
            let shift = add.min(rect.y);
            rect.y = rect.y.saturating_sub(shift);
            rect.h = rect.h.saturating_add(shift);
        }
    }
}

fn rect_wh(tiles_x: u32, tiles_y: u32, count: u32) -> (u32, u32) {
    if count >= tiles_x.saturating_mul(tiles_y) {
        return (tiles_x, tiles_y);
    }
    let aspect = f64::from(tiles_x) / f64::from(tiles_y.max(1));
    let mut h = trunc_u32(((f64::from(count) / aspect).sqrt()).floor().max(1.0));
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

/// Push constants for one frame. `tail` is display size, the spawn cap, and the coarse-slot cap.
/// A zero display size keeps the symmetric eye ray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SchedulePush {
    pub words: [u32; 4],
    pub tail: [u32; 4],
    pub plan: ScreenPlan,
    pub lit: LitFrame,
    pub scale: f64,
    pub lighting_cap_ms: f32,
}

impl Default for SchedulePush {
    fn default() -> Self {
        Self {
            words: [0; 4],
            tail: [0; 4],
            plan: clear_plan(),
            lit: LitFrame {
                lit_w: 0,
                lit_h: 0,
                crop_x: 0,
                crop_y: 0,
                display_w: 0,
                display_h: 0,
                left: 0.0,
                right: 0.0,
                top: 0.0,
                bottom: 0.0,
            },
            scale: 1.0,
            lighting_cap_ms: 8.0,
        }
    }
}

fn clear_plan() -> ScreenPlan {
    ScreenPlan {
        direct_gate: false,
        atrous_gate: false,
        trace_gate: false,
        tile_x: 0,
        tile_y: 0,
        tile_w: 0,
        tile_h: 0,
        starve_y: 0,
        starve_h: 0,
        cache_ray_grant: 0,
    }
}

/// Work-cap multiply. `scale` may rise to 4 for the test knob. Zero stays zero.
#[must_use]
pub fn grant_u32(value: u32, scale: f64) -> u32 {
    if value == 0 || !scale.is_finite() || scale <= 0.0 {
        return 0;
    }
    let scaled = (f64::from(value) * clamp_scale(scale, 4.0)).floor();
    if scaled <= 1.0 {
        1
    } else {
        trunc_u32(scaled)
    }
}

/// Lit picture for this camera step. Global mode and a zero guard stay on the display.
#[must_use]
pub fn lit_target(
    settings: &genos_scene::LightingSettings,
    display_w: u32,
    display_h: u32,
    tile: u32,
    yaw_delta: f32,
    pitch_delta: f32,
) -> LitFrame {
    let display_w = display_w.max(1);
    let display_h = display_h.max(1);
    let tile = tile.max(1);
    let edge = settings.guard_edge;
    let edges = if settings.schedule == genos_scene::ScheduleMode::Global
        || !edge.is_finite()
        || edge <= 0.0
    {
        GuardEdges {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        }
    } else {
        bias_guard(settings.guard_edge, yaw_delta, pitch_delta)
    };
    lit_frame(display_w, display_h, edges, tile)
}

/// Gates, crop, and caps for the frame. Global mode pushes zeros.
#[must_use]
pub fn schedule_push(
    settings: &genos_scene::LightingSettings,
    controller_scale: f64,
    lit: LitFrame,
    tile: u32,
    probe_rays: u32,
    changes: u32,
    frame: u32,
) -> SchedulePush {
    let preset = settings.preset_label();
    let caps = caps_for(preset);
    if settings.schedule == genos_scene::ScheduleMode::Global {
        let display = lit_frame(
            lit.display_w.max(1),
            lit.display_h.max(1),
            GuardEdges {
                left: 0.0,
                right: 0.0,
                top: 0.0,
                bottom: 0.0,
            },
            1,
        );
        return SchedulePush {
            words: [0; 4],
            tail: [0; 4],
            plan: clear_plan(),
            lit: display,
            scale: 1.0,
            lighting_cap_ms: caps.lighting_ms,
        };
    }
    let knob = if settings.budget_scale.is_finite() {
        f64::from(settings.budget_scale.clamp(SCALE_FLOOR, 4.0))
    } else {
        1.0
    };
    // Off holds the preset caps. `budget_scale` still multiplies them.
    let ctrl = if !settings.controller {
        1.0
    } else if controller_scale.is_finite() {
        controller_scale.clamp(f64::from(SCALE_FLOOR), 1.0)
    } else {
        1.0
    };
    let scale = (ctrl * knob).clamp(f64::from(SCALE_FLOOR), 4.0);
    let scaled = scale < 0.999;
    let tile = tile.max(1);
    let tiles_x = lit.lit_w.div_ceil(tile).max(1);
    let tiles_y = lit.lit_h.div_ceil(tile).max(1);
    let pixels = lit.lit_w.saturating_mul(lit.lit_h);
    let probes = tiles_x.saturating_mul(tiles_y);
    let trace_demand = if settings.gi.enabled && settings.probes.enabled {
        probes.saturating_mul(probe_rays)
    } else {
        0
    };
    let denoise_demand = if settings.denoiser.enabled { pixels } else { 0 };
    let plan = screen_plan(ScreenBudget {
        tiles_x,
        tiles_y,
        shadow_grant: grant_u32(caps.fresh_shadow_rays, scale),
        shadow_demand: pixels,
        denoise_grant: grant_u32(caps.denoise_items, scale),
        denoise_demand,
        trace_grant: grant_u32(caps.screen_probe_rays, scale),
        trace_demand,
        changes,
        scaled,
        frame,
        cache_rays: grant_u32(caps.cache_rays, scale),
        display_x: lit.crop_x / tile,
        display_y: lit.crop_y / tile,
        display_tiles_w: lit.display_w.div_ceil(tile),
        display_tiles_h: lit.display_h.div_ceil(tile),
        turn_x: turn_sign(lit.right, lit.left),
        turn_y: turn_sign(lit.top, lit.bottom),
    });
    let mut words = plan.words(lit.crop_x, lit.crop_y);
    // Bit 3: detail levels (25 cm through 4 m). The shader also requires world-cache mode 3.
    words[0] |= 1 << 3;
    if plan.cache_ray_grant > 0 {
        words[0] |= plan.cache_ray_grant.min(0x00ff_ffff) << 8;
    }
    let coarse_slots = u32::try_from(coarse_cap_bytes(preset) / 368).unwrap_or(u32::MAX);
    SchedulePush {
        words,
        tail: [
            lit.display_w,
            lit.display_h,
            grant_u32(caps.spawns, scale),
            coarse_slots,
        ],
        plan,
        lit,
        scale,
        lighting_cap_ms: caps.lighting_ms,
    }
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
    fn a_turn_writes_the_leading_change_and_a_still_camera_writes_none() {
        let display = (2u32, 1u32, 4u32, 2u32);
        let still = screen_items(8, 4, display, 0, 0);
        assert!(still.iter().all(|item| item.change_mag == 0.0));
        let turning = screen_items(8, 4, display, 1, 0);
        let at = |tx: u32, ty: u32| {
            turning
                .iter()
                .find(|item| item.index == ty * 8 + tx)
                .map(|item| item.change_mag)
                .unwrap_or(-1.0)
        };
        assert!((at(6, 1) - 1.0).abs() < 1.0e-4, "turn-side tile");
        assert_eq!(at(0, 1), 0.0, "off the turn");
        assert!(at(5, 1) > at(2, 1), "leading edge ranks above the trailing edge");
        assert!((at(5, 1) - 1.0).abs() < 1.0e-4);
        assert!(at(2, 1).abs() < 1.0e-4);
    }

    #[test]
    fn guard_bias_splits_the_axis_and_leads_the_turn() {
        let full = bias_guard(0.15, std::f32::consts::PI, 0.0);
        assert!((full.left - 0.075).abs() < 1.0e-4);
        assert!((full.right - 0.175).abs() < 1.0e-4);
        let slow = bias_guard(0.15, 0.05, 0.0);
        assert!(slow.right > slow.left);
        assert!(slow.left + slow.right < 0.17, "0.05 rad no longer takes the whole extra");
        let still = bias_guard(0.15, 0.0, 0.0);
        assert!((still.left - still.right).abs() < 1.0e-4);
        assert!((still.top + still.bottom - 0.15).abs() < 1.0e-4);
    }

    #[test]
    fn a_full_turn_grows_only_the_leading_edge() {
        let tile = 16;
        let still = lit_frame(320, 180, bias_guard(0.15, 0.0, 0.0), tile);
        let turning = lit_frame(320, 180, bias_guard(0.15, std::f32::consts::PI, 0.0), tile);
        assert!(turning.lit_w > still.lit_w);
        assert_eq!(still.lit_h, turning.lit_h);
        assert_eq!(still.crop_x % tile, 0);
        assert_eq!(turning.crop_x % tile, 0);
    }

    #[test]
    fn high_guard_stays_inside_the_area_band() {
        let tile = 16;
        let still = lit_frame(1920, 1080, bias_guard(0.15, 0.0, 0.0), tile);
        let turning = lit_frame(1920, 1080, bias_guard(0.15, std::f32::consts::PI, 0.0), tile);
        let display = 1920.0_f64 * 1080.0;
        let still_area = f64::from(still.lit_w) * f64::from(still.lit_h) / display;
        let turn_area = f64::from(turning.lit_w) * f64::from(turning.lit_h) / display;
        assert!(still_area > 1.25 && still_area < 1.40, "{still_area}");
        assert!(turn_area > still_area && turn_area < 1.50, "{turn_area}");
        assert_eq!(turning.lit_h, still.lit_h);
    }

    #[test]
    fn trunc_u32_matches_the_integer_part() {
        assert_eq!(trunc_u32(0.0), 0);
        assert_eq!(trunc_u32(-2.0), 0);
        assert_eq!(trunc_u32(f64::NAN), 0);
        assert_eq!(trunc_u32(f64::INFINITY), u32::MAX);
        assert_eq!(trunc_u32(0.9), 0);
        assert_eq!(trunc_u32(1.0), 1);
        assert_eq!(trunc_u32(1.9), 1);
        assert_eq!(trunc_u32(2.0), 2);
        assert_eq!(trunc_u32(255.9), 255);
        assert_eq!(trunc_u32(65_536.0), 65_536);
        assert_eq!(trunc_u32(1_000_000.7), 1_000_000);
        assert_eq!(trunc_u32(16_777_216.0), 16_777_216);
        assert_eq!(trunc_u32(2_147_483_647.5), 2_147_483_647);
        assert_eq!(trunc_u32(4_294_967_295.0), u32::MAX);
    }

    #[test]
    fn the_controller_scales_down_and_does_not_open_the_frame() {
        let mut controller = BudgetController::new();
        controller.observe(None, 8.0);
        assert!((controller.scale - 1.0).abs() < 1.0e-6);
        controller.observe(Some(16.0), 8.0);
        assert!(controller.scale < 0.6);
        assert!(controller.scale >= f64::from(SCALE_FLOOR));
        controller.scale = f64::from(SCALE_FLOOR) / 2.0;
        controller.observe(Some(1000.0), 8.0);
        assert!((controller.scale - f64::from(SCALE_FLOOR)).abs() < 1.0e-6);
        controller.observe(Some(1.0), 8.0);
        assert!(controller.scale <= 1.0);
    }

    #[test]
    fn a_covered_frame_does_not_gate() {
        let plan = screen_plan(ScreenBudget {
            tiles_x: 20,
            tiles_y: 12,
            shadow_grant: 2_000_000,
            shadow_demand: 57_600,
            denoise_grant: 16_000,
            denoise_demand: 57_600,
            trace_grant: 522_000,
            trace_demand: 15_360,
            changes: 0,
            scaled: false,
            frame: 0,
            cache_rays: 256_000,
            display_x: 0,
            display_y: 0,
            display_tiles_w: 0,
            display_tiles_h: 0,
            turn_x: 0,
            turn_y: 0,
        });
        assert!(plan.gates_off());
        assert_eq!(plan.words(0, 0)[0], 0);
    }

    #[test]
    fn a_mover_gates_denoise_to_a_centered_rect() {
        let plan = screen_plan(ScreenBudget {
            tiles_x: 20,
            tiles_y: 12,
            shadow_grant: 2_000_000,
            shadow_demand: 57_600,
            denoise_grant: 16_000,
            denoise_demand: 57_600,
            trace_grant: 522_000,
            trace_demand: 15_360,
            changes: 1,
            scaled: false,
            frame: 3,
            cache_rays: 256_000,
            display_x: 0,
            display_y: 0,
            display_tiles_w: 0,
            display_tiles_h: 0,
            turn_x: 0,
            turn_y: 0,
        });
        assert!(plan.atrous_gate);
        assert!(!plan.direct_gate);
        assert!(plan.tile_w > 0 && plan.tile_h > 0);
        let center_x = plan.tile_x + plan.tile_w / 2;
        assert!((i64::from(center_x) - 10).abs() <= 1);
        assert!(plan.tile_w * plan.tile_h < 20 * 12);
    }

    #[test]
    fn a_small_grant_centers_on_the_display_and_does_not_grow_into_the_guard() {
        // 128 tiles, grant 8. The display is 24 tiles, so the turn guard is not picked.
        let plan = screen_plan(ScreenBudget {
            tiles_x: 16,
            tiles_y: 8,
            shadow_grant: 8,
            shadow_demand: 128,
            denoise_grant: 1_000_000,
            denoise_demand: 1,
            trace_grant: 1_000_000,
            trace_demand: 1,
            changes: 1,
            scaled: false,
            frame: 0,
            cache_rays: 0,
            display_x: 2,
            display_y: 2,
            display_tiles_w: 6,
            display_tiles_h: 4,
            turn_x: 1,
            turn_y: 0,
        });
        assert!(plan.direct_gate);
        assert_eq!(plan.tile_x, 3);
        assert!(plan.tile_x + plan.tile_w <= 2 + 6);
        let lit_origin = (16 - plan.tile_w) / 2;
        assert_ne!(plan.tile_x, lit_origin);
    }

    #[test]
    fn a_grant_past_the_display_grows_toward_the_turn() {
        let plan = screen_plan(ScreenBudget {
            tiles_x: 16,
            tiles_y: 8,
            shadow_grant: 30,
            shadow_demand: 128,
            denoise_grant: 1_000_000,
            denoise_demand: 1,
            trace_grant: 1_000_000,
            trace_demand: 1,
            changes: 1,
            scaled: false,
            frame: 0,
            cache_rays: 0,
            display_x: 2,
            display_y: 2,
            display_tiles_w: 6,
            display_tiles_h: 4,
            turn_x: 1,
            turn_y: 0,
        });
        assert!(plan.direct_gate);
        let display_right = 2 + 6;
        let rect_right = plan.tile_x + plan.tile_w;
        let right_extra = rect_right.saturating_sub(display_right);
        let left_extra = 2u32.saturating_sub(plan.tile_x);
        assert!(
            right_extra > left_extra,
            "rect {}+{} did not grow right of the display",
            plan.tile_x,
            plan.tile_w
        );
    }

    #[test]
    fn high_caps_match_the_design_table() {
        let caps = caps_for("high");
        assert_eq!(caps.fresh_shadow_rays, 2_000_000);
        assert_eq!(caps.denoise_items, 16_000);
        assert!((caps.lighting_ms - 8.0).abs() < 1.0e-6);
        assert_eq!(caps_for("potato").lighting_ms.to_bits(), 1.5_f32.to_bits());
    }

    fn display(w: u32, h: u32) -> LitFrame {
        lit_frame(
            w,
            h,
            GuardEdges {
                left: 0.0,
                right: 0.0,
                top: 0.0,
                bottom: 0.0,
            },
            16,
        )
    }

    #[test]
    fn a_covered_high_picture_pushes_no_gate() {
        let settings = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::High);
        let push = schedule_push(&settings, 1.0, display(1280, 720), 16, 64, 0, 1);
        assert!(push.plan.gates_off());
        assert_eq!(push.words[0], 1 << 3);
        assert_eq!(push.tail[0], 1280);
        assert_eq!(push.tail[1], 720);
    }

    #[test]
    fn budget_mode_pushes_detail_levels_and_the_coarse_cap() {
        let settings = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::High);
        let push = schedule_push(&settings, 1.0, display(1280, 720), 16, 64, 0, 1);
        assert_eq!(push.words[0] & (1 << 3), 1 << 3);
        assert_eq!(push.tail[2], 8_000);
        assert_eq!(push.tail[3], 24 * 1024 * 1024 / 368);
        let mut global = settings;
        global.schedule = genos_scene::ScheduleMode::Global;
        let off = schedule_push(&global, 1.0, display(1280, 720), 16, 64, 0, 1);
        assert_eq!(off.words[0], 0);
        let potato = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::Potato);
        let none = schedule_push(&potato, 1.0, display(1280, 720), 16, 64, 0, 1);
        assert_eq!(none.tail[2], 0);
        assert_eq!(none.words[0] & (1 << 3), 1 << 3);
    }

    #[test]
    fn a_mover_at_720p_gates_denoise_and_leaves_direct_and_trace() {
        let settings = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::High);
        let push = schedule_push(&settings, 1.0, display(1280, 720), 16, 64, 2, 4);
        assert!(push.plan.atrous_gate);
        assert!(!push.plan.direct_gate);
        assert!(!push.plan.trace_gate);
        assert!(push.plan.tile_w * push.plan.tile_h < 80 * 45);
    }

    #[test]
    fn high_1080p_with_a_change_gates_direct_and_trace() {
        let settings = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::High);
        let push = schedule_push(&settings, 1.0, display(1920, 1080), 16, 64, 1, 1);
        assert!(push.plan.direct_gate);
        assert!(push.plan.trace_gate);
        assert!(push.plan.atrous_gate);
    }

    #[test]
    fn a_held_controller_keeps_the_preset_scale() {
        let mut settings = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::High);
        settings.controller = false;
        settings.budget_scale = 0.5;
        let lit = display(1920, 1080);
        let held = schedule_push(&settings, 0.25, lit, 16, 64, 1, 1);
        let mut scaled = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::High);
        scaled.budget_scale = 0.5;
        let full = schedule_push(&scaled, 1.0, lit, 16, 64, 1, 1);
        assert!((held.scale - 0.5).abs() < 1.0e-6);
        assert_eq!(held.plan.tile_w, full.plan.tile_w);
        assert_eq!(held.plan.tile_h, full.plan.tile_h);
    }

    #[test]
    fn the_controller_scale_shrinks_the_refresh_rect() {
        let settings = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::High);
        let lit = display(1920, 1080);
        let full = schedule_push(&settings, 1.0, lit, 16, 64, 1, 1);
        let half = schedule_push(&settings, 0.5, lit, 16, 64, 1, 1);
        let full_n = full.plan.tile_w * full.plan.tile_h;
        let half_n = half.plan.tile_w * half.plan.tile_h;
        assert!(half_n < full_n);
        assert!((half.scale - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn global_mode_pushes_zero_schedule_words() {
        let mut settings = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::High);
        settings.schedule = genos_scene::ScheduleMode::Global;
        let push = schedule_push(&settings, 0.25, display(1280, 720), 16, 64, 4, 1);
        assert_eq!(push.words, [0; 4]);
        assert_eq!(push.tail, [0; 4]);
        assert!(push.plan.gates_off());
        assert_eq!(push.lit.lit_w, 1280);
        assert_eq!(push.lit.crop_x, 0);
    }

    #[test]
    fn high_guard_grows_the_lit_frame_around_the_display() {
        let settings = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::High);
        let lit = lit_target(&settings, 1280, 720, 16, 0.0, 0.0);
        assert!(lit.lit_w > 1280);
        assert!(lit.lit_h > 720);
        assert_eq!(lit.lit_w % 16, 0);
        assert_eq!(lit.crop_x % 16, 0);
        assert!(lit.crop_x + 1280 <= lit.lit_w);
        assert!(lit.crop_y + 720 <= lit.lit_h);
        let mid = lit.crop_x + 640;
        assert!((i64::from(mid) - i64::from(lit.lit_w) / 2).abs() < 32);
    }
}
