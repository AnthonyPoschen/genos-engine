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
//! and the storage buffer the shader samples holds the resident mip.

use genos_render::{
    identity_pose, Bounds, DebugView, DrawKind, MaterialAtlas, Object, Renderer, Surface, ViewMode,
    World,
};
use genos_scene::{Camera, Floor, Scene, Vec3};
use genos_window::Window;

fn open() -> (std::sync::MutexGuard<'static, ()>, Window, Renderer) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
