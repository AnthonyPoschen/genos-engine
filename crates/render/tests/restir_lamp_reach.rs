#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]
#![cfg_attr(
    test,
    expect(
        clippy::doc_markdown,
        reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
    )
)]

//! One lamp moves. Surfaces only the still lamps reach must keep direct-light
//! history. Header word 39 used to drop that history for the whole screen.
//!
//! Grain is the mean absolute neighbour difference of the tonemapped luma, in
//! 8-bit steps, the same quantity measured on the stress scene (left wall
//! about 4.3 with lamps moving, about 0.6 with every lamp still). Brightness
//! is `ref_err_near` against the labelled exact sum, ceiling `REF_ERR` 0.05.

use genos_render::{lamp_range, tone, DebugView, Renderer, ViewMode, World};
use genos_scene::{AmbientMode, Camera, LampPickMode, LightingPreset, LightingSettings, Scene};
use genos_window::Window;

const REF_ERR: f64 = 0.05;
const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
/// Long enough for a kept mean to settle, short of the stress scene's frame 400.
const FRAMES: u32 = 40;
/// Metres. Each lamp's peak is scaled so [`lamp_range`] is this sphere.
const LAMP_RANGE: f32 = 5.0;
const LEFT_X: f32 = 2.5;
const RIGHT_X: f32 = 18.0;
const LAMP_Z: f32 = 1.05;
const WALL_Z: f32 = 0.15;
/// Step per frame. Large enough to change the position bits, small enough
/// that a 12-frame fade stays near the current integral.
const MOVE_STEP: f32 = 0.012;
/// The shade tests `pos + n * 0.02`. The wall faces +Z.
const LIFT: f32 = 0.02;
/// Frames after a lamp leaves. The leave caps the mean at two samples, so the
/// old light's weight is gone well before this. The patch is dim (two lamps),
/// and `REF_ERR` is relative, so the new samples need this long to sit under
/// 0.05. A frozen mean does not get there: at 40 frames it was still 0.26.
const FADE_FRAMES: u32 = 128;
const LEFT_X0: f32 = LEFT_X - 1.3;
const LEFT_X1: f32 = LEFT_X + 1.3;
const RIGHT_X0: f32 = RIGHT_X - 1.0;
const RIGHT_X1: f32 = RIGHT_X + 1.0;
const PATCH_Y0: f32 = 0.7;
const PATCH_Y1: f32 = 2.5;

struct Rect {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

struct Shot {
    left_grain: f64,
    right_grain: f64,
    left_err: f64,
    right_err: f64,
    left_mean: f64,
    right_mean: f64,
    brute_left: f64,
    brute_right: f64,
}

fn open() -> (
    std::sync::MutexGuard<'static, ()>,
    genos_window::Lifecycle<Renderer>,
) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)
            .expect("renderer");
    (
        guard,
        genos_window::Lifecycle::new(window, renderer, permit),
    )
}

fn direct_settings(restir: bool) -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::Potato);
    settings.direct.max_lights = 0;
    settings.direct.refresh_frames = 1;
    settings.direct.resolution_scale = 1;
    settings.sky = false;
    settings.ambient.mode = AmbientMode::Off;
    settings.shadows.enabled = true;
    // The running mean reprojects through last frame's camera, and that
    // camera is written only when the world cache is allocated
    // (restir_mean_shadow.rs). Cache off is one fresh sample every frame,
    // so a global history wipe cannot show up as extra grain.
    settings.gi.enabled = true;
    settings.bounce.count = 1;
    settings.probes.enabled = false;
    settings.world_cache.enabled = true;
    settings.denoiser.enabled = false;
    settings.denoiser.screen = false;
    // The direct mean lives in the pixel reservoir path (lamp mode 2..=4).
    // Off shades the reservoir for the product gate but never folds the
    // running average, so a history wipe cannot show up as grain.
    settings.lamp_pick.mode = LampPickMode::Reuse;
    settings.lamp_pick.radius = 0;
    settings.compose_reuse = false;
    settings.restir_di = restir;
    settings.restir_spatial_neighbors = 4;
    settings
}

