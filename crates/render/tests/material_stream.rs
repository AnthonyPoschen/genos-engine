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
//! and both readbacks are the resident mip: the device atlas and the page
//! image the blit copied. The CPU chain does not supply either texel. A
//! staging-buffer copy into the atlas leaves those two bytes different.

use genos_render::{
    identity_pose, Bounds, DebugView, DrawKind, MaterialAtlas, MaterialStats, Object, Renderer,
    StreamedTexture, Surface, ViewMode, World,
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
        hdr: genos_scene::HdrSettings::default(),
        fog: genos_scene::FogSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
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

fn solid_atlas(size: u32, red: u8) -> MaterialAtlas {
    let texels = usize::try_from(u64::from(size) * u64::from(size)).unwrap_or(0);
    let mut rgba = vec![0u8; texels * 4];
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel[0] = red;
        pixel[3] = 255;
    }
    MaterialAtlas::from_rgba(size, size, rgba)
}

/// A card facing the camera at `z`, `span` metres on a side. `bounds` is the
/// distance the streamer reads, independent of the card that sets the footprint.
fn stream_card(span: f32, place_z: f32, bounds_z: f32) -> Object {
    let y = 1.7f32;
    let half = span * 0.5;
    Object {
        hidden: false,
        affects_light: false,
        bounds: Bounds {
            center: [0.0, y, bounds_z],
            half: [40.0, 40.0, 40.0],
        },
        kind: DrawKind::Mesh {
            vertices: vec![
                [-half, y - half, place_z],
                [half, y - half, place_z],
                [-half, y + half, place_z],
            ],
            normals: Vec::new(),
            texcoords: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            color: [1.0, 1.0, 1.0],
            pose: identity_pose(),
            emission: [0.0; 3],
            surface: None,
        },
    }
}

fn push_stream(world: &mut World, size: u32, red: u8, span: f32, place_z: f32, bounds_z: f32) {
    let object = world.objects.len();
    world.objects.push(stream_card(span, place_z, bounds_z));
    world.stream_textures.push(StreamedTexture {
        atlas: solid_atlas(size, red),
        object,
    });
}

/// Four 256² chains, each large on screen, so the heap test is not one 340-byte image.
fn large_world() -> World {
    let mut world = World::from_scene(scene());
    for slot in 0..4 {
        push_stream(&mut world, 256, 40 * (slot + 1), 1.64, 2.0, 2.0);
    }
    world
}

/// Three 256² chains, each large and near so the first frames can admit mip 0.
fn ordered_world() -> World {
    let mut world = World::from_scene(scene());
    push_stream(&mut world, 256, 255, 1.64, 2.0, 2.0);
    push_stream(&mut world, 256, 128, 1.64, 2.0, 2.0);
    push_stream(&mut world, 256, 64, 1.64, 2.0, 2.0);
    world
}

/// Move the three cards to ρ ≈ 1, 12, and 22 at distances 100, 5, and 1.
/// The triangle stays 2 m in front; the bounds carry the distance.
fn place_pressure(world: &mut World) {
    // Camera::new(0, 4, 0) sits at z = 4. Spans sit mid-band so λ floors to 0, 3, and 4.
    let cards = [
        (1.642_f32, 4.0 - 100.0),
        (0.137, 4.0 - 5.0),
        (0.075, 4.0 - 1.0),
    ];
    for (index, (span, bounds_z)) in cards.into_iter().enumerate() {
        let Some(slot) = world.stream_textures.get(index).map(|item| item.object) else {
            continue;
        };
        let Some(object) = world.objects.get_mut(slot) else {
            continue;
        };
        place_card(object, span, 2.0, bounds_z);
    }
}

fn place_card(object: &mut Object, span: f32, place_z: f32, bounds_z: f32) {
    let y = 1.7f32;
    let half = span * 0.5;
    object.bounds.center = [0.0, y, bounds_z];
    object.bounds.half = [40.0, 40.0, 40.0];
    let DrawKind::Mesh { vertices, .. } = &mut object.kind else {
        return;
    };
    *vertices = vec![
        [-half, y - half, place_z],
        [half, y - half, place_z],
        [-half, y + half, place_z],
    ];
}

