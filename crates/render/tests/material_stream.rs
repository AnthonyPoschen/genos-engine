#![cfg_attr(
    test,
    expect(
        clippy::expect_used,
        reason = "a test aborts when the window or the draw fails"
    )
)]
#![cfg_attr(
    test,
    expect(clippy::as_conversions, reason = "a miss count is a handful of faults")
)]
#![cfg_attr(
    test,
    expect(
        clippy::cast_precision_loss,
        reason = "a miss count is a handful of faults"
    )
)]

//! The frame's streamer, read back from a headless draw.
//!
//! ```text
//! cargo test -p genos-render --test material_stream -- --nocapture
//! ```
//!
//! `frame_streamer` is 0 when the renderer's own streamer has the colour
//! atlas, mip 0 is resident, the first albedo readback is that coarse page,
//! and the device-atlas texel the transfer copied out is the resident mip.
//! The CPU chain does not supply that texel.

use genos_render::{
    identity_pose, Bounds, DebugView, DrawKind, MaterialAtlas, MaterialStats, Object, Renderer,
    Surface, ViewMode, World,
};
use genos_scene::{Camera, Floor, Scene, Vec3};
use genos_window::Window;

fn open() -> (std::sync::MutexGuard<'static, ()>, Window, Renderer) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(640, 360).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, window, renderer)
}

fn scene() -> Scene {
    Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 8.0,
            half_z: 8.0,
            color: [0.0, 0.0, 0.0],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: None,
        lighting: genos_scene::LightingSettings::default(),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    }
}

fn checker_atlas() -> MaterialAtlas {
    let width = 8u32;
    let height = 8u32;
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let index = ((y * width + x) * 4) as usize;
            if x < width / 2 {
                rgba[index] = 255;
            } else {
                rgba[index + 2] = 255;
            }
            rgba[index + 3] = 255;
        }
    }
    MaterialAtlas::from_rgba(width, height, rgba)
}

fn world() -> World {
    let mut world = World::from_scene(scene());
    let atlas = checker_atlas();
    let width = atlas.width;
    let height = atlas.height;
    world.color_atlas = atlas;
    world.surfaces.push(Surface {
        color: [0, 0, width, height],
        normal: [0, 0, 0, 0],
        cutoff: 0.0,
        two_sided: false,
    });
    let vertices = vec![[-1.2, 0.3, 0.0], [1.2, 0.3, 0.0], [0.0, 2.4, 0.0]];
    let texcoords = vec![[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]];
    world.objects.push(Object {
        hidden: false,
        affects_light: false,
        bounds: Bounds {
            center: [0.0, 1.2, 0.0],
            half: [3.0, 3.0, 3.0],
        },
        kind: DrawKind::Mesh {
            vertices,
            normals: Vec::new(),
            texcoords,
            color: [1.0, 1.0, 1.0],
            pose: identity_pose(),
            emission: [0.0; 3],
            surface: Some(0),
        },
    });
    world
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback")
}

#[test]
fn material_stats_line_names_the_counters() {
    let stats = MaterialStats {
        committed: 340,
        budget: 170,
        refused: 1,
        evictions: 2,
        thrash: 3,
        holes: 4,
        rate: 5,
        stream_ms: 1.5,
    };
    assert_eq!(
        stats.line(),
        "materials committed=340 budget=170 refused=1 evictions=2 thrash=3 holes=4 rate=5 stream_ms=1.500"
    );
}

fn channel_max(pixels: &[u8], offset: usize) -> u8 {
    pixels
        .chunks(4)
        .filter_map(|pixel| pixel.get(offset).copied())
        .max()
        .unwrap_or(0)
}

