//! Direct-path gains from the renderer's resident occluders. The query does not present.

use genos_render::{Renderer, World};
use genos_scene::{Floor, Scene, Vec3, Wall};
use genos_window::Window;

fn wall(z0: f32, z1: f32, absorption: f32) -> Wall {
    let min_z = z0.min(z1);
    let max_z = z0.max(z1);
    Wall {
        position: Vec3::new(0.0, 0.0, (min_z + max_z) * 0.5),
        half_x: 1.0,
        half_z: (max_z - min_z) * 0.5,
        height: 3.0,
        color: [1.0, 1.0, 1.0],
        absorption,
        reflectance: -1.0,
        color_mix: -1.0,
    }
}

fn world(walls: Vec<Wall>) -> World {
    World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 8.0,
            half_z: 8.0,
            color: [1.0, 1.0, 1.0],
        },
        walls,
        solids: Vec::new(),
        lights: Vec::new(),
    })
}

fn open() -> Result<(Window, Renderer), String> {
    let mut window = Window::open_proof(64, 64).map_err(|err| err.to_string())?;
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    Ok((window, renderer))
}

#[test]
fn resident_occluders_change_direct_path_gain() {
    let (_window, mut renderer) = match open() {
        Ok(pair) => pair,
        Err(err) => {
            println!("device creation failed: {err}");
            return;
        }
    };
    let listener = [0.0, 1.5, 0.0];
    let blocked = [0.0, 1.5, -6.0];
    let clear = [6.0, 1.5, -6.0];
    let also_blocked = [0.2, 1.5, -6.0];
    let mut scene = world(vec![wall(-3.2, -3.0, 3.0)]);
    renderer.retain_scene(&scene).expect("resident scene");
    let first = renderer
        .transmission_gains(listener, &[blocked, clear, also_blocked])
        .expect("one pass");
    println!(
        "gains blocked={} clear={} second={}",
        first[0], first[1], first[2]
    );
    assert_eq!(first.len(), 3);
    assert!(
        first.iter().any(|gain| *gain > 0.5),
        "live device returned silence"
    );
    assert!(
        first[1] > first[0] * 1.3,
        "clear path {} blocked {}",
        first[1],
        first[0]
    );
    assert!(first[1] > 0.9, "clear gain {}", first[1]);
    assert!((first[0] - first[2]).abs() < 0.08);

    scene.scene.walls[0] = wall(-3.2, -2.0, 3.0);
    renderer.retain_scene(&scene).expect("thickness update");
    let next = renderer
        .transmission_gains(listener, &[blocked])
        .expect("after thickness");
    println!("gains thick={}", next[0]);
    assert!(
        next[0] < first[0] * 0.5,
        "thicker {} was not quieter than {}",
        next[0],
        first[0]
    );

    scene.scene.walls[0] = wall(-3.4, -3.0, 0.05);
    renderer.retain_scene(&scene).unwrap();
    let soft = renderer.transmission_gains(listener, &[blocked]).unwrap()[0];
    println!("gains transmit={soft}");
    assert!(soft > 0.8, "transmitting material {soft}");

    scene.scene.walls[0] = wall(-3.5, -3.0, 20.0);
    renderer.retain_scene(&scene).unwrap();
    let hard = renderer.transmission_gains(listener, &[blocked]).unwrap()[0];
    println!("gains block={hard}");
    assert!(hard < 0.05, "blocking material {hard}");

    scene = world(vec![wall(-4.3, -4.0, 2.0)]);
    renderer.retain_scene(&scene).unwrap();
    let single = renderer.transmission_gains(listener, &[blocked]).unwrap()[0];
    scene = world(vec![wall(-4.3, -4.0, 2.0), wall(-2.3, -2.0, 2.0)]);
    renderer.retain_scene(&scene).unwrap();
    let both = renderer
        .transmission_gains(listener, &[blocked, clear])
        .unwrap();
    println!("gains one={single} two={} clear={}", both[0], both[1]);
    assert!(
        both[0] < single * 0.85,
        "two barriers {} one {single}",
        both[0]
    );
    assert!(both[1] > 0.9);
}
