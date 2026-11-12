//! CPU reference for screen-tile fill.
//!
//! Stage F1 estimators live in the other modules: need-weighted picks,
//! water-filling, a fixed-point deposit, one writer at resolve, and the
//! reconnection weight. Stage F2 is [`sched`]: the always-on budget, the
//! clipmap, and the single-ray fold the GPU scheduler mirrors. Stage F3 adds
//! the check ray, the decaying lambda, and the water-fill of that budget.
//! Nothing here
//! opens a window or a device. `--tile-fill` defaults off; that flag is the
//! F1 checker, not the live F2 scheduler.
//!
//! The resolve rule is the one the age-of-shown-mean runs measured
//! (`MEASURED_COMMIT`). On the axis-aligned red-box texels, 1M commits are
//! younger than flag off, so those runs did not select a quarter hold.
//! The yawed face is a different feeder (probes here, 25 cm cells on #265);
//! that age is in the F1 experiment note and is not this rule. `HoldQuarters`
//! stays as the comparison the axis-aligned measurement did not select.
//!
//! Departures from Lumen's World Cache fill and from `ReSTIR` GI (Ouyang et al.,
//! HPG 2021), recorded in the lighting README:
//! - Picks are need-weighted and water-filled. Lumen draws one uniform pixel per tile.
//! - Cell size does not grow with distance.
//! - One path vertex. The design's longer paths are stage F5.
//! - The reconnection weight is self-normalised. There is no pairwise MIS, no
//!   visibility ray, and no reservoir. `t_min` is 2 m and the weight clamps at 2.
//! - The origin estimator is the cache's cosine-weighted hemisphere sum
//!   (`pi` times the mean of cosine-weighted samples).
//!
//! Replicated state: none. Picks and deposits stay in the checker.
//! Authority: the checker process.
//! Prediction / rollback: none.
//! Determinism: hashes of tile, frame and cell, and integer deposit sums.
//! Headless server: this crate is headless. A game server does not run it.

mod commit;
mod deposit;
mod pick;
pub mod sched;
mod share;
mod water;

pub use commit::{CommitRule, Shown, MEASURED_COMMIT};
pub use deposit::{own_excludes_shared, Deposit, Faults};
pub use pick::{far_is_fed, need, prox, under, PixelPick};
pub use share::{
    axis_id, near_hit_stays_home, pillar, plane_rel_error, share_weight, thin_wall,
    unclamped_exceeds_cap, wall_base, Leak, Plane, Reject, ShareCell, ShareHit, T_MIN, W_CAP,
};
pub use water::{assign, bucket, uniform_assign, TileNeed, BUCKETS};

/// `tile_fill` defaults off. The live picture does not read this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub tile_fill: bool,
    pub commit: CommitRule,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tile_fill: false,
            commit: MEASURED_COMMIT,
        }
    }
}

/// What `fill-ref run` prints. `ran` is false when the flag is off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grade {
    pub ran: bool,
    pub pass: bool,
    pub plane_rel: f32,
    pub leak_other: f32,
    pub ceiling_picks: u32,
    pub far_picks: u32,
    pub faults_caught: bool,
}

/// Grade the estimators. Flag off is a pass that does not run them.
#[must_use]
pub fn grade(config: &Config) -> Grade {
    if !config.tile_fill {
        return Grade {
            ran: false,
            pass: true,
            plane_rel: 0.0,
            leak_other: 0.0,
            ceiling_picks: 0,
            far_picks: 0,
            faults_caught: false,
        };
    }
    let plane = share::plane_rel_error();
    let leak = share::thin_wall(false);
    let filled = water::hall_budget();
    let resolve_ok = commit::resolve_matches(config.commit);
    let faults_caught = faults_are_caught();
    Grade {
        ran: true,
        pass: plane < 0.01
            && share::leak_ok(leak)
            && filled.within_ceiling
            && resolve_ok
            && faults_caught,
        plane_rel: plane,
        leak_other: leak.other,
        ceiling_picks: filled.total,
        far_picks: filled.far,
        faults_caught,
    }
}

fn faults_are_caught() -> bool {
    let leaks = !share::leak_ok(share::thin_wall(true))
        && !share::leak_ok(share::pillar(true))
        && !share::leak_ok(share::wall_base(true));
    let open = deposit::Faults {
        t_min_off: true,
        ..deposit::Faults::default()
    };
    let near = !share::near_hit_stays_home(&open);
    let hall = water::hall_tiles();
    let uniform = water::uniform_assign(&hall, 8);
    let weighted = water::assign(&hall, 8, 4);
    let starved = !pick::far_is_fed(uniform[0], uniform[1]) && weighted[0] > uniform[0];
    let mut broken = deposit::Deposit::default();
    deposit::add_shared(
        &mut broken,
        genos_math::Vec3::new(1.0, 1.0, 1.0),
        1.0,
        deposit::Faults {
            count_shared_as_own: true,
            ..deposit::Faults::default()
        },
    );
    leaks
        && near
        && starved
        && !deposit::own_excludes_shared(&broken)
        && share::unclamped_exceeds_cap()
        && share::constant_weight_plane_error() >= 0.01
        && share::firefly_bound_holds()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_defaults_off() {
        let config = Config::default();
        assert!(!config.tile_fill);
        let grade = grade(&config);
        assert!(grade.pass);
        assert!(!grade.ran);
    }

    #[test]
    fn the_flag_on_grades_the_plane_and_the_wall() {
        let config = Config {
            tile_fill: true,
            commit: MEASURED_COMMIT,
        };
        let grade = grade(&config);
        assert!(grade.ran);
        assert!(grade.pass, "plane {} leak {}", grade.plane_rel, grade.leak_other);
        assert!(grade.plane_rel < 0.01);
    }
}
