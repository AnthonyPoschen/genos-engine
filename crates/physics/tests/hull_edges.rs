#![cfg_attr(
    test,
    expect(clippy::panic, reason = "a test aborts when a fixture fails")
)]

//! Hull-vs-hull edge-to-edge contact. Two unit cubes, one turned 45 degrees
//! about z and one about y, meet along x with crossed edges. Only the
//! edge-edge axis (z cross y = x) separates them; face normals overlap.

use genos_math::{Quat, Vec3};
use genos_physics::{step, Body, Piece, Shape, World};

const HALF: f32 = 0.5;
const DT: f32 = 1.0 / 60.0;

fn outward_box(h: f32) -> Vec<[Vec3; 3]> {
    let nnn = Vec3::new(-h, -h, -h);
    let pnn = Vec3::new(h, -h, -h);
    let ppn = Vec3::new(h, h, -h);
    let npn = Vec3::new(-h, h, -h);
    let nnp = Vec3::new(-h, -h, h);
    let pnp = Vec3::new(h, -h, h);
    let ppp = Vec3::new(h, h, h);
    let npp = Vec3::new(-h, h, h);
    vec![
        [pnn, ppn, ppp],
        [pnn, ppp, pnp],
        [nnn, nnp, npp],
        [nnn, npp, npn],
        [npn, npp, ppp],
        [npn, ppp, ppn],
        [nnn, pnn, pnp],
        [nnn, pnp, nnp],
        [nnp, pnp, ppp],
        [nnp, ppp, npp],
        [nnn, npn, ppn],
        [nnn, ppn, pnn],
    ]
}

/// `a` has an edge along z at x = +reach; `b` has an edge along y at
/// x = gap + reach, so the edges are `gap` apart along x.
fn crossed_cubes(gap: f32, mass_a: f32) -> World {
    let reach = HALF * std::f32::consts::SQRT_2;
    let quarter = std::f32::consts::FRAC_PI_4;
    let mut a = Body::mesh(Vec3::ZERO, mass_a, &outward_box(HALF));
    a.orientation = Quat::from_axis_angle(Vec3::Z, quarter);
    let mut b = Body::mesh(
        Vec3::new(2.0 * reach + gap, 0.0, 0.0),
        1.0,
        &outward_box(HALF),
    );
    b.orientation = Quat::from_axis_angle(Vec3::Y, quarter);
    for body in [&a, &b] {
        let Shape::Mesh(mesh) = body.shape else {
            panic!("mesh")
        };
        assert!(
            matches!(mesh.pieces[0], Piece::Hull(_)) && mesh.count == 1,
            "cube must reduce to one hull"
        );
    }
    let mut world = World::new(Vec3::ZERO);
    world.insert(a);
    world.insert(b);
    world
}

#[test]
fn crossed_edges_apart_do_not_touch() {
    // Face-only SAT overlaps on every face axis here (about 0.32 m deep on
    // a's diagonal faces) and reports a false contact.
    for gap in [0.1_f32, 0.01] {
        let mut world = crossed_cubes(gap, 1.0);
        let before = [world.bodies[0].position, world.bodies[1].position];
        step(&mut world, DT);
        assert_eq!(world.bodies[0].position, before[0], "gap {gap}: a moved");
        assert_eq!(world.bodies[1].position, before[1], "gap {gap}: b moved");
    }
}

/// Positional slop, correction fraction, and narrow-phase passes in `step`.
/// One positional iteration runs on each pass (`solver.rs`, `lib.rs`).
const SLOP: f32 = 0.001;
const PERCENT: f32 = 0.8;
const POSITION_PASSES: u32 = 4;
/// A tenth of the slop. Wider than the anchor round-trip, and a 1e-8 nudge
/// stays far outside it.
const CORRECTION_BAND: f32 = SLOP * 0.1;

/// Separation after `POSITION_PASSES` of `extra * PERCENT`, in `f32`.
fn separation_from_solver(depth: f32) -> f32 {
    let mut left = depth;
    for _ in 0..POSITION_PASSES {
        let extra = left - SLOP;
        if extra <= 0.0 {
            break;
        }
        left -= extra * PERCENT;
    }
    depth - left
}

