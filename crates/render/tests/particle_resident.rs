//! A resident particle image stays uploaded when only the frame selection changes.

use genos_render::{
    frame_at, pack_frame, DrawKind, FrameMemory, Object, ParticleFrame, ParticleImage, World,
};
use genos_scene::{view_proj, Camera, Floor, Scene, Vec3};

fn floor_scene() -> Scene {
    Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 8.0,
            half_z: 8.0,
            color: [1.0, 1.0, 1.0],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
    }
}

fn clip() -> ParticleImage {
    ParticleImage {
        width: 4,
        height: 1,
        pixels: vec![10, 0, 0, 255, 20, 0, 0, 255, 30, 0, 0, 255, 40, 0, 0, 255],
        frames: vec![
            ParticleFrame {
                x: 0,
                y: 0,
                width: 2,
                height: 1,
            },
            ParticleFrame {
                x: 2,
                y: 0,
                width: 2,
                height: 1,
            },
        ],
        frame_seconds: 0.5,
    }
}

fn world_at(position: [f32; 3], age: f32, image: ParticleImage) -> World {
    let mut world = World::from_scene(floor_scene());
    world.objects.push(Object {
        hidden: false,
        affects_light: false,
        bounds: genos_render::Bounds {
            center: position,
            half: [0.4, 0.4, 0.4],
        },
        kind: DrawKind::Particles {
            points: vec![position],
            ages: vec![age],
            color: [1.0, 1.0, 1.0],
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

#[test]
fn a_later_frame_reuses_resident_particle_pixels() {
    let image = clip();
    let camera = Camera::new(0.0, 4.0, 0.0);
    let view = view_proj(&camera, 16.0 / 9.0);
    let eye = [camera.position.x, camera.position.y, camera.position.z];
    let mut memory = FrameMemory::default();
    memory.begin_frame();
    pack_frame(
        &world_at([0.0, 1.2, 1.0], 0.0, image.clone()),
        &view,
        eye,
        &mut memory,
    );
    assert!(
        memory.image_rebuilt(),
        "the first admit did not store pixels"
    );
    assert!(memory.image_uploaded(), "the first admit did not upload");
    let first = memory.sampled_frames().to_vec();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0], frame_at(&image, 0.0));

    memory.begin_frame();
    pack_frame(
        &world_at([0.35, 1.6, 0.4], 0.5, image.clone()),
        &view,
        eye,
        &mut memory,
    );
    assert!(
        !memory.image_rebuilt(),
        "a new frame index rebuilt the pixel buffer"
    );
    assert!(
        !memory.image_uploaded(),
        "a new frame index uploaded image bytes"
    );
    let second = memory.sampled_frames();
    assert_eq!(second.len(), 1);
    assert_ne!(second[0], first[0]);
    assert_eq!(second[0], frame_at(&image, 0.5));
}
