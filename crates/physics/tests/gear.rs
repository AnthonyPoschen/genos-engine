//! T1 gear on an aircraft. The wing and the wheels are not bodies.
//!
//! A miss adds no force, so the chassis matches the A1 twin. On the plane,
//! lift and the four springs share the weight. Space thrust and sea are not
//! in this stage. Angle of attack is not read.

use genos_math::Vec3;
use genos_physics::{step, Body, GroundContact, MotionQuality, World, GRAVITY};

const DT: f32 = 1.0 / 60.0;
const MISS_STEPS: u32 = 120;
const SHARE_STEPS: u32 = 180;
/// `0.4 - (9810 - 1569.6) / 4 / k` with `k = 250 * (2π * 4)²`.
const SETTLED_Y: f32 = 0.386_954;
/// A plane this far under the trimmed centre sits inside `max_length` 0.5.
const MUTATED_PLANE: f32 = 99.7;

#[test]
fn a_gear_that_misses_matches_the_aircraft() {
    let mut geared = genos_physics::gear_miss_world();
    let mut twin = genos_physics::gear_miss_world();
    let mut aircraft = genos_physics::aero_trim_world();
    let start_y = chassis(&geared).position.y;
    for _ in 0..MISS_STEPS {
        step(&mut geared, DT);
        step(&mut twin, DT);
        step(&mut aircraft, DT);
        let body = chassis(&geared);
        assert!(
            same_body(&body, &chassis(&aircraft)),
            "a miss diverged from the aircraft at {:?} v {:?}",
            body.position,
            body.velocity
        );
        assert!(
            same_body(&body, &chassis(&twin)),
            "two gear runs diverged"
        );
        assert!(same_lengths(&geared, &twin), "wheel lengths diverged");
        assert_eq!(geared.step_stats.wheel_rays, 4);
        assert_eq!(geared.step_stats.vehicle_count, 1);
        assert_eq!(aircraft.step_stats.wheel_rays, 0);
        assert_eq!(body.position.y.to_bits(), start_y.to_bits());
    }
    let body = chassis(&geared);
    assert_eq!(body.velocity.x.to_bits(), 50.0_f32.to_bits());
    assert_eq!(body.velocity.y.to_bits(), 0.0_f32.to_bits());
    assert_eq!(body.angular_velocity, Vec3::ZERO);
    for length in lengths(&geared) {
        assert_eq!(length.to_bits(), 0.5_f32.to_bits());
    }
    assert!(body.spring.is_none());
    assert!(!body.motor);
    assert_eq!(body.motion, MotionQuality::Discrete);
}

#[test]
fn a_gear_on_the_plane_shares_the_weight() {
    let mut world = genos_physics::gear_share_world();
    let mut twin = genos_physics::gear_share_world();
    assert_eq!(world.vehicles[0].chassis, 1);
    for _ in 0..SHARE_STEPS {
        step(&mut world, DT);
        step(&mut twin, DT);
        let body = chassis(&world);
        assert!(
            body.position.y.is_finite() && body.velocity.x.is_finite(),
            "gear left the finite range at {:?} v {:?}",
            body.position,
            body.velocity
        );
        assert!(
            same_body(&body, &chassis(&twin)),
            "two share runs diverged at {:?}",
            body.position
        );
        assert!(same_lengths(&world, &twin), "wheel lengths diverged");
        assert_eq!(world.step_stats.wheel_rays, 4);
        assert_eq!(world.step_stats.vehicle_count, 1);
    }
    let body = chassis(&world);
    let height_error = (body.position.y - SETTLED_Y).abs();
    assert!(
        height_error < 0.02,
        "centre y {} is {height_error} m from {SETTLED_Y}",
        body.position.y
    );
    assert!(
        body.velocity.y.abs() < 0.05,
        "vertical speed {}",
        body.velocity.y
    );
    assert!(
        (body.velocity.x - 20.0).abs() < 0.05,
        "forward speed {}",
        body.velocity.x
    );
    let up = body.orientation.rotate(Vec3::Y);
    let tilt = up.dot(Vec3::Y).clamp(-1.0, 1.0).acos();
    assert!(tilt < 0.01, "body up tilted {tilt} rad");
    assert_eq!(body.normal_impulse.to_bits(), 0.0_f32.to_bits());
    assert!(!world.step_stats.ccd_applied);
    assert_eq!(world.step_stats.toi_failed, 0);
    assert_eq!(world.step_stats.pair_slices, 0);
    assert!(body.spring.is_none());
    assert!(!body.motor);
    assert_eq!(body.motion, MotionQuality::Discrete);
    assert_eq!(world.gravity, GRAVITY);
}

