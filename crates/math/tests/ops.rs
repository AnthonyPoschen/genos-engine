use genos_math::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};
use std::f32::consts::FRAC_PI_2;

fn near(got: f32, expected: f32) {
    assert!(
        (got - expected).abs() <= 1.0e-5,
        "got {got} expected {expected}"
    );
}

fn near_vec3(got: Vec3, expected: Vec3) {
    near(got.x, expected.x);
    near(got.y, expected.y);
    near(got.z, expected.z);
}

#[test]
fn dot_cross_length_normalize_and_lerp_match_hand_values() {
    let left = Vec3::new(1.0, 2.0, 3.0);
    let right = Vec3::new(4.0, 5.0, 6.0);
    near(left.dot(right), 32.0);
    near_vec3(left.cross(right), Vec3::new(-3.0, 6.0, -3.0));
    near_vec3(Vec3::X.cross(Vec3::Y), Vec3::Z);
    near(Vec3::new(3.0, 4.0, 0.0).length(), 5.0);
    near_vec3(Vec3::new(0.0, 3.0, 0.0).normalize(), Vec3::Y);
    near_vec3(Vec3::ZERO.normalize(), Vec3::ZERO);
    near_vec3(
        Vec3::ZERO.lerp(Vec3::new(2.0, 4.0, 6.0), 0.5),
        Vec3::new(1.0, 2.0, 3.0),
    );
}

#[test]
fn vec2_and_vec4_place_a_point() {
    let a = Vec2::new(1.0, 2.0);
    let b = Vec2::new(3.0, -4.0);
    near(a.dot(b), -5.0);
    near(b.length(), 5.0);
    near_vec2(b.normalize(), Vec2::new(0.6, -0.8));
    near_vec2(a.lerp(b, 0.5), Vec2::new(2.0, -1.0));
    near_vec2(a + b * 2.0, Vec2::new(7.0, -6.0));

    let c = Vec4::new(1.0, 0.0, 0.0, 1.0);
    let d = Vec4::new(0.0, 2.0, 0.0, 1.0);
    near(c.dot(d), 1.0);
    near(Vec4::new(0.0, 3.0, 0.0, 0.0).length(), 3.0);
    near_vec4(c.lerp(d, 0.5), Vec4::new(0.5, 1.0, 0.0, 1.0));
    near_vec4(
        c + Vec4::new(4.0, 0.0, 0.0, 0.0),
        Vec4::new(5.0, 0.0, 0.0, 1.0),
    );
}

fn near_vec2(got: Vec2, expected: Vec2) {
    near(got.x, expected.x);
    near(got.y, expected.y);
}

fn near_vec4(got: Vec4, expected: Vec4) {
    near(got.x, expected.x);
    near(got.y, expected.y);
    near(got.z, expected.z);
    near(got.w, expected.w);
}

#[test]
fn a_scaled_direction_moves_a_position() {
    let position = Vec3::new(1.0, 2.0, 3.0);
    let direction = Vec3::new(0.0, 3.0, 0.0).normalize();
    near_vec3(position + direction * 4.0, Vec3::new(1.0, 6.0, 3.0));
}

#[test]
fn a_translated_rotated_and_scaled_point_matches_the_composed_matrix() {
    let point = Vec3::new(1.0, 0.0, 0.0);
    let scale = Mat4::from_scale(Vec3::new(2.0, 3.0, 4.0));
    let rotation = Mat4::from_axis_angle(Vec3::Z, FRAC_PI_2);
    let translation = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
    let placed = (translation * rotation * scale).transform_point(point);
    // Scale x to 2, rotate +90° around Z to +Y, then add (1, 2, 3).
    near_vec3(placed, Vec3::new(1.0, 4.0, 3.0));
    near_vec3(
        (Mat4::IDENTITY * translation).transform_point(point),
        Vec3::new(2.0, 2.0, 3.0),
    );
}

#[test]
fn quaternion_axis_angle_matches_the_matrix() {
    let turn = Quat::from_axis_angle(Vec3::Z, FRAC_PI_2);
    let direction = Vec3::X;
    near_vec3(turn.rotate(direction), Vec3::Y);
    near_vec3(Mat4::from_quat(turn).transform_vector(direction), Vec3::Y);
    near_vec3(Mat3::from_quat(turn).transform(direction), Vec3::Y);
    near_vec3(
        Mat3::from_axis_angle(Vec3::Y, FRAC_PI_2).transform(Vec3::Z),
        Vec3::X,
    );
}

#[test]
fn slerp_with_a_negative_dot_takes_the_short_arc() {
    let turn = Quat::from_axis_angle(Vec3::Y, 170.0_f32.to_radians());
    let stored = -turn;
    assert!(
        Quat::IDENTITY.dot(stored) < 0.0,
        "the stored side is the long quaternion"
    );
    let mid = Quat::IDENTITY.slerp(stored, 0.5);
    let aimed = mid.rotate(Vec3::X);
    // The short arc is +85°. The long arc would aim X toward -X.
    assert!(aimed.x > 0.0, "short arc x {aimed:?}");
    assert!(aimed.z < 0.0, "short arc z {aimed:?}");
    near(turn.normalize().length(), 1.0);
}
