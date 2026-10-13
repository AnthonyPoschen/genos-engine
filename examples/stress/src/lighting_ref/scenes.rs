//! Fixed cameras for the reference scenes. Coordinates match the stress building.

use genos_debug::RefSetup;
use genos_scene::{Camera, Scene, Vec3, CAMERA_HEIGHT};

use crate::building::{self, Building, Layout, LightMix, View};

use super::generate::TraceProfile;

pub struct Shot {
    pub scene_id: &'static str,
    pub name: &'static str,
    pub note: &'static str,
    pub eye: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub day: f32,
    pub lights: usize,
    pub dynamic_pct: u32,
    pub box_time: f32,
    pub lamp_time: f32,
}

pub fn shot(scene: &str, name: &str) -> Option<Shot> {
    let desc = genos_lighting_ref::checkpoints()
        .iter()
        .find(|cp| cp.scene == scene && cp.name == name)?;
    let mut shot = match (scene, name) {
        ("stress-still", "hall") => from_view(View::Hall, 0.25, 5, 0, 0.0, 0.0),
        ("stress-still", "room-a") => from_view(View::RoomA, 0.25, 5, 0, 0.0, 0.0),
        ("stress-moving", "hall-moving") => from_view(View::Hall, 0.25, 25, 50, 1.2, 1.2),
        ("stress-moving", "box-step-before") => from_view(View::Hall, 0.25, 5, 0, 0.0, 0.0),
        ("stress-moving", "box-step") => from_view(View::Hall, 0.25, 5, 0, 0.45, 0.0),
        ("bounce-corner", "corner") => corner(),
        ("hell-100", "night-hall") => from_view(View::Hall, 0.70, 100, 75, 0.8, 0.8),
        _ => return None,
    };
    shot.scene_id = desc.scene;
    shot.name = desc.name;
    shot.note = desc.note;
    Some(shot)
}

pub fn every_available_shot_has_a_camera() -> bool {
    genos_lighting_ref::available_checkpoints().all(|cp| shot(cp.scene, cp.name).is_some())
}

fn from_view(
    view: View,
    day: f32,
    lights: usize,
    dynamic_pct: u32,
    box_time: f32,
    lamp_time: f32,
) -> Shot {
    let (x, z, yaw, pitch) = view.pose();
    Shot {
        scene_id: "",
        name: "",
        note: "",
        eye: [x, CAMERA_HEIGHT, z],
        yaw,
        pitch,
        day,
        lights,
        dynamic_pct,
        box_time,
        lamp_time,
    }
}

fn corner() -> Shot {
    let eye = [5.2, CAMERA_HEIGHT, 2.4];
    let at = [1.0, 0.7, 1.0];
    let (yaw, pitch) = aim(eye, at);
    Shot {
        scene_id: "",
        name: "",
        note: "",
        eye,
        yaw,
        pitch,
        day: 0.25,
        lights: 5,
        dynamic_pct: 0,
        box_time: 0.0,
        lamp_time: 0.0,
    }
}

fn aim(eye: [f32; 3], at: [f32; 3]) -> (f32, f32) {
    let d = [at[0] - eye[0], at[1] - eye[1], at[2] - eye[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1.0e-8);
    ((d[0]).atan2(-d[2]), (d[1] / len).asin())
}

/// `full_effort` spends the maximum lighting knobs. The path trace ignores
/// those knobs. Live capture uses them only when the candidate asks for it.
pub fn scene_and_camera(shot: &Shot, full_effort: bool) -> (Scene, Camera) {
    let building = Building::new(1, 1, 1);
    let mut solids = building.scene.solids.clone();
    building.place_movers(&mut solids, shot.box_time);
    let mix = LightMix {
        count: shot.lights,
        dynamic_pct: shot.dynamic_pct,
        layout: Layout::Spread,
    };
    let mut lights = building.lamps(mix, shot.lamp_time, 1.0);
    lights.extend(building::sun(shot.day));
    let mut scene = building.scene;
    scene.solids = solids;
    scene.lights = lights;
    scene.sky = building::sky(shot.day);
    if full_effort {
        scene.lighting.apply_full_effort();
    }
    let mut camera = Camera::new(shot.eye[0], shot.eye[2], shot.yaw);
    camera.set_pose(
        Vec3::new(shot.eye[0], shot.eye[1], shot.eye[2]),
        shot.yaw,
        shot.pitch,
    );
    (scene, camera)
}

pub fn reference_setup(shot: &Shot, profile: &TraceProfile) -> RefSetup {
    let (scene, camera) = scene_and_camera(shot, false);
    let mut setup =
        RefSetup::from_camera(&scene, &camera, profile.width, profile.height, profile.spp);
    setup.noise_target = profile.noise_target;
    setup.max_spp = profile.max_spp.max(profile.spp);
    setup.seconds = profile.seconds;
    setup.max_bounces = profile.max_bounces;
    setup
}