fn as_f32(value: u32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "frame and pixel counts fit in f32"
    )]
    {
        value as f32
    }
}

fn color_for_range(range: f32, tint: [f32; 3]) -> [f32; 3] {
    let unit = lamp_range([1.0, 0.0, 0.0]).max(1.0e-3);
    let peak = tint.into_iter().fold(0.0_f32, f32::max).max(1.0e-6);
    let want = (range / unit) * (range / unit);
    let scale = want / peak;
    tint.map(|channel| channel * scale)
}

fn ball_reaches(pos: [f32; 3], range: f32, point: [f32; 3]) -> bool {
    if range <= 0.0 {
        return false;
    }
    let dx = pos[0] - point[0];
    let dy = pos[1] - point[1];
    let dz = pos[2] - point[2];
    dx * dx + dy * dy + dz * dz <= range * range
}

fn reaches(pos: [f32; 3], color: [f32; 3], point: [f32; 3]) -> bool {
    ball_reaches(pos, lamp_range(color), point)
}

fn lifted_points(x0: f32, x1: f32, y0: f32, y1: f32) -> [[f32; 3]; 9] {
    let z = WALL_Z + LIFT;
    let xm = (x0 + x1) * 0.5;
    let ym = (y0 + y1) * 0.5;
    [
        [x0, y0, z],
        [x0, ym, z],
        [x0, y1, z],
        [xm, y0, z],
        [xm, ym, z],
        [xm, y1, z],
        [x1, y0, z],
        [x1, ym, z],
        [x1, y1, z],
    ]
}

fn world_and_mover() -> (World, usize) {
    let mut lights = Vec::new();
    for i in 0..12u32 {
        let col = i % 4;
        let row = i / 4;
        let tint = match i % 3 {
            0 => [1.0, 0.12, 0.05],
            1 => [0.05, 1.0, 0.12],
            _ => [0.08, 0.18, 1.0],
        };
        lights.push(genos_scene::Light {
            position: genos_scene::Vec3::new(
                LEFT_X - 0.9 + as_f32(col) * 0.6,
                1.15 + as_f32(row) * 0.45,
                LAMP_Z,
            ),
            color: color_for_range(LAMP_RANGE, tint),
            direction: genos_scene::Vec3::ZERO,
        });
    }
    for i in 0..8u32 {
        let col = i % 4;
        let row = i / 4;
        let tint = match i % 3 {
            0 => [1.0, 0.4, 0.1],
            1 => [0.2, 0.9, 0.3],
            _ => [0.3, 0.35, 1.0],
        };
        lights.push(genos_scene::Light {
            position: genos_scene::Vec3::new(
                RIGHT_X - 0.6 + as_f32(col) * 0.4,
                1.2 + as_f32(row) * 0.5,
                LAMP_Z,
            ),
            color: color_for_range(LAMP_RANGE, tint),
            direction: genos_scene::Vec3::ZERO,
        });
    }
    let mover = lights.len();
    lights.push(genos_scene::Light {
        position: genos_scene::Vec3::new(RIGHT_X, 1.6, LAMP_Z),
        color: color_for_range(LAMP_RANGE, [1.0, 0.85, 0.55]),
        direction: genos_scene::Vec3::ZERO,
    });
    let world = World::from_scene(Scene {
        floor: genos_scene::Floor {
            position: genos_scene::Vec3::new(10.0, 0.0, 6.0),
            half_x: 24.0,
            half_z: 16.0,
            color: [0.25, 0.25, 0.25],
        },
        walls: vec![genos_scene::Wall {
            position: genos_scene::Vec3::new(10.0, 0.0, 0.0),
            half_x: 16.0,
            half_z: 0.15,
            height: 4.0,
            base: 0.0,
            color: [0.72, 0.72, 0.72],
            absorption: 0.0,
            reflectance: 0.8,
            color_mix: -1.0,
        }],
        solids: Vec::new(),
        lights,
        ceiling: None,
        sky: None,
        lighting: direct_settings(true),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::for_preset(LightingPreset::High),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
        particles: genos_scene::ParticleSettings::default(),
    });
    (world, mover)
}

