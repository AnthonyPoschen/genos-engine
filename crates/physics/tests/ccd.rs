//! Speculative contacts stop a bullet that starts inside the margin.
//! A swept spin uses the rotational term in the time-of-impact bound.

use genos_math::Vec3;
use genos_physics::{step, Body, MotionQuality, Shape, World};

const DT: f32 = 1.0 / 60.0;

#[test]
fn a_bullet_inside_the_margin_does_not_tunnel() {
    let mut world = World::new(Vec3::ZERO);
    let mut bullet = Body::sphere(Vec3::ZERO, 0.02, 0.05);
    bullet.velocity = Vec3::new(10.0, 0.0, 0.0);
    bullet.motion = MotionQuality::Speculative;
    world.insert(bullet);
    world.insert(Body::cuboid(
        Vec3::new(0.25, 0.0, 0.0),
        0.0,
        Vec3::new(0.05, 1.0, 1.0),
    ));
    step(&mut world, DT);
    let center = world.bodies[0].position.x;
    assert!(
        center < 0.22,
        "speculative bullet center {center} left the impact side of the wall at 0.2"
    );
}

#[test]
fn a_fast_bullet_inside_the_margin_stops_and_the_discrete_one_tunnels() {
    let speculative = shot(MotionQuality::Speculative);
    let discrete = shot(MotionQuality::Discrete);
    let far_face = 0.45;
    assert!(
        speculative.trailing <= far_face,
        "speculative trailing {} passed the far face {far_face}",
        speculative.trailing
    );
    assert!(
        discrete.trailing > far_face,
        "discrete trailing {} should have tunneled past {far_face}",
        discrete.trailing
    );
    assert!(
        speculative.world.step_stats.ccd_applied,
        "speculative step did not record CCD"
    );
    assert!(
        !discrete.world.step_stats.ccd_applied,
        "discrete step recorded CCD"
    );
}

struct Shot {
    trailing: f32,
    world: World,
}

#[test]
fn rotational_energy_matches_the_box_and_sphere_moments() {
    let mut cube = Body::cuboid(Vec3::ZERO, 1.0, Vec3::new(1.0, 1.0, 1.0));
    cube.angular_velocity = Vec3::new(0.0, 0.0, 40.0);
    let expected = 0.5 * (1.0 / 3.0) * (1.0 + 1.0) * 40.0 * 40.0;
    let energy = cube.rotational_energy();
    assert!(
        (energy - expected).abs() < 1.0e-3,
        "cube energy {energy} wanted {expected}"
    );

    let mut ball = Body::sphere(Vec3::ZERO, 2.0, 0.5);
    ball.angular_velocity = Vec3::new(0.0, 3.0, 0.0);
    let moment = 0.4 * 2.0 * 0.5 * 0.5;
    let ball_expected = 0.5 * moment * 9.0;
    let ball_energy = ball.rotational_energy();
    assert!(
        (ball_energy - ball_expected).abs() < 1.0e-4,
        "sphere energy {ball_energy} wanted {ball_expected}"
    );
    assert_eq!(Body::sphere(Vec3::ZERO, 1.0, 1.0).rotational_energy(), 0.0);
}

#[test]
fn a_spinning_box_stays_on_the_approach_side_of_a_wall() {
    let mut cube = Body::cuboid(Vec3::ZERO, 8.0, Vec3::new(1.0, 1.0, 1.0));
    cube.velocity = Vec3::new(30.0, 0.0, 0.0);
    cube.angular_velocity = Vec3::new(0.0, 0.0, 40.0);
    cube.motion = MotionQuality::Swept;
    let wall = Body::cuboid(Vec3::new(3.1, 0.0, 0.0), 0.0, Vec3::new(0.1, 2.0, 2.0));
    let mut world = World::new(Vec3::ZERO);
    world.insert(cube);
    world.insert(wall);
    for _ in 0..8 {
        step(&mut world, DT);
    }
    let max_x = box_max_x(&world.bodies[0]);
    assert!(
        max_x <= 3.02,
        "spinning box reached {max_x}, the wall face is at 3"
    );
}

fn box_max_x(body: &Body) -> f32 {
    let Shape::Box { half_extents: half } = body.shape else {
        return f32::MAX;
    };
    let mut max_x = f32::MIN;
    for x in [-half.x, half.x] {
        for y in [-half.y, half.y] {
            for z in [-half.z, half.z] {
                let point = body.position + body.orientation.rotate(Vec3::new(x, y, z));
                max_x = max_x.max(point.x);
            }
        }
    }
    max_x
}

fn shot(motion: MotionQuality) -> Shot {
    let mut bullet = Body::sphere(Vec3::new(0.0, 2.0, 4.0), 0.02, 0.05);
    bullet.velocity = Vec3::new(3000.0, 0.0, 0.0);
    bullet.motion = motion;
    let wall = Body::cuboid(Vec3::new(0.3, 2.0, 4.0), 0.0, Vec3::new(0.15, 5.0, 1.0));
    let mut world = World::new(Vec3::ZERO);
    world.insert(bullet);
    world.insert(wall);
    step(&mut world, DT);
    let trailing = world.bodies[0].position.x - 0.05;
    Shot { trailing, world }
}
