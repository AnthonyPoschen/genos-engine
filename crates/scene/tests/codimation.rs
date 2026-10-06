use genos_physics::Body;
use genos_scene::{
    update, Actions, Camera, Codimation, Easing, Floor, Light, Scene, Shape, Solid, Vec3, Wall,
};

const SEGMENT_SECONDS: f32 = 1.0;

fn room(blocker: bool) -> Scene {
    let mut walls = vec![Wall {
        position: Vec3::new(0.0, 0.0, -4.0),
        half_x: 6.0,
        half_z: 0.2,
        height: 3.0,
        color: [1.0, 1.0, 1.0],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    }];
    if blocker {
        walls.push(Wall {
            position: Vec3::new(1.2, 0.0, 0.4),
            half_x: 0.15,
            half_z: 2.5,
            height: 4.0,
            color: [0.8, 0.8, 0.8],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        });
    }
    Scene {
        floor: Floor {
            position: Vec3::ZERO,
            half_x: 30.0,
            half_z: 30.0,
            color: [1.0, 1.0, 1.0],
        },
        walls,
        solids: vec![
            Solid {
                shape: Shape::Square,
                position: Vec3::ZERO,
                size: 1.0,
                height: 1.2,
                color: [1.0, 0.0, 0.0],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            },
            Solid {
                shape: Shape::Square,
                position: Vec3::new(-6.0, 0.0, 3.0),
                size: 1.0,
                height: 1.2,
                color: [0.0, 1.0, 0.0],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            },
        ],
        lights: vec![Light {
            position: Vec3::new(0.0, 7.0, 0.0),
            color: [1.0, 1.0, 1.0],
        }],
    }
}

fn rig(scene: &Scene) -> Camera {
    let mut camera = Camera::new(0.0, 8.0, 0.0);
    camera.attach_scene(scene);
    camera
}

fn frames(camera: &mut Camera, scene: &mut Scene, count: i32) {
    for _ in 0..count {
        update(camera, scene, &Actions::default(), 1.0 / 60.0);
    }
}

fn bind(camera: &mut Camera, scene: &Scene, easing: Easing, points: Vec<Vec3>) {
    let motion = Codimation::new(points, easing, SEGMENT_SECONDS).expect("path");
    assert!(camera.codimate(scene, 0, motion));
}

fn solid_body<'a>(camera: &'a Camera, scene: &Scene, solid: usize) -> &'a Body {
    let index = camera.view - scene.solids.len() + solid;
    &camera.physics.bodies[index]
}

fn same_volume(solid: &Solid, body: &Body) {
    let lift = match solid.shape {
        Shape::Square => solid.height * 0.5,
        Shape::Circle => 0.0,
    };
    let center = solid.position + Vec3::new(0.0, lift, 0.0);
    let error = (body.position - center).length();
    assert!(error < 0.05, "scene and body differ by {error}");
}

fn path() -> Vec<Vec3> {
    vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(3.0, 1.0, 1.0)]
}

#[test]
fn a_clear_segment_ends_on_the_point_and_the_loop_leaves_that_point() {
    let points = path();
    let end = points[1];
    let start = points[0];
    let mut scene = room(false);
    scene.solids[0].position = start;
    let parked = scene.solids[1].position;
    let mut camera = rig(&scene);
    let parked_body = solid_body(&camera, &scene, 1).position;
    bind(&mut camera, &scene, Easing::Linear, points);
    frames(&mut camera, &mut scene, 60);
    let arrived = (scene.solids[0].position - end).length();
    assert!(
        arrived < 0.05,
        "segment end {:?} error {arrived}",
        scene.solids[0].position
    );
    same_volume(&scene.solids[0], solid_body(&camera, &scene, 0));

    frames(&mut camera, &mut scene, 60);
    let back = (scene.solids[0].position - start).length();
    let stuck = (scene.solids[0].position - end).length();
    assert!(back < 0.05, "cycle did not return, error {back}");
    assert!(stuck > 0.5, "stuck at the last position");
    same_volume(&scene.solids[0], solid_body(&camera, &scene, 0));

    frames(&mut camera, &mut scene, 15);
    let after = scene.solids[0].position;
    assert!(
        (after - start).length() > 0.1,
        "time past the cycle did not move the solid"
    );
    assert!(
        (after - end).length() > (after - start).length(),
        "past the cycle the solid stayed on the last position"
    );
    assert_eq!(scene.solids[1].position, parked);
    assert_eq!(solid_body(&camera, &scene, 1).position, parked_body);
}

