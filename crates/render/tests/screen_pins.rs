//! World-cell probes for the two finest cascades.

use genos_render::{
    far_screen_hits, gather_pinned, on_screen, pin_texels, update_screen_pins, PinView, ScreenPin,
    ScreenPins, FINE_SPACING, HASH_BASE0, HASH_DIM0, HASH_ORIGIN0, PIN_POS0, PIN_POS1, PROBE_CAP0,
    PROBE_CAP1,
};
use genos_scene::{Camera, Floor, Light, Scene, Shape, Solid, Vec3, Wall};

fn room() -> Scene {
    Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 16.0,
            half_z: 16.0,
            color: [1.0, 1.0, 1.0],
        },
        walls: vec![Wall {
            position: Vec3::new(-4.0, 0.0, 0.0),
            half_x: 0.2,
            half_z: 8.0,
            height: 4.0,
            color: [0.8, 0.8, 0.8],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        solids: Vec::new(),
        lights: vec![Light {
            position: Vec3::new(0.0, 7.0, 0.0),
            color: [1.0, 1.0, 1.0],
            direction: Vec3::ZERO,
        }],
    }
}

fn floor_scene() -> Scene {
    let mut scene = room();
    scene.walls.clear();
    scene.lights.clear();
    scene
}

fn solid_scene(shape: Shape, x: f32, size: f32, height: f32) -> Scene {
    let mut scene = floor_scene();
    scene.solids.push(Solid {
        shape,
        position: Vec3::new(x, 0.0, 0.0),
        size,
        height,
        color: [0.8, 0.2, 0.2],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    });
    scene
}

fn view_at(x: f32, z: f32, yaw: f32, pitch: f32) -> PinView {
    view_sized(x, z, yaw, pitch, 128, 72)
}

fn view_sized(x: f32, z: f32, yaw: f32, pitch: f32, width: u32, height: u32) -> PinView {
    let mut camera = Camera::new(x, z, yaw);
    camera.pitch = pitch;
    PinView::from_camera(&camera, 16.0 / 9.0, width, height)
}

fn same(a: [f32; 3], b: [f32; 3]) -> bool {
    (a[0] - b[0]).abs() < 1.0e-4 && (a[1] - b[1]).abs() < 1.0e-4 && (a[2] - b[2]).abs() < 1.0e-4
}

fn energy(color: [f32; 3]) -> f32 {
    color[0] + color[1] + color[2]
}

fn positions_of(pins: &[ScreenPin], object: bool) -> Vec<[f32; 3]> {
    let mut found: Vec<[f32; 3]> = pins
        .iter()
        .filter(|pin| pin.object == object)
        .map(|pin| pin.position)
        .collect();
    found.sort_by(|a, b| {
        a[0].total_cmp(&b[0])
            .then(a[1].total_cmp(&b[1]))
            .then(a[2].total_cmp(&b[2]))
    });
    found
}

fn contains_position(pins: &[ScreenPin], position: [f32; 3]) -> bool {
    pins.iter().any(|pin| same(pin.position, position))
}

fn hash_slot(eye: [f32; 3], pos: [f32; 3]) -> Option<usize> {
    let spacing = FINE_SPACING;
    let dim = HASH_DIM0 as i32;
    let half = (HASH_DIM0 / 2) as f32 * spacing;
    let origin = [
        (eye[0] / spacing).floor() * spacing - half,
        (eye[1] / spacing).floor() * spacing - half,
        (eye[2] / spacing).floor() * spacing - half,
    ];
    let cell = [
        ((pos[0] - origin[0]) / spacing).floor() as i32,
        ((pos[1] - origin[1]) / spacing).floor() as i32,
        ((pos[2] - origin[2]) / spacing).floor() as i32,
    ];
    if cell.iter().any(|value| *value < 0 || *value >= dim) {
        return None;
    }
    Some(((cell[1] * dim + cell[2]) * dim + cell[0]) as usize)
}

