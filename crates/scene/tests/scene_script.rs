use genos_scene::{load_path, load_str, Shape};

fn shipped() -> genos_scene::Scene {
    load_path(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    )))
    .unwrap()
}

#[test]
fn shipped_script_places_the_mvp_scene() {
    let scene = shipped();
    assert!(scene.floor.color.iter().all(|c| (*c - 1.0).abs() < 1.0e-4));
    assert!(scene.walls_share_a_corner(), "walls must meet");
    let red = scene.solid_by_color([1.0, 0.0, 0.0]).unwrap();
    let blue = scene.solid_by_color([0.0, 0.0, 1.0]).unwrap();
    let green = scene.solid_by_color([0.0, 1.0, 0.0]).unwrap();
    assert!(matches!(red.shape, Shape::Square | Shape::Circle));
    assert!(matches!(blue.shape, Shape::Square | Shape::Circle));
    assert!(matches!(green.shape, Shape::Square | Shape::Circle));
    assert!(!scene.lights.is_empty());
    let light = &scene.lights[0];
    assert!((light.position.y - 7.0).abs() < 1.0e-4, "light height is Y");
    assert!((light.position.z - 0.0).abs() < 1.0e-4, "light depth is Z");
    assert!(
        light.color.iter().all(|c| (*c - 1.0).abs() < 1.0e-4),
        "the lamp is white"
    );
    let long = scene
        .walls
        .iter()
        .find(|wall| wall.half_x > 3.0 && (wall.position.z - 5.0).abs() < 0.2)
        .expect("long wall");
    assert!(
        scene
            .walls
            .iter()
            .any(|wall| wall.position.z > long.position.z + 1.0),
        "the corridor is on the +Z side of the long wall"
    );
    let north = scene
        .walls
        .iter()
        .map(|wall| wall.position.z + wall.half_z)
        .fold(long.position.z, f32::max);
    assert!(
        scene.floor.position.z + scene.floor.half_z >= north - 0.05,
        "the floor stops short of the corridor"
    );
}

#[test]
fn a_second_script_moves_one_solid() {
    let original = shipped();
    let red = original.solid_by_color([1.0, 0.0, 0.0]).unwrap();
    let moved_x = red.position.x + 4.0;
    let source = format!(
        "floor(0.0, 0.0, 16.0, 16.0);\n\
         wall(2.0, 5.0, 8.0, 0.4, 2.6);\n\
         wall(6.0, 1.5, 0.4, 7.0, 2.6);\n\
         solid(\"square\", {moved_x}, 0.0, 1.5, 1.0, 0.0, 0.0);\n\
         solid(\"circle\", -3.0, 2.0, 1.4, 0.0, 0.0, 1.0);\n\
         solid(\"square\", 2.5, -2.0, 1.4, 0.0, 1.0, 0.0);\n\
         light(0.0, 4.0, 2.0, 1.0, 1.0, 1.0);\n"
    );
    let next = load_str(&source).unwrap();
    let next_red = next.solid_by_color([1.0, 0.0, 0.0]).unwrap();
    assert!((next_red.position.x - moved_x).abs() < 1.0e-4);
    assert!((next_red.position.x - red.position.x).abs() > 1.0);
}

fn base_script(extra_walls: usize, lights: usize) -> String {
    let mut source = String::from(
        "floor(0.0, 0.0, 16.0, 16.0);\n\
         wall(2.0, 5.0, 8.0, 0.4, 2.6);\n\
         wall(6.0, 1.5, 0.4, 7.0, 2.6);\n\
         solid(\"square\", 0.0, 0.0, 1.5, 1.0, 0.0, 0.0);\n\
         solid(\"circle\", -3.0, 2.0, 1.4, 0.0, 0.0, 1.0);\n\
         solid(\"square\", 2.5, -2.0, 1.4, 0.0, 1.0, 0.0);\n",
    );
    for i in 0..extra_walls {
        source += &format!("wall({}.0, -6.0, 0.4, 0.4, 2.0);\n", i as i32 - 6);
    }
    for i in 0..lights {
        source += &format!("light({}.0, 4.0, 2.0, 1.0, 1.0, 1.0);\n", i);
    }
    source
}

#[test]
fn a_script_past_the_light_limits_is_rejected() {
    let room = genos_scene::MAX_OCCLUDERS - 5;
    load_str(&base_script(room, genos_scene::MAX_LAMPS)).expect("16 occluders and 32 lights fit");
    let err = load_str(&base_script(room + 1, 1)).unwrap_err();
    assert!(err.contains("17 walls and solids") && err.contains("at most 16"), "{err}");
    let err = load_str(&base_script(0, genos_scene::MAX_LAMPS + 1)).unwrap_err();
    assert!(err.contains("33 lights") && err.contains("at most 32"), "{err}");
}
