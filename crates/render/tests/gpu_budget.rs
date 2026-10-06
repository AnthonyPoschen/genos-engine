//! Shape blocks and particle images share one byte budget.

use genos_render::{
    pack_frame, DrawKind, FrameMemory, Object, PackedDraw, ParticleFrame, ParticleImage, World,
};
use genos_scene::{view_proj, Camera, Floor, Scene, Shape, Solid, Vec3};

fn floor_scene() -> Scene {
    Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 4.0,
            half_z: 4.0,
            color: [1.0, 1.0, 1.0],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
    }
}

fn view() -> ([f32; 16], [f32; 3]) {
    let camera = Camera::new(0.0, 3.0, 0.0);
    let matrix = view_proj(&camera, 16.0 / 9.0);
    let eye = [camera.position.x, camera.position.y, camera.position.z];
    (matrix, eye)
}

fn particle_world() -> (World, u64) {
    let width = 32u32;
    let height = 32u32;
    let image = ParticleImage {
        width,
        height,
        pixels: vec![255u8; (width * height * 4) as usize],
        frames: vec![ParticleFrame {
            x: 0,
            y: 0,
            width,
            height,
        }],
        frame_seconds: 1.0,
    };
    let pixels = image.pixels.len() as u64;
    let mut world = World::from_scene(floor_scene());
    world.objects.push(Object {
        hidden: false,
        affects_light: false,
        bounds: genos_render::Bounds {
            center: [0.0, 1.2, 1.0],
            half: [0.4, 0.4, 0.4],
        },
        kind: DrawKind::Particles {
            points: vec![[0.0, 1.2, 1.0]],
            ages: vec![0.0],
            color: [1.0, 0.4, 0.1],
            size: 0.4,
            emission: [0.0, 0.0, 0.0],
            density: 0.0,
            lit: false,
            face_camera: true,
            angle: 0.0,
            image: Some(image),
        },
    });
    (world, pixels)
}

#[test]
fn a_tight_budget_drops_the_unused_shape_and_keeps_the_new_one() {
    let mut memory = FrameMemory::with_limit(1_799);
    memory.begin_frame();
    memory.admit_shapes(&[(1, 1_000)]);
    assert!(memory.contains_shape(1));
    assert_eq!(memory.accounted(), 1_000);
    memory.begin_frame();
    assert!(
        memory.contains_shape(1),
        "the unused block left before the next admit"
    );
    memory.admit_shapes(&[(2, 800)]);
    assert!(
        !memory.contains_shape(1),
        "the low-priority block stayed over the budget"
    );
    assert!(memory.contains_shape(2));
    assert!(memory.accounted() <= memory.limit());
}

#[test]
fn required_geometry_stays_when_the_particle_image_does_not_fit() {
    let (matrix, eye) = view();
    let floor = World::from_scene(floor_scene());
    let mut shapes = FrameMemory::default();
    shapes.begin_frame();
    pack_frame(&floor, &matrix, eye, &mut shapes);
    let geometry = shapes.accounted();
    assert!(geometry > 0, "the floor was not admitted");

    let (world, pixels) = particle_world();
    let mut wide = FrameMemory::default();
    wide.begin_frame();
    pack_frame(&world, &matrix, eye, &mut wide);
    let image = wide.image_resident_bytes();
    assert!(
        image >= pixels,
        "the particle image was not admitted: {image} vs {pixels}"
    );

    let mut tight = FrameMemory::with_limit(geometry + image - 1);
    tight.begin_frame();
    let pack = pack_frame(&world, &matrix, eye, &mut tight);
    let kept_geometry = pack
        .draws
        .iter()
        .any(|draw| matches!(draw, PackedDraw::Shape { .. }));
    assert!(kept_geometry, "in-view geometry was dropped");
    assert!(tight.accounted() <= tight.limit());
    assert!(
        !tight.image_resident() || tight.image_resident_bytes() < image,
        "the particle image kept its full residency"
    );
}

fn solid(x: f32, z: f32, size: f32) -> Solid {
    Solid {
        shape: Shape::Square,
        position: Vec3::new(x, 0.0, z),
        size,
        height: 1.0,
        color: [0.8, 0.2, 0.2],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    }
}