fn camera() -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(genos_scene::Vec3::new(10.0, 1.6, 12.0), 0.0, 0.0);
    camera
}

fn parked_frame() -> u32 {
    FRAMES.saturating_sub(1)
}

fn place_mover(world: &mut World, mover: usize, frame: u32, moving: bool) {
    let at = if moving { frame } else { parked_frame() };
    if let Some(lamp) = world.scene.lights.get_mut(mover) {
        lamp.position.x = RIGHT_X + MOVE_STEP * as_f32(at);
    }
}

fn assert_ranges(world: &World, mover: usize) {
    let left_pts = lifted_points(LEFT_X0, LEFT_X1, PATCH_Y0, PATCH_Y1);
    let right_pts = lifted_points(RIGHT_X0, RIGHT_X1, PATCH_Y0, PATCH_Y1);
    let mover_lamp = world.scene.lights.get(mover).expect("mover");
    let start = [
        mover_lamp.position.x,
        mover_lamp.position.y,
        mover_lamp.position.z,
    ];
    let end = [
        RIGHT_X + MOVE_STEP * as_f32(parked_frame()),
        mover_lamp.position.y,
        mover_lamp.position.z,
    ];
    for point in left_pts {
        assert!(
            !reaches(start, mover_lamp.color, point),
            "the mover starts inside the still patch {point:?}"
        );
        assert!(
            !reaches(end, mover_lamp.color, point),
            "the mover walks into the still patch {point:?}"
        );
    }
    for point in right_pts {
        assert!(
            reaches(start, mover_lamp.color, point),
            "the mover misses its own patch {point:?}"
        );
        assert!(
            reaches(end, mover_lamp.color, point),
            "the mover leaves its own patch {point:?}"
        );
    }
    for point in left_pts {
        assert!(
            world.scene.lights.iter().enumerate().any(|(index, lamp)| {
                index != mover
                    && reaches(
                        [lamp.position.x, lamp.position.y, lamp.position.z],
                        lamp.color,
                        point,
                    )
            }),
            "no still lamp reaches the still patch {point:?}"
        );
    }
    for point in right_pts {
        assert!(
            world.scene.lights.iter().enumerate().any(|(index, lamp)| {
                index != mover
                    && reaches(
                        [lamp.position.x, lamp.position.y, lamp.position.z],
                        lamp.color,
                        point,
                    )
            }),
            "no still lamp reaches the moved patch {point:?}"
        );
    }
    for lamp in &world.scene.lights {
        let pos = [lamp.position.x, lamp.position.y, lamp.position.z];
        let hits_left = left_pts.iter().any(|point| reaches(pos, lamp.color, *point));
        let hits_right = right_pts
            .iter()
            .any(|point| reaches(pos, lamp.color, *point));
        assert!(
            !(hits_left && hits_right),
            "one lamp reaches both patches"
        );
    }
}

fn screen_u(t: f32, span: u32) -> u32 {
    let scaled = (t * as_f32(span)).floor();
    if !scaled.is_finite() || scaled <= 0.0 {
        return 0;
    }
    let last = as_f32(span.saturating_sub(1));
    #[expect(
        clippy::cast_possible_truncation,
        reason = "clamped into the pixel span"
    )]
    #[expect(clippy::cast_sign_loss, reason = "clamped to a non-negative pixel")]
    let px = scaled.min(last) as u32;
    px
}

