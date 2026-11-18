//! One GI ray allocator. Every cell has one value. A cell with no rays starts
//! at 1, and the value falls as the mean converges. The fixed budget is spent
//! highest rank first. A trickle floor is reserved inside that budget and is
//! drawn by cells in range, including cells whose value has reached 0.
//!
//! Replicated state: none. The rank stays on the shading client.
//! Authority: the client that shades.
//! Prediction / rollback: none.
//! Determinism: the rank is the value, the distance, and the on-screen weight.
//! The trickle draw is a hash of the cell and the frame.
//! Headless server: a server that does not shade does not run this.

use crate::sched::{closeness, remaining_error, Tunables};
use crate::water::bucket;

/// Off-screen cells stay in the same ranking, below an on-screen cell of the
/// same value and distance.
pub const OFF_SCREEN_WEIGHT: f32 = 0.25;

/// A disagreement past this many standard deviations raises the cell.
const DISAGREE_SIGMA: f32 = 2.0;

#[derive(Clone, Copy, Debug)]
pub struct CellBid {
    pub id: u32,
    pub mean: f32,
    pub deviation: f32,
    pub rays: u32,
    pub lambda: f32,
    pub distance: f32,
    pub on_screen: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grant {
    pub id: u32,
    pub rays: u32,
    pub trickle: bool,
}

/// A new cell asks for rays. A cell that already has rays keeps the remaining
/// error, raised by the measured change.
#[must_use]
pub fn cell_value(cell: &CellBid, tun: &Tunables) -> f32 {
    if cell.rays == 0 {
        return 1.0;
    }
    let err = remaining_error(cell.mean, cell.deviation, cell.rays, tun);
    let lam = if cell.lambda.is_finite() {
        cell.lambda.clamp(0.0, 1.0)
    } else {
        0.0
    };
    (err + tun.change_weight * lam).max(0.0)
}

/// Value, weighted by closeness and by whether the cell is on screen.
#[must_use]
pub fn cell_rank(cell: &CellBid, tun: &Tunables) -> f32 {
    let screen = if cell.on_screen {
        1.0
    } else {
        OFF_SCREEN_WEIGHT
    };
    screen * closeness(cell.distance, tun) * cell_value(cell, tun)
}

/// Rays reserved before the ranked spend. Zero when the floor is 0.
#[must_use]
pub fn trickle_count(budget: u32, fraction: f32) -> u32 {
    if budget == 0 || !fraction.is_finite() || fraction <= 0.0 {
        return 0;
    }
    let fraction = fraction.clamp(0.0, 1.0);
    let target = (f64::from(budget) * f64::from(fraction)).ceil();
    if !target.is_finite() || target <= 0.0 {
        return 0;
    }
    if target >= f64::from(budget) {
        return budget;
    }
    let mut lo = 0u32;
    let mut hi = budget;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if f64::from(mid) < target {
            lo = mid.saturating_add(1);
        } else {
            hi = mid;
        }
    }
    lo
}

fn hash_u32(mut v: u32) -> u32 {
    v ^= v >> 16;
    v = v.wrapping_mul(0x7feb_352d);
    v ^= v >> 15;
    v = v.wrapping_mul(0x846c_a68b);
    v ^= v >> 16;
    v
}

/// The rotating draw. `frame` chooses the turn. It does not change the seed
/// of the ray itself.
#[must_use]
pub fn trickle_due(id: u32, frame: u32, rate: u32) -> bool {
    let rate = rate.max(1);
    hash_u32(id ^ hash_u32(frame)).is_multiple_of(rate)
}

/// True when a fresh sample is outside the noise the stored variance allows.
#[must_use]
pub fn trickle_disagrees(sample: f32, mean: f32, variance: f32) -> bool {
    if !sample.is_finite() || !mean.is_finite() {
        return false;
    }
    let noise = variance.max(0.0).sqrt().max(1.0e-4);
    (sample - mean).abs() > DISAGREE_SIGMA * noise
}

/// The relative step that disagreement writes into the cell's value.
#[must_use]
pub fn disagreement_raise(sample: f32, mean: f32) -> f32 {
    let denom = sample.abs().max(mean.abs()).max(1.0e-4);
    ((sample - mean).abs() / denom).clamp(0.0, 1.0)
}

/// How much of the stored mean to keep. Confidence is one minus the cell's
/// value: a cell that wants many new rays keeps little of its history.
#[must_use]
pub fn history_confidence(value: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    1.0 - value.clamp(0.0, 1.0)
}

