//! Always-on single-ray fill (stages F2 and F3).
//!
//! The world cache is the only store. A cache round spends `fill_budget` rays.
//! F3 water-fills that budget by priority: closeness times remaining error plus
//! `fill_change_weight` times a per-cell lambda. Lambda comes from one same-seed
//! check ray per `fill_check_rate` block and decays when the block is not
//! re-checked. The GPU scheduler in `lighting_fill.glsl` uses these radii, this
//! weight, and this fold. Starting values are not a gate.
//!
//! Keep [`Tunables::high`] in step with `FillSettings::high` in `genos-scene`.

use genos_math::Vec3;

/// Integer to f32 for values the scheduler actually uses (at most 2^24).
fn u_f32(v: u32) -> f32 {
    let mut acc = 0.0f32;
    let mut bit = 1.0f32;
    let mut n = v.min(1 << 24);
    while n > 0 {
        if n & 1 == 1 {
            acc += bit;
        }
        bit += bit;
        n >>= 1;
    }
    acc
}

/// Whole number held exactly by f32, up to 2^24.
fn whole_u32(v: f32) -> u32 {
    let v = v.round().max(0.0);
    if v >= u_f32(1 << 24) {
        return 1 << 24;
    }
    let mut lo = 0u32;
    let mut hi = 1 << 24;
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if u_f32(mid) <= v {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

fn pow2(level: u32) -> f32 {
    2f32.powi(i32::try_from(level.min(31)).unwrap_or(0))
}

fn u_usize(v: u32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// Named F2 tunables. High's starting values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tunables {
    pub budget: u32,
    pub tile: u32,
    pub base_cell: f32,
    pub lod_near: f32,
    pub cell_max: f32,
    pub leash_scale: f32,
    pub look_ahead: f32,
    pub far: f32,
    pub far_share: f32,
    pub offscreen_floor: f32,
    pub error_target: f32,
    pub prox_min: f32,
    pub change_weight: f32,
    /// Cells per check block. 9 is one ray in each 3×3 block.
    pub check_rate: u32,
    /// Most fill rays one cell takes in a round. Water-filling stops here.
    pub max_picks: u32,
    /// Per-frame multiply on a lambda that was not re-measured.
    pub lambda_decay: f32,
    /// Fraction of the ray history kept when lambda is 1. 1 leaves the window.
    pub window_floor: f32,
    /// Highest blend weight the shortened window may use.
    pub catchup_max: f32,
    /// Subtracted, times two, from the relative check delta (A-SVGF 2019).
    pub check_sigma: f32,
    pub warmup_rays: u32,
    pub ray_cap: u32,
    pub frame_dt: f32,
}

impl Tunables {
    #[must_use]
    pub fn high() -> Self {
        Self {
            budget: 100_000,
            tile: 8,
            base_cell: 0.25,
            lod_near: 8.0,
            cell_max: 4.0,
            leash_scale: 1.5,
            look_ahead: 0.5,
            far: 200.0,
            far_share: 0.05,
            offscreen_floor: 0.25,
            error_target: 0.03,
            prox_min: 0.5,
            change_weight: 1.0,
            check_rate: 9,
            max_picks: 4,
            lambda_decay: 0.8,
            window_floor: 0.25,
            catchup_max: 0.5,
            check_sigma: 0.0,
            warmup_rays: 16,
            ray_cap: 512,
            frame_dt: 1.0 / 60.0,
        }
    }

    #[must_use]
    pub fn lod_max(self) -> u32 {
        let ratio = (self.cell_max / self.base_cell.max(1.0e-6)).max(1.0);
        whole_u32(ratio.log2().max(0.0))
    }
}

/// Clipmap level at distance `d` from the create centre.
///
/// `d <= lod_near` is level 0. Past that, `ceil(log2(d / lod_near))`, held at
/// [`Tunables::lod_max`].
#[must_use]
pub fn level_at(d: f32, tun: &Tunables) -> u32 {
    if d <= tun.lod_near {
        return 0;
    }
    let raw = (d / tun.lod_near.max(1.0e-6)).log2().ceil();
    whole_u32(raw.max(0.0)).min(tun.lod_max())
}

#[must_use]
pub fn cell_size(level: u32, tun: &Tunables) -> f32 {
    (tun.base_cell * pow2(level)).min(tun.cell_max)
}

#[must_use]
pub fn create_radius(level: u32, tun: &Tunables) -> f32 {
    if level >= tun.lod_max() {
        tun.far
    } else {
        tun.lod_near * pow2(level)
    }
}

/// Drop radius. Always `leash_scale` times the create radius, including the
/// coarsest level, so a jitter at `fill_far` does not create and drop.
#[must_use]
pub fn leash_radius(level: u32, tun: &Tunables) -> f32 {
    tun.leash_scale * create_radius(level, tun)
}

/// Level of a stored cell from its size. A texel finer than the base cell is level 0.
#[must_use]
pub fn level_of_size(size: f32, tun: &Tunables) -> u32 {
    if size <= tun.base_cell {
        return 0;
    }
    whole_u32((size / tun.base_cell.max(1.0e-6)).log2().max(0.0)).min(tun.lod_max())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Residency {
    Create { level: u32 },
    Keep,
    /// The stored cell is finer than the distance still allows. The survivor is
    /// `level` (coarser). Evict only past the coarsest leash.
    Coarsen { level: u32 },
    Drop,
}

/// Create ahead of `velocity * look_ahead`. A stored cell stays while it is
/// inside its leash plus that same look distance, so the leading edge is not
/// freed on the next visit. Past the leash it coarsens, then drops.
#[must_use]
pub fn residency(
    player: Vec3,
    velocity: Vec3,
    point: Vec3,
    stored: Option<u32>,
    tun: &Tunables,
) -> Residency {
    let look = velocity * tun.look_ahead;
    let extra = look.length();
    let d_player = (point - player).length();
    if let Some(level) = stored {
        let mut target = level.min(tun.lod_max());
        let cap = tun.lod_max();
        while target < cap && d_player > leash_radius(target, tun) + extra {
            target += 1;
        }
        if d_player > leash_radius(target, tun) + extra {
            Residency::Drop
        } else if target > level {
            Residency::Coarsen { level: target }
        } else {
            Residency::Keep
        }
    } else {
        let d_create = (point - (player + look)).length();
        if d_create > tun.far {
            Residency::Drop
        } else {
            Residency::Create {
                level: level_at(d_create, tun),
            }
        }
    }
}

#[must_use]
pub fn closeness(distance: f32, tun: &Tunables) -> f32 {
    1.0 / distance.max(tun.prox_min).max(1.0e-6)
}

/// Relative error past the target. A cell with no rays is not sufficient: its
/// remaining error is 1, the same scale as a fully dirty lambda.
#[must_use]
pub fn remaining_error(mean: f32, deviation: f32, rays: u32, tun: &Tunables) -> f32 {
    if rays == 0 {
        return 1.0;
    }
    let mean = mean.abs().max(1.0e-6);
    let ratio = deviation.max(0.0).sqrt() / mean;
    (ratio - tun.error_target).max(0.0)
}

/// `closeness * (remaining error + change_weight * lambda)`.
#[must_use]
pub fn priority(
    distance: f32,
    mean: f32,
    deviation: f32,
    lambda: f32,
    rays: u32,
    tun: &Tunables,
) -> f32 {
    let lambda = if lambda.is_finite() {
        lambda.clamp(0.0, 1.0)
    } else {
        0.0
    };
    closeness(distance, tun)
        * (remaining_error(mean, deviation, rays, tun) + tun.change_weight * lambda)
}

/// `lighting_hash` in `lighting_common.glsl`. The check seed uses it so a frame
/// number cannot sneak into the direction.
fn hash_u32(mut v: u32) -> u32 {
    v ^= v >> 16;
    v = v.wrapping_mul(0x7feb_352d);
    v ^= v >> 15;
    v = v.wrapping_mul(0x846c_a68b);
    v ^= v >> 16;
    v
}

/// Block side for `fill_check_rate`. 9 → 3, one cell of the 3×3.
#[must_use]
pub fn check_side(rate: u32) -> u32 {
    let rate = rate.max(1);
    let mut side = 1u32;
    while side < 8 && side * side < rate {
        side += 1;
    }
    side
}

/// Same-seed check ray. `frame` chooses which cell in the block is due. It does
/// not change the seed or the stratum.
#[must_use]
pub fn check_ray(cell: u32, _frame: u32) -> (u32, u32) {
    let seed = hash_u32(cell ^ 0x0000_C3E7).max(1);
    (seed, seed % 64)
}

#[must_use]
pub fn check_seed(cell: u32) -> u32 {
    check_ray(cell, 0).0
}

/// Screen tile `(tx, ty)` is this frame's check for its block.
#[must_use]
pub fn check_due(tx: u32, ty: u32, frame: u32, tun: &Tunables) -> bool {
    let side = check_side(tun.check_rate);
    let area = side.saturating_mul(side).max(1);
    let bx = tx / side;
    let by = ty / side;
    let ox = tx % side;
    let oy = ty % side;
    let pick = hash_u32(bx ^ hash_u32(by ^ hash_u32(frame))) % area;
    oy * side + ox == pick
}

/// One resident cell in each group of `fill_check_rate`.
#[must_use]
pub fn check_due_index(index: u32, frame: u32, tun: &Tunables) -> bool {
    let rate = tun.check_rate.max(1);
    let block = index / rate;
    let local = index % rate;
    hash_u32(block ^ hash_u32(frame)) % rate == local
}

/// A-SVGF eq. 6, then `max(0, d - 2 sigma)` (Schied et al. 2019). Sigma 0 is
/// the 2018 ratio.
#[must_use]
pub fn measure_lambda(now: f32, then: f32, sigma: f32) -> f32 {
    if !now.is_finite() || !then.is_finite() || !sigma.is_finite() {
        return 0.0;
    }
    let denom = now.abs().max(then.abs()).max(1.0e-4);
    let delta = ((now - then).abs() / denom).min(1.0);
    (delta - 2.0 * sigma.max(0.0)).clamp(0.0, 1.0)
}

/// A fresh measurement replaces lambda. A skipped check decays it.
#[must_use]
pub fn step_lambda(previous: f32, measured: Option<f32>, tun: &Tunables) -> f32 {
    let decay = if tun.lambda_decay.is_finite() {
        tun.lambda_decay.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let previous = if previous.is_finite() {
        previous.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let decayed = (previous * decay).clamp(0.0, 1.0);
    match measured {
        Some(value) if value.is_finite() => value.clamp(0.0, 1.0),
        Some(_) => 0.0,
        None => decayed,
    }
}

/// Rays this cell may still take. `max_picks` is per cell, so a second tile
/// win does not grant another full set.
#[must_use]
pub fn cell_grant(already: u32, want: u32, max_picks: u32) -> u32 {
    let cap = max_picks.max(1);
    want.min(cap.saturating_sub(already.min(cap)))
}

/// Due indices admitted this frame. The walk starts at `phase` in a table of
/// `span` slots and wraps, taking at most `cap`. The device uses
/// `lighting_round()` as the phase (`checks_from_phase`).
#[must_use]
pub fn checks_from_phase(due: &[u32], span: u32, cap: u32, phase: u32) -> Vec<u32> {
    if span == 0 || cap == 0 || due.is_empty() {
        return Vec::new();
    }
    let phase = phase % span;
    let mut taken = Vec::new();
    for step in 0..span {
        if u32::try_from(taken.len()).unwrap_or(u32::MAX) >= cap {
            break;
        }
        let index = (phase + step) % span;
        if due.binary_search(&index).is_ok() {
            taken.push(index);
        }
    }
    taken
}

/// Check state for one slot. A new key drops the previous occupant's lambda.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckCell {
    pub key: u32,
    pub lambda: f32,
    pub luma: f32,
    pub have: bool,
}

impl CheckCell {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            key: 0,
            lambda: 0.0,
            luma: 0.0,
            have: false,
        }
    }

    pub fn bind(&mut self, key: u32) {
        if self.key != key {
            *self = Self {
                key,
                lambda: 0.0,
                luma: 0.0,
                have: false,
            };
        }
    }

    pub fn note(&mut self, luma: f32, sigma: f32) {
        if !luma.is_finite() {
            self.lambda = if self.lambda.is_finite() {
                self.lambda.clamp(0.0, 1.0)
            } else {
                0.0
            };
            if !self.luma.is_finite() {
                self.luma = 0.0;
                self.have = false;
            }
            return;
        }
        if self.have && self.luma.is_finite() {
            self.lambda = measure_lambda(luma, self.luma, sigma);
        }
        self.luma = luma;
        self.have = true;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TileNeed {
    pub distance: f32,
    pub mean: f32,
    pub deviation: f32,
    pub lambda: f32,
    /// Deposited rays. Zero means the cell is not sufficient yet.
    pub rays: u32,
}

/// A resident clipmap cell the off-screen share can spend a ray on.
#[derive(Clone, Copy, Debug)]
pub struct CellNeed {
    pub distance: f32,
    pub level: u32,
    pub mean: f32,
    pub deviation: f32,
    pub lambda: f32,
    /// Deposited rays. Zero means the cell is not sufficient yet.
    pub rays: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct RayPick {
    pub on_screen: bool,
    /// Reserved for a cell at `lod_max`, or a discovery ray that may only
    /// deposit in that band.
    pub far: bool,
    /// No resident cell took this ray. The direction discovers a surface.
    pub discovery: bool,
    pub dir: Vec3,
    pub tile: Option<usize>,
    pub cell: Option<usize>,
}

fn offscreen_floor_rays(tun: &Tunables) -> u32 {
    whole_u32((u_f32(tun.budget) * tun.offscreen_floor).ceil()).min(tun.budget)
}

fn far_rays(tun: &Tunables) -> u32 {
    whole_u32((u_f32(tun.budget) * tun.far_share).ceil())
}

/// Fibonacci sphere. `n` directions, index `i`, full 360 degrees.
#[must_use]
pub fn fibonacci(i: u32, n: u32) -> Vec3 {
    if n == 0 {
        return Vec3::Y;
    }
    let y = 1.0 - (u_f32(i) + 0.5) / u_f32(n) * 2.0;
    let r = (1.0 - y * y).max(0.0).sqrt();
    let phi = 2.399_963_1 * u_f32(i);
    Vec3::new(r * phi.cos(), y, r * phi.sin())
}

fn cell_score(cell: &CellNeed, tun: &Tunables) -> f32 {
    priority(
        cell.distance,
        cell.mean,
        cell.deviation,
        cell.lambda,
        cell.rays,
        tun,
    )
}

fn tile_score(tile: &TileNeed, tun: &Tunables) -> f32 {
    priority(
        tile.distance,
        tile.mean,
        tile.deviation,
        tile.lambda,
        tile.rays,
        tun,
    )
}

fn sum_u32(values: &[u32]) -> u32 {
    values.iter().copied().fold(0, u32::saturating_add)
}

/// Water-fill `share` rays. Higher log-buckets are filled first, each cell at
/// most `max_picks`. The counts line up with `needs`.
fn fill_counts(needs: &[f32], share: u32, max_picks: u32) -> Vec<u32> {
    let tiles: Vec<crate::water::TileNeed> = needs
        .iter()
        .enumerate()
        .map(|(index, &need)| crate::water::TileNeed {
            id: u32::try_from(index).unwrap_or(u32::MAX),
            need,
        })
        .collect();
    crate::water::assign(&tiles, share, max_picks.max(1))
}

fn push_copies(picks: &mut Vec<RayPick>, count: u32, pick: RayPick) {
    for _ in 0..count {
        picks.push(pick);
    }
}

/// Spend the budget. There is no settle mode: leftover rays discover a surface.
/// On-screen water-fill uses `budget - ceil(budget * offscreen_floor)`. The
/// off-screen share water-fills resident cells by the same priority.
/// `ceil(budget * far_share)` of it is reserved for cells at `lod_max`.
#[must_use]
pub fn schedule(tun: &Tunables, tiles: &[TileNeed], cells: &[CellNeed]) -> Vec<RayPick> {
    let budget = tun.budget;
    if budget == 0 {
        return Vec::new();
    }
    let floor = offscreen_floor_rays(tun);
    let on_cap = budget.saturating_sub(floor);
    let on_needs: Vec<f32> = tiles.iter().map(|tile| tile_score(tile, tun)).collect();
    let on_counts = fill_counts(&on_needs, on_cap, tun.max_picks);
    let on = sum_u32(&on_counts);
    let off = budget.saturating_sub(on);
    let far_n = far_rays(tun).min(off);
    let near_n = off.saturating_sub(far_n);
    let mut far_at = Vec::new();
    let mut near_at = Vec::new();
    for (index, cell) in cells.iter().enumerate() {
        if cell.level == tun.lod_max() {
            far_at.push(index);
        } else {
            near_at.push(index);
        }
    }
    let far_needs: Vec<f32> = far_at
        .iter()
        .map(|&index| cell_score(&cells[index], tun))
        .collect();
    let near_needs: Vec<f32> = near_at
        .iter()
        .map(|&index| cell_score(&cells[index], tun))
        .collect();
    let far_counts = fill_counts(&far_needs, far_n, tun.max_picks);
    let near_counts = fill_counts(&near_needs, near_n, tun.max_picks);
    let mut picks = Vec::with_capacity(u_usize(budget));
    let mut on_order: Vec<usize> = (0..tiles.len()).collect();
    on_order.sort_by(|&a, &b| on_counts[b].cmp(&on_counts[a]).then(a.cmp(&b)));
    for index in on_order {
        push_copies(
            &mut picks,
            on_counts[index],
            RayPick {
                on_screen: true,
                far: false,
                discovery: false,
                dir: Vec3::ZERO,
                tile: Some(index),
                cell: None,
            },
        );
    }
    for (subset, counts) in [(&far_at, &far_counts), (&near_at, &near_counts)] {
        let mut order: Vec<usize> = (0..subset.len()).collect();
        order.sort_by(|&a, &b| counts[b].cmp(&counts[a]).then(a.cmp(&b)));
        for local in order {
            let index = subset[local];
            push_copies(
                &mut picks,
                counts[local],
                RayPick {
                    on_screen: false,
                    far: cells[index].level == tun.lod_max(),
                    discovery: false,
                    dir: Vec3::ZERO,
                    tile: None,
                    cell: Some(index),
                },
            );
        }
    }
    let disc = budget.saturating_sub(u32::try_from(picks.len()).unwrap_or(budget));
    let far_got = sum_u32(&far_counts);
    let far_disc = far_n.saturating_sub(far_got);
    for i in 0..disc {
        picks.push(RayPick {
            on_screen: false,
            far: i < far_disc,
            discovery: true,
            dir: fibonacci(i, disc),
            tile: None,
            cell: None,
        });
    }
    picks
}

/// Plain mean while `rays < warmup`. After that the weight stays positive so a
/// later ray still moves the mean. Lambda shortens the history:
/// `rays * (1 - lambda * (1 - window_floor))`. The sped-up weight stops at
/// `catchup_max`. Lambda 0 is the calm weight.
#[must_use]
pub fn blend_weight(rays: u32, n: u32, lambda: f32, tun: &Tunables) -> f32 {
    let n = u_f32(n.max(1));
    let rays_f = u_f32(rays);
    let lam = if lambda.is_finite() {
        lambda.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let floor = tun.window_floor.clamp(0.0, 1.0);
    let scale = 1.0 - lam * (1.0 - floor);
    let effective = rays_f * scale;
    let calm = n / (rays_f + n);
    let calm = if rays < tun.warmup_rays {
        calm
    } else {
        calm.max(1.0 / 256.0)
    };
    let fast = n / (effective + n);
    let fast = if tun.catchup_max > 0.0 {
        fast.min(tun.catchup_max)
    } else {
        fast
    };
    let cap = tun.catchup_max.max(calm);
    let target = calm.max(fast).min(cap);
    calm + (target - calm) * lam
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimator {
    pub mean: f32,
    pub rays: u32,
}

impl Default for Estimator {
    fn default() -> Self {
        Self::new()
    }
}

impl Estimator {
    #[must_use]
    pub fn new() -> Self {
        Self { mean: 0.0, rays: 0 }
    }

    /// Fold `n` rays whose mean is `sample`. During warm-up this matches `n`
    /// sequential one-ray folds of that same mean. A high `lambda` shortens
    /// the history, so the new sample moves the mean further.
    pub fn fold(&mut self, sample: f32, n: u32, lambda: f32, tun: &Tunables) {
        let n = n.max(1);
        let k = blend_weight(self.rays, n, lambda, tun);
        if self.rays == 0 {
            self.mean = sample;
        } else {
            self.mean += (sample - self.mean) * k;
        }
        let cap = tun.ray_cap.max(tun.warmup_rays);
        self.rays = self.rays.saturating_add(n).min(cap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct Live {
        radiance: f32,
        seen: f32,
        lambda: f32,
        got: u32,
    }

    fn high() -> Tunables {
        Tunables::high()
    }

    #[test]
    fn a_round_spends_at_most_the_budget() {
        let tun = high();
        let tiles: Vec<TileNeed> = (0..200_000)
            .map(|i| TileNeed {
                distance: 1.0 + u_f32(u32::try_from(i % 50).unwrap_or(0)),
                mean: 1.0,
                deviation: 0.2,
                lambda: 0.0,
                rays: 8,
            })
            .collect();
        let picks = schedule(&tun, &tiles, &[]);
        assert!(picks.len() <= u_usize(tun.budget));
        assert_eq!(picks.len(), u_usize(tun.budget));
        let on = picks.iter().filter(|p| p.on_screen).count();
        let off = picks.len() - on;
        let floor = u_usize(whole_u32((u_f32(tun.budget) * tun.offscreen_floor).ceil()));
        assert!(off >= floor);
        assert!(on + off <= u_usize(tun.budget));
    }

    #[test]
    fn an_empty_screen_still_spends_the_budget_off_screen() {
        let tun = high();
        let picks = schedule(&tun, &[], &[]);
        assert_eq!(picks.len(), u_usize(tun.budget));
        assert!(picks.iter().all(|p| !p.on_screen));
    }

    #[test]
    fn a_zero_budget_traces_nothing() {
        let mut tun = high();
        tun.budget = 0;
        assert!(schedule(&tun, &[], &[]).is_empty());
    }

    #[test]
    fn off_screen_rays_cover_every_octant() {
        let mut tun = high();
        tun.budget = 256;
        let picks = schedule(&tun, &[], &[]);
        let mut seen = [false; 8];
        for pick in &picks {
            let d = pick.dir;
            assert!(d.length() > 0.5, "direction {}", d.length());
            let bit = usize::from(d.x >= 0.0)
                | (usize::from(d.y >= 0.0) << 1)
                | (usize::from(d.z >= 0.0) << 2);
            seen[bit] = true;
        }
        assert!(seen.iter().all(|s| *s), "missing octant in {seen:?}");
    }

    #[test]
    fn the_far_bin_is_the_reserved_share() {
        let tun = high();
        let picks = schedule(&tun, &[], &[]);
        let far = picks.iter().filter(|p| p.far).count();
        let want = u_usize(whole_u32((u_f32(tun.budget) * tun.far_share).ceil()));
        assert_eq!(far, want);
    }

    #[test]
    fn the_nearer_tile_wins_the_screen_slot() {
        let mut tun = high();
        tun.budget = 4;
        tun.offscreen_floor = 0.75;
        let tiles = [
            TileNeed {
                distance: 20.0,
                mean: 1.0,
                deviation: 0.25,
                lambda: 0.0,
                rays: 8,
            },
            TileNeed {
                distance: 1.0,
                mean: 1.0,
                deviation: 0.25,
                lambda: 0.0,
                rays: 8,
            },
        ];
        let picks = schedule(&tun, &tiles, &[]);
        let on: Vec<_> = picks.iter().filter(|p| p.on_screen).collect();
        assert_eq!(on.len(), 1);
        assert_eq!(on[0].tile, Some(1));
        assert!(picks.len() <= u_usize(tun.budget));
    }

    #[test]
    fn the_boundary_does_not_create_and_drop() {
        let tun = high();
        let point = Vec3::new(8.0, 0.0, 0.0);
        let mut stored = Some(0u32);
        let mut creates = 0u32;
        let mut drops = 0u32;
        let mut coarsens = 0u32;
        for i in 0..32 {
            let shift = if i % 2 == 0 { 0.0 } else { 0.4 };
            let player = Vec3::new(-shift, 0.0, 0.0);
            match residency(player, Vec3::ZERO, point, stored, &tun) {
                Residency::Create { level } => {
                    creates += 1;
                    stored = Some(level);
                }
                Residency::Drop => {
                    drops += 1;
                    stored = None;
                }
                Residency::Coarsen { level } => {
                    coarsens += 1;
                    stored = Some(level);
                }
                Residency::Keep => {}
            }
        }
        assert_eq!(creates, 0);
        assert_eq!(drops, 0);
        assert_eq!(coarsens, 0);
        assert_eq!(stored, Some(0));
        let past = Vec3::new(-5.0, 0.0, 0.0);
        assert_eq!(
            residency(past, Vec3::ZERO, point, Some(0), &tun),
            Residency::Coarsen { level: 1 }
        );
        assert_eq!(
            residency(Vec3::new(-0.4, 0.0, 0.0), Vec3::ZERO, point, None, &tun),
            Residency::Create { level: 1 }
        );
        assert_eq!(level_at(8.0, &tun), 0);
        assert!((leash_radius(0, &tun) - 12.0).abs() < 1.0e-4);
    }

    #[test]
    fn look_ahead_creates_the_finer_cell_sooner() {
        let tun = high();
        let point = Vec3::new(10.0, 0.0, 0.0);
        let velocity = Vec3::new(4.0, 0.0, 0.0);
        assert_eq!(
            residency(Vec3::ZERO, velocity, point, None, &tun),
            Residency::Create { level: 0 }
        );
        assert_eq!(
            residency(Vec3::ZERO, Vec3::ZERO, point, None, &tun),
            Residency::Create { level: 1 }
        );
    }

    #[test]
    fn one_ray_moves_the_mean() {
        let tun = high();
        let mut est = Estimator::new();
        est.fold(1.0, 1, 0.0, &tun);
        assert!((est.mean - 1.0).abs() < 1.0e-6);
        est.fold(0.0, 1, 0.0, &tun);
        assert!((est.mean - 0.5).abs() < 1.0e-5);
        assert_eq!(est.rays, 2);
    }

    #[test]
    fn an_n_ray_fold_matches_sequential_rays_during_warmup() {
        let tun = high();
        let mut seq = Estimator::new();
        for sample in [1.0, 0.0, 0.0, 0.0] {
            seq.fold(sample, 1, 0.0, &tun);
        }
        let mut once = Estimator::new();
        once.fold(0.25, 4, 0.0, &tun);
        assert!((seq.mean - 0.25).abs() < 1.0e-5);
        assert!((once.mean - seq.mean).abs() < 1.0e-5);
        assert_eq!(once.rays, 4);
    }

    #[test]
    fn a_ray_after_warmup_still_moves_the_mean() {
        let tun = high();
        let mut est = Estimator {
            mean: 1.0,
            rays: tun.warmup_rays,
        };
        let before = est.mean;
        est.fold(0.0, 1, 0.0, &tun);
        assert!(est.mean < before);
        assert!(blend_weight(tun.warmup_rays, 1, 0.0, &tun) > 0.0);
    }

    #[test]
    fn a_high_lambda_shortens_the_window() {
        let tun = high();
        let mut calm = Estimator {
            mean: 1.0,
            rays: tun.warmup_rays,
        };
        let mut hot = calm;
        calm.fold(0.0, 1, 0.0, &tun);
        hot.fold(0.0, 1, 1.0, &tun);
        assert!(hot.mean < calm.mean);
        let sped = blend_weight(tun.warmup_rays, 1, 1.0, &tun);
        let still = blend_weight(tun.warmup_rays, 1, 0.0, &tun);
        assert!(sped > still);
        assert!(sped <= tun.catchup_max + 1.0e-5);
    }

    #[test]
    fn lambda_decays_to_nothing_on_a_static_scene() {
        let tun = high();
        let mut lambda = 1.0f32;
        for _ in 0..40 {
            let measured = measure_lambda(1.0, 1.0, tun.check_sigma);
            assert!(measured < 1.0e-5);
            lambda = step_lambda(lambda, Some(measured), &tun);
        }
        assert!(lambda < 1.0e-3);
        lambda = 1.0;
        for _ in 0..40 {
            lambda = step_lambda(lambda, None, &tun);
        }
        assert!(lambda < 1.0e-3);
    }

    #[test]
    fn a_non_finite_sample_clears_lambda() {
        let tun = high();
        assert_eq!(measure_lambda(f32::NAN, 1.0, 0.0), 0.0);
        assert_eq!(measure_lambda(1.0, f32::INFINITY, 0.0), 0.0);
        assert_eq!(step_lambda(f32::NAN, None, &tun), 0.0);
        assert_eq!(step_lambda(1.0, Some(f32::NAN), &tun), 0.0);
        let mut cell = CheckCell::empty();
        cell.bind(4);
        cell.note(1.0, 0.0);
        cell.note(f32::NAN, 0.0);
        assert!(cell.lambda.is_finite());
        assert!(cell.luma.is_finite());
        cell.note(1.0, 0.0);
        assert!(cell.lambda < 1.0e-5);
        cell.note(f32::INFINITY, 0.0);
        assert_eq!(cell.luma, 1.0, "an infinite sample must not replace the stored luma");
        assert!(cell.lambda < 1.0e-5);
        cell.note(1.0, 0.0);
        assert!(cell.lambda < 1.0e-5, "the sample after an infinity is not a change");
    }

    #[test]
    fn a_reused_slot_drops_the_previous_lambda() {
        let mut cell = CheckCell::empty();
        cell.bind(1);
        cell.note(1.0, 0.0);
        cell.note(4.0, 0.0);
        assert!(cell.lambda > 0.5);
        cell.bind(2);
        assert_eq!(cell.lambda, 0.0);
        assert!(!cell.have);
        cell.note(4.0, 0.0);
        assert_eq!(cell.lambda, 0.0, "the first sample of a new key is not a change");
        cell.note(4.0, 0.0);
        assert!(cell.lambda < 1.0e-5);
    }

    #[test]
    fn one_cell_cannot_spend_a_grant_per_tile() {
        let max_picks = 4;
        let mut already = 0;
        let mut total = 0;
        for _ in 0..3 {
            let grant = cell_grant(already, max_picks, max_picks);
            total += grant;
            already += grant;
        }
        assert_eq!(total, max_picks);
        assert_eq!(cell_grant(max_picks, max_picks, max_picks), 0);
    }

    #[test]
    fn check_rays_reach_the_tail_of_the_table() {
        let span = 30u32;
        let due: Vec<u32> = (0..span).filter(|index| index % 3 == 0).collect();
        let cap = 2u32;
        let mut seen = vec![0u32; usize::try_from(span).unwrap_or(0)];
        for phase in 0..span {
            let taken = checks_from_phase(&due, span, cap, phase);
            assert!(u32::try_from(taken.len()).unwrap_or(u32::MAX) <= cap);
            for index in taken {
                let at = usize::try_from(index).unwrap_or(0);
                seen[at] += 1;
            }
        }
        for index in due {
            let at = usize::try_from(index).unwrap_or(0);
            assert!(seen[at] > 0, "index {index} was never checked");
        }
    }

    #[test]
    fn check_rays_reuse_the_seed() {
        let tun = high();
        assert_eq!(check_side(9), 3);
        assert_eq!(check_ray(3, 0), check_ray(3, 50));
        assert_eq!(check_seed(3), check_ray(3, 7).0);
        let mut seen = 0u32;
        let mut ray_seed = 0u32;
        for frame in 0..36 {
            if !check_due(0, 0, frame, &tun) {
                continue;
            }
            let (next, stratum) = check_ray(0, frame);
            if seen == 0 {
                ray_seed = next;
            } else {
                assert_eq!(next, ray_seed);
            }
            assert_eq!(stratum, ray_seed % 64);
            seen += 1;
        }
        assert!(seen >= 2, "the corner tile was due {seen} times");
        let mut index_hits = 0u32;
        for frame in 0..9 {
            let mut due = 0u32;
            for index in 0..9 {
                if check_due_index(index, frame, &tun) {
                    due += 1;
                }
            }
            assert_eq!(due, 1);
            if check_due_index(4, frame, &tun) {
                index_hits += 1;
                assert_eq!(check_ray(4, frame).0, check_seed(4));
            }
        }
        assert_eq!(index_hits, 1);
    }

    #[test]
    fn a_moved_cell_gets_more_rays_than_a_static_one() {
        let mut tun = high();
        tun.budget = 9;
        tun.offscreen_floor = 1.0;
        tun.far_share = 0.0;
        tun.max_picks = 4;
        tun.check_rate = 9;
        tun.change_weight = 1.0;
        let mut cells = [Live {
            radiance: 1.0,
            seen: 1.0,
            lambda: 0.0,
            got: 0u32,
        }; 9];
        for frame in 0..36u32 {
            let mut checks = 0u32;
            for (index, cell) in cells.iter_mut().enumerate() {
                let at = u32::try_from(index).unwrap_or(0);
                if index == 0 {
                    cell.radiance = 1.0 + u_f32(frame + 1);
                }
                if check_due_index(at, frame, &tun) {
                    let measured = measure_lambda(cell.radiance, cell.seen, tun.check_sigma);
                    cell.lambda = step_lambda(cell.lambda, Some(measured), &tun);
                    cell.seen = cell.radiance;
                    cell.got = cell.got.saturating_add(1);
                    checks = checks.saturating_add(1);
                    let _ = check_ray(at, frame);
                } else {
                    cell.lambda = step_lambda(cell.lambda, None, &tun);
                }
            }
            let needs: Vec<CellNeed> = cells
                .iter()
                .map(|cell| CellNeed {
                    distance: 2.0,
                    level: 0,
                    mean: 1.0,
                    deviation: 0.04,
                    lambda: cell.lambda,
                    rays: 32,
                })
                .collect();
            let mut fill = tun;
            fill.budget = tun.budget.saturating_sub(checks);
            let picks = schedule(&fill, &[], &needs);
            assert!(
                u32::try_from(picks.len())
                    .unwrap_or(u32::MAX)
                    .saturating_add(checks)
                    <= tun.budget
            );
            for pick in &picks {
                if let Some(index) = pick.cell {
                    cells[index].got = cells[index].got.saturating_add(1);
                }
            }
        }
        let moved = cells[0].got;
        let static_max = cells[1..].iter().map(|cell| cell.got).max().unwrap_or(0);
        assert!(moved > static_max, "moved {moved} static {static_max}");
        assert!(cells.iter().all(|cell| cell.lambda < 1.0));
    }

    #[test]
    fn a_converged_screen_still_spends_the_budget() {
        let mut tun = high();
        tun.budget = 32;
        let tiles = vec![
            TileNeed {
                distance: 1.0,
                mean: 1.0,
                deviation: 0.0,
                lambda: 0.0,
                rays: 64,
            };
            10
        ];
        let picks = schedule(&tun, &tiles, &[]);
        assert_eq!(picks.len(), u_usize(tun.budget));
        assert!(picks.iter().all(|pick| !pick.on_screen));
    }

    #[test]
    fn the_far_share_picks_coarse_cells() {
        let mut tun = high();
        tun.budget = 8;
        tun.far_share = 0.5;
        tun.offscreen_floor = 1.0;
        let mut cells = vec![CellNeed {
            distance: 2.0,
            level: 0,
            mean: 1.0,
            deviation: 1.0,
            lambda: 0.0,
            rays: 8,
        }];
        let coarse = tun.lod_max();
        for i in 0..3u32 {
            cells.push(CellNeed {
                distance: 80.0 + u_f32(i),
                level: coarse,
                mean: 1.0,
                deviation: 0.05,
                lambda: 0.0,
                rays: 8,
            });
        }
        let picks = schedule(&tun, &[], &cells);
        let far: Vec<_> = picks.iter().filter(|p| p.far && !p.discovery).collect();
        assert!(far.len() >= 3);
        assert!(far
            .iter()
            .all(|p| matches!(p.cell, Some(i) if cells[i].level == coarse)));
        for id in 1..4 {
            assert!(picks.iter().any(|p| p.cell == Some(id)));
        }
        assert!(picks.iter().any(|p| matches!(p.cell, Some(0))));
        let want =
            usize::try_from(whole_u32((u_f32(tun.budget) * tun.far_share).ceil())).unwrap_or(0);
        assert_eq!(picks.iter().filter(|p| p.far).count(), want);
    }

    #[test]
    fn off_screen_rays_prefer_the_closer_cell() {
        let mut tun = high();
        tun.budget = 4;
        tun.offscreen_floor = 1.0;
        tun.far_share = 0.0;
        let cells = [
            CellNeed {
                distance: 30.0,
                level: 2,
                mean: 1.0,
                deviation: 0.5,
                lambda: 0.0,
                rays: 8,
            },
            CellNeed {
                distance: 1.0,
                level: 0,
                mean: 1.0,
                deviation: 0.5,
                lambda: 0.0,
                rays: 8,
            },
        ];
        let picks = schedule(&tun, &[], &cells);
        let cells_picked: Vec<_> = picks.iter().filter_map(|p| p.cell).collect();
        assert_eq!(cells_picked.first().copied(), Some(1));
    }

    #[test]
    fn the_far_edge_does_not_create_and_drop() {
        let tun = high();
        let level = tun.lod_max();
        let point = Vec3::new(tun.far, 0.0, 0.0);
        let mut stored = Some(level);
        let mut creates = 0u32;
        let mut drops = 0u32;
        for i in 0..32 {
            let shift = if i % 2 == 0 { 0.0 } else { 0.4 };
            let player = Vec3::new(-shift, 0.0, 0.0);
            match residency(player, Vec3::ZERO, point, stored, &tun) {
                Residency::Create { .. } => {
                    creates += 1;
                    stored = Some(level);
                }
                Residency::Drop => {
                    drops += 1;
                    stored = None;
                }
                Residency::Coarsen { level: to } => {
                    stored = Some(to);
                }
                Residency::Keep => {}
            }
        }
        assert_eq!(creates, 0);
        assert_eq!(drops, 0);
        assert_eq!(stored, Some(level));
        let outside = Vec3::new(tun.far + 0.4, 0.0, 0.0);
        assert!(matches!(
            residency(Vec3::ZERO, Vec3::ZERO, outside, None, &tun),
            Residency::Drop
        ));
        assert!((leash_radius(level, &tun) - tun.leash_scale * tun.far).abs() < 1.0e-3);
    }

    #[test]
    fn look_ahead_keeps_the_cell_it_just_created() {
        let tun = high();
        let point = Vec3::new(13.0, 0.0, 0.0);
        let velocity = Vec3::new(16.0, 0.0, 0.0);
        assert_eq!(
            residency(Vec3::ZERO, velocity, point, None, &tun),
            Residency::Create { level: 0 }
        );
        assert_eq!(
            residency(Vec3::ZERO, velocity, point, Some(0), &tun),
            Residency::Keep
        );
    }

    #[test]
    fn a_fine_cell_past_its_leash_coarsens_before_it_drops() {
        let tun = high();
        let point = Vec3::new(13.0, 0.0, 0.0);
        assert_eq!(
            residency(Vec3::ZERO, Vec3::ZERO, point, Some(0), &tun),
            Residency::Coarsen { level: 1 }
        );
        let gone = Vec3::new(tun.far * tun.leash_scale + 1.0, 0.0, 0.0);
        assert_eq!(
            residency(Vec3::ZERO, Vec3::ZERO, gone, Some(tun.lod_max()), &tun),
            Residency::Drop
        );
    }

    #[test]
    fn the_clipmap_stops_doubling_at_the_cell_cap() {
        let tun = high();
        assert_eq!(tun.lod_max(), 4);
        assert!((cell_size(0, &tun) - 0.25).abs() < 1.0e-6);
        assert!((cell_size(4, &tun) - 4.0).abs() < 1.0e-4);
        assert!((cell_size(8, &tun) - 4.0).abs() < 1.0e-4);
        assert!((create_radius(4, &tun) - tun.far).abs() < 1.0e-3);
    }
}
