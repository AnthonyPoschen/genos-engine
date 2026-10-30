//! Physics debug binary core: scene picker, a fixed-step session, per-step
//! stats, a bitwise state hash for replay, and snapshot plus restore.
//! Headless: no window, no draw. `main.rs` adds the CLI and the MCP commands.

pub mod scenes;

use std::time::Instant;

use genos_physics::{step, Body, World};

pub use scenes::{find, SceneInfo, SCENES};

pub const DT: f32 = 1.0 / 60.0;

/// What one step did, as one JSON line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StepRecord {
    pub step: u32,
    pub step_us: u128,
    pub bodies: usize,
    pub ccd_applied: bool,
    pub toi_failed: u32,
    pub toi_deferred: u32,
    pub pair_slices: u32,
    pub dynamic_pair_impulses: u32,
    pub linear_energy: f32,
    pub rotational_energy: f32,
    pub max_speed: f32,
    pub min_y: f32,
    pub hash: u64,
}

impl StepRecord {
    #[must_use]
    pub fn json(&self, scene: &str) -> String {
        format!(
            "{{\"scene\":\"{scene}\",\"step\":{},\"step_us\":{},\"bodies\":{},\"ccd_applied\":{},\"toi_failed\":{},\"toi_deferred\":{},\"pair_slices\":{},\"dynamic_pair_impulses\":{},\"linear_energy\":{:.6},\"rotational_energy\":{:.6},\"max_speed\":{:.6},\"min_y\":{:.6},\"hash\":\"{:016x}\"}}",
            self.step,
            self.step_us,
            self.bodies,
            self.ccd_applied,
            self.toi_failed,
            self.toi_deferred,
            self.pair_slices,
            self.dynamic_pair_impulses,
            self.linear_energy,
            self.rotational_energy,
            self.max_speed,
            self.min_y,
            self.hash,
        )
    }
}

