#![cfg_attr(test, expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_lossless, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_possible_wrap, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_sign_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::too_many_lines, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::uninlined_format_args, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]

//! A held camera must publish a 25 cm world cell whose stored light changed.
//!
//! The grade is the toward view of the shipped hall: eye (0.2, 1.7, 3.2), yaw 0,
//! pitch -0.45, 1280×720, 5×5 window centred on pixel (506, 420). `cells_only`
//! reads those cells. Compose reuse stays on. After the picture has held, two
//! draws with `compose_reuse` off and `freeze_cache` on read the same stored
//! cells and must match that picture. Dither is a function of the pixel, so the
//! same radiance is the same code.

use genos_render::{LightingSunStats, Renderer, World};
use genos_scene::{
    Camera, LightingPreset, LightingSettings, LIGHTING_TOGGLE_CELLS_ONLY, LIGHTING_TOGGLE_FREEZE_CACHE,
};
use genos_window::Window;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const GRADE_X: i32 = 506;
const GRADE_Y: i32 = 420;

fn open() -> (std::sync::MutexGuard<'static, ()>, genos_window::Lifecycle<Renderer>) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height).expect("renderer");
    assert_eq!(renderer.width(), WIDTH, "grade is defined at 1280");
    assert_eq!(renderer.height(), HEIGHT, "grade is defined at 720");
    (guard, genos_window::Lifecycle::new(window, renderer, permit))
}

fn shipped_hall() -> World {
    let scene = genos_scene::load_path(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    )))
    .expect("shipped scene");
    World::from_scene(scene)
}

fn toward() -> Camera {
    let mut camera = Camera::new(0.2, 3.2, 0.0);
    camera.pitch = -0.45;
    camera
}

fn cells_only(sh_cells: bool) -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.surface_cache.sh_cells = sh_cells;
    settings.experiments.toggles = LIGHTING_TOGGLE_CELLS_ONLY;
    settings.compose_reuse = true;
    assert_eq!(
        settings.experiments.sc_flags & 16384,
        0,
        "cell-centre flag 16384 stays off"
    );
    settings
}

fn red_at(pixels: &[u8], x: i32, y: i32) -> f32 {
    let i = ((y as u32 * WIDTH + x as u32) * 4) as usize;
    pixels[i + 2] as f32
}

/// Mean red of the 5×5 and the centre pixel. Red is byte 2 of the BGRA readback.
fn grade(pixels: &[u8]) -> (f32, f32) {
    let mut sum = 0.0_f32;
    let mut count = 0.0_f32;
    for dy in -2..=2 {
        for dx in -2..=2 {
            sum += red_at(pixels, GRADE_X + dx, GRADE_Y + dy);
            count += 1.0;
        }
    }
    (sum / count, red_at(pixels, GRADE_X, GRADE_Y))
}

