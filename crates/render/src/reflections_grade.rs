#![expect(
    clippy::excessive_precision,
    reason = "these digits are the f64 trace's copied plane literals"
)]

//! Presented-frame scene: one mirror, one portal with rotated bases, and water
//! seen from the incident side and from the transmitted side.
//!
//! The planes are tilted so a pixel is at normal incidence and another is past
//! the critical angle. That is the incident-medium side and the transmitted-medium
//! side, not a pool under world up.

use genos_math::optics::ReflectionPreset;
use genos_scene::{
    Camera, DebugSettings, EventLoopSettings, Floor, FpsCounterSettings, LightingPreset,
    LightingSettings, MenuSettings, ReflectionSettings, Scene, Sky, Vec3,
};

use crate::world::{
    identity_pose, Bounds, DrawKind, Object, Reflector, ReflectorKind, ShadePlane, World,
};

pub const GRADE_WIDTH: u32 = 160;
pub const GRADE_HEIGHT: u32 = 120;

const RED: [f32; 3] = [0.84, 0.32, 0.28];
const BLUE: [f32; 3] = [0.22, 0.48, 0.90];
const LEAK: [f32; 3] = [0.95, 0.25, 0.85];
const REFL: [f32; 3] = [0.96, 0.55, 0.30];
const TRANS: [f32; 3] = [0.25, 0.72, 0.40];
const PINK: [f32; 3] = [0.90, 0.42, 0.62];
const ALBEDO: [f32; 3] = [0.28, 0.30, 0.34];
const SKY: [f32; 3] = [0.45, 0.58, 0.72];

const AIR_O: [f32; 3] = [-0.549_450_977, 1.910_349_464, 2.554_414_672];
const AIR_N: [f32; 3] = [0.343_406_860, -0.256_468_415, 0.903_490_830];
const AIR_T: [f32; 3] = [-0.934_755_976, 0.0, 0.355_290_396];
const AIR_B: [f32; 3] = [0.091_120_765, 0.966_552_612, 0.239_735_383];
const BELOW_O: [f32; 3] = [0.596_769_694, 1.934_684_098, 2.468_688_252];
const BELOW_N: [f32; 3] = [-0.630_530_365, 0.255_696_528, -0.732_837_461];
const BELOW_T: [f32; 3] = [0.758_036_805, 0.0, -0.652_211_778];
const BELOW_B: [f32; 3] = [0.166_768_287, 0.966_757_097, 0.193_827_379];
/// Beer backstop. Shifted along the below-water basis so every water pixel hits
/// it and every mirror ray misses it.
const BEER_BACK: [f32; 3] = [2.469_792_454, 2.861_310_052, -0.184_100_106];
/// On the below-water reflection ray, closer than the portal's blue wall.
const PINK_O: [f32; 3] = [3.063_806_48, 1.929_368_31, 2.064_088_16];
const PINK_N: [f32; 3] = [-0.986_814_714, 0.002_126_320, 0.161_840_041];
const PINK_T: [f32; 3] = [0.161_840_407, 0.0, 0.986_816_945];
const PINK_B: [f32; 3] = [0.002_098_289, 0.999_997_739, -0.000_344_124];

/// Camera for [`grade_world`]. Yaw and pitch are zero, so it looks down −Z.
#[must_use]
pub fn grade_camera() -> Camera {
    let mut camera = Camera::new(0.0, 4.0, 0.0);
    camera.position = Vec3::new(0.0, 1.5, 4.0);
    camera.yaw = 0.0;
    camera.pitch = 0.0;
    camera
}

/// Deterministic grade scene. `preset` is the reflection budget. Lighting stays
/// on Potato; the reflection passes read `ReflectionSettings`, not that preset.
#[must_use]
pub fn grade_world(preset: ReflectionPreset) -> World {
    let mut world = World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(80.0, -40.0, 80.0),
            half_x: 0.05,
            half_z: 0.05,
            color: [0.2, 0.2, 0.2],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: Some(Sky { color: SKY }),
        lighting: LightingSettings::preset(LightingPreset::Potato),
        debug: DebugSettings::default(),
        fps_counter: FpsCounterSettings::default(),
        event_loop: EventLoopSettings::default(),
        reflections: ReflectionSettings::for_preset(preset),
        menu: MenuSettings::default(),
    });
    world.reflection_swatch = true;
    world.reflection_sky = SKY;
    world.reflectors = grade_reflectors();
    for plane in grade_planes() {
        world.objects.push(plane_mesh(plane));
        world.shade_planes.push(plane);
    }
    world
}