#[test]
fn the_frame_streamer_is_read_back() {
    let (_gpu, mut window, mut renderer) = open();
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Albedo,
        lamp: 0,
    });
    let world = world();
    let camera = Camera::new(0.0, 4.0, 0.0);
    let first = draw(&mut window, &mut renderer, &world, &camera);
    let (count, _) = renderer.material_residency();
    let coarse = channel_max(&first, 2);
    let mut finest = None;
    let mut texel = [0u8; 2];
    for _ in 0..4 {
        let _frame = draw(&mut window, &mut renderer, &world, &camera);
        finest = renderer.material_residency().1;
        texel = renderer.material_texel();
    }
    let mut misses = 0u32;
    if count == 0 {
        misses += 1;
    }
    if finest != Some(0) {
        misses += 1;
    }
    // Linear of the 1×1 average (sRGB 127) is about 54. A missed draw stays 0.
    if !(40..=80).contains(&coarse) {
        misses += 1;
    }
    if texel != [255, 255] {
        misses += 1;
    }
    let value = misses as f32;
    let status = if value <= 0.0 { "PASS" } else { "FAIL" };
    eprintln!(
        "frame_streamer {value:.6} 0.000000 {status} textures {count} finest {finest:?} coarse {coarse} texel {texel:?}"
    );
    assert!(
        value <= 0.0,
        "frame_streamer {value} finest {finest:?} coarse {coarse} texel {texel:?}"
    );
}

#[test]
fn a_smaller_heap_budget_evicts_on_the_renderer() {
    let (_gpu, mut window, mut renderer) = open();
    let mut settings = renderer.material_settings();
    settings.headroom_percent = 0;
    settings.headroom_floor_mb = 0;
    renderer.set_material_settings(settings);
    let world = world();
    let camera = Camera::new(0.0, 4.0, 0.0);
    let mut committed = 0u64;
    let mut finest = None;
    for _ in 0..8 {
        let _frame = draw(&mut window, &mut renderer, &world, &camera);
        committed = renderer.material_committed();
        finest = renderer.material_residency().1;
        if finest == Some(0) && committed > 0 {
            break;
        }
    }
    let ready = finest == Some(0) && committed > 0;
    let non_texture = renderer.vram_budget().non_texture;
    // Half the resident chain, so the engine is over heapBudget and dwell does not hold.
    let heap = non_texture.saturating_add(committed / 2);
    renderer.set_heap_budget_override(Some((heap, 0)));
    let mut until = 7u32;
    let mut now = committed;
    let mut target = 0u64;
    if ready {
        for frame in 1..=6 {
            let _frame = draw(&mut window, &mut renderer, &world, &camera);
            now = renderer.material_committed();
            target = renderer.vram_budget().texture_target();
            if now <= target {
                until = frame;
                break;
            }
        }
    }
    let value = if ready { until as f32 } else { 7.0 };
    let status = if value <= 6.0 { "PASS" } else { "FAIL" };
    eprintln!(
        "heap_shrink {value:.6} 6.000000 {status} committed {now} target {target} before {committed}"
    );
    assert!(
        value <= 6.0,
        "heap_shrink {value} committed {now} target {target}"
    );
}

#[test]
fn the_renderer_reports_the_live_streamer() {
    let (_gpu, mut window, mut renderer) = open();
    let world = world();
    let camera = Camera::new(0.0, 4.0, 0.0);
    let mut stats = renderer.material_stats();
    for _ in 0..8 {
        let _frame = draw(&mut window, &mut renderer, &world, &camera);
        stats = renderer.material_stats();
        if renderer.material_residency().1 == Some(0) && stats.committed > 0 {
            break;
        }
    }
    assert_eq!(stats.committed, renderer.material_committed());
    assert_eq!(stats.budget, renderer.vram_budget().texture_target());
    assert!(stats.committed > 0, "committed {}", stats.committed);
    assert!(stats.budget > stats.committed, "budget {}", stats.budget);
    let line = stats.line();
    assert!(
        line.contains(&format!("committed={}", stats.committed)),
        "{line}"
    );
    assert!(line.contains(&format!("budget={}", stats.budget)), "{line}");
    eprintln!("material_stats {line}");
}