fn patch(camera: &Camera, x0: f32, x1: f32, y0: f32, y1: f32) -> Rect {
    let aspect = as_f32(WIDTH) / as_f32(HEIGHT);
    let mut min_x = WIDTH;
    let mut max_x = 0u32;
    let mut min_y = HEIGHT;
    let mut max_y = 0u32;
    for x in [x0, x1] {
        for y in [y0, y1] {
            let uv =
                genos_scene::viewport_uv(camera, aspect, [x, y, WALL_Z]).expect("patch on screen");
            let px = screen_u(uv[0], WIDTH);
            let py = screen_u(uv[1], HEIGHT);
            min_x = min_x.min(px);
            max_x = max_x.max(px);
            min_y = min_y.min(py);
            max_y = max_y.max(py);
        }
    }
    assert!(
        max_x > min_x + 8 && max_y > min_y + 8,
        "patch is too small {min_x}..{max_x} {min_y}..{max_y}"
    );
    Rect {
        x0: min_x.saturating_add(2),
        y0: min_y.saturating_add(2),
        x1: max_x.saturating_sub(2),
        y1: max_y.saturating_sub(2),
    }
}

fn luma_image(rgb: &[f64]) -> Vec<f64> {
    let pixels = rgb.len() / 3;
    let mut out = Vec::with_capacity(pixels);
    for pixel in 0..pixels {
        let i = pixel.saturating_mul(3);
        let r = rgb.get(i).copied().unwrap_or(0.0);
        let g = rgb.get(i.saturating_add(1)).copied().unwrap_or(0.0);
        let b = rgb.get(i.saturating_add(2)).copied().unwrap_or(0.0);
        out.push(0.2126 * r + 0.7152 * g + 0.0722 * b);
    }
    out
}

fn luma8(y: f64) -> i32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "display luma is a finite f32"
    )]
    let shown = tone(y.clamp(0.0, 4.0) as f32);
    let code = (f64::from(shown).clamp(0.0, 1.0) * 255.0).round();
    #[expect(clippy::cast_possible_truncation, reason = "0..=255")]
    {
        code as i32
    }
}

fn index_of(width: u32, x: u32, y: u32) -> usize {
    usize::try_from(u64::from(y) * u64::from(width) + u64::from(x)).unwrap_or(0)
}

fn grain(luma: &[f64], width: u32, rect: &Rect) -> f64 {
    let mut sum = 0.0;
    let mut count = 0u32;
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let here = index_of(width, x, y);
            let a = luma8(luma.get(here).copied().unwrap_or(0.0));
            if x + 1 < rect.x1 {
                let right = here.saturating_add(1);
                let b = luma8(luma.get(right).copied().unwrap_or(0.0));
                sum += f64::from((a - b).abs());
                count = count.saturating_add(1);
            }
            if y + 1 < rect.y1 {
                let down = here.saturating_add(usize::try_from(width).unwrap_or(0));
                let b = luma8(luma.get(down).copied().unwrap_or(0.0));
                sum += f64::from((a - b).abs());
                count = count.saturating_add(1);
            }
        }
    }
    if count == 0 {
        0.0
    } else {
        sum / f64::from(count)
    }
}

fn patch_mean(luma: &[f64], width: u32, rect: &Rect) -> f64 {
    let mut sum = 0.0;
    let mut count = 0u32;
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let index = index_of(width, x, y);
            sum += luma.get(index).copied().unwrap_or(0.0);
            count = count.saturating_add(1);
        }
    }
    if count == 0 {
        0.0
    } else {
        sum / f64::from(count)
    }
}

fn ref_err(live: &[f64], brute: &[f64], distance: &[f64], width: u32, rect: &Rect) -> f64 {
    let mut num = 0.0;
    let mut den = 0.0;
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let index = index_of(width, x, y);
            let refer = brute.get(index).copied().unwrap_or(0.0);
            let sample = live.get(index).copied().unwrap_or(0.0);
            let depth = distance.get(index).copied().unwrap_or(0.5);
            let weight = 1.0 / depth.max(0.5);
            num += weight * (sample - refer).abs();
            den += weight * refer.abs();
        }
    }
    if den <= 1.0e-8 {
        1.0
    } else {
        num / den
    }
}

