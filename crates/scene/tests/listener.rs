//! The camera pose is the audio listener. The mix hash moves when the camera does.

use genos_audio::{mix, Listener, Source, Vec3};
use genos_load::{hex, sha256, Pcm};
use genos_scene::{
    listener_state, look_direction, update, Actions, Camera, Floor, LightingSettings, Scene,
};

fn room() -> Scene {
    Scene {
        floor: Floor {
            position: Vec3::ZERO,
            half_x: 20.0,
            half_z: 20.0,
            color: [1.0, 1.0, 1.0],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: None,
        lighting: LightingSettings::default(),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    }
}

fn tick(camera: &mut Camera, actions: &Actions, dt: f32) {
    let mut scene = room();
    update(camera, &mut scene, actions, dt);
}

fn pose(camera: &Camera) -> Listener {
    let (position, facing, velocity) = listener_state(camera);
    Listener {
        position,
        facing,
        velocity,
    }
}

fn tone() -> Pcm {
    Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..128)
            .map(|index| i16::try_from(index * 200).unwrap_or(0))
            .collect(),
    }
}

fn digest(listener: &Listener, pcm: &Pcm) -> String {
    let source = Source {
        pcm,
        position: Vec3::new(-1.5, 1.2, -4.0),
        velocity: Vec3::new(0.0, 0.0, 4.0),
        looping: true,
        positional: true,
        transmission: None,
    };
    let mixed = mix(std::slice::from_ref(&source), listener, &[], 48_000, 64);
    let mut bytes = Vec::with_capacity(mixed.len() * 2);
    for sample in mixed {
        bytes.extend(sample.to_le_bytes());
    }
    hex(&sha256(&bytes))
}

#[test]
fn a_camera_step_changes_the_offline_hash() {
    let mut camera = Camera::new(0.0, 8.0, 0.0);
    let before = pose(&camera);
    assert_eq!(before.position, camera.position);
    assert_eq!(before.facing, look_direction(camera.yaw, camera.pitch));
    assert_eq!(before.velocity, camera.physics.bodies[camera.view].velocity);

    tick(
        &mut camera,
        &Actions {
            forward: 1.0,
            ..Actions::default()
        },
        1.0 / 60.0,
    );
    let after = pose(&camera);
    assert_ne!(after.position, before.position);
    assert_eq!(after.velocity, camera.physics.bodies[camera.view].velocity);

    let pcm = tone();
    assert_ne!(digest(&before, &pcm), digest(&after, &pcm));
}

#[test]
fn a_scripted_path_has_its_own_hash() {
    let mut camera = Camera::new(-2.0, 4.0, 0.3);
    let step = 1.0 / 60.0;
    tick(
        &mut camera,
        &Actions {
            forward: 1.0,
            ..Actions::default()
        },
        step,
    );
    tick(
        &mut camera,
        &Actions {
            forward: 1.0,
            strafe: 0.25,
            ..Actions::default()
        },
        step,
    );
    tick(
        &mut camera,
        &Actions {
            look_x: 0.4,
            ..Actions::default()
        },
        step,
    );
    let pcm = tone();
    let digest = digest(&pose(&camera), &pcm);
    assert_eq!(digest, include_str!("listener.sha256").trim());
}