#[test]
fn pins_stay_put_when_the_camera_moves_a_short_way() {
    let scene = room();
    let first = view_at(0.0, 2.0, 0.0, -0.45);
    let mut pins = ScreenPins::default();
    update_screen_pins(&mut pins, &scene, &first);
    for layer in 0..2 {
        assert!(
            !pins.layers[layer].is_empty(),
            "layer {layer} cast no surface hits"
        );
        assert!(
            pins.layers[layer].len() < 5_000,
            "layer {layer} grew to a screen-grid density"
        );
    }
    let kept = pins.clone();
    let moved = view_at(0.05, 2.0, 0.0, -0.45);
    update_screen_pins(&mut pins, &scene, &moved);
    for layer in 0..2 {
        for old in &kept.layers[layer] {
            assert!(
                contains_position(&pins.layers[layer], old.position),
                "a probe moved off {:?}",
                old.position
            );
        }
    }
    let stable = positions_of(&pins.layers[0], false);
    update_screen_pins(&mut pins, &scene, &moved);
    assert_eq!(positions_of(&pins.layers[0], false), stable);

    let sky = view_at(0.0, 0.0, 0.0, 1.4);
    let mut empty = ScreenPins::default();
    update_screen_pins(&mut empty, &scene, &sky);
    assert!(
        empty.layers[0]
            .iter()
            .chain(&empty.layers[1])
            .all(|pin| !on_screen(&sky, pin.position)),
        "a miss inside the view was stored"
    );
    let sky_count = empty.layers[0].len();
    update_screen_pins(&mut empty, &scene, &sky);
    assert_eq!(empty.layers[0].len(), sky_count);
}

#[test]
fn a_turn_onto_a_wall_keeps_old_cells_and_adds_the_wall() {
    let scene = room();
    let along = view_at(0.0, 2.0, 0.0, -0.5);
    let mut pins = ScreenPins::default();
    update_screen_pins(&mut pins, &scene, &along);
    let before = pins.clone();
    let turned = view_at(0.0, 2.0, -std::f32::consts::FRAC_PI_2, -0.2);
    update_screen_pins(&mut pins, &scene, &turned);
    for layer in 0..2 {
        for old in &before.layers[layer] {
            let away = {
                let d = [
                    old.position[0] - turned.eye[0],
                    old.position[1] - turned.eye[1],
                    old.position[2] - turned.eye[2],
                ];
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
            };
            if away > 10.0 {
                continue;
            }
            assert!(
                contains_position(&pins.layers[layer], old.position),
                "layer {layer} dropped a cell still near the camera at {:?}",
                old.position
            );
        }
        assert!(
            pins.layers[layer].len()
                <= if layer == 0 {
                    PROBE_CAP0 as usize
                } else {
                    PROBE_CAP1 as usize
                }
        );
        assert!(
            pins.layers[layer].len() < 5_000,
            "layer {layer} grew to a screen-grid density"
        );
    }
    assert!(
        pins.layers[0].iter().any(|pin| {
            !pin.object
                && (pin.position[0] + 3.8).abs() < 0.08
                && pin.position[1] > 0.05
                && pin.normal[0] > 0.5
        }),
        "the wall face did not receive a cell"
    );
    let count = pins.layers[0].len();
    let positions = positions_of(&pins.layers[0], false);
    update_screen_pins(&mut pins, &scene, &turned);
    assert_eq!(pins.layers[0].len(), count);
    assert_eq!(positions_of(&pins.layers[0], false), positions);

    let far_along = far_screen_hits(&scene, &along);
    let far_turned = far_screen_hits(&scene, &turned);
    assert!(!far_along.is_empty() && !far_turned.is_empty());
    assert_ne!(
        far_along, far_turned,
        "the farthest cascade ignored the turn"
    );
    assert_eq!(far_screen_hits(&scene, &turned), far_turned);
}

