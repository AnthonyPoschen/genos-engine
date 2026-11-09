//! Physics debug binary core: scene picker, a fixed-step session, per-step
//! stats, a bitwise state hash for replay, and snapshot plus restore.
//! Headless: no window, no draw. `main.rs` adds the CLI and the MCP commands.

pub mod scenes;

use std::time::Instant;

use genos_physics::{step, World};

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
    pub contact_points: u32,
    pub sleeping_bodies: u32,
    pub pair_tests: u32,
    pub broadphase_builds: u32,
    pub tree_nodes: u32,
    pub islands: u32,
    pub awake_bodies: u32,
    pub solver_bodies: u32,
    pub wheel_rays: u32,
    pub vehicle_count: u32,
    pub tyre_contacts: u32,
    pub drive_wheels: u32,
}

impl StepRecord {
    #[must_use]
    pub fn json(&self, scene: &str) -> String {
        format!(
            "{{\"scene\":\"{scene}\",\"step\":{},\"step_us\":{},\"bodies\":{},\"ccd_applied\":{},\"toi_failed\":{},\"toi_deferred\":{},\"pair_slices\":{},\"dynamic_pair_impulses\":{},\"linear_energy\":{:.6},\"rotational_energy\":{:.6},\"max_speed\":{:.6},\"min_y\":{:.6},\"hash\":\"{:016x}\",\"contact_points\":{},\"sleeping_bodies\":{},\"pair_tests\":{},\"broadphase_builds\":{},\"tree_nodes\":{},\"islands\":{},\"awake_bodies\":{},\"solver_bodies\":{},\"wheel_rays\":{},\"vehicle_count\":{},\"tyre_contacts\":{},\"drive_wheels\":{}}}",
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
            self.contact_points,
            self.sleeping_bodies,
            self.pair_tests,
            self.broadphase_builds,
            self.tree_nodes,
            self.islands,
            self.awake_bodies,
            self.solver_bodies,
            self.wheel_rays,
            self.vehicle_count,
            self.tyre_contacts,
            self.drive_wheels,
        )
    }
}

/// FNV-1a over every body's pose and velocities, then each wheel length and omega.
#[must_use]
pub fn state_hash(world: &World) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for body in &world.bodies {
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
            mix_f32(&mut hash, value);
        }
    }
    for vehicle in &world.vehicles {
        let Some(genos_physics::GroundContact::Wheeled(ground)) = vehicle.ground else {
            continue;
        };
        for (length, wheel) in ground.lengths.iter().zip(ground.wheels.iter()) {
            mix_f32(&mut hash, *length);
            mix_f32(&mut hash, wheel.omega);
        }
    }
    hash
}