fn grab(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
) -> genos_render::SceneRead {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback");
    renderer.read_scene().expect("scene radiance")
}

fn run(moving: bool) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let (mut world, mover) = world_and_mover();
    let camera = camera();
    renderer.set_lighting_settings(direct_settings(true));
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        lamp: 0,
    });
    let mut last = genos_render::SceneRead {
        width: WIDTH,
        height: HEIGHT,
        rgb: Vec::new(),
        distance_m: Vec::new(),
    };
    for frame in 0..FRAMES {
        place_mover(&mut world, mover, frame, moving);
        last = grab(window, renderer, &world, &camera);
    }
    let live = luma_image(&last.rgb);
    let distance = last.distance_m.clone();
    place_mover(&mut world, mover, parked_frame(), false);
    renderer.set_lighting_settings(direct_settings(false));
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        lamp: 0,
    });
    let brute_read = grab(window, renderer, &world, &camera);
    let brute = luma_image(&brute_read.rgb);
    (live, brute, distance)
}

fn measure(name: &str, moving: bool, left: &Rect, right: &Rect) -> Shot {
    let (live, brute, distance) = run(moving);
    let shot = Shot {
        left_grain: grain(&live, WIDTH, left),
        right_grain: grain(&live, WIDTH, right),
        left_err: ref_err(&live, &brute, &distance, WIDTH, left),
        right_err: ref_err(&live, &brute, &distance, WIDTH, right),
        left_mean: patch_mean(&live, WIDTH, left),
        right_mean: patch_mean(&live, WIDTH, right),
        brute_left: patch_mean(&brute, WIDTH, left),
        brute_right: patch_mean(&brute, WIDTH, right),
    };
    eprintln!(
        "{name}: left_grain {:.3} right_grain {:.3} left_err {:.4} right_err {:.4} left_mean {:.4} brute_left {:.4} right_mean {:.4} brute_right {:.4}",
        shot.left_grain,
        shot.right_grain,
        shot.left_err,
        shot.right_err,
        shot.left_mean,
        shot.brute_left,
        shot.right_mean,
        shot.brute_right
    );
    shot
}