fn flood_meter(renderer: &mut Renderer) {
    renderer.observe_material_disk(1_000_000_000, 1);
    renderer.observe_material_decode(1_000_000_000, 1);
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
    // [atlas, page]. Both are the resident mip only when the blit sourced the page.
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
    flood_meter(&mut renderer);
    let world = large_world();
    let camera = Camera::new(0.0, 4.0, 0.0);
    let mut committed = 0u64;
    let mut finest = None;
    for _ in 0..16 {
        let _frame = draw(&mut window, &mut renderer, &world, &camera);
        committed = renderer.material_committed();
        finest = renderer.material_residency().1;
        if finest == Some(0) && committed > 65_536 {
            break;
        }
    }
    let ready = finest == Some(0) && committed > 65_536;
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
fn an_out_of_device_memory_retry_frees_device_memory() {
    let (_gpu, mut window, mut renderer) = open();
    let world = world();
    let camera = Camera::new(0.0, 4.0, 0.0);
    let mut pages = 0u64;
    for _ in 0..8 {
        let _frame = draw(&mut window, &mut renderer, &world, &camera);
        pages = renderer.material_page_bytes();
        if renderer.material_residency().1 == Some(0) && pages > 0 {
            break;
        }
    }
    let before = renderer.device_bytes();
    renderer.fail_next_device_allocs(1);
    let retried = renderer.retry_texture_alloc();
    let after = renderer.device_bytes();
    let pages_after = renderer.material_page_bytes();
    let dropped = before.saturating_sub(after);
    let value = u32::from(retried.is_err() || after >= before);
    let status = if value == 0 { "PASS" } else { "FAIL" };
    eprintln!(
        "oom_device {value}.000000 0.000000 {status} device_bytes {before} -> {after} dropped {dropped} pages {pages} -> {pages_after}"
    );
    assert!(retried.is_ok(), "oom retry failed: {retried:?}");
    assert!(
        after < before,
        "device bytes did not drop: {before} -> {after} pages {pages} -> {pages_after}"
    );
}

#[test]
fn eviction_order_on_real_textures_follows_importance() {
    let (_gpu, mut window, mut renderer) = open();
    let mut settings = renderer.material_settings();
    settings.headroom_percent = 0;
    settings.headroom_floor_mb = 1;
    settings.dwell_frames = 0;
    renderer.set_material_settings(settings);
    flood_meter(&mut renderer);
    let mut world = ordered_world();
    let camera = Camera::new(0.0, 4.0, 0.0);
    let mut loaded = false;
    for _ in 0..16 {
        let _frame = draw(&mut window, &mut renderer, &world, &camera);
        loaded = (0..3).all(|index| renderer.material_finest(index) == Some(0));
        if loaded {
            break;
        }
    }
    let committed = renderer.material_committed();
    let non_texture = renderer.vram_budget().non_texture;
    if loaded {
        place_pressure(&mut world);
        renderer.set_heap_budget_override(Some((non_texture.saturating_add(committed), 0)));
    }
    let mut got = Vec::new();
    if loaded {
        for _ in 0..4 {
            let _frame = draw(&mut window, &mut renderer, &world, &camera);
            got = renderer.material_evicted();
            if got.len() >= 5 {
                break;
            }
        }
    }
    let expected = [(2usize, 0u32), (2, 1), (2, 2), (1, 0), (1, 1)];
    let mut misses = 0u32;
    if !loaded {
        misses = u32::try_from(expected.len()).unwrap_or(0);
    }
    for (index, wanted) in expected.iter().enumerate() {
        if got.get(index).copied() != Some(*wanted) {
            misses = misses.saturating_add(1);
        }
    }
    let value = misses as f32;
    let status = if value <= 0.0 { "PASS" } else { "FAIL" };
    eprintln!(
        "evict_order {value:.6} 0.000000 {status} textures {} committed {committed} victims {got:?}",
        renderer.material_texture_count()
    );
    assert!(
        value <= 0.0,
        "evict_order {value} victims {got:?} committed {committed}"
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