/// FNV-1a over every body's pose and velocities, bit for bit.
#[must_use]
pub fn state_hash(bodies: &[Body]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for body in bodies {
        let o = body.orientation;
        for value in [
            body.position.x,
            body.position.y,
            body.position.z,
            o.x,
            o.y,
            o.z,
            o.w,
            body.velocity.x,
            body.velocity.y,
            body.velocity.z,
            body.angular_velocity.x,
            body.angular_velocity.y,
            body.angular_velocity.z,
        ] {
            for byte in value.to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    hash
}

/// One loaded scene stepping on a fixed step from step 0.
#[derive(Clone)]
pub struct Session {
    pub scene: &'static SceneInfo,
    pub world: World,
    pub steps: u32,
    pub records: Vec<StepRecord>,
}

impl Session {
    #[must_use]
    pub fn load(scene: &'static SceneInfo) -> Self {
        Self {
            scene,
            world: scene.build(),
            steps: 0,
            records: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::load(self.scene);
    }

    pub fn step(&mut self) -> StepRecord {
        let start = Instant::now();
        step(&mut self.world, DT);
        let step_us = start.elapsed().as_micros();
        let stats = self.world.step_stats;
        let mut record = StepRecord {
            step: self.steps,
            step_us,
            bodies: self.world.bodies.len(),
            ccd_applied: stats.ccd_applied,
            toi_failed: stats.toi_failed,
            toi_deferred: stats.toi_deferred,
            pair_slices: stats.pair_slices,
            dynamic_pair_impulses: stats.dynamic_pair_impulses,
            linear_energy: 0.0,
            rotational_energy: 0.0,
            max_speed: 0.0,
            min_y: f32::INFINITY,
            hash: state_hash(&self.world.bodies),
        };
        for body in self
            .world
            .bodies
            .iter()
            .filter(|body| body.inverse_mass > 0.0)
        {
            let speed = body.velocity.length();
            record.linear_energy += 0.5 * body.mass() * speed * speed;
            record.rotational_energy += body.rotational_energy();
            record.max_speed = record.max_speed.max(speed);
            record.min_y = record.min_y.min(body.position.y);
        }
        self.steps += 1;
        self.records.push(record);
        record
    }

    pub fn run(&mut self, steps: u32) -> &[StepRecord] {
        let from = self.records.len();
        for _ in 0..steps {
            self.step();
        }
        &self.records[from..]
    }
}

/// Run a scene twice from step 0 and compare the hash after every step.
///
/// # Errors
///
/// The first step whose hashes differ.
pub fn replay(scene: &'static SceneInfo, steps: u32) -> Result<u64, u32> {
    let mut first = Session::load(scene);
    let mut second = Session::load(scene);
    let mut last = 0;
    for index in 0..steps {
        let a = first.step().hash;
        let b = second.step().hash;
        if a != b {
            return Err(index);
        }
        last = a;
    }
    Ok(last)
}

/// Snapshot at `at`, run on to `steps`, restore, run again: both tails must
/// hash the same.
///
/// # Errors
///
/// The first mismatched step.
pub fn checkpoint(scene: &'static SceneInfo, at: u32, steps: u32) -> Result<u64, u32> {
    let mut session = Session::load(scene);
    session.run(at);
    let snapshot = session.clone();
    let straight: Vec<u64> = session
        .run(steps.saturating_sub(at))
        .iter()
        .map(|r| r.hash)
        .collect();
    let mut restored = snapshot;
    let again: Vec<u64> = restored
        .run(steps.saturating_sub(at))
        .iter()
        .map(|r| r.hash)
        .collect();
    for (offset, (a, b)) in straight.iter().zip(&again).enumerate() {
        if a != b {
            return Err(at + u32::try_from(offset).unwrap_or(u32::MAX));
        }
    }
    Ok(straight
        .last()
        .copied()
        .unwrap_or_else(|| state_hash(&restored.world.bodies)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scene_builds_and_replays_bitwise() {
        for scene in &SCENES {
            let steps = scene.steps.min(30);
            assert!(replay(scene, steps).is_ok(), "{} diverged", scene.name);
        }
    }

    #[test]
    fn checkpoint_restore_matches_a_straight_run() {
        for scene in &SCENES {
            assert!(
                checkpoint(scene, 10, 25).is_ok(),
                "{} restore diverged",
                scene.name
            );
        }
    }

    #[test]
    fn ship_lanes_never_cross() {
        let mut session = Session::load(find("ship_lanes").unwrap_or(&SCENES[0]));
        session.run(60);
        for pair in session.world.bodies.chunks(2) {
            assert!(pair[0].position.x < pair[1].position.x, "a lane tunnelled");
        }
        let failed: u32 = session.records.iter().map(|r| r.toi_failed).sum();
        assert_eq!(failed, 0);
    }

    #[test]
    fn resting_pile_stays_above_the_slab_and_settles() {
        let mut session = Session::load(find("resting_pile").unwrap_or(&SCENES[0]));
        session.run(240);
        let last = session.records.last().copied().unwrap_or(session.step());
        assert!(last.min_y > 0.3, "a cube sank to {}", last.min_y);
        assert!(
            last.max_speed < 0.5,
            "pile still moving at {}",
            last.max_speed
        );
    }

    #[test]
    fn falling_edge_rests_on_the_static_edge() {
        let mut session = Session::load(find("hull_edges").unwrap_or(&SCENES[0]));
        session.run(60);
        let top = session.world.bodies[5];
        let reach = 0.5 * std::f32::consts::SQRT_2;
        assert!(
            (top.position.y - 2.0 * reach).abs() < 0.01,
            "edge rest at {}",
            top.position.y
        );
        assert!(top.velocity.length() < 1.0e-3);
    }

    #[test]
    fn static_grid_holds_ten_thousand_static_bodies() {
        let session = Session::load(find("static_grid_10k").unwrap_or(&SCENES[0]));
        let statics = session
            .world
            .bodies
            .iter()
            .filter(|b| b.inverse_mass == 0.0)
            .count();
        assert_eq!(statics, 10_000);
    }

    #[test]
    fn json_line_names_the_scene_and_hash() {
        let mut session = Session::load(&SCENES[0]);
        let line = session.step().json("ship_lanes");
        assert!(line.starts_with("{\"scene\":\"ship_lanes\",\"step\":0,"));
        assert!(genos_mcp::parse(&line).is_ok());
    }
}
