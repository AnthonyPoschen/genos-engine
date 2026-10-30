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

#[test]
fn crossed_edges_overlapping_separate_along_x_by_the_edge_depth() {
    let depth = 0.05_f32;
    let mut world = crossed_cubes(-depth, 1.0);
    let before = [world.bodies[0].position, world.bodies[1].position];
    step(&mut world, DT);
    let moved_a = world.bodies[0].position - before[0];
    let moved_b = world.bodies[1].position - before[1];
    // Pushed apart along x only.
    assert!(
        moved_a.x < 0.0 && moved_b.x > 0.0,
        "a {moved_a:?} b {moved_b:?}"
    );
    for moved in [moved_a, moved_b] {
        assert!(
            moved.y.abs() <= 1.0e-6 && moved.z.abs() <= 1.0e-6,
            "sideways {moved:?}"
        );
    }
    // Correction is bounded by the true edge depth, not a face overlap.
    let pushed = moved_b.x - moved_a.x;
    assert!(pushed <= depth, "pushed {pushed} for depth {depth}");
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
