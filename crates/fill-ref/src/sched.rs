//! Always-on single-ray fill (stage F2).
//!
//! The world cache is the only store. A cache round spends `fill_budget` rays:
//! about one need-weighted pick per screen tile, and the rest on a 360-degree
//! clipmap around the player. The GPU scheduler in `lighting_fill.glsl` uses
//! these radii, this weight, and this fold. Starting values are not a gate.
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
    /// Named stub. F2 does not schedule check rays. F3 does.
    pub check_rate: u32,
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

/// F3 hook. F2 passes 0, so the measured-change term is zero.
#[must_use]
pub fn measured_change() -> f32 {
    0.0
}

#[must_use]
pub fn closeness(distance: f32, tun: &Tunables) -> f32 {
    1.0 / distance.max(tun.prox_min).max(1.0e-6)
}

/// `closeness * (remaining error + change_weight * lambda)`.
#[must_use]
pub fn priority(distance: f32, mean: f32, deviation: f32, lambda: f32, tun: &Tunables) -> f32 {
    let mean = mean.abs().max(1.0e-6);
    let ratio = deviation.max(0.0).sqrt() / mean;
    let remaining = (ratio - tun.error_target).max(0.0);
    closeness(distance, tun) * (remaining + tun.change_weight * lambda)
}

#[derive(Clone, Copy, Debug)]
pub struct TileNeed {
    pub distance: f32,
    pub mean: f32,
    pub deviation: f32,
    pub lambda: f32,
}

/// A resident clipmap cell the off-screen share can spend a ray on.
#[derive(Clone, Copy, Debug)]
pub struct CellNeed {
    pub distance: f32,
    pub level: u32,
    pub mean: f32,
    pub deviation: f32,
    pub lambda: f32,
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

fn rank_by_priority(score_of: impl Fn(usize) -> f32, n: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| score_of(b).total_cmp(&score_of(a)).then(a.cmp(&b)));
    order
}

fn cell_score(cell: &CellNeed, tun: &Tunables) -> f32 {
    priority(cell.distance, cell.mean, cell.deviation, cell.lambda, tun)
}

/// Spend the budget. On-screen picks are the highest priorities that fit in
/// `budget - ceil(budget * offscreen_floor)`. The off-screen share ranks
/// resident cells by the same priority. `ceil(budget * far_share)` of it is
/// reserved for cells at `lod_max`. Rays the resident set does not take
/// discover surfaces on a Fibonacci sphere.
#[must_use]
pub fn schedule(tun: &Tunables, tiles: &[TileNeed], cells: &[CellNeed]) -> Vec<RayPick> {
    let budget = tun.budget;
    if budget == 0 {
        return Vec::new();
    }
    let floor = offscreen_floor_rays(tun);
    let on_cap = budget.saturating_sub(floor);
    let tile_order = rank_by_priority(
        |i| {
            priority(
                tiles[i].distance,
                tiles[i].mean,
                tiles[i].deviation,
                tiles[i].lambda,
                tun,
            )
        },
        tiles.len(),
    );
    let on = u32::try_from(tile_order.len()).unwrap_or(u32::MAX).min(on_cap);
    let off = budget - on;
    let far_n = far_rays(tun).min(off);
    let near_n = off - far_n;
    let cell_order = rank_by_priority(|i| cell_score(&cells[i], tun), cells.len());
    let mut chosen = vec![false; cells.len()];
    let mut far_ids = Vec::new();
    for &i in &cell_order {
        if cells[i].level != tun.lod_max() {
            continue;
        }
        far_ids.push(i);
        chosen[i] = true;
        if u32::try_from(far_ids.len()).unwrap_or(u32::MAX) >= far_n {
            break;
        }
    }
    let mut near_ids = Vec::new();
    for &i in &cell_order {
        if chosen[i] {
            continue;
        }
        near_ids.push(i);
        if u32::try_from(near_ids.len()).unwrap_or(u32::MAX) >= near_n {
            break;
        }
    }
    let mut picks = Vec::with_capacity(u_usize(budget));
    for i in 0..on {
        picks.push(RayPick {
            on_screen: true,
            far: false,
            discovery: false,
            dir: Vec3::ZERO,
            tile: Some(tile_order[u_usize(i)]),
            cell: None,
        });
    }
    for id in far_ids.iter().chain(near_ids.iter()) {
        picks.push(RayPick {
            on_screen: false,
            far: cells[*id].level == tun.lod_max(),
            discovery: false,
            dir: Vec3::ZERO,
            tile: None,
            cell: Some(*id),
        });
    }
    let disc = budget.saturating_sub(u32::try_from(picks.len()).unwrap_or(budget));
    let far_disc = far_n.saturating_sub(u32::try_from(far_ids.len()).unwrap_or(0));
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
/// later ray still moves the mean (the GPU uses the MSME catch-up; this floor
/// is the contract the test locks).
#[must_use]
pub fn blend_weight(rays: u32, n: u32, tun: &Tunables) -> f32 {
    let n = u_f32(n.max(1));
    let rays_f = u_f32(rays);
    let plain = n / (rays_f + n);
    if rays < tun.warmup_rays {
        plain
    } else {
        plain.max(1.0 / 256.0)
    }
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
    /// sequential one-ray folds of that same mean.
    pub fn fold(&mut self, sample: f32, n: u32, tun: &Tunables) {
        let n = n.max(1);
        let k = blend_weight(self.rays, n, tun);
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
            },
            TileNeed {
                distance: 1.0,
                mean: 1.0,
                deviation: 0.25,
                lambda: 0.0,
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
        est.fold(1.0, 1, &tun);
        assert!((est.mean - 1.0).abs() < 1.0e-6);
        est.fold(0.0, 1, &tun);
        assert!((est.mean - 0.5).abs() < 1.0e-5);
        assert_eq!(est.rays, 2);
    }

    #[test]
    fn an_n_ray_fold_matches_sequential_rays_during_warmup() {
        let tun = high();
        let mut seq = Estimator::new();
        for sample in [1.0, 0.0, 0.0, 0.0] {
            seq.fold(sample, 1, &tun);
        }
        let mut once = Estimator::new();
        once.fold(0.25, 4, &tun);
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
        est.fold(0.0, 1, &tun);
        assert!(est.mean < before);
        assert!(blend_weight(tun.warmup_rays, 1, &tun) > 0.0);
    }

    #[test]
    fn measured_change_is_a_stub() {
        assert_eq!(measured_change(), 0.0);
        let tun = high();
        let with = priority(1.0, 1.0, 0.25, measured_change(), &tun);
        let without = priority(1.0, 1.0, 0.25, 0.0, &tun);
        assert!((with - without).abs() < 1.0e-6);
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
        }];
        let coarse = tun.lod_max();
        for i in 0..3u32 {
            cells.push(CellNeed {
                distance: 80.0 + u_f32(i),
                level: coarse,
                mean: 1.0,
                deviation: 0.05,
                lambda: 0.0,
            });
        }
        let picks = schedule(&tun, &[], &cells);
        let far: Vec<_> = picks.iter().filter(|p| p.far && !p.discovery).collect();
        assert_eq!(far.len(), 3);
        assert!(far.iter().all(|p| matches!(p.cell, Some(i) if cells[i].level == coarse)));
        assert!(picks.iter().any(|p| matches!(p.cell, Some(0))));
        assert!(picks.iter().filter(|p| p.far).count() >= 3);
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
            },
            CellNeed {
                distance: 1.0,
                level: 0,
                mean: 1.0,
                deviation: 0.5,
                lambda: 0.0,
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