/// Negative control for the miss fixture.
///
/// The mutation puts a plane 0.3 m under the centre, inside `max_length` 0.5.
/// That hit must fail the A1 bit match: the ray pushes. The unmutated gear
/// still matches. A step that ignored the hit would keep the mutation equal
/// to the aircraft, and this test would fail.
#[test]
fn a_plane_inside_max_length_fails_the_aircraft_match() {
    let mut clean = genos_physics::gear_miss_world();
    let mut aircraft = genos_physics::aero_trim_world();
    let mut mutated = genos_physics::gear_miss_world();
    mutated.insert(Body::plane(Vec3::Y, MUTATED_PLANE));
    step(&mut clean, DT);
    step(&mut aircraft, DT);
    step(&mut mutated, DT);

    assert!(
        same_body(&chassis(&clean), &chassis(&aircraft)),
        "the unmutated miss left the aircraft"
    );
    assert!(
        !same_body(&chassis(&mutated), &chassis(&aircraft)),
        "the in-range plane still matched the aircraft"
    );
    assert!(
        chassis(&mutated).velocity.y > chassis(&aircraft).velocity.y,
        "the hit did not push up"
    );
    let hit = 100.0 - MUTATED_PLANE;
    for length in lengths(&mutated) {
        assert_eq!(length.to_bits(), hit.to_bits());
    }
    for length in lengths(&clean) {
        assert_eq!(length.to_bits(), 0.5_f32.to_bits());
    }
    assert_eq!(clean.step_stats.wheel_rays, 4);
    assert_eq!(mutated.step_stats.wheel_rays, 4);
    assert_eq!(aircraft.step_stats.wheel_rays, 0);
    assert!(chassis(&mutated).spring.is_none());
    assert!(!chassis(&mutated).motor);
    assert_eq!(chassis(&mutated).motion, MotionQuality::Discrete);
}

fn chassis(world: &World) -> Body {
    world.bodies[world.vehicles[0].chassis]
}

fn lengths(world: &World) -> [f32; 4] {
    let Some(GroundContact::Wheeled(ground)) = world.vehicles[0].ground else {
        return [f32::NAN; 4];
    };
    ground.lengths
}

fn same_lengths(left: &World, right: &World) -> bool {
    lengths(left)
        .iter()
        .zip(lengths(right))
        .all(|(a, b)| a.to_bits() == b.to_bits())
}

fn same_body(left: &Body, right: &Body) -> bool {
    left.position.x.to_bits() == right.position.x.to_bits()
        && left.position.y.to_bits() == right.position.y.to_bits()
        && left.position.z.to_bits() == right.position.z.to_bits()
        && left.velocity.x.to_bits() == right.velocity.x.to_bits()
        && left.velocity.y.to_bits() == right.velocity.y.to_bits()
        && left.velocity.z.to_bits() == right.velocity.z.to_bits()
        && left.angular_velocity.x.to_bits() == right.angular_velocity.x.to_bits()
        && left.angular_velocity.y.to_bits() == right.angular_velocity.y.to_bits()
        && left.angular_velocity.z.to_bits() == right.angular_velocity.z.to_bits()
        && left.orientation.x.to_bits() == right.orientation.x.to_bits()
        && left.orientation.y.to_bits() == right.orientation.y.to_bits()
        && left.orientation.z.to_bits() == right.orientation.z.to_bits()
        && left.orientation.w.to_bits() == right.orientation.w.to_bits()
}
