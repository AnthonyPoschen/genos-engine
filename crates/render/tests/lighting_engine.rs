use genos_render::{identity_pose, Bounds, DrawKind, FixedPart, Lighting, Object, World};
use genos_scene::{load_path, Camera, Floor, Light, Scene, Shape, Solid, Vec3, Wall};

fn example() -> World {
    let scene = load_path(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    )))
    .unwrap();
    World::from_scene(scene)
}

#[test]
fn the_opening_view_uses_one_detail_and_reuses_it() {
    let world = example();
    let camera = Camera::opening();
    let aspect = 16.0 / 9.0;
    let mut lighting = Lighting::new();
    let spacing = lighting.spacing(&world);
    assert!(
        (spacing - 0.16).abs() < 1.0e-4,
        "near spacing left the frame budget: {spacing}"
    );
    let verts = lighting.vertices_for_camera(&world, &camera, aspect);
    assert!(verts.len() > 1000, "the opening view drew nothing");
    assert!(
        verts.len() < 400_000,
        "opening mesh has {} vertices",
        verts.len()
    );
    assert!(
        lighting.ray_count() < 2_000_000,
        "opening gather cast {} rays",
        lighting.ray_count()
    );

    let cell = |name: &str, coords: Vec<f32>| {
        let raw = coords.len();
        let mut coords = coords;
        coords.sort_by(|a, b| a.partial_cmp(b).unwrap());
        coords.dedup_by(|a, b| (*a - *b).abs() < 0.005);
        let mut gaps: Vec<f32> = coords
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .filter(|gap| *gap > 0.01)
            .collect();
        assert!(!gaps.is_empty(), "{name} has no cells from {raw} samples");
        gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());
        gaps[gaps.len() / 2]
    };
    let floor_z0 = world.scene.floor.position.z - world.scene.floor.half_z;
    let floor_row = |z: f32| {
        let n = ((z - floor_z0) / spacing).round().max(0.0);
        floor_z0 + n * spacing
    };
    let floor_at = |z_target: f32| {
        cell(
            "floor",
            verts
                .iter()
                .filter(|vertex| vertex.pos[1] < 0.001 && (vertex.pos[2] - z_target).abs() < 0.02)
                .map(|vertex| vertex.pos[0])
                .collect(),
        )
    };
    let wall_x = cell(
        "long wall",
        verts
            .iter()
            .filter(|vertex| {
                (vertex.pos[2] - 5.2).abs() < 0.02 && vertex.pos[1] > 0.2 && vertex.pos[1] < 2.2
            })
            .map(|vertex| vertex.pos[0])
            .collect(),
    );
    let wall_z = cell(
        "corner wall",
        verts
            .iter()
            .filter(|vertex| {
                (vertex.pos[0] - 6.2).abs() < 0.02 && vertex.pos[1] > 0.2 && vertex.pos[1] < 2.2
            })
            .map(|vertex| vertex.pos[2])
            .collect(),
    );
    let red_face = cell(
        "red face",
        verts
            .iter()
            .filter(|vertex| (vertex.pos[0] - 0.75).abs() < 0.02 && vertex.pos[2].abs() < 0.2)
            .map(|vertex| vertex.pos[1])
            .collect(),
    );
    let blue_side = cell(
        "blue side",
        verts
            .iter()
            .filter(|vertex| {
                let dx = vertex.pos[0] + 3.0;
                let dz = vertex.pos[2] - 2.0;
                let radial = (dx * dx + dz * dz).sqrt();
                radial > 0.6 && radial < 0.85 && vertex.pos[1] > 0.2 && vertex.pos[1] < 1.0
            })
            .map(|vertex| vertex.pos[1])
            .collect(),
    );
    let green_face = cell(
        "green face",
        verts
            .iter()
            .filter(|vertex| {
                (vertex.pos[0] - 3.2).abs() < 0.02 && (vertex.pos[2] + 2.0).abs() < 0.15
            })
            .map(|vertex| vertex.pos[1])
            .collect(),
    );
    for (name, gap) in [
        ("floor by red", floor_at(floor_row(0.0))),
        ("floor by wall", floor_at(floor_row(1.5))),
        ("long wall", wall_x),
        ("corner wall", wall_z),
        ("red face", red_face),
        ("green face", green_face),
        ("blue side", blue_side),
    ] {
        let slack = spacing * 0.35;
        assert!(
            (gap - spacing).abs() < slack,
            "{name} cell is {gap}, spacing is {spacing}"
        );
    }

    for (name, x, z) in [("blue", -3.0, 2.0), ("red", 0.0, 0.0), ("green", 2.5, -2.0)] {
        let solid = world
            .scene
            .solids
            .iter()
            .find(|solid| (solid.position.x - x).abs() < 0.2 && (solid.position.z - z).abs() < 0.2)
            .unwrap_or_else(|| panic!("{name} solid is missing"));
        let lamp = &world.scene.lights[0];
        let dx = lamp.position.x - solid.position.x;
        let dz = lamp.position.z - solid.position.z;
        let horizontal = (dx * dx + dz * dz).sqrt();
        let half = solid.size * 0.5 + 0.06;
        let y = solid.height * 0.5;
        let (front, back) = if horizontal < 0.25 {
            let top = lighting.light_at(
                &world,
                solid.position.x,
                solid.height + 0.05,
                solid.position.z,
                [0.0, 1.0, 0.0],
            );
            let side = lighting.light_at(
                &world,
                solid.position.x + half,
                y,
                solid.position.z,
                [1.0, 0.0, 0.0],
            );
            (top, side)
        } else {
            let nx = dx / horizontal;
            let nz = dz / horizontal;
            let facing = lighting.light_at(
                &world,
                solid.position.x + nx * half,
                y,
                solid.position.z + nz * half,
                [nx, 0.0, nz],
            );
            let away = lighting.light_at(
                &world,
                solid.position.x - nx * half,
                y,
                solid.position.z - nz * half,
                [-nx, 0.0, -nz],
            );
            (facing, away)
        };
        assert!(
            front > back + 0.05,
            "{name} shine-through: front {front} back {back}"
        );
        let under = lighting.light_at(
            &world,
            solid.position.x,
            0.0,
            solid.position.z,
            [0.0, 1.0, 0.0],
        );
        assert!(under < 0.02, "{name} is lit underneath: {under}");
    }

    let lamp_side = floor_point_toward_lamp(-3.0, 2.0, 0.7 + 0.02, &world.scene.lights[0]);
    let far_side = floor_point_toward_lamp(-3.0, 2.0, -(0.7 + 0.02), &world.scene.lights[0]);
    let lamp_side_light =
        lighting.light_at(&world, lamp_side[0], 0.0, lamp_side[1], [0.0, 1.0, 0.0]);
    assert!(
        lamp_side_light > 0.2,
        "the lamp side of the blue solid is in a shadow: {lamp_side_light} at {lamp_side:?}"
    );
    let lamp_color = floor_color_at(&verts, lamp_side[0], lamp_side[1]);
    let far_color = floor_color_at(&verts, far_side[0], far_side[1]);
    assert!(
        lamp_color > far_color + 0.15,
        "floor on the lamp side is not brighter: lamp {lamp_color} far {far_color}"
    );
    let base = verts
        .iter()
        .filter(|vertex| {
            let dx = vertex.pos[0] + 3.0;
            let dz = vertex.pos[2] - 2.0;
            let radial = (dx * dx + dz * dz).sqrt();
            radial > 0.65
                && radial < 0.85
                && vertex.pos[1] < 0.08
                && (vertex.pos[0] - lamp_side[0]).abs() < 0.15
                && (vertex.pos[2] - lamp_side[1]).abs() < 0.15
        })
        .map(|vertex| vertex.color[0] + vertex.color[1] + vertex.color[2])
        .fold(0.0_f32, f32::max);
    assert!(
        base > 0.25,
        "the bright base of the blue solid is shadowed: {base}"
    );

    for z in [6.0_f32, 6.5, 7.0] {
        let ray = boundary_x(&world, &mut lighting, z);
        let mesh = mesh_boundary_x(&verts, z, ray);
        assert!(
            (ray - mesh).abs() < 0.005,
            "shadow at z={z} is off the light ray: mesh {mesh} ray {ray}"
        );
    }

    let gathers = lighting.gather_count();
    let rays = lighting.ray_count();
    let builds = lighting.mesh_builds();
    let _ = lighting.vertices_for_camera(&world, &camera, aspect);
    assert_eq!(
        lighting.gather_count(),
        gathers,
        "unchanged scene gathered again"
    );
    assert_eq!(lighting.ray_count(), rays);
    assert_eq!(
        lighting.mesh_builds(),
        builds,
        "the same camera rebuilt the mesh"
    );
    let looked = Camera::new(-5.8, 6.2, std::f32::consts::FRAC_PI_4 + 0.1);
    let _ = lighting.vertices_for_camera(&world, &looked, 16.0 / 9.0);
    assert_eq!(
        lighting.mesh_builds(),
        builds,
        "moving the camera rebuilt the mesh"
    );

    let mut changed = world.clone();
    changed.scene.lights[0].color = [0.2, 0.2, 1.0];
    let _ = lighting.vertices_for_camera(&changed, &camera, aspect);
    assert!(
        lighting.gather_count() > gathers,
        "a light change did not gather"
    );

    let mut lighting = Lighting::new();
    let _ = lighting.vertices_for_camera(&world, &camera, aspect);
    let gathers = lighting.gather_count();
    let red_index = world
        .scene
        .solids
        .iter()
        .position(|solid| solid.color[0] > 0.9 && solid.color[1] < 0.1)
        .unwrap();
    let mut hidden = world.clone();
    let red_object = hidden
        .objects
        .iter_mut()
        .find(|object| matches!(object.kind, DrawKind::Fixed(FixedPart::Solid(index)) if index == red_index))
        .unwrap();
    red_object.hidden = true;
    let hidden_verts = lighting.vertices_for_camera(&hidden, &camera, aspect);
    assert_eq!(
        lighting.gather_count(),
        gathers,
        "hiding a solid gathered again"
    );
    assert!(
        hidden_verts.iter().all(|vertex| {
            let inside =
                vertex.pos[0].abs() < 0.7 && vertex.pos[2].abs() < 0.7 && vertex.pos[1] > 0.3;
            !inside
        }),
        "a hidden solid stayed on screen"
    );

    let mut posed = world.clone();
    let mut pose = identity_pose();
    posed.objects.push(Object {
        hidden: false,
        affects_light: false,
        bounds: Bounds {
            center: [0.0, 0.5, 0.0],
            half: [0.3, 0.5, 0.3],
        },
        kind: DrawKind::Mesh {
            vertices: vec![[-0.2, 0.2, 0.0], [0.2, 0.2, 0.0], [0.0, 0.8, 0.0]],
            color: [0.2, 0.9, 0.2],
            pose,
        },
    });
    let posed_verts = lighting.vertices_for_camera(&posed, &camera, aspect);
    assert_eq!(
        lighting.gather_count(),
        gathers,
        "a non-occluding mesh gathered"
    );
    let mesh_apex = |vertex: &genos_render::Vertex, x: f32| {
        (vertex.pos[0] - x).abs() < 0.05
            && (vertex.pos[1] - 0.8).abs() < 0.05
            && vertex.pos[2].abs() < 0.05
    };
    assert!(
        posed_verts.iter().any(|vertex| mesh_apex(vertex, 0.0)),
        "the mesh did not draw at its pose"
    );
    pose[12] = 2.5;
    if let DrawKind::Mesh { pose: slot, .. } = &mut posed.objects.last_mut().unwrap().kind {
        *slot = pose;
    }
    let moved = lighting.vertices_for_camera(&posed, &camera, aspect);
    assert_eq!(
        lighting.gather_count(),
        gathers,
        "moving a non-occluding mesh gathered"
    );
    assert!(
        moved.iter().any(|vertex| mesh_apex(vertex, 2.5)),
        "the mesh stayed at the old pose"
    );
    assert!(
        moved.iter().all(|vertex| !mesh_apex(vertex, 0.0)),
        "the old mesh pose is still drawn"
    );

    let open = floor_color_at(&verts, 4.0, -4.0);
    let mut ignored = world.clone();
    ignored.scene.solids.push(Solid {
        shape: Shape::Square,
        position: Vec3::new(3.4, 0.0, -3.4),
        size: 1.2,
        height: 2.0,
        color: [1.0, 0.0, 1.0],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    });
    let index = ignored.scene.solids.len() - 1;
    ignored.objects.push(Object {
        hidden: true,
        affects_light: false,
        bounds: Bounds {
            center: [3.4, 1.0, -3.4],
            half: [0.6, 1.0, 0.6],
        },
        kind: DrawKind::Fixed(FixedPart::Solid(index)),
    });
    let ignored_verts = lighting.vertices_for_camera(&ignored, &camera, aspect);
    assert_eq!(lighting.gather_count(), gathers, "an unused solid gathered");
    assert!(
        ignored_verts.iter().all(|vertex| {
            let body = (vertex.pos[0] - 3.4).abs() < 0.5
                && (vertex.pos[2] + 3.4).abs() < 0.5
                && vertex.pos[1] > 0.4;
            !body
        }),
        "an unused solid was drawn"
    );
    let still = floor_color_at(&ignored_verts, 4.0, -4.0);
    assert!(
        (still - open).abs() < 0.03,
        "an unused solid cast a floor shadow: before {open} after {still}"
    );
}