#[test]
fn a_moving_lamp_does_not_salt_pixels_only_still_lamps_reach() {
    let (world, mover) = world_and_mover();
    assert_ranges(&world, mover);
    let cam = camera();
    let left = patch(&cam, LEFT_X0, LEFT_X1, PATCH_Y0, PATCH_Y1);
    let right = patch(&cam, RIGHT_X0, RIGHT_X1, PATCH_Y0, PATCH_Y1);
    let still = measure("all_still", false, &left, &right);
    let moving = measure("one_moves", true, &left, &right);
    assert!(
        still.brute_left > 1.0e-3 && still.brute_right > 1.0e-3,
        "a patch is dark: left {} right {}",
        still.brute_left,
        still.brute_right
    );
    // Unfixed lighting-v3: still-only grain 3.74 and moved-lamp grain 4.53
    // (one sample). A kept mean on this scene is about 0.95 and 1.45.
    assert!(
        still.left_grain < 1.6 && still.right_grain < 2.2,
        "all-still grain left {:.3} right {:.3} never settled",
        still.left_grain,
        still.right_grain
    );
    assert!(
        moving.left_grain < 1.6,
        "still-lit grain {:.3} while one lamp moves (all-still {:.3}); the whole-screen reset is about 3.7",
        moving.left_grain,
        still.left_grain
    );
    assert!(
        moving.left_grain <= still.left_grain * 1.25 + 0.15,
        "still-lit grain {:.3} drifted from the all-still {:.3}",
        moving.left_grain,
        still.left_grain
    );
    assert!(
        moving.right_grain < 2.5,
        "moved-lamp grain {:.3} (all-still {:.3}); a one-sample reset is about 4.5",
        moving.right_grain,
        still.right_grain
    );
    assert!(
        moving.left_err <= REF_ERR,
        "still-patch ref_err {:.4} above {REF_ERR} (mean {:.4} vs brute {:.4})",
        moving.left_err,
        moving.left_mean,
        moving.brute_left
    );
    assert!(
        moving.right_err <= REF_ERR,
        "moved-lamp patch ref_err {:.4} above {REF_ERR} (mean {:.4} vs brute {:.4})",
        moving.right_err,
        moving.right_mean,
        moving.brute_right
    );
    eprintln!(
        "grain table 8-bit: still-only all-still {:.3} moving {:.3}; moved-lamp all-still {:.3} moving {:.3}",
        still.left_grain,
        moving.left_grain,
        still.right_grain,
        moving.right_grain
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leave {
    Jump,
    Dim,
    Replace,
    Off,
}

fn leave_world() -> (World, usize) {
    let (mut world, _) = world_and_mover();
    world.scene.lights.retain(|lamp| lamp.position.x < 10.0);
    world.scene.lights.push(genos_scene::Light {
        position: genos_scene::Vec3::new(RIGHT_X - 0.45, 1.25, LAMP_Z),
        color: color_for_range(LAMP_RANGE, [1.0, 0.45, 0.15]),
        direction: genos_scene::Vec3::ZERO,
    });
    world.scene.lights.push(genos_scene::Light {
        position: genos_scene::Vec3::new(RIGHT_X + 0.45, 2.05, LAMP_Z),
        color: color_for_range(LAMP_RANGE, [0.25, 0.85, 0.35]),
        direction: genos_scene::Vec3::ZERO,
    });
    let mover = world.scene.lights.len();
    world.scene.lights.push(genos_scene::Light {
        position: genos_scene::Vec3::new(RIGHT_X, 1.6, LAMP_Z),
        color: color_for_range(LAMP_RANGE, [1.0, 0.85, 0.55]),
        direction: genos_scene::Vec3::ZERO,
    });
    (world, mover)
}

fn apply_leave(lamp: &mut genos_scene::Light, kind: Leave) {
    match kind {
        Leave::Jump => lamp.position.x = 40.0,
        Leave::Dim => lamp.color = color_for_range(0.4, [1.0, 0.85, 0.55]),
        Leave::Replace => lamp.color = color_for_range(0.4, [0.15, 0.35, 1.0]),
        Leave::Off => lamp.color = [0.0, 0.0, 0.0],
    }
}

fn assert_leave_geometry(kind: Leave) {
    let (mut world, mover) = leave_world();
    let left = lifted_points(LEFT_X0, LEFT_X1, PATCH_Y0, PATCH_Y1);
    let right = lifted_points(RIGHT_X0, RIGHT_X1, PATCH_Y0, PATCH_Y1);
    let lamp = world.scene.lights.get(mover).expect("mover").clone();
    let home = [lamp.position.x, lamp.position.y, lamp.position.z];
    for point in left {
        assert!(
            !reaches(home, lamp.color, point),
            "{kind:?} home reaches the still patch {point:?}"
        );
    }
    for point in right {
        assert!(
            reaches(home, lamp.color, point),
            "{kind:?} home misses the patch it must leave {point:?}"
        );
        assert!(
            world.scene.lights.iter().enumerate().any(|(index, other)| {
                index != mover
                    && reaches(
                        [other.position.x, other.position.y, other.position.z],
                        other.color,
                        point,
                    )
            }),
            "the patch a lamp leaves has no other lamp {point:?}"
        );
    }
    apply_leave(world.scene.lights.get_mut(mover).expect("mover"), kind);
    let gone = world.scene.lights.get(mover).expect("mover");
    let at = [gone.position.x, gone.position.y, gone.position.z];
    for point in left.into_iter().chain(right) {
        assert!(
            !reaches(at, gone.color, point),
            "{kind:?} still reaches {point:?} after it leaves"
        );
    }
    if kind != Leave::Jump {
        for point in left {
            assert!(
                !ball_reaches(home, lamp_range(lamp.color), point),
                "the previous ball reaches a pixel the lamp never lit"
            );
        }
    }
}

fn leave_shot(kind: Leave, left: &Rect, right: &Rect) -> Shot {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let (mut world, mover) = leave_world();
    let camera = camera();
    renderer.set_lighting_settings(direct_settings(true));
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        lamp: 0,
    });
    let mut last = genos_render::SceneRead {
        width: WIDTH,
        height: HEIGHT,
        rgb: Vec::new(),
        distance_m: Vec::new(),
    };
    for _ in 0..FRAMES {
        last = grab(window, renderer, &world, &camera);
    }
    if let Some(lamp) = world.scene.lights.get_mut(mover) {
        apply_leave(lamp, kind);
    }
    for _ in 0..FADE_FRAMES {
        last = grab(window, renderer, &world, &camera);
    }
    let live = luma_image(&last.rgb);
    let distance = last.distance_m.clone();
    renderer.set_lighting_settings(direct_settings(false));
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        lamp: 0,
    });
    let brute_read = grab(window, renderer, &world, &camera);
    let brute = luma_image(&brute_read.rgb);
    let shot = Shot {
        left_grain: grain(&live, WIDTH, left),
        right_grain: grain(&live, WIDTH, right),
        left_err: ref_err(&live, &brute, &distance, WIDTH, left),
        right_err: ref_err(&live, &brute, &distance, WIDTH, right),
        left_mean: patch_mean(&live, WIDTH, left),
        right_mean: patch_mean(&live, WIDTH, right),
        brute_left: patch_mean(&brute, WIDTH, left),
        brute_right: patch_mean(&brute, WIDTH, right),
    };
    eprintln!(
        "leave {kind:?}: left_grain {:.3} right_grain {:.3} left_err {:.4} right_err {:.4} left_mean {:.4} brute_left {:.4} right_mean {:.4} brute_right {:.4}",
        shot.left_grain,
        shot.right_grain,
        shot.left_err,
        shot.right_err,
        shot.left_mean,
        shot.brute_left,
        shot.right_mean,
        shot.brute_right
    );
    shot
}

