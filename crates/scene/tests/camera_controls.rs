use genos_scene::{
    transform_point, update, view_proj, viewport_uv, Actions, Camera, CAMERA_HEIGHT,
    PITCH_LIMIT,
};
use std::f32::consts::FRAC_PI_2;

fn tick(camera: &mut Camera, actions: &Actions, dt: f32) {
    update(camera, actions, dt);
}

fn cold() -> Camera {
    Camera::new(0.0, 8.0, 0.0)
}

fn on_open_floor(x: f32, z: f32, yaw: f32) -> Camera {
    let scene = genos_scene::load_path(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    )))
    .unwrap();
    let mut camera = Camera::new(x, z, yaw);
    camera.attach_scene(&scene);
    camera
}

fn eye_near_rest(camera: &Camera) {
    let error = (camera.position.y - CAMERA_HEIGHT).abs();
    assert!(error < 0.05, "eye y {} error {error}", camera.position.y);
}

#[test]
fn forward_back_and_strafe_follow_yaw_near_eye_height() {
    let mut camera = on_open_floor(-6.0, 6.0, 0.0);
    tick(
        &mut camera,
        &Actions {
            forward: 1.0,
            ..Actions::default()
        },
        0.5,
    );
    eye_near_rest(&camera);
    assert!(camera.position.z < 6.0, "yaw 0 faces -Z");
    let z_after_forward = camera.position.z;

    tick(
        &mut camera,
        &Actions {
            forward: -1.0,
            ..Actions::default()
        },
        0.5,
    );
    assert!(camera.position.z > z_after_forward);
    eye_near_rest(&camera);

    let mut strafe = on_open_floor(-6.0, 6.0, 0.0);
    tick(
        &mut strafe,
        &Actions {
            strafe: 1.0,
            ..Actions::default()
        },
        0.5,
    );
    assert!(strafe.position.x > -6.0, "D moves toward +X");
    eye_near_rest(&strafe);

    let mut left = on_open_floor(-6.0, 6.0, 0.0);
    tick(
        &mut left,
        &Actions {
            strafe: -1.0,
            ..Actions::default()
        },
        0.25,
    );
    assert!(left.position.x < -6.0);
}

#[test]
fn mouse_and_right_stick_change_yaw_and_pitch_and_pitch_clamps() {
    let mut camera = cold();
    tick(
        &mut camera,
        &Actions {
            capture_click: true,
            mouse_dx: 40.0,
            mouse_dy: -20.0,
            ..Actions::default()
        },
        0.016,
    );
    assert!(camera.captured);
    assert!(camera.yaw > 0.0);
    assert!(camera.pitch > 0.0);

    let yaw_before_stick = camera.yaw;
    let pitch_before_stick = camera.pitch;
    tick(
        &mut camera,
        &Actions {
            look_x: 1.0,
            look_y: 1.0,
            ..Actions::default()
        },
        0.2,
    );
    assert!(camera.yaw > yaw_before_stick);
    assert!(camera.pitch > pitch_before_stick);

    tick(
        &mut camera,
        &Actions {
            capture_click: true,
            mouse_dy: -100_000.0,
            ..Actions::default()
        },
        0.016,
    );
    assert!((camera.pitch - PITCH_LIMIT).abs() < 1.0e-4);
    assert!(camera.pitch < FRAC_PI_2);

    tick(
        &mut camera,
        &Actions {
            capture_click: true,
            mouse_dy: 100_000.0,
            ..Actions::default()
        },
        0.016,
    );
    assert!((camera.pitch + PITCH_LIMIT).abs() < 1.0e-4);
    assert!(camera.pitch > -FRAC_PI_2);
}

#[test]
fn first_click_captures_and_escape_releases() {
    let mut camera = cold();
    assert!(!camera.captured);
    tick(
        &mut camera,
        &Actions {
            capture_click: true,
            ..Actions::default()
        },
        0.016,
    );
    assert!(camera.captured);
    let yaw = camera.yaw;
    tick(
        &mut camera,
        &Actions {
            escape: true,
            mouse_dx: 80.0,
            ..Actions::default()
        },
        0.016,
    );
    assert!(!camera.captured);
    assert_eq!(camera.yaw, yaw);
}

#[test]
fn a_point_in_front_of_the_camera_lands_in_vulkan_clip() {
    let camera = cold();
    let vp = view_proj(&camera, 16.0 / 9.0);
    let clip = transform_point(&vp, [0.0, CAMERA_HEIGHT, 6.0]);
    assert!(clip[3] > 0.0);
    let ndc_z = clip[2] / clip[3];
    assert!(ndc_z > 0.0 && ndc_z < 1.0, "ndc z {ndc_z}");
}

#[test]
fn the_opening_view_puts_the_floor_low_and_the_corner_on_the_right() {
    let aspect = 16.0 / 9.0;
    let camera = Camera::opening();
    let floor = [0.0, 0.6, 0.0];
    let floor_clip = transform_point(&view_proj(&camera, aspect), floor);
    assert!(floor_clip[3] > 0.0, "floor is behind the camera");
    let floor_ndc_y = floor_clip[1] / floor_clip[3];
    assert!(
        floor_ndc_y > 0.0,
        "floor below the eye must sit in the lower half, ndc y {floor_ndc_y}"
    );
    let floor_uv = viewport_uv(&camera, aspect, floor).unwrap();
    assert!(floor_uv[1] > 0.5, "viewport v {floor_uv:?}");

    let corner = [6.0, 1.0, 5.0];
    let corner_clip = transform_point(&view_proj(&camera, aspect), corner);
    assert!(corner_clip[3] > 0.0, "corner is behind the camera");
    let corner_ndc_x = corner_clip[0] / corner_clip[3];
    assert!(
        corner_ndc_x > 0.0,
        "the corner must be screen-right, ndc x {corner_ndc_x}"
    );
    let corner_uv = viewport_uv(&camera, aspect, corner).unwrap();
    assert!(corner_uv[0] > 0.5, "viewport u {corner_uv:?}");

    let yaw0 = Camera::new(0.0, 8.0, 0.0);
    let strafe_right = [1.5, CAMERA_HEIGHT, 6.0];
    let strafe_clip = transform_point(&view_proj(&yaw0, aspect), strafe_right);
    assert!(strafe_clip[3] > 0.0);
    let strafe_ndc_x = strafe_clip[0] / strafe_clip[3];
    assert!(
        strafe_ndc_x > 0.0,
        "strafe toward +X must be screen-right, ndc x {strafe_ndc_x}"
    );
    eprintln!("ndc floor_y={floor_ndc_y:.3} corner_x={corner_ndc_x:.3} strafe_x={strafe_ndc_x:.3}");
}