#[test]
fn floor_pins_behind_a_wall_stay_on_the_floor() {
    let scene = Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 16.0,
            half_z: 16.0,
            color: [1.0, 1.0, 1.0],
        },
        walls: vec![Wall {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 0.15,
            half_z: 10.0,
            height: 4.0,
            color: [0.8, 0.8, 0.8],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        solids: Vec::new(),
        lights: Vec::new(),
    };
    let west = view_sized(-4.0, 0.0, std::f32::consts::FRAC_PI_2, -0.55, 1280, 720);
    let mut pins = ScreenPins::default();
    update_screen_pins(&mut pins, &scene, &west);
    let floor_before = positions_of(&pins.layers[0], false)
        .into_iter()
        .filter(|position| position[1] < 0.05)
        .collect::<Vec<_>>();
    assert!(
        !floor_before.is_empty(),
        "the west view stored no floor cells"
    );
    let east = view_sized(4.0, 0.0, -std::f32::consts::FRAC_PI_2, -0.2, 1280, 720);
    update_screen_pins(&mut pins, &scene, &east);
    assert!(pins.layers[0].len() as u32 <= PROBE_CAP0);
    for old in &floor_before {
        let away = {
            let d = [
                old[0] - east.eye[0],
                old[1] - east.eye[1],
                old[2] - east.eye[2],
            ];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
        };
        if away > 10.0 {
            continue;
        }
        assert!(
            contains_position(&pins.layers[0], *old),
            "a floor cell slid onto the wall from {old:?}"
        );
    }
    assert!(
        pins.layers[0].iter().any(|pin| {
            !pin.object
                && (pin.position[0] - 0.15).abs() < 0.05
                && pin.position[1] > 0.05
                && pin.normal[0] > 0.5
        }),
        "the east face has no probe of its own"
    );
    assert!(
        pins.layers[0].iter().all(|pin| {
            pin.normal[1] <= 0.5 || pin.position[1] < 0.05 || pin.position[0].abs() > 0.2
        }),
        "a floor probe was stored on the wall"
    );
}

#[test]
fn a_solid_lattice_follows_the_solid_and_keeps_the_floor_probe() {
    let view = view_at(0.0, 2.0, 0.0, -0.8);
    let mut pins = ScreenPins::default();
    update_screen_pins(&mut pins, &floor_scene(), &view);
    let planted = positions_of(&pins.layers[0], false);
    assert!(
        planted
            .iter()
            .any(|position| position[0].abs() <= 0.5 && position[2].abs() <= 0.5),
        "no floor cell was planted under the solid"
    );

    update_screen_pins(&mut pins, &solid_scene(Shape::Square, 0.0, 1.0, 1.0), &view);
    for old in &planted {
        assert!(contains_position(&pins.layers[0], *old));
    }
    assert!(
        pins.layers[0].iter().any(|pin| {
            !pin.object
                && pin.hidden
                && pin.position[0].abs() <= 0.5
                && pin.position[2].abs() <= 0.5
        }),
        "the floor cell under the solid was deleted"
    );
    let objects = positions_of(&pins.layers[0], true);
    assert!(!objects.is_empty(), "the solid has no face lattice");
    assert!(pins.layers[0]
        .iter()
        .filter(|pin| pin.object)
        .all(|pin| pin.normal[1] > -0.5));
    assert!(pins.layers[0]
        .iter()
        .filter(|pin| !pin.object)
        .all(|pin| pin.position[1] < 0.05));
    let world_count = pins.layers[0].iter().filter(|pin| !pin.object).count();

    update_screen_pins(&mut pins, &solid_scene(Shape::Square, 0.1, 1.0, 1.0), &view);
    assert_eq!(
        pins.layers[0].iter().filter(|pin| !pin.object).count(),
        world_count,
        "a sub-cell move allocated a world cell"
    );
    assert_eq!(positions_of(&pins.layers[0], false), planted);
    let shifted = positions_of(&pins.layers[0], true);
    assert_eq!(shifted.len(), objects.len());
    for (before, after) in objects.iter().zip(&shifted) {
        assert!((after[0] - before[0] - 0.1).abs() < 1.0e-4);
        assert!((after[1] - before[1]).abs() < 1.0e-4);
        assert!((after[2] - before[2]).abs() < 1.0e-4);
    }

    update_screen_pins(&mut pins, &solid_scene(Shape::Square, 4.0, 1.0, 1.0), &view);
    assert!(
        pins.layers[0].iter().any(|pin| {
            !pin.object
                && !pin.hidden
                && pin.position[0].abs() <= 0.5
                && pin.position[2].abs() <= 0.5
        }),
        "the floor cell stayed hidden after the solid left"
    );
}