#[test]
fn two_easings_differ_mid_segment_and_both_finish_on_the_end() {
    let points = path();
    let end = points[1];
    let start = points[0];
    let mut linear_scene = room(false);
    let mut eased_scene = room(false);
    linear_scene.solids[0].position = start;
    eased_scene.solids[0].position = start;
    let mut linear = rig(&linear_scene);
    let mut eased = rig(&eased_scene);
    bind(&mut linear, &linear_scene, Easing::Linear, points.clone());
    bind(&mut eased, &eased_scene, Easing::EaseIn, points);
    frames(&mut linear, &mut linear_scene, 15);
    frames(&mut eased, &mut eased_scene, 15);
    let linear_pos = linear_scene.solids[0].position;
    let eased_pos = eased_scene.solids[0].position;
    assert!(
        (linear_pos - start).length() > 0.05,
        "linear did not leave the start"
    );
    assert!(
        (eased_pos - start).length() > 0.05,
        "ease-in did not leave the start"
    );
    assert!(
        (linear_pos - end).length() > 0.2,
        "linear already finished the segment"
    );
    assert!(
        (eased_pos - end).length() > 0.2,
        "ease-in already finished the segment"
    );
    let gap = (linear_pos - eased_pos).length();
    assert!(gap > 0.01, "easings differ by {gap}");

    frames(&mut linear, &mut linear_scene, 45);
    frames(&mut eased, &mut eased_scene, 45);
    assert!((linear_scene.solids[0].position - end).length() < 0.05);
    assert!((eased_scene.solids[0].position - end).length() < 0.05);
    same_volume(
        &linear_scene.solids[0],
        solid_body(&linear, &linear_scene, 0),
    );
    same_volume(&eased_scene.solids[0], solid_body(&eased, &eased_scene, 0));
}

#[test]
fn a_wall_blocks_the_segment_and_a_clear_path_reaches_the_end() {
    let points = path();
    let end = points[1];
    let start = points[0];
    let mut blocked_scene = room(true);
    let mut clear_scene = room(false);
    blocked_scene.solids[0].position = start;
    clear_scene.solids[0].position = start;
    let parked = blocked_scene.solids[1].position;
    let mut blocked = rig(&blocked_scene);
    let mut clear = rig(&clear_scene);
    bind(&mut blocked, &blocked_scene, Easing::Linear, points.clone());
    bind(&mut clear, &clear_scene, Easing::Linear, points);
    frames(&mut blocked, &mut blocked_scene, 60);
    frames(&mut clear, &mut clear_scene, 60);

    let stopped = blocked_scene.solids[0].position;
    assert!(
        stopped.x > start.x + 0.2,
        "blocked solid did not move toward the wall, x {}",
        stopped.x
    );
    let wall_face = 1.2 - 0.15;
    let solid_face = stopped.x + blocked_scene.solids[0].size * 0.5;
    assert!(
        solid_face < wall_face + 0.02,
        "solid entered the wall, face {solid_face} wall {wall_face}"
    );
    assert!(
        (stopped - end).length() > 0.5,
        "blocked solid reached the end"
    );
    same_volume(
        &blocked_scene.solids[0],
        solid_body(&blocked, &blocked_scene, 0),
    );
    assert_eq!(blocked_scene.solids[1].position, parked);

    let arrived = (clear_scene.solids[0].position - end).length();
    assert!(arrived < 0.05, "clear path error {arrived}");
    same_volume(&clear_scene.solids[0], solid_body(&clear, &clear_scene, 0));
    assert_eq!(clear_scene.solids[1].position, parked);
}