fn centre_bytes(pixels: &[u8]) -> [u8; 4] {
    let i = ((GRADE_Y as u32 * WIDTH + GRADE_X as u32) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    renderer.draw(world, camera, true).expect("draw").expect("readback")
}

struct Held {
    frame: u32,
    mean: f32,
    centre: f32,
    compose: LightingSunStats,
    trace: String,
}

/// Draw until the grade has been still, then keep drawing so a live cell can move
/// while a stale compose would keep the old codes.
fn hold_picture(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    settings: LightingSettings,
) -> (Held, Vec<u8>) {
    renderer.set_lighting_settings(settings);
    let mut trace = String::new();
    let mut previous = -1.0_f32;
    let mut stable = 0_u32;
    let mut held_from = 0_u32;
    let mut last_pixels = Vec::new();
    let mut last_stats = LightingSunStats {
        gpu_sun: false,
        gpu_catch: false,
        slice: 0,
        host_frames: 0,
        catch_left: 0,
        full_effort: false,
        compose: 0,
        dispatches: 0,
        blocks: 0,
    };
    let mut peak_blocks = 0_u32;
    for frame in 1..=48 {
        last_pixels = draw(window, renderer, world, camera);
        let (mean, centre) = grade(&last_pixels);
        last_stats = renderer.lighting_sun_stats();
        peak_blocks = peak_blocks.max(last_stats.blocks);
        trace.push_str(&format!(
            "f{frame} mean {mean:.2} centre {centre:.0} compose {} dispatches {} blocks {}\n",
            last_stats.compose, last_stats.dispatches, last_stats.blocks
        ));
        if (mean - previous).abs() < 0.51 && frame > 1 {
            stable += 1;
            if stable == 3 && held_from == 0 {
                held_from = frame;
            }
        } else {
            stable = 0;
        }
        previous = mean;
        // The picture has held, and later frames had time to relight the cells.
        if held_from > 0 && frame >= held_from + 16 {
            break;
        }
    }
    let (mean, centre) = grade(&last_pixels);
    (
        Held {
            frame: trace.lines().count() as u32,
            mean,
            centre,
            compose: last_stats,
            trace: format!("peak_blocks {peak_blocks}\n{trace}"),
        },
        last_pixels,
    )
}

fn publish_case(sh_cells: bool) {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = shipped_hall();
    let camera = toward();
    let mut settings = cells_only(sh_cells);
    let (held, held_pixels) = hold_picture(window, renderer, &world, &camera, settings.clone());
    assert_eq!(
        held.compose.compose, 2,
        "the held frame must be the dirty list, not a full compose\n{}",
        held.trace
    );
    assert!(
        held.compose.dispatches == 1,
        "compose ran once\n{}",
        held.trace
    );

    // Same stored cells. Freeze stops the next relight from writing. Reuse off
    // composes every pixel from those cells. Two draws must agree with each
    // other: the cell state did not move between them.
    settings.compose_reuse = false;
    settings.experiments.toggles = LIGHTING_TOGGLE_CELLS_ONLY | LIGHTING_TOGGLE_FREEZE_CACHE;
    renderer.set_lighting_settings(settings.clone());
    let fresh_a = draw(window, renderer, &world, &camera);
    let fresh_b = draw(window, renderer, &world, &camera);
    let (mean_a, centre_a) = grade(&fresh_a);
    let (mean_b, centre_b) = grade(&fresh_b);
    let bytes = centre_bytes(&held_pixels);
    assert!(
        (mean_a - mean_b).abs() <= 1.0 && (centre_a - centre_b).abs() <= 1.0,
        "freeze did not hold the cell state: {mean_a:.2}/{centre_a:.0} then {mean_b:.2}/{centre_b:.0}\n{}",
        held.trace
    );
    assert!(
        (held.mean - mean_b).abs() <= 1.0 && (held.centre - centre_b).abs() <= 1.0,
        "reused compose is stale after the world cells relit\n\
         sh_cells {sh_cells} held frame {} mean {:.2} centre {:.0} raw {bytes:?}\n\
         fresh mean {mean_b:.2} centre {centre_b:.0}\n{}",
        held.frame, held.mean, held.centre, held.trace
    );

    // Nothing in the cells changed. Reuse must stay on the list and keep the picture.
    settings.compose_reuse = true;
    renderer.set_lighting_settings(settings);
    let again = draw(window, renderer, &world, &camera);
    let (mean_again, centre_again) = grade(&again);
    let quiet = renderer.lighting_sun_stats();
    assert_eq!(
        quiet.compose, 2,
        "a frozen cell field must stay on the dirty list, not a full compose"
    );
    assert!(
        (mean_again - mean_b).abs() <= 1.0 && (centre_again - centre_b).abs() <= 1.0,
        "reuse changed a frozen picture: {mean_again:.2}/{centre_again:.0} vs {mean_b:.2}/{centre_b:.0}"
    );
    let full_blocks = (WIDTH.div_ceil(8)) * (HEIGHT.div_ceil(8));
    assert!(
        quiet.blocks < 256,
        "frozen reuse listed {quiet_blocks} blocks (picture is {full_blocks})",
        quiet_blocks = quiet.blocks
    );
    eprintln!(
        "compose-cell sh_cells {sh_cells} held frame {} mean {:.2} centre {:.0} blocks {} fresh mean {mean_b:.2} centre {centre_b:.0} quiet mean {mean_again:.2} centre {centre_again:.0} blocks {} compose {}\n{}",
        held.frame, held.mean, held.centre, held.compose.blocks, quiet.blocks, quiet.compose, held.trace
    );
}

#[test]
fn reused_cells_only_picture_matches_a_fresh_compose_of_the_same_cells() {
    publish_case(true);
}

#[test]
fn reused_cells_only_without_sh_matches_a_fresh_compose_of_the_same_cells() {
    publish_case(false);
}