#[test]
fn a_cylinder_lattice_sits_on_the_side_and_the_top() {
    let view = view_at(0.0, 2.0, 0.0, -0.6);
    let mut pins = ScreenPins::default();
    update_screen_pins(&mut pins, &solid_scene(Shape::Circle, 0.0, 1.0, 1.0), &view);
    let faces: Vec<&ScreenPin> = pins.layers[0].iter().filter(|pin| pin.object).collect();
    assert!(faces.len() > 4, "the cylinder lattice is missing");
    assert!(faces.iter().all(|pin| pin.normal[1] > -0.5));
    assert!(faces
        .iter()
        .any(|pin| (pin.position[1] - 1.0).abs() < 1.0e-4));
    assert!(faces.iter().any(|pin| {
        pin.normal[1].abs() < 0.2
            && (pin.position[0] * pin.position[0] + pin.position[2] * pin.position[2] - 0.25).abs()
                < 0.02
    }));
}

#[test]
fn the_gather_traces_the_pinned_positions() {
    let scene = room();
    let view = view_at(0.0, 0.0, 0.0, -0.8);
    let mut pins = ScreenPins::default();
    update_screen_pins(&mut pins, &scene, &view);
    let gathered = gather_pinned(&scene, &pins);
    let texels = pin_texels(&pins, &view);
    for layer in 0..2 {
        assert_eq!(gathered.positions[layer].len(), pins.layers[layer].len());
        for (got, pin) in gathered.positions[layer].iter().zip(&pins.layers[layer]) {
            assert!(same(*got, pin.position));
        }
    }
    let origin = (PIN_POS1 - PIN_POS0) as usize;
    for (index, pin) in pins.layers[0].iter().enumerate() {
        assert!(same(
            [texels[index][0], texels[index][1], texels[index][2]],
            pin.position
        ));
    }
    for (index, pin) in pins.layers[1].iter().enumerate() {
        let texel = texels[origin + index];
        assert!(same([texel[0], texel[1], texel[2]], pin.position));
    }
    let info = texels[(HASH_ORIGIN0 - PIN_POS0) as usize];
    assert!((info[3] - FINE_SPACING).abs() < 1.0e-6);
    let hashed = pins.layers[0].iter().enumerate().find(|(index, pin)| {
        !pin.hidden && hash_slot(view.eye, pin.position).is_some() && *index < 2048
    });
    let (index, pin) = hashed.expect("no probe landed in the hash window");
    let slot = hash_slot(view.eye, pin.position).unwrap();
    let texel = texels[(HASH_BASE0 - PIN_POS0) as usize + slot];
    assert!(
        (texel[0] - index as f32).abs() < 1.0e-3 || (texel[1] - index as f32).abs() < 1.0e-3,
        "the hash missed probe {index} at {:?}",
        pin.position
    );
    let bright: f32 = gathered.radiance[0].iter().copied().map(energy).sum();
    let mut dim = scene.clone();
    dim.lights[0].color = [0.05, 0.05, 0.05];
    let again = gather_pinned(&dim, &pins);
    let dark: f32 = again.radiance[0].iter().copied().map(energy).sum();
    assert!(
        bright > dark * 2.0,
        "a lamp change reused the previous sample, bright {bright} dark {dark}"
    );
    for (got, pin) in again.positions[0].iter().zip(&pins.layers[0]) {
        assert!(same(*got, pin.position));
    }
}

#[test]
fn a_full_hd_grid_updates_without_a_quadratic_scan() {
    let scene = room();
    let mut pins = ScreenPins::default();
    let started = std::time::Instant::now();
    for step in 0..20 {
        let view = view_sized(step as f32 * 0.15, 2.0, 0.0, -0.4, 1280, 720);
        update_screen_pins(&mut pins, &scene, &view);
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed.as_millis() < 80,
        "twenty lattice updates took {elapsed:?}"
    );
    assert!(pins.layers[0].len() as u32 <= PROBE_CAP0);
    assert!(pins.layers[1].len() as u32 <= PROBE_CAP1);
    assert!(
        pins.layers[0].len() < 5_000,
        "the finest layer grew to a screen-grid density"
    );
}
