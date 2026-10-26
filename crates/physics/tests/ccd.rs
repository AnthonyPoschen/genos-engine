//! Speculative contacts stop a bullet that starts inside the margin.

use genos_math::Vec3;
use genos_physics::{step, Body, MotionQuality, World};

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