struct StepMove {
    moved_a: Vec3,
    moved_b: Vec3,
    share_a: f32,
    share_b: f32,
    velocity_a: Vec3,
    velocity_b: Vec3,
    spin_a: Vec3,
    spin_b: Vec3,
}

/// `None` when the centres took the configured correction. A 1e-8 nudge,
/// an overshoot past `depth`, a sideways drift, or a new velocity is `Some`.
fn separation_fault(depth: f32, step: &StepMove) -> Option<String> {
    let predicted = separation_from_solver(depth);
    let pushed = step.moved_b.x - step.moved_a.x;
    if step.share_a > 0.0 && step.moved_a.x >= 0.0 {
        return Some(format!("a did not move toward -x: {:?}", step.moved_a));
    }
    if step.share_b > 0.0 && step.moved_b.x <= 0.0 {
        return Some(format!("b did not move toward +x: {:?}", step.moved_b));
    }
    for moved in [step.moved_a, step.moved_b] {
        if moved.y.abs() > 1.0e-6 || moved.z.abs() > 1.0e-6 {
            return Some(format!("sideways {moved:?}"));
        }
    }
    if pushed > depth {
        return Some(format!("pushed {pushed} past depth {depth}"));
    }
    if (pushed - predicted).abs() > CORRECTION_BAND {
        return Some(format!(
            "pushed {pushed}, solver correction {predicted} for depth {depth}"
        ));
    }
    let expect_a = -predicted * step.share_a;
    let expect_b = predicted * step.share_b;
    if (step.moved_a.x - expect_a).abs() > CORRECTION_BAND {
        return Some(format!(
            "a.x {} , share {} of {predicted}",
            step.moved_a.x, step.share_a
        ));
    }
    if (step.moved_b.x - expect_b).abs() > CORRECTION_BAND {
        return Some(format!(
            "b.x {} , share {} of {predicted}",
            step.moved_b.x, step.share_b
        ));
    }
    if step.share_a <= 0.0 && step.moved_a != Vec3::ZERO {
        return Some(format!("static a moved {:?}", step.moved_a));
    }
    if step.share_b <= 0.0 && step.moved_b != Vec3::ZERO {
        return Some(format!("static b moved {:?}", step.moved_b));
    }
    if step.velocity_a != Vec3::ZERO || step.velocity_b != Vec3::ZERO {
        return Some(format!(
            "velocity {:?} {:?}",
            step.velocity_a, step.velocity_b
        ));
    }
    if step.spin_a != Vec3::ZERO || step.spin_b != Vec3::ZERO {
        return Some(format!("spin {:?} {:?}", step.spin_a, step.spin_b));
    }
    None
}

fn separate_along_x(world: &mut World, depth: f32) {
    let before = [world.bodies[0].position, world.bodies[1].position];
    let inv_a = world.bodies[0].inverse_mass;
    let inv_b = world.bodies[1].inverse_mass;
    let inv = inv_a + inv_b;
    step(world, DT);
    let moved_a = world.bodies[0].position - before[0];
    let moved_b = world.bodies[1].position - before[1];
    let fault = separation_fault(
        depth,
        &StepMove {
            moved_a,
            moved_b,
            share_a: inv_a / inv,
            share_b: inv_b / inv,
            velocity_a: world.bodies[0].velocity,
            velocity_b: world.bodies[1].velocity,
            spin_a: world.bodies[0].angular_velocity,
            spin_b: world.bodies[1].angular_velocity,
        },
    );
    assert!(fault.is_none(), "{}", fault.unwrap_or_default());
}

#[test]
fn crossed_edges_overlapping_separate_along_x_by_the_edge_depth() {
    let depth = 0.05_f32;
    let mut world = crossed_cubes(-depth, 1.0);
    // Correction is bounded by the true edge depth, not a face overlap.
    separate_along_x(&mut world, depth);
}

#[test]
fn a_thin_edge_overlap_still_separates_along_x() {
    // 5 mm is above slop. The face-clip samples of this overlap clamp to 0.
    let depth = 0.005_f32;
    let mut world = crossed_cubes(-depth, 1.0);
    separate_along_x(&mut world, depth);
}

#[test]
fn a_static_hull_stays_and_the_dynamic_edge_takes_the_correction() {
    let depth = 0.05_f32;
    let mut world = crossed_cubes(-depth, 0.0);
    separate_along_x(&mut world, depth);
}

