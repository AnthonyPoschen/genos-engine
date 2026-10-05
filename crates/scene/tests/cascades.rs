use genos_scene::{build, load_path, probe_counts, sample, sample_world, Shape, Solid};

#[test]
fn a_scene_with_no_lamps_is_black() {
    let mut scene = shipped();
    scene.lights.clear();
    assert_eq!(genos_scene::illuminate(&scene, 0.0, 1.0, 0.0), 0.0);
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
    let outside_x = red.x + red.size * 0.5 + 0.4;
    let outside_z = red.z;
    assert!(!red.contains_xz(outside_x, outside_z));
    let tint = sample(&first, outside_x, outside_z);
    assert!(tint[0] > tint[1], "red {tint:?} should lead green");
    assert!(tint[0] > tint[2], "red {tint:?} should lead blue");
    assert!(tint[0] > 0.02, "floor sample missing the solid color: {tint:?}");

    let mut moved = scene.clone();
    let solid = moved.solid_by_color([1.0, 0.0, 0.0]).unwrap();
    let index = moved
        .solids
        .iter()
        .position(|item| (item.x - solid.x).abs() < 1.0e-4 && item.color[0] > 0.9)
        .unwrap();
    moved.solids[index].x = 80.0;
    let second = build(&moved);
    let faded = sample(&second, outside_x, outside_z);
    assert!(
        tint[0] - faded[0] > 0.1,
        "second build still carries the first field: first {tint:?} second {faded:?}"
    );
}

#[test]
fn each_material_tints_the_floor_with_its_own_color() {
    let scene = shipped();
    let field = build(&scene);
    for (color, name) in [
        ([1.0, 0.0, 0.0], "red"),
        ([0.0, 0.0, 1.0], "blue"),
        ([0.0, 1.0, 0.0], "green"),
    ] {
        let solid = scene.solid_by_color(color).unwrap();
        let outside_x = solid.x + solid.size * 0.5 + 0.4;
        let outside_z = solid.z;
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
        x: scene.floor.x + scene.floor.half_x + span * 2.0,
        z: scene.floor.z,
        size: span * 0.2,
        height: 2.0,
        color: [1.0, 0.0, 1.0],
    });
    let field = build(&scene);
    let plain_field = build(&plain);
    assert!(field.world.directions > field.far.directions);
    assert!(field.world.spacing > field.far.spacing);
    assert!(field.world.interval_start + 1.0e-3 >= field.far.interval_end);

    let edge_x = scene.floor.x + scene.floor.half_x - field.near.spacing;
    let carried = sample_world(&field, edge_x, scene.floor.z);
    let base = sample_world(&plain_field, edge_x, scene.floor.z);
    assert!(
        carried[0] > carried[1] && carried[2] > carried[1],
        "world probe lost the off-screen magenta material: {carried:?}"
    );
    assert!(
        carried[0] > base[0] && carried[2] > base[2],
        "world probe ignored the off-screen solid: with {carried:?} without {base:?}"
    );
}
