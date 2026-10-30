//! ADR 0016 ship scene: four head-on lanes of convex hull ships at 30, 120,
//! 600 and 3 000 m/s. Each lane runs alone, so a lane cannot hide another.
//! Prints one `lane=... step=... ` stats line per step for the metrics path.

use genos_math::Vec3;
use genos_physics::{step, Body, MotionQuality, World};

const DT: f32 = 1.0 / 60.0;
const STEPS: u32 = 60;
const SPEEDS: [f32; 4] = [30.0, 120.0, 600.0, 3_000.0];
/// Nose to nose at the start. 40 m needs several advances at 3 000 m/s.
const START_GAP: f32 = 40.0;
const HALF_LENGTH: f32 = 2.0;
const PENETRATION_LIMIT: f32 = 0.05;

/// A wedge hull: a 4 m long box whose nose tapers to a 1 m wide face.
fn hull_triangles(nose: f32) -> Vec<[Vec3; 3]> {
    let l = HALF_LENGTH;
    let p = [
        Vec3::new(-l * nose, -1.0, -1.0),
        Vec3::new(-l * nose, 1.0, -1.0),
        Vec3::new(-l * nose, 1.0, 1.0),
        Vec3::new(-l * nose, -1.0, 1.0),
        Vec3::new(l * nose, -0.5, -0.5),
        Vec3::new(l * nose, 0.5, -0.5),
        Vec3::new(l * nose, 0.5, 0.5),
        Vec3::new(l * nose, -0.5, 0.5),
    ];
    let quads = [
        [0, 1, 2, 3],
        [4, 7, 6, 5],
        [0, 4, 5, 1],
        [3, 2, 6, 7],
        [0, 3, 7, 4],
        [1, 5, 6, 2],
    ];
    let mut tris = Vec::new();
    for q in quads {
        tris.push([p[q[0]], p[q[1]], p[q[2]]]);
        tris.push([p[q[0]], p[q[2]], p[q[3]]]);
    }
    tris
}

fn ship(x: f32, speed: f32, nose: f32) -> Body {
    let mut body = Body::mesh(Vec3::new(x, 0.0, 0.0), 1_000.0, &hull_triangles(nose));
    body.velocity = Vec3::new(speed, 0.0, 0.0);
    body.motion = MotionQuality::Swept;
    body.restitution = 0.0;
    body
}

struct Lane {
    exchanged: bool,
    max_penetration: f32,
    toi_failed: u32,
    pair_slices: u32,
    impulse_steps: u32,
    momentum_error: f32,
    closing_after: f32,
    trace: Vec<String>,
}

fn run_lane(speed: f32) -> Lane {
    let mut world = World::new(Vec3::ZERO);
    let gap = START_GAP + 2.0 * HALF_LENGTH;
    world.insert(ship(0.0, speed, 1.0));
    world.insert(ship(gap, -speed, -1.0));
    let mut lane = Lane {
        exchanged: false,
        max_penetration: 0.0,
        toi_failed: 0,
        pair_slices: 0,
        impulse_steps: 0,
        momentum_error: 0.0,
        closing_after: 0.0,
        trace: Vec::new(),
    };
    for index in 0..STEPS {
        step(&mut world, DT);
        let (a, b) = (world.bodies[0], world.bodies[1]);
        let stats = world.step_stats;
        lane.exchanged |= a.position.x >= b.position.x;
        let depth = (2.0 * HALF_LENGTH - (b.position.x - a.position.x)).max(0.0);
        lane.max_penetration = lane.max_penetration.max(depth);
        lane.toi_failed += stats.toi_failed;
        lane.pair_slices += stats.pair_slices;
        lane.impulse_steps += u32::from(stats.dynamic_pair_impulses > 0);
        let momentum = (a.velocity.x + b.velocity.x).abs() / speed;
        lane.momentum_error = lane.momentum_error.max(momentum);
        lane.closing_after = a.velocity.x - b.velocity.x;
        lane.trace.push(format!(
            "lane={speed} step={index} gap_m={:.4} toi_failed={} toi_deferred={} pair_slices={} dynamic_pair_impulses={} closing_mps={:.3}",
            b.position.x - a.position.x - 2.0 * HALF_LENGTH,
            stats.toi_failed,
            stats.toi_deferred,
            stats.pair_slices,
            stats.dynamic_pair_impulses,
            lane.closing_after,
        ));
    }
    lane
}

#[test]
fn hull_ships_meet_head_on_without_tunnelling_at_every_lane_speed() {
    for speed in SPEEDS {
        let lane = run_lane(speed);
        for line in &lane.trace {
            println!("{line}");
        }
        println!(
            "lane={speed} summary max_penetration_m={:.4} pair_slices={} toi_failed={} momentum_error={:.2e} closing_after_mps={:.3}",
            lane.max_penetration, lane.pair_slices, lane.toi_failed, lane.momentum_error, lane.closing_after
        );
        assert!(!lane.exchanged, "lane {speed}: ships exchanged places");
        assert!(
            lane.max_penetration <= PENETRATION_LIMIT,
            "lane {speed}: penetration {}",
            lane.max_penetration
        );
        // Finding: at 3 000 m/s from 40 m the wedge hulls use all 16 advances and
        // clamp 0.81 m short (ADR 0016 safe clamp); contact lands the next step.
        // The box ships in `ccd.rs` converge on the same gap. Other lanes never clamp.
        let clamp_allowance = u32::from(speed >= 3_000.0);
        assert!(
            lane.toi_failed <= clamp_allowance,
            "lane {speed}: toi_failed {}",
            lane.toi_failed
        );
        assert!(lane.impulse_steps >= 1, "lane {speed}: no contact impulse");
        // Equal masses head-on: total momentum stays near zero.
        assert!(
            lane.momentum_error <= 1.0e-3,
            "lane {speed}: momentum drift {}",
            lane.momentum_error
        );
        // Inelastic contact: they no longer close faster than 1% of the approach.
        assert!(
            lane.closing_after <= speed * 0.02,
            "lane {speed}: still closing at {}",
            lane.closing_after
        );
    }
}

#[test]
fn hull_ship_lanes_are_bitwise_deterministic() {
    for speed in SPEEDS {
        assert_eq!(run_lane(speed).trace, run_lane(speed).trace, "lane {speed}");
    }
}

#[test]
fn discrete_hull_ships_tunnel_at_ship_speed() {
    let mut world = World::new(Vec3::ZERO);
    let mut left = ship(0.0, 3_000.0, 1.0);
    let mut right = ship(START_GAP + 2.0 * HALF_LENGTH, -3_000.0, -1.0);
    left.motion = MotionQuality::Discrete;
    right.motion = MotionQuality::Discrete;
    world.insert(left);
    world.insert(right);
    step(&mut world, DT);
    assert!(
        world.bodies[0].position.x > world.bodies[1].position.x,
        "control case should tunnel"
    );
}