fn mix_f32(hash: &mut u64, value: f32) {
    for byte in value.to_bits().to_le_bytes() {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
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
            hash: state_hash(&self.world),
            wheel_rays: stats.wheel_rays,
            vehicle_count: stats.vehicle_count,
            tyre_contacts: stats.tyre_contacts,
            drive_wheels: stats.drive_wheels,
            contact_points: stats.contact_points,
            sleeping_bodies: stats.sleeping_bodies,
            pair_tests: stats.pair_tests,
            broadphase_builds: stats.broadphase_builds,
            tree_nodes: stats.tree_nodes,
            islands: stats.islands,
            awake_bodies: stats.awake_bodies,
            solver_bodies: stats.solver_bodies,
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
        .unwrap_or_else(|| state_hash(&restored.world)))
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
            last.max_speed.is_finite() && last.max_speed < 0.5,
            "pile ran away at {}",
            last.max_speed
        );
    }

    #[test]
    fn resting_pile_settles_with_sleep_off() {
        let mut session = Session::load(find("resting_pile").unwrap_or(&SCENES[0]));
        // Sleep would zero the residual. The gate has to hold while every body stays awake.
        session.world.sleep_time = 1.0e9;
        session.run(240);
        let last = session.records.last().copied().unwrap_or(session.step());
        assert!(last.min_y > 0.3, "a cube sank to {}", last.min_y);
        assert!(
            last.max_speed.is_finite() && last.max_speed < 0.5,
            "pile ran away at {}",
            last.max_speed
        );
        let mut peak_spin = 0.0_f32;
        for body in session.world.bodies.iter().skip(1) {
            peak_spin = peak_spin.max(body.angular_velocity.length());
        }
        assert!(
            peak_spin < 0.05,
            "resting hull still spinning at {peak_spin} rad/s"
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
    fn layer_filter_drops_the_debris_pair() {
        let mut session = Session::load(find("layer_filter").unwrap_or(&SCENES[0]));
        let record = session.step();
        assert_eq!(record.pair_tests, 0);
        assert!(!session.world.in_static_tree(0));
        assert!(session.world.in_dynamic_tree(0));
        assert!(session.world.in_dynamic_tree(1));
    }

    #[test]
    fn static_grid_keeps_statics_out_of_the_dynamic_tree() {
        let session = Session::load(find("static_grid_10k").unwrap_or(&SCENES[0]));
        assert!(session.world.in_static_tree(0));
        assert!(!session.world.in_dynamic_tree(0));
        let sphere = session.world.bodies.len() - 1;
        assert!(session.world.in_dynamic_tree(sphere));
        assert!(!session.world.in_static_tree(sphere));
    }

    #[test]
    fn sleep_island_wakes_only_the_touched_stack() {
        let mut session = Session::load(find("sleep_island").unwrap_or(&SCENES[0]));
        session.run(180);
        assert!(session.world.bodies[1].sleeping);
        assert!(session.world.bodies[2].sleeping);
        assert!(session.world.bodies[3].sleeping);
        assert!(session.world.bodies[4].sleeping);
        assert_eq!(
            session.world.bodies[1].island,
            session.world.bodies[2].island
        );
        let sphere = session.world.insert(genos_physics::Body::sphere(
            genos_math::Vec3::new(0.0, 3.0, 0.0),
            0.0,
            0.4,
        ));
        session
            .world
            .set_position(sphere, genos_math::Vec3::new(0.0, 1.6, 0.0));
        let record = session.step();
        assert!(!session.world.bodies[1].sleeping);
        assert!(!session.world.bodies[2].sleeping);
        assert!(session.world.bodies[3].sleeping);
        assert!(record.solver_bodies >= 2);
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
    fn chassis_rest_casts_four_rays_and_the_hash_covers_lengths() {
        let mut session = Session::load(find("chassis_rest").unwrap_or(&SCENES[0]));
        assert_eq!(session.scene.name, "chassis_rest");
        let record = session.step();
        assert_eq!(record.wheel_rays, 4);
        assert_eq!(record.vehicle_count, 1);
        let line = record.json("chassis_rest");
        assert!(line.contains("\"wheel_rays\":4"));
        assert!(line.contains("\"vehicle_count\":1"));
        assert_eq!(record.tyre_contacts, 0);
        assert_eq!(record.drive_wheels, 0);
        assert!(line.contains("\"tyre_contacts\":0"));
        assert!(line.contains("\"drive_wheels\":0"));
        assert!(genos_mcp::parse(&line).is_ok());
        let before = record.hash;
        let Some(genos_physics::GroundContact::Wheeled(ref mut ground)) =
            session.world.vehicles[0].ground
        else {
            assert!(session.world.vehicles[0].ground.is_some());
            return;
        };
        ground.lengths[0] += 0.01;
        assert_ne!(state_hash(&session.world), before);
    }

    #[test]
    fn locked_wheel_slows_and_the_hash_covers_omega() {
        let Some(scene) = find("locked_wheel") else {
            assert!(find("locked_wheel").is_some());
            return;
        };
        let mut session = Session::load(scene);
        let mut twin = Session::load(scene);
        let first = session.step();
        let twin_first = twin.step();
        assert_eq!(first.tyre_contacts, 4);
        assert_eq!(first.wheel_rays, 4);
        assert_eq!(first.hash, twin_first.hash);
        let line = first.json("locked_wheel");
        assert!(line.contains("\"tyre_contacts\":4"));
        assert!(genos_mcp::parse(&line).is_ok());
        let before = first.hash;
        assert!(
            set_first_omega(&mut session.world, 0.25),
            "locked scene has no wheeled ground"
        );
        assert_ne!(state_hash(&session.world), before);
        assert!(set_first_omega(&mut session.world, 0.0));
        assert_eq!(state_hash(&session.world), before);
        for _ in 1..60 {
            session.step();
            twin.step();
        }
        let last = session.records.last().copied();
        let twin_last = twin.records.last().copied();
        let (Some(last), Some(twin_last)) = (last, twin_last) else {
            assert!(session.records.last().is_some());
            return;
        };
        assert_eq!(last.hash, twin_last.hash);
        assert!(
            last.max_speed < 6.0,
            "locked scene speed {}",
            last.max_speed
        );
        assert_eq!(last.tyre_contacts, 4);
    }

    fn set_first_omega(world: &mut genos_physics::World, omega: f32) -> bool {
        let Some(vehicle) = world.vehicles.first_mut() else {
            return false;
        };
        let Some(genos_physics::GroundContact::Wheeled(ref mut ground)) = vehicle.ground else {
            return false;
        };
        let Some(wheel) = ground.wheels.first_mut() else {
            return false;
        };
        wheel.omega = omega;
        true
    }

    #[test]
    fn first_gear_drives_and_counts_driven_wheels() {
        let Some(scene) = find("first_gear") else {
            assert!(find("first_gear").is_some());
            return;
        };
        let mut session = Session::load(scene);
        let mut twin = Session::load(scene);
        let first = session.step();
        let twin_first = twin.step();
        assert_eq!(first.drive_wheels, 4);
        assert_eq!(first.wheel_rays, 4);
        assert_eq!(first.hash, twin_first.hash);
        let line = first.json("first_gear");
        assert!(line.contains("\"drive_wheels\":4"));
        assert!(genos_mcp::parse(&line).is_ok());
        for _ in 1..120 {
            session.step();
            twin.step();
        }
        let last = session.records.last().copied();
        let twin_last = twin.records.last().copied();
        let (Some(last), Some(twin_last)) = (last, twin_last) else {
            assert!(session.records.last().is_some());
            return;
        };
        assert_eq!(last.hash, twin_last.hash);
        assert!(last.max_speed > 1.0, "first gear speed {}", last.max_speed);
        assert_eq!(last.drive_wheels, 4);

        let mut open = Session::load(scene);
        assert!(set_clutch(&mut open.world, 0.0), "first gear has no clutch");
        let opened = open.step();
        assert_eq!(opened.drive_wheels, 0);
        assert!(
            opened.max_speed < 1.0,
            "open clutch speed {}",
            opened.max_speed
        );
    }

    fn set_clutch(world: &mut genos_physics::World, clutch: f32) -> bool {
        let Some(vehicle) = world.vehicles.first_mut() else {
            return false;
        };
        let Some(genos_physics::GroundContact::Wheeled(ref mut ground)) = vehicle.ground else {
            return false;
        };
        let Some(drive) = ground.drive.as_mut() else {
            return false;
        };
        drive.clutch = clutch;
        true
    }

    #[test]
    fn json_line_names_the_scene_and_hash() {
        let mut session = Session::load(&SCENES[0]);
        let line = session.step().json("ship_lanes");
        assert!(line.starts_with("{\"scene\":\"ship_lanes\",\"step\":0,"));
        assert!(genos_mcp::parse(&line).is_ok());
    }
}