#[test]
fn a_negligible_push_fails_the_separation_check() {
    let depth = 0.05_f32;
    let fault = separation_fault(
        depth,
        &StepMove {
            moved_a: Vec3::new(-1.0e-8, 0.0, 0.0),
            moved_b: Vec3::new(1.0e-8, 0.0, 0.0),
            share_a: 0.5,
            share_b: 0.5,
            velocity_a: Vec3::ZERO,
            velocity_b: Vec3::ZERO,
            spin_a: Vec3::ZERO,
            spin_b: Vec3::ZERO,
        },
    );
    let predicted = separation_from_solver(depth);
    assert!(
        fault.as_deref().is_some_and(|text| text.contains("pushed"))
            && (predicted - 2.0e-8).abs() > CORRECTION_BAND,
        "a 1e-8 nudge passed the separation check: {fault:?}"
    );
}

/// Eight ulps of yaw. The edge overlap stays, and face-clip penetrations clamp to 0.
fn yawed_crossed_cubes(gap: f32) -> World {
    let mut world = crossed_cubes(gap, 1.0);
    let quarter = std::f32::consts::FRAC_PI_4;
    let yaw = f32::from_bits(quarter.to_bits().wrapping_add(8));
    world.bodies[1].orientation = Quat::from_axis_angle(Vec3::Y, yaw);
    world
}

#[test]
fn a_few_ulps_of_yaw_still_separates_a_crossed_edge_overlap() {
    let depth = 0.05_f32;
    let mut world = yawed_crossed_cubes(-depth);
    separate_along_x(&mut world, depth);
}

#[test]
fn the_same_yaw_with_a_gap_does_not_move() {
    let mut world = yawed_crossed_cubes(0.01);
    let before = [world.bodies[0].position, world.bodies[1].position];
    step(&mut world, DT);
    assert_eq!(world.bodies[0].position, before[0]);
    assert_eq!(world.bodies[1].position, before[1]);
}

#[test]
fn aligned_hulls_overlapping_on_x_still_separate() {
    let depth = 0.05_f32;
    let a = Body::mesh(Vec3::ZERO, 1.0, &outward_box(HALF));
    let b = Body::mesh(Vec3::new(1.0 - depth, 0.0, 0.0), 1.0, &outward_box(HALF));
    let mut world = World::new(Vec3::ZERO);
    world.insert(a);
    world.insert(b);
    separate_along_x(&mut world, depth);
}

#[test]
fn an_edge_rests_on_a_crossed_static_edge() {
    // b (edge along y... turned so its low edge runs along x) rests on a's
    // top edge (along z) under gravity.
    let reach = HALF * std::f32::consts::SQRT_2;
    let quarter = std::f32::consts::FRAC_PI_4;
    let mut a = Body::mesh(Vec3::ZERO, 0.0, &outward_box(HALF));
    a.orientation = Quat::from_axis_angle(Vec3::Z, quarter);
    let mut b = Body::mesh(
        Vec3::new(0.0, 2.0 * reach + 0.2, 0.0),
        1.0,
        &outward_box(HALF),
    );
    b.orientation = Quat::from_axis_angle(Vec3::X, quarter);
    let mut world = World::new(Vec3::new(0.0, -9.81, 0.0));
    world.insert(a);
    world.insert(b);
    for _ in 0..240 {
        step(&mut world, DT);
    }
    let body = world.bodies[1];
    let gap = body.position.y - 2.0 * reach;
    assert!(gap.abs() <= 0.02, "edge rest gap {gap}");
    assert!(
        body.position.x.abs() <= 1.0e-4 && body.position.z.abs() <= 1.0e-4,
        "drift {:?}",
        body.position
    );
    assert!(
        body.velocity.length() <= 0.05,
        "still moving {:?}",
        body.velocity
    );
}

#[test]
fn edge_contact_is_bitwise_deterministic() {
    let run = || {
        let mut world = crossed_cubes(-0.05, 1.0);
        world.bodies[1].velocity = Vec3::new(-3.0, 0.2, -0.1);
        let mut trace = Vec::new();
        for _ in 0..60 {
            step(&mut world, DT);
            trace.push(world.bodies[1].position.x.to_bits());
            trace.push(world.bodies[1].position.y.to_bits());
        }
        trace
    };
    assert_eq!(run(), run());
}