fn shape_keys(pack: &genos_render::Pack) -> (Vec<u64>, usize) {
    let mut keys = Vec::new();
    let mut draws = 0usize;
    for draw in &pack.draws {
        if let PackedDraw::Shape { key, .. } = draw {
            draws += 1;
            let key = *key;
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    (keys, draws)
}

#[test]
fn identical_shapes_share_one_block_under_the_unique_budget() {
    let (matrix, eye) = view();
    let mut scene = floor_scene();
    scene.floor.half_x = 8.0;
    scene.floor.half_z = 8.0;
    scene.solids = vec![
        solid(-1.2, 0.2, 1.0),
        solid(1.2, 0.2, 1.0),
        solid(0.0, -1.4, 2.0),
    ];
    let world = World::from_scene(scene);
    let mut wide = FrameMemory::default();
    wide.begin_frame();
    let pack = pack_frame(&world, &matrix, eye, &mut wide);
    let (keys, draws) = shape_keys(&pack);
    assert!(
        draws > keys.len(),
        "two solids did not share a shape key: {draws} draws, {} keys",
        keys.len()
    );
    assert!(keys.len() >= 3, "the other shape was not packed");
    let mut unique = 0u64;
    for key in &keys {
        unique += wide
            .shape_bytes(*key)
            .expect("a packed shape has no resident bytes");
    }
    assert_eq!(
        wide.accounted(),
        unique,
        "duplicate instances were billed twice"
    );

    let mut tight = FrameMemory::with_limit(unique);
    tight.begin_frame();
    let packed = pack_frame(&world, &matrix, eye, &mut tight);
    let (kept, _) = shape_keys(&packed);
    for key in &keys {
        assert!(
            tight.contains_shape(*key),
            "a required shape was dropped under the unique-block budget"
        );
        assert!(kept.contains(key), "the draw omitted a shape that fits");
    }
    assert_eq!(tight.accounted(), unique);
    assert!(tight.accounted() <= tight.limit());
}

fn clip_world(age: f32) -> World {
    let width = 64u32;
    let height = 32u32;
    let image = ParticleImage {
        width,
        height,
        pixels: vec![180u8; (width * height * 4) as usize],
        frames: vec![
            ParticleFrame {
                x: 0,
                y: 0,
                width: 32,
                height: 32,
            },
            ParticleFrame {
                x: 32,
                y: 0,
                width: 32,
                height: 32,
            },
        ],
        frame_seconds: 0.5,
    };
    let mut world = World::from_scene(floor_scene());
    world.objects.push(Object {
        hidden: false,
        affects_light: false,
        bounds: genos_render::Bounds {
            center: [0.0, 1.2, 1.0],
            half: [0.4, 0.4, 0.4],
        },
        kind: DrawKind::Particles {
            points: vec![[0.0, 1.2, 1.0]],
            ages: vec![age],
            color: [1.0, 0.4, 0.1],
            size: 0.4,
            emission: [0.0, 0.0, 0.0],
            density: 0.0,
            lit: false,
            face_camera: true,
            angle: 0.0,
            image: Some(image),
        },
    });
    world
}

fn full_frame_world() -> World {
    let width = 32u32;
    let height = 32u32;
    let image = ParticleImage {
        width,
        height,
        pixels: vec![90u8; (width * height * 4) as usize],
        frames: vec![ParticleFrame {
            x: 0,
            y: 0,
            width,
            height,
        }],
        frame_seconds: 1.0,
    };
    let mut world = World::from_scene(floor_scene());
    world.objects.push(Object {
        hidden: false,
        affects_light: false,
        bounds: genos_render::Bounds {
            center: [0.0, 1.2, 1.0],
            half: [0.4, 0.4, 0.4],
        },
        kind: DrawKind::Particles {
            points: vec![[0.0, 1.2, 1.0]],
            ages: vec![0.0],
            color: [1.0, 0.5, 0.1],
            size: 0.4,
            emission: [0.0, 0.0, 0.0],
            density: 0.0,
            lit: false,
            face_camera: true,
            angle: 0.0,
            image: Some(image),
        },
    });
    world
}

fn textured_uvs(pack: &genos_render::Pack) -> Vec<[f32; 2]> {
    let mut uvs = Vec::new();
    for draw in &pack.draws {
        if let PackedDraw::Dynamic(verts) = draw {
            for vert in verts {
                if vert.uv[0] >= 0.0 {
                    uvs.push(vert.uv);
                }
            }
        }
    }
    uvs
}

fn assert_uvs_inside(uvs: &[[f32; 2]]) {
    assert!(
        !uvs.is_empty(),
        "the scaled image produced no textured vertices"
    );
    for uv in uvs {
        assert!(
            (0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]),
            "a scaled frame landed outside the resident image: {uv:?}"
        );
    }
}

#[test]
fn a_scaled_atlas_keeps_frame_uvs_inside_the_resident_image() {
    let (matrix, eye) = view();
    let floor = World::from_scene(floor_scene());
    let mut shapes = FrameMemory::default();
    shapes.begin_frame();
    pack_frame(&floor, &matrix, eye, &mut shapes);
    let geometry = shapes.accounted();

    let full = full_frame_world();
    let mut wide = FrameMemory::default();
    wide.begin_frame();
    pack_frame(&full, &matrix, eye, &mut wide);
    let full_bytes = wide.image_resident_bytes();
    assert!(full_bytes >= 32 * 32 * 4);

    let mut halved = FrameMemory::with_limit(geometry + full_bytes / 4);
    halved.begin_frame();
    let pack = pack_frame(&full, &matrix, eye, &mut halved);
    assert_eq!(halved.image_size(), (16, 16), "the atlas was not halved");
    let uvs = textured_uvs(&pack);
    assert_uvs_inside(&uvs);
    let max_x = uvs.iter().map(|uv| uv[0]).fold(0.0f32, f32::max);
    assert!(
        max_x > 0.9,
        "the full frame did not cover the scaled image: {max_x}"
    );

    let later = clip_world(0.5);
    let mut clip_wide = FrameMemory::default();
    clip_wide.begin_frame();
    pack_frame(&later, &matrix, eye, &mut clip_wide);
    let clip_bytes = clip_wide.image_resident_bytes();
    assert!(clip_bytes >= 64 * 32 * 4);
    let mut clip_half = FrameMemory::with_limit(geometry + clip_bytes / 4);
    clip_half.begin_frame();
    let pack = pack_frame(&later, &matrix, eye, &mut clip_half);
    assert_eq!(clip_half.image_size(), (32, 16));
    let uvs = textured_uvs(&pack);
    assert_uvs_inside(&uvs);
    let min_x = uvs.iter().map(|uv| uv[0]).fold(1.0f32, f32::min);
    let max_x = uvs.iter().map(|uv| uv[0]).fold(0.0f32, f32::max);
    assert!(
        min_x > 0.45 && max_x > 0.9,
        "the later frame did not stay in the right half: {min_x}..{max_x}"
    );
}