#[test]
fn a_lamp_that_leaves_a_patch_drops_its_stale_mean() {
    for kind in [Leave::Jump, Leave::Dim, Leave::Replace, Leave::Off] {
        assert_leave_geometry(kind);
    }
    let cam = camera();
    let left = patch(&cam, LEFT_X0, LEFT_X1, PATCH_Y0, PATCH_Y1);
    let right = patch(&cam, RIGHT_X0, RIGHT_X1, PATCH_Y0, PATCH_Y1);
    let mut failed = String::new();
    for kind in [Leave::Jump, Leave::Dim, Leave::Replace, Leave::Off] {
        let shot = leave_shot(kind, &left, &right);
        if shot.brute_right <= 1.0e-3 {
            failed.push_str(&format!("{kind:?} left a dark patch {}\n", shot.brute_right));
        }
        if shot.left_grain >= 1.6 {
            failed.push_str(&format!(
                "{kind:?} salted the still patch grain {:.3}\n",
                shot.left_grain
            ));
        }
        if shot.right_err > REF_ERR {
            failed.push_str(&format!(
                "{kind:?} stale light ref_err {:.4} (mean {:.4} vs brute {:.4})\n",
                shot.right_err, shot.right_mean, shot.brute_right
            ));
        }
    }
    assert!(failed.is_empty(), "{failed}");
}
