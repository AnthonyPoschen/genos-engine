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
