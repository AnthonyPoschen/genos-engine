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
/// Flat patch facing the grade camera. Its centre ray is the normal, so the
/// patch is at normal incidence and the shaded pixel is the water F0.
const F0_O: [f32; 3] = [-1.023_523, 1.599_509_2, 2.522_671_8];
const F0_N: [f32; 3] = [0.568_623_9, -0.055_282_88, 0.820_737_9];
const F0_T: [f32; 3] = [0.821_994_94, 0.0, -0.569_494_8];
const F0_B: [f32; 3] = [0.031_483_31, 0.998_470_7, 0.045_442_25];
const F0_WHITE_O: [f32; 3] = [-0.966_147_65, 1.593_955, 2.604_326_5];
const F0_WHITE_N: [f32; 3] = [-0.568_908_6, 0.055_297_96, -0.820_539_5];
const F0_WHITE_T: [f32; 3] = [-0.821_797, 0.0, 0.569_780_4];
const F0_WHITE_B: [f32; 3] = [0.031_507_7, 0.998_469_9, 0.045_443_7];
const F0_BLACK_O: [f32; 3] = [-1.045_739_5, 1.601_692_2, 2.489_488_1];
const F0_BLACK_N: [f32; 3] = [0.568_410_3, -0.055_271_56, 0.820_886_6];
const F0_BLACK_T: [f32; 3] = [0.822_143_3, 0.0, -0.569_280_5];
const F0_BLACK_B: [f32; 3] = [0.031_465_02, 0.998_471_4, 0.045_441_15];
/// Water tilted 55° off the centre ray of x 52..60, y 48..56. The incident
/// ray and the refracted ray separate by about 17°.
const SNELL_O: [f32; 3] = [-0.269_148_23, 1.589_716_1, 2.834_019_7];
const SNELL_N: [f32; 3] = [0.926_810_74, -0.042_882_52, 0.373_072_33];
const SNELL_T: [f32; 3] = [0.373_415_83, 0.0, -0.927_664_1];
const SNELL_B: [f32; 3] = [0.039_780_576, 0.999_080_12, 0.016_013_011];
/// On the incident ray, 0.5 m past that surface. The refracted ray misses it
/// and hits the green backstop.
const SNELL_OCC_O: [f32; 3] = [-0.380_753_7, 1.627_173_8, 2.347_205_4];
const SNELL_OCC_N: [f32; 3] = [0.224_290_19, -0.074_763_395, 0.971_650_3];
const SNELL_OCC_T: [f32; 3] = [0.974_377_33, 0.0, -0.224_919_68];
const SNELL_OCC_B: [f32; 3] = [0.016_815_759, 0.997_201_3, 0.072_847_754];
const SNELL_OCC_C: [f32; 3] = [0.92, 0.12, 0.08];

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
        [0, 0, 80, 46],
    );
    // The same plane continues beside the Snell column. The flat F0 quad takes
    // the corner, so this scissor starts at x = 16 and stops at x = 52.
    let air_tail = reflector(
        ReflectorKind::Water,
        AIR_O,
        AIR_N,
        AIR_T,
        AIR_B,
        TRANS,
        [16, 46, 36, 14],
    );
    let air_right = reflector(
        ReflectorKind::Water,
        AIR_O,
        AIR_N,
        AIR_T,
        AIR_B,
        TRANS,
        [60, 46, 20, 14],
    );
    // Tilted so the incident ray hits the red block and the refracted ray hits
    // the green backstop. The column is x 52..60, y 46..60. The gate reads the
    // inner 8×8, where those two rays disagree.
    let snell = reflector(
        ReflectorKind::Water,
        SNELL_O,
        SNELL_N,
        SNELL_T,
        SNELL_B,
        TRANS,
        [52, 46, 8, 14],
    );
    // Albedo is black. The reflection target is white, so a pixel on this quad
    // is the Fresnel weight at normal incidence.
    let f0 = reflector(
        ReflectorKind::Water,
        F0_O,
        F0_N,
        F0_T,
        F0_B,
        [0.0, 0.0, 0.0],
        [0, 46, 16, 14],
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
    vec![
        mirror, portal, extra, air, air_tail, air_right, snell, f0, below,
    ]
}

fn grade_planes() -> [ShadePlane; 9] {
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
        // White sits on the F0 reflection ray. Black sits on the transmitted ray.
        // Both are small enough that the mirror, the portal, and the other water miss them.
        shade(
            F0_WHITE_O,
            F0_WHITE_N,
            F0_WHITE_T,
            F0_WHITE_B,
            0.077_033_48,
            0.079_672_84,
            [1.0, 1.0, 1.0],
        ),
        shade(
            F0_BLACK_O,
            F0_BLACK_N,
            F0_BLACK_T,
            F0_BLACK_B,
            0.074_491_31,
            0.077_033_76,
            [0.0, 0.0, 0.0],
        ),
        shade(
            SNELL_OCC_O,
            SNELL_OCC_N,
            SNELL_OCC_T,
            SNELL_OCC_B,
            0.061_653_294,
            0.064_625_71,
            SNELL_OCC_C,
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
