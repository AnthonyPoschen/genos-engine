//! Rotational contact. These lock the behaviour ADR 0034 requires from the
//! per-point impulse: a body tips when its centre passes an edge, spin on a
//! surface bleeds off through friction at the contact, a hull pile settles
//! and sleeps, mechanical energy does not grow, and a resting stack stays.

use genos_math::{Quat, Vec3};
use genos_physics::{step, Body, World, GRAVITY};

const DT: f32 = 1.0 / 60.0;

fn cube_tris(half: Vec3) -> Vec<[Vec3; 3]> {
    let v = |x: f32, y: f32, z: f32| Vec3::new(x * half.x, y * half.y, z * half.z);
    let (nnn, pnn, ppn, npn) = (
        v(-1.0, -1.0, -1.0),
        v(1.0, -1.0, -1.0),
        v(1.0, 1.0, -1.0),
        v(-1.0, 1.0, -1.0),
    );
    let (nnp, pnp, ppp, npp) = (
        v(-1.0, -1.0, 1.0),
        v(1.0, -1.0, 1.0),
        v(1.0, 1.0, 1.0),
        v(-1.0, 1.0, 1.0),
    );
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

fn tilt_z(orientation: Quat) -> f32 {
    2.0 * orientation.z.atan2(orientation.w)
}

#[test]
fn a_box_past_the_edge_tips_and_one_over_the_ledge_stays() {
    let mut past = World::new(GRAVITY);
    past.insert(Body::cuboid(
        Vec3::new(-1.0, -0.5, 0.0),
        0.0,
        Vec3::new(1.0, 0.5, 1.0),
    ));
    let mut box_past = Body::cuboid(Vec3::new(0.2, 0.5, 0.0), 1.0, Vec3::new(0.5, 0.5, 0.5));
    box_past.friction = 0.9;
    past.insert(box_past);
    for _ in 0..180 {
        step(&mut past, DT);
    }
    let tipped = past.bodies[1];
    assert!(
        tipped.angular_velocity.length() > 0.3 || tilt_z(tipped.orientation).abs() > 0.2,
        "overhang did not rotate: ω={} tilt={}",
        tipped.angular_velocity.length(),
        tilt_z(tipped.orientation)
    );

    let mut held = World::new(GRAVITY);
    held.insert(Body::cuboid(
        Vec3::new(-1.0, -0.5, 0.0),
        0.0,
        Vec3::new(1.0, 0.5, 1.0),
    ));
    let mut box_held = Body::cuboid(Vec3::new(-0.25, 0.5, 0.0), 1.0, Vec3::new(0.5, 0.5, 0.5));
    box_held.friction = 0.9;
    held.insert(box_held);
    for _ in 0..180 {
        step(&mut held, DT);
    }
    let stayed = held.bodies[1];
    assert!(
        stayed.position.y > 0.35,
        "supported box fell through at y {}",
        stayed.position.y
    );
    assert!(
        tilt_z(stayed.orientation).abs() < 0.15,
        "supported box tipped by {}",
        tilt_z(stayed.orientation)
    );
    assert!(
        stayed.position.x < 0.0,
        "supported box walked off to x {}",
        stayed.position.x
    );
}

#[test]
fn a_spinning_box_on_the_floor_slows_down() {
    let mut world = World::new(GRAVITY);
    world.insert(Body::plane(Vec3::Y, 0.0));
    let mut box_body = Body::cuboid(Vec3::new(0.0, 0.5, 0.0), 1.0, Vec3::new(0.5, 0.5, 0.5));
    box_body.angular_velocity = Vec3::new(0.0, 6.0, 0.0);
    box_body.friction = 0.8;
    world.insert(box_body);
    for _ in 0..120 {
        step(&mut world, DT);
    }
    let spin = world.bodies[1].angular_velocity.y.abs();
    assert!(spin < 3.0, "floor friction left spin {spin} rad/s from 6");
}

#[test]
fn a_hull_pile_settles_and_sleeps_without_jitter() {
    let mut world = World::new(GRAVITY);
    world.insert(Body::mesh(
        Vec3::new(0.0, -0.5, 0.0),
        0.0,
        &cube_tris(Vec3::new(6.0, 0.5, 6.0)),
    ));
    let cube = cube_tris(Vec3::new(0.4, 0.4, 0.4));
    for ((x, y, z), turn) in [
        (0.0, 0.5, 0.0),
        (0.05, 1.35, 0.02),
        (-0.04, 2.2, -0.03),
        (0.02, 3.05, 0.04),
    ]
    .into_iter()
    .zip([0.0_f32, 0.2, 0.45, 0.7])
    {
        let mut body = Body::mesh(Vec3::new(x, y, z), 1.0, &cube);
        body.friction = 0.8;
        body.orientation = Quat::from_axis_angle(Vec3::new(0.2, 1.0, 0.1), turn);
        world.insert(body);
    }
    for _ in 0..540 {
        step(&mut world, DT);
    }
    let marks: Vec<Vec3> = world
        .bodies
        .iter()
        .skip(1)
        .map(|body| body.position)
        .collect();
    for _ in 0..60 {
        step(&mut world, DT);
    }
    for (body, mark) in world.bodies.iter().skip(1).zip(marks) {
        let drift = (body.position - mark).length();
        assert!(
            body.sleeping,
            "hull at {:?} stayed awake, speed {}",
            body.position,
            body.velocity.length()
        );
        assert!(drift < 5.0e-3, "sleeping hull jittered {drift} m");
    }
}

#[test]
fn mechanical_energy_does_not_grow_over_six_hundred_steps() {
    let mut world = World::new(Vec3::ZERO);
    let mut tumble = Body::cuboid(Vec3::new(0.0, 2.0, 0.0), 2.0, Vec3::new(0.2, 0.5, 0.8));
    tumble.angular_velocity = Vec3::new(3.0, 1.1, 0.4);
    world.insert(tumble);
    world.insert(Body::plane(Vec3::Y, 0.0));
    let mut drop = Body::cuboid(Vec3::new(3.0, 1.5, 0.0), 1.0, Vec3::new(0.3, 0.3, 0.3));
    drop.friction = 0.4;
    world.insert(drop);
    // Gravity for the drop only: the tumble is far from the plane and is given
    // no initial fall. Both share the world gravity so the drop can land.
    world.gravity = GRAVITY;
    let start = mechanical(&world);
    for _ in 0..600 {
        step(&mut world, DT);
        let now = mechanical(&world);
        assert!(
            now <= start * (1.0 + 1.0e-4) + 1.0e-3,
            "energy grew from {start} to {now}"
        );
    }
}

fn mechanical(world: &World) -> f32 {
    let mut energy = 0.0;
    for body in &world.bodies {
        if body.inverse_mass == 0.0 {
            continue;
        }
        let mass = body.mass();
        let speed = body.velocity.length();
        energy += 0.5 * mass * speed * speed;
        energy += body.rotational_energy();
        energy += mass * -GRAVITY.y * body.position.y;
    }
    energy
}

#[test]
fn a_resting_stack_stays_put() {
    let mut world = World::new(GRAVITY);
    world.insert(Body::cuboid(
        Vec3::new(0.0, -0.5, 0.0),
        0.0,
        Vec3::new(2.0, 0.5, 2.0),
    ));
    let half = Vec3::new(0.4, 0.4, 0.4);
    let starts = [0.4_f32, 1.2, 2.0];
    for y in starts {
        let mut body = Body::cuboid(Vec3::new(0.0, y, 0.0), 1.0, half);
        body.friction = 0.5;
        world.insert(body);
    }
    for _ in 0..300 {
        step(&mut world, DT);
    }
    for (index, start_y) in starts.into_iter().enumerate() {
        let body = world.bodies[index + 1];
        assert!(
            (body.position.x).abs() < 0.02 && body.position.z.abs() < 0.02,
            "box {index} slid to {:?}",
            body.position
        );
        assert!(
            (body.position.y - start_y).abs() < 0.05,
            "box {index} height {} left {start_y}",
            body.position.y
        );
        assert!(
            tilt_z(body.orientation).abs() < 0.08,
            "box {index} rolled to {}",
            tilt_z(body.orientation)
        );
    }
}

#[test]
fn two_boxes_colliding_off_centre_exchange_angular_momentum() {
    let mut world = World::new(Vec3::ZERO);
    let mut hitter = Body::cuboid(Vec3::new(-1.2, 0.5, 0.0), 1.0, Vec3::new(0.5, 0.5, 0.5));
    hitter.velocity = Vec3::new(3.0, 0.0, 0.0);
    world.insert(hitter);
    world.insert(Body::cuboid(
        Vec3::new(0.15, 0.5, 0.55),
        1.0,
        Vec3::new(0.5, 0.5, 0.5),
    ));
    for _ in 0..40 {
        step(&mut world, DT);
    }
    let hitter = &world.bodies[0];
    let target = &world.bodies[1];
    let spin_hitter = hitter.angular_velocity.length();
    let spin_target = target.angular_velocity.length();
    assert!(
        spin_hitter > 0.15,
        "off-centre hitter gained no spin ({spin_hitter} rad/s)"
    );
    assert!(
        spin_target > 0.15,
        "off-centre target gained no spin ({spin_target} rad/s)"
    );
    assert!(
        target.velocity.x > 0.4,
        "target took no forward momentum ({})",
        target.velocity.x
    );
    assert!(
        hitter.velocity.x < 2.4,
        "hitter kept its speed ({})",
        hitter.velocity.x
    );
    let shared = hitter.angular_velocity.y * target.angular_velocity.y;
    assert!(
        shared > 0.0,
        "offset along z gives both bodies the same spin sense, got {:?} and {:?}",
        hitter.angular_velocity,
        target.angular_velocity
    );
}

#[test]
fn a_yawed_hull_pile_sleeps_without_walking() {
    let mut world = World::new(GRAVITY);
    world.insert(Body::mesh(
        Vec3::new(0.0, -0.5, 0.0),
        0.0,
        &cube_tris(Vec3::new(4.0, 0.5, 4.0)),
    ));
    let cube = cube_tris(Vec3::new(0.45, 0.35, 0.45));
    let layers = [
        (0.4_f32, 0.4_f32),
        (1.15, 0.9),
        (1.9, 1.3),
        (2.65, 1.8),
        (3.4, 2.2),
    ];
    for (height, yaw) in layers {
        let mut body = Body::mesh(Vec3::new(0.0, height, 0.0), 1.0, &cube);
        body.friction = 0.7;
        body.orientation = Quat::from_axis_angle(Vec3::Y, yaw);
        world.insert(body);
    }
    let start: Vec<Vec3> = world
        .bodies
        .iter()
        .skip(1)
        .map(|body| body.position)
        .collect();
    for _ in 0..600 {
        step(&mut world, DT);
    }
    let mut drift = 0.0_f32;
    for (body, from) in world.bodies.iter().skip(1).zip(start) {
        assert!(
            body.sleeping,
            "yawed hull at {:?} stayed awake, speed {} spin {}",
            body.position,
            body.velocity.length(),
            body.angular_velocity.length()
        );
        let moved = Vec3::new(body.position.x - from.x, 0.0, body.position.z - from.z).length();
        drift = drift.max(moved);
        assert!(
            body.position.x.abs() < 1.5 && body.position.z.abs() < 1.5,
            "hull walked to {:?}",
            body.position
        );
    }
    assert!(drift < 0.75, "yawed pile drifted {drift} m across the slab");
}

#[test]
fn a_face_offset_one_percent_does_not_yaw() {
    let mut world = World::new(GRAVITY);
    world.insert(Body::cuboid(
        Vec3::new(0.0, -0.5, 0.0),
        0.0,
        Vec3::new(2.0, 0.5, 2.0),
    ));
    let half = 0.5_f32;
    let mut box_body = Body::cuboid(Vec3::new(0.01, half, 0.0), 1.0, Vec3::new(half, half, half));
    box_body.friction = 0.6;
    world.insert(box_body);
    for _ in 0..180 {
        step(&mut world, DT);
    }
    let body = world.bodies[1];
    let yaw = yaw_about_y(body.orientation).abs();
    assert!(
        yaw < 0.02,
        "1% face offset yawed {yaw} rad, spin {:?}",
        body.angular_velocity
    );
    assert!(
        body.angular_velocity.y.abs() < 0.05,
        "1% face offset spun about the normal at {}",
        body.angular_velocity.y
    );
}

fn yaw_about_y(orientation: Quat) -> f32 {
    let sin_y = 2.0 * (orientation.w * orientation.y + orientation.x * orientation.z);
    let cos_y = 1.0 - 2.0 * (orientation.y * orientation.y + orientation.z * orientation.z);
    sin_y.atan2(cos_y)
}

#[test]
fn two_runs_of_the_overhang_match_bit_for_bit() {
    fn pose_bits(world: &World) -> Vec<u32> {
        let mut bits = Vec::new();
        for body in &world.bodies {
            bits.push(body.position.x.to_bits());
            bits.push(body.position.y.to_bits());
            bits.push(body.position.z.to_bits());
            bits.push(body.orientation.x.to_bits());
            bits.push(body.orientation.y.to_bits());
            bits.push(body.orientation.z.to_bits());
            bits.push(body.orientation.w.to_bits());
            bits.push(body.angular_velocity.x.to_bits());
            bits.push(body.angular_velocity.y.to_bits());
            bits.push(body.angular_velocity.z.to_bits());
        }
        bits
    }
    fn run() -> Vec<u32> {
        let mut world = World::new(GRAVITY);
        world.insert(Body::cuboid(
            Vec3::new(-1.0, -0.5, 0.0),
            0.0,
            Vec3::new(1.0, 0.5, 1.0),
        ));
        let mut box_body = Body::cuboid(Vec3::new(0.2, 0.5, 0.0), 1.0, Vec3::new(0.5, 0.5, 0.5));
        box_body.friction = 0.9;
        world.insert(box_body);
        for _ in 0..90 {
            step(&mut world, DT);
        }
        pose_bits(&world)
    }
    assert_eq!(run(), run());
}