/// Blend one new sample into the stored mean. `confidence` is the weight of
/// the history. Zero replaces the mean. One leaves it.
#[must_use]
pub fn blend_with_confidence(mean: f32, confidence: f32, sample: f32) -> f32 {
    let keep = if confidence.is_finite() {
        confidence.clamp(0.0, 1.0)
    } else {
        0.0
    };
    keep * mean + (1.0 - keep) * sample
}

/// One agreement measure. A sample inside the stored noise lowers the value.
/// A sample outside it raises the value by the relative step.
#[must_use]
pub fn apply_agreement(value: f32, sample: f32, mean: f32, variance: f32) -> f32 {
    let value = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if trickle_disagrees(sample, mean, variance) {
        (value + disagreement_raise(sample, mean)).min(1.0)
    } else {
        (value - 1.0 / 3.0).max(0.0)
    }
}

/// One ray per live cell, then whatever of `budget` remains is the burst.
/// The floor is not capped by the budget: a budget smaller than the floor
/// returns the budget and a zero burst.
#[must_use]
pub fn split_floor_and_burst(live_cells: u32, budget: u32, rays_per_cell: u32) -> (u32, u32) {
    let floor = live_cells.saturating_mul(rays_per_cell.max(1));
    if floor >= budget {
        (budget, 0)
    } else {
        (floor, budget - floor)
    }
}

/// Spend `budget` once. Ranked rays go to the highest rank. Trickle rays are
/// the reserved floor, one each, on cells the ranked spend did not take.
#[must_use]
pub fn allocate(cells: &[CellBid], budget: u32, tun: &Tunables, frame: u32) -> Vec<Grant> {
    let mut grants = cells
        .iter()
        .map(|cell| Grant {
            id: cell.id,
            rays: 0,
            trickle: false,
        })
        .collect::<Vec<_>>();
    if budget == 0 || cells.is_empty() {
        return grants;
    }
    if tun.floor_rays == 0 {
        return allocate_fraction(cells, budget, tun, frame, grants);
    }
    let live = u32::try_from(cells.len()).unwrap_or(u32::MAX);
    let (floor_budget, rest) = split_floor_and_burst(live, budget, tun.floor_rays);
    let burst_budget = if tun.burst == 0 { rest } else { rest.min(tun.burst) };
    let mut order: Vec<usize> = (0..cells.len()).collect();
    order.sort_by(|&a, &b| {
        cell_rank(&cells[b], tun)
            .total_cmp(&cell_rank(&cells[a], tun))
            .then_with(|| cells[a].id.cmp(&cells[b].id))
    });
    let per = tun.floor_rays.max(1);
    let mut floor_left = floor_budget;
    for &index in &order {
        if floor_left == 0 {
            break;
        }
        let give = per.min(floor_left);
        grants[index].rays = give;
        grants[index].trickle = cell_value(&cells[index], tun) <= 0.0;
        floor_left -= give;
    }
    let max_per = tun.max_picks.max(1);
    let mut burst_left = burst_budget;
    for &index in &order {
        if burst_left == 0 {
            break;
        }
        if cell_value(&cells[index], tun) <= 0.0 {
            continue;
        }
        let give = max_per.min(burst_left);
        grants[index].rays = grants[index].rays.saturating_add(give);
        grants[index].trickle = false;
        burst_left -= give;
    }
    grants
}

/// The fraction floor. `floor_rays` 0 keeps this path so a zero per-cell
/// floor still draws the reserved share.
fn allocate_fraction(
    cells: &[CellBid],
    budget: u32,
    tun: &Tunables,
    frame: u32,
    mut grants: Vec<Grant>,
) -> Vec<Grant> {
    let floor_n = trickle_count(budget, tun.offscreen_floor);
    let mut left = budget.saturating_sub(floor_n);
    let max_per = tun.max_picks.max(1);
    let mut groups: Vec<Vec<usize>> = (0..64).map(|_| Vec::new()).collect();
    for (index, cell) in cells.iter().enumerate() {
        let value = cell_value(cell, tun);
        if value <= 0.0 {
            continue;
        }
        let level = bucket(cell_rank(cell, tun));
        if level > 0 {
            groups[level].push(index);
        }
    }
    for level in (1..64).rev() {
        if left == 0 {
            break;
        }
        let group = &groups[level];
        if group.is_empty() {
            continue;
        }
        let count = u32::try_from(group.len()).unwrap_or(u32::MAX).max(1);
        let base = (left / count).min(max_per);
        let mut extra = left.saturating_sub(base.saturating_mul(count));
        let mut order = group.clone();
        order.sort_by_key(|&index| cells[index].id);
        for index in order {
            if left == 0 {
                break;
            }
            let mut m = base.min(left);
            if extra > 0 && m < max_per {
                m += 1;
                extra -= 1;
            }
            m = m.min(max_per).min(left);
            grants[index].rays = m;
            left -= m;
        }
    }
    let mut due: Vec<usize> = (0..cells.len())
        .filter(|&index| grants[index].rays == 0)
        .filter(|&index| trickle_due(cells[index].id, frame, tun.check_rate))
        .collect();
    due.sort_by_key(|&index| cells[index].id);
    let mut trickle_left = floor_n;
    for index in due {
        if trickle_left == 0 {
            break;
        }
        grants[index].rays = 1;
        grants[index].trickle = true;
        trickle_left -= 1;
    }
    grants
}

