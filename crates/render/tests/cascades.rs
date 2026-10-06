use genos_render::{build, probe_counts, sample, sample_world};
use genos_scene::{load_path, Shape, Solid, Vec3};

#[test]
fn a_wall_stops_the_lamp_and_a_short_wall_does_not() {
    use genos_render::illuminate;
    use genos_scene::{Light, Vec3, Wall};
    let mut scene = shipped();
    scene.solids.clear();
    scene.walls.clear();
    scene.walls.push(Wall {
        position: Vec3::new(0.0, 0.0, 2.0),
        half_x: 4.0,
        half_z: 0.3,
        height: 3.0,
        color: [1.0, 1.0, 1.0],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    });
    scene.lights = vec![Light {
        position: Vec3::new(0.0, 2.0, 0.0),
        color: [1.0, 1.0, 1.0],

        direction: Vec3::ZERO,
    }];
    let lit = illuminate(&scene, 0.0, 0.2, 0.5);
    let shadow = illuminate(&scene, 0.0, 0.2, 4.0);
    let seam = illuminate(&scene, 0.0, 0.0, 2.32);
    assert!(lit > 0.2, "same side of the wall should be lit: {lit}");
    assert!(
        shadow < 0.02,
        "the wall let the lamp through: shadow {shadow} lit {lit}"
    );
    assert!(
        seam < 0.02,
        "the lamp reaches the floor under the wall: {seam}"
    );

    scene.walls[0].height = 0.4;
    scene.lights[0].position.y = 6.0;
    let over = illuminate(&scene, 0.0, 0.2, 4.0);
    assert!(
        over > shadow + 0.01,
        "a ray over a short wall should still arrive: {over} blocked {shadow}"
    );
}

#[test]
fn a_scene_with_no_lamps_is_black() {
    let mut scene = shipped();
    scene.lights.clear();
    assert_eq!(genos_render::illuminate(&scene, 0.0, 1.0, 0.0), 0.0);
    let field = build(&scene);
    let bounced = sample(&field, 1.2, 0.0);
    assert!(
        bounced.iter().all(|channel| *channel < 1.0e-4),
        "bounce without a lamp: {bounced:?}"
    );
}

fn shipped() -> genos_scene::Scene {
    load_path(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    )))
    .unwrap()
}

#[test]
fn cascades_merge_a_tint_and_do_not_reuse_the_previous_field() {
    let scene = shipped();
    let first = build(&scene);
    let (near_probes, far_probes) = probe_counts(&first);
    assert!(near_probes > far_probes);
    assert!(first.near.directions < first.far.directions);
    assert!(first.near.interval_end <= first.far.interval_start + 1.0e-4);

    let red = scene.solid_by_color([1.0, 0.0, 0.0]).unwrap();
    let outside_x = red.position.x + red.size * 0.5 + 0.4;
    let outside_z = red.position.z;
    assert!(!red.contains_xz(outside_x, outside_z));
    let tint = sample(&first, outside_x, outside_z);
    assert!(tint[0] > tint[1], "red {tint:?} should lead green");
    assert!(tint[0] > tint[2], "red {tint:?} should lead blue");
    assert!(
        tint[0] > 0.002,
        "floor sample missing the solid color: {tint:?}"
    );

    let mut moved = scene.clone();
    let solid = moved.solid_by_color([1.0, 0.0, 0.0]).unwrap();
    let index = moved
        .solids
        .iter()
        .position(|item| (item.position.x - solid.position.x).abs() < 1.0e-4 && item.color[0] > 0.9)
        .unwrap();
    moved.solids[index].position.x = 80.0;
    let second = build(&moved);
    let faded = sample(&second, outside_x, outside_z);
    assert!(
        tint[0] > faded[0] * 2.0,
        "second build still carries the first field: first {tint:?} second {faded:?}"
    );
}

#[test]
fn each_material_tints_the_floor_with_its_own_color() {
    let scene = shipped();
    for (color, name) in [
        ([1.0, 0.0, 0.0], "red"),
        ([0.0, 0.0, 1.0], "blue"),
        ([0.0, 1.0, 0.0], "green"),
    ] {
        let solid = scene.solid_by_color(color).unwrap().clone();
        let mut alone = scene.clone();
        alone.solids = vec![solid.clone()];
        let field = build(&alone);
        let outside_x = solid.position.x + solid.size * 0.5 + 0.15;
        let outside_z = solid.position.z;
        assert!(!solid.contains_xz(outside_x, outside_z));
        let tint = sample(&field, outside_x, outside_z);
        let lead = match name {
            "red" => tint[0] > tint[1] && tint[0] > tint[2],
            "blue" => tint[2] > tint[0] && tint[2] > tint[1],
            "green" => tint[1] > tint[0] && tint[1] > tint[2],
            _ => false,
        };
        assert!(lead, "{name} material did not lead the bounce: {tint:?}");
    }
}

#[test]
fn a_world_probe_carries_an_offscreen_material() {
    let plain = shipped();
    let mut scene = plain.clone();
    let span = (scene.floor.half_x * 2.0).max(scene.floor.half_z * 2.0);
    scene.solids.push(Solid {
        shape: Shape::Square,
        position: Vec3::new(
            scene.floor.position.x + scene.floor.half_x + span * 0.2,
            0.0,
            scene.floor.position.z,
        ),
        size: span * 0.2,
        height: 2.0,
        color: [1.0, 0.0, 1.0],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    });
    let field = build(&scene);
    let plain_field = build(&plain);
    assert!(field.world.directions > field.far.directions);
    assert!(field.world.spacing > field.far.spacing);
    assert!(field.world.interval_start + 1.0e-3 >= field.far.interval_end);

    let edge_x = scene.floor.position.x + scene.floor.half_x - field.near.spacing;
    let carried = sample_world(&field, edge_x, scene.floor.position.z);
    let base = sample_world(&plain_field, edge_x, scene.floor.position.z);
    assert!(
        carried[0] > carried[1] && carried[2] > carried[1],
        "world probe lost the off-screen magenta material: {carried:?}"
    );
    assert!(
        carried[0] > base[0] && carried[2] > base[2],
        "world probe ignored the off-screen solid: with {carried:?} without {base:?}"
    );
}