#[test]
fn a_visible_part_that_does_not_affect_light_follows_scene_edits() {
    let scene = Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 2.0,
            half_z: 2.0,
            color: [1.0, 1.0, 1.0],
        },
        walls: vec![Wall {
            position: Vec3::new(1.6, 0.0, 0.0),
            half_x: 0.15,
            half_z: 0.4,
            height: 1.2,
            color: [0.8, 0.8, 0.8],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        solids: vec![Solid {
            shape: Shape::Square,
            position: Vec3::new(0.0, 0.0, 0.0),
            size: 0.8,
            height: 1.0,
            color: [1.0, 0.0, 0.0],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        lights: vec![Light {
            position: Vec3::new(0.0, 3.0, -1.5),
            color: [1.0, 1.0, 1.0],
        }],
    };
    let mut world = World::from_scene(scene);
    for object in &mut world.objects {
        if matches!(
            object.kind,
            DrawKind::Fixed(FixedPart::Solid(_)) | DrawKind::Fixed(FixedPart::Wall(_))
        ) {
            object.affects_light = false;
        }
    }
    let camera = Camera::opening();
    let aspect = 16.0 / 9.0;
    let mut lighting = Lighting::new();
    let _ = lighting.vertices_for_camera(&world, &camera, aspect);
    let gathers = lighting.gather_count();

    world.scene.solids[0].color = [0.1, 0.15, 0.9];
    world.scene.solids[0].position.x = -1.1;
    let moved = lighting.vertices_for_camera(&world, &camera, aspect);
    assert_eq!(
        lighting.gather_count(),
        gathers,
        "recoloring a non-occluder gathered"
    );
    assert!(
        moved.iter().any(|vertex| {
            (vertex.pos[0] + 1.1).abs() < 0.45
                && vertex.pos[1] > 0.35
                && vertex.pos[2].abs() < 0.45
                && vertex.color[2] > vertex.color[0] + 0.05
        }),
        "the solid kept its old color or position"
    );
    assert!(
        moved.iter().all(|vertex| {
            let old_body =
                vertex.pos[0].abs() < 0.35 && vertex.pos[2].abs() < 0.35 && vertex.pos[1] > 0.35;
            !old_body
        }),
        "the solid is still drawn at the old position"
    );

    world.scene.walls[0].color = [0.05, 0.8, 0.1];
    world.scene.walls[0].position.z = 0.8;
    let wall = lighting.vertices_for_camera(&world, &camera, aspect);
    assert_eq!(
        lighting.gather_count(),
        gathers,
        "moving a non-occluding wall gathered"
    );
    assert!(
        wall.iter().any(|vertex| {
            (vertex.pos[0] - 1.6).abs() < 0.25
                && (vertex.pos[2] - 0.8).abs() < 0.5
                && vertex.pos[1] > 0.3
                && vertex.color[1] > vertex.color[0] + 0.05
        }),
        "the wall kept its old color or position"
    );
}

fn floor_point_toward_lamp(x: f32, z: f32, distance: f32, lamp: &genos_scene::Light) -> [f32; 2] {
    let dx = lamp.position.x - x;
    let dz = lamp.position.z - z;
    let len = (dx * dx + dz * dz).sqrt().max(1.0e-4);
    [x + dx / len * distance, z + dz / len * distance]
}

fn floor_color_at(verts: &[genos_render::Vertex], x: f32, z: f32) -> f32 {
    let mut best = 0.0;
    let mut found = false;
    for tri in verts.chunks(3) {
        if tri.len() < 3 || tri.iter().any(|vertex| vertex.pos[1] > 0.001) {
            continue;
        }
        let Some((b0, b1, b2)) = barycentric(x, z, tri[0].pos, tri[1].pos, tri[2].pos) else {
            continue;
        };
        let color = [
            tri[0].color[0] * b0 + tri[1].color[0] * b1 + tri[2].color[0] * b2,
            tri[0].color[1] * b0 + tri[1].color[1] * b1 + tri[2].color[1] * b2,
            tri[0].color[2] * b0 + tri[1].color[2] * b1 + tri[2].color[2] * b2,
        ];
        best = color[0] + color[1] + color[2];
        found = true;
        break;
    }
    assert!(found, "no floor triangle covers {x},{z}");
    best
}

fn barycentric(x: f32, z: f32, a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Option<(f32, f32, f32)> {
    let ax = b[0] - a[0];
    let az = b[2] - a[2];
    let bx = c[0] - a[0];
    let bz = c[2] - a[2];
    let denom = ax * bz - az * bx;
    if denom.abs() < 1.0e-8 {
        return None;
    }
    let px = x - a[0];
    let pz = z - a[2];
    let b1 = (px * bz - pz * bx) / denom;
    let b2 = (ax * pz - az * px) / denom;
    let b0 = 1.0 - b1 - b2;
    if b0 >= -1.0e-3 && b1 >= -1.0e-3 && b2 >= -1.0e-3 {
        Some((b0, b1, b2))
    } else {
        None
    }
}

fn boundary_x(world: &World, lighting: &mut Lighting, z: f32) -> f32 {
    let dark = lighting.light_at(world, 0.0, 0.0, z, [0.0, 1.0, 0.0]);
    let lit = lighting.light_at(world, -5.0, 0.0, z, [0.0, 1.0, 0.0]);
    assert!(
        lit > dark + 0.15,
        "no shadow edge at z={z}: lit {lit} dark {dark}"
    );
    let mid = (lit + dark) * 0.5;
    let mut lo = -5.0;
    let mut hi = 0.0;
    for _ in 0..24 {
        let x = (lo + hi) * 0.5;
        if lighting.light_at(world, x, 0.0, z, [0.0, 1.0, 0.0]) > mid {
            lo = x;
        } else {
            hi = x;
        }
    }
    (lo + hi) * 0.5
}

fn mesh_boundary_x(verts: &[genos_render::Vertex], z: f32, near: f32) -> f32 {
    let mut best_x = near;
    let mut best_drop = 0.0;
    let mut previous = floor_color_at(verts, near - 0.04, z);
    let mut x = near - 0.04;
    while x < near + 0.04 {
        let next = x + 0.002;
        let color = floor_color_at(verts, next, z);
        let drop = previous - color;
        if drop > best_drop {
            best_drop = drop;
            best_x = (x + next) * 0.5;
        }
        previous = color;
        x = next;
    }
    assert!(
        best_drop > 0.05,
        "no sharp floor shadow near {near} at z={z}"
    );
    best_x
}