/// Fold `n` samples of `sample` into the mean. Returns the new mean and count.
#[must_use]
pub fn blend_mean(mean: f32, rays: u32, sample: f32, n: u32) -> (f32, u32) {
    if n == 0 {
        return (mean, rays);
    }
    let total = rays.saturating_add(n);
    let rays_n = f32_from_u32(rays);
    let n_n = f32_from_u32(n);
    let mixed = (rays_n * mean + n_n * sample) / f32_from_u32(total);
    (mixed, total)
}

/// Exact for every `v` below 2^24, which covers a fill budget.
fn f32_from_u32(v: u32) -> f32 {
    let mut n = v;
    let mut acc = 0.0f32;
    let mut place = 1.0f32;
    while n > 0 {
        if n & 1 == 1 {
            acc += place;
        }
        place += place;
        n >>= 1;
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bid(id: u32, rays: u32, distance: f32) -> CellBid {
        CellBid {
            id,
            mean: 1.0,
            deviation: 1.0e-6,
            rays,
            lambda: 0.0,
            distance,
            on_screen: true,
        }
    }

    #[test]
    fn a_new_cell_starts_at_one_and_takes_the_budget() {
        let tun = Tunables::high();
        let fresh = bid(1, 0, 2.0);
        let done = bid(2, 64, 2.0);
        assert_eq!(cell_value(&fresh, &tun), 1.0);
        assert!(cell_value(&done, &tun) < cell_value(&fresh, &tun));
        let grants = allocate(&[done, fresh], 4, &tun, 0);
        let fresh_rays = grants.iter().find(|g| g.id == 1).map(|g| g.rays);
        let done_rays = grants.iter().find(|g| g.id == 2).map(|g| g.rays);
        assert!(fresh_rays > done_rays);
        assert!(fresh_rays.is_some_and(|rays| rays > 0));
    }

    #[test]
    fn the_highest_value_is_spent_first() {
        let mut tun = Tunables::high();
        tun.offscreen_floor = 0.0;
        tun.max_picks = 1;
        let mut low = bid(1, 32, 2.0);
        low.lambda = 0.05;
        let mut mid = bid(2, 32, 2.0);
        mid.lambda = 0.4;
        let mut high = bid(3, 0, 2.0);
        high.lambda = 0.0;
        let grants = allocate(&[low, mid, high], 1, &tun, 0);
        assert_eq!(grants.iter().find(|g| g.id == 3).map(|g| g.rays), Some(1));
        assert_eq!(grants.iter().filter(|g| g.rays > 0).count(), 1);
    }

    #[test]
    fn an_on_screen_cell_outranks_the_same_cell_off_screen() {
        let tun = Tunables::high();
        let mut on = bid(1, 0, 4.0);
        on.on_screen = true;
        let mut off = bid(2, 0, 4.0);
        off.on_screen = false;
        assert!(cell_rank(&on, &tun) > cell_rank(&off, &tun));
    }

    #[test]
    fn the_grant_never_passes_the_budget() {
        let tun = Tunables::high();
        let cells: Vec<_> = (0..40u32)
            .map(|id| bid(id, id % 5, 1.0 + f32::from(u16::try_from(id).unwrap_or(0))))
            .collect();
        let grants = allocate(&cells, tun.budget, &tun, 3);
        let total: u32 = grants.iter().map(|g| g.rays).sum();
        assert!(total <= tun.budget);
        let cap = tun.max_picks.max(1).saturating_add(tun.floor_rays);
        assert!(grants.iter().all(|g| g.rays <= cap || g.trickle));
    }

    #[test]
    fn a_small_change_keeps_more_of_the_mean_than_a_reset() {
        let old = 1.0_f32;
        let truth = 1.02_f32;
        let noisy = 1.6_f32;
        let kept = blend_with_confidence(old, history_confidence(0.05), noisy);
        let reset = blend_with_confidence(old, history_confidence(1.0), noisy);
        let kept_err = (kept - truth).abs();
        let reset_err = (reset - truth).abs();
        assert!(
            kept_err < reset_err,
            "proportional history {kept_err} should beat a reset {reset_err}"
        );
        let mut mean = kept;
        let mut reset_mean = reset;
        let mut kept_frame = 8u32;
        let mut reset_frame = 8u32;
        for frame in 0..8 {
            mean = blend_with_confidence(mean, history_confidence(0.05), truth);
            reset_mean = blend_with_confidence(reset_mean, history_confidence(0.05), truth);
            if kept_frame == 8 && (mean - truth).abs() <= 0.01 {
                kept_frame = frame;
            }
            if reset_frame == 8 && (reset_mean - truth).abs() <= 0.01 {
                reset_frame = frame;
            }
        }
        assert!(
            kept_frame <= reset_frame,
            "kept reached the floor at {kept_frame}, reset at {reset_frame}"
        );
        assert!((mean - truth).abs() <= (reset_mean - truth).abs() + 1.0e-4);
    }

    #[test]
    fn a_false_alarm_stops_asking_for_rays() {
        let mut tun = Tunables::high();
        tun.offscreen_floor = 0.0;
        tun.max_picks = 4;
        let mut value = 1.0_f32;
        let mean = 1.0_f32;
        let variance = 1.0e-4_f32;
        let mut last_ranked = 0u32;
        for frame in 0..4 {
            value = apply_agreement(value, mean, mean, variance);
            let mut cell = bid(1, 32, 2.0);
            cell.lambda = value;
            let grant = allocate(std::slice::from_ref(&cell), 8, &tun, frame)[0];
            last_ranked = if grant.trickle { 0 } else { grant.rays };
        }
        assert_eq!(value, 0.0, "three agreeing frames clear a false alarm");
        assert_eq!(last_ranked, 0, "the ranked spend stops once confidence is back");
    }

    #[test]
    fn the_floor_is_one_ray_per_live_cell_and_the_burst_is_the_rest() {
        let (floor, burst) = split_floor_and_burst(1000, 100_000, 1);
        assert_eq!(floor, 1000);
        assert_eq!(burst, 99_000);
        let (all, none) = split_floor_and_burst(100_000, 80_000, 1);
        assert_eq!(all, 80_000);
        assert_eq!(none, 0);
    }

    #[test]
    fn a_converged_cell_still_draws_the_trickle_floor() {
        let mut tun = Tunables::high();
        tun.floor_rays = 0;
        tun.check_rate = 1;
        tun.offscreen_floor = 0.25;
        let done = bid(7, 128, 3.0);
        assert_eq!(cell_value(&done, &tun), 0.0);
        let grants = allocate(&[done], 8, &tun, 0);
        assert_eq!(grants[0].rays, 1);
        assert!(grants[0].trickle);
        tun.offscreen_floor = 0.0;
        let none = allocate(&[done], 8, &tun, 0);
        assert_eq!(none[0].rays, 0);
    }

    #[test]
    fn an_unsignalled_change_recovers_when_the_floor_is_open() {
        let truth = 4.0_f32;
        let with_floor = recover(0.0, 1, truth);
        let no_floor = recover(0.0, 0, truth);
        assert!(
            with_floor < 0.5,
            "trickle plus the raised value should reach the new light, error {with_floor}"
        );
        assert!(
            no_floor > 2.0,
            "a zero floor never sees the change, error {no_floor}"
        );
        assert!(with_floor < no_floor);
    }

    fn recover(floor: f32, floor_rays: u32, truth: f32) -> f32 {
        let mut tun = Tunables::high();
        tun.floor_rays = floor_rays;
        tun.offscreen_floor = floor;
        tun.budget = 8;
        tun.max_picks = 4;
        tun.check_rate = 1;
        let mut mean = 1.0_f32;
        let variance = 1.0e-4_f32;
        let mut rays = 64_u32;
        let mut lambda = 0.0_f32;
        for frame in 0..120_u32 {
            let cell = CellBid {
                id: 1,
                mean,
                deviation: variance,
                rays,
                lambda,
                distance: 2.0,
                on_screen: true,
            };
            let grant = allocate(std::slice::from_ref(&cell), tun.budget, &tun, frame)[0];
            if grant.rays == 0 {
                continue;
            }
            if grant.trickle && trickle_disagrees(truth, mean, variance) {
                lambda = disagreement_raise(truth, mean);
            }
            let (next, next_rays) = blend_mean(mean, rays, truth, grant.rays);
            mean = next;
            rays = next_rays;
        }
        (mean - truth).abs()
    }
}