fn grade_reflectors() -> Vec<Reflector> {
    let mirror = reflector(
        ReflectorKind::Mirror,
        [0.0, 1.5, 1.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.5, 0.5, 0.55],
        [0, 60, 80, 60],
    );
    // The opening faces the camera so the scissor is the portal image. The
    // bases stay a quarter-turn about Y into a quarter-turn about X; the
    // oblique plane is the portal image of this window, and it keeps the blue wall.
    let mut portal = reflector(
        ReflectorKind::Portal,
        [1.15, 1.5, 2.4],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.5, 0.5, 0.55],
        [80, 60, 40, 60],
    );
    portal.portal_to = [10.0, 1.5, 0.0];
    portal.from_basis = [0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0];
    portal.to_basis = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0];
    let extra = reflector(
        ReflectorKind::Mirror,
        [0.0, 1.5, 1.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        ALBEDO,
        [120, 60, 40, 60],
    );
    let air = reflector(
        ReflectorKind::Water,
        AIR_O,
        AIR_N,
        AIR_T,
        AIR_B,
        TRANS,
        [0, 0, 80, 60],
    );
    let below = reflector(
        ReflectorKind::Water,
        BELOW_O,
        BELOW_N,
        BELOW_T,
        BELOW_B,
        TRANS,
        [80, 0, 80, 60],
    );
    vec![mirror, portal, extra, air, below]
}

fn grade_planes() -> [ShadePlane; 6] {
    // Leak sits on the reflected camera's side of the mirror (z = 0, mirror at
    // z = 1). The oblique near plane drops it. The red wall fills the mirror
    // scissor and stops short of the water reflection ray. The beer back is
    // sized to the water pixels only: its v range starts above every mirror ray.
    [
        shade(
            [-0.77, 0.92, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            1.05,
            0.85,
            LEAK,
        ),
        shade(
            [-2.75, -0.52, 5.0],
            [0.0, 0.0, -1.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            2.85,
            2.15,
            RED,
        ),
        shade(
            [1.0, 1.5, 8.0],
            [0.0, 0.0, -1.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            6.0,
            4.0,
            REFL,
        ),
        shade(BEER_BACK, BELOW_N, BELOW_T, BELOW_B, 4.55, 1.406, TRANS),
        shade(PINK_O, PINK_N, PINK_T, PINK_B, 1.4, 1.4, PINK),
        shade(
            [12.0, 1.5, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            6.0,
            4.0,
            BLUE,
        ),
    ]
}

fn reflector(
    kind: ReflectorKind,
    origin: [f32; 3],
    normal: [f32; 3],
    tangent: [f32; 3],
    bitangent: [f32; 3],
    albedo: [f32; 3],
    scissor: [u32; 4],
) -> Reflector {
    Reflector {
        kind,
        origin,
        normal,
        tangent,
        bitangent,
        albedo,
        scissor,
        half_u: 12.0,
        half_v: 12.0,
        ..Reflector::default()
    }
}

fn shade(
    origin: [f32; 3],
    normal: [f32; 3],
    tangent: [f32; 3],
    bitangent: [f32; 3],
    half_u: f32,
    half_v: f32,
    color: [f32; 3],
) -> ShadePlane {
    ShadePlane {
        origin,
        normal,
        tangent,
        bitangent,
        half_u,
        half_v,
        color,
    }
}

fn plane_mesh(plane: ShadePlane) -> Object {
    let corner = |u: f32, v: f32| {
        [
            plane.origin[0]
                + plane.tangent[0] * u * plane.half_u
                + plane.bitangent[0] * v * plane.half_v,
            plane.origin[1]
                + plane.tangent[1] * u * plane.half_u
                + plane.bitangent[1] * v * plane.half_v,
            plane.origin[2]
                + plane.tangent[2] * u * plane.half_u
                + plane.bitangent[2] * v * plane.half_v,
        ]
    };
    let neg_neg = corner(-1.0, -1.0);
    let pos_neg = corner(1.0, -1.0);
    let pos_pos = corner(1.0, 1.0);
    let neg_pos = corner(-1.0, 1.0);
    let normal = plane.normal;
    Object {
        hidden: false,
        affects_light: false,
        bounds: Bounds {
            center: plane.origin,
            half: [
                plane.half_u + plane.half_v,
                plane.half_u + plane.half_v,
                plane.half_u + plane.half_v,
            ],
        },
        kind: DrawKind::Mesh {
            vertices: vec![neg_neg, pos_neg, pos_pos, neg_neg, pos_pos, neg_pos],
            normals: vec![normal, normal, normal, normal, normal, normal],
            texcoords: Vec::new(),
            color: plane.color,
            pose: identity_pose(),
            emission: [0.0; 3],
            surface: None,
        },
    }
}
