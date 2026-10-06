//! The camera frame applies a lighting-panel picture press before the draw.

use genos_render::{
    identity_pose, Antialias, Bounds, DrawKind, Object, Renderer, ScreenRect, World,
};
use genos_scene::{viewport_uv, Camera, Floor, Light, Scene, Vec3};
use genos_ui::{apply_frame_action, id, lighting_frame, PictureMode, Pointer, State};
use genos_window::Window;

fn open() -> (std::sync::MutexGuard<'static, ()>, Window, Renderer) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    let mut window = Window::open_proof(1280, 720).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, window, renderer)
}

fn silhouette_world() -> World {
    let mut world = World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 8.0,
            half_z: 8.0,
            color: [0.0, 0.0, 0.0],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: vec![Light {
            position: Vec3::new(0.0, 1.5, 1.5),
            color: [1.0, 1.0, 1.0],
        }],
    });
    world.objects.push(Object {
        hidden: false,
        affects_light: false,
        bounds: Bounds {
            center: [0.0, 1.2, 0.0],
            half: [3.0, 3.0, 3.0],
        },
        kind: DrawKind::Mesh {
            vertices: vec![[-1.2, 0.3, 0.0], [1.2, 0.3, 0.0], [0.0, 2.4, 0.0]],
            color: [1.0, 1.0, 1.0],
            pose: identity_pose(),
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

fn press_mode(mode_id: u32) -> PictureMode {
    let camera = Camera::opening();
    let lamp = Some([0.0, 4.0, 0.0]);
    let mut state = State::default();
    let idle = lighting_frame(
        &mut state,
        [1280.0, 720.0],
        &camera,
        lamp,
        Pointer {
            x: 0.0,
            y: 0.0,
            down: false,
        },
    );
    let button = idle
        .shown
        .iter()
        .find(|item| item.id == mode_id)
        .expect("picture control");
    let mut state = State::default();
    let press = lighting_frame(
        &mut state,
        [1280.0, 720.0],
        &camera,
        lamp,
        Pointer {
            x: button.rect.x + button.rect.w * 0.5,
            y: button.rect.y + button.rect.h * 0.5,
            down: true,
        },
    );
    assert!(!press.look_capture, "a picture press captured look");
    let action = press.actions.first().copied().expect("picture action");
    let mut scene = Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 1.0,
            half_z: 1.0,
            color: [0.0, 0.0, 0.0],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: vec![Light {
            position: Vec3::new(0.0, 4.0, 0.0),
            color: [1.0, 1.0, 1.0],
        }],
    };
    apply_frame_action(&mut scene, action).expect("picture mode")
}

#[test]
fn the_panel_press_selects_the_next_picture() {
    let (_gpu, mut window, mut renderer) = open();
    let world = silhouette_world();
    let camera = Camera::new(0.0, 4.0, 0.0);
    assert_eq!(renderer.antialias(), Antialias::Off);
    let _warmup = draw(&mut window, &mut renderer, &world, &camera);
    let off = draw(&mut window, &mut renderer, &world, &camera);

    let selected = press_mode(id::AA_FXAA);
    assert_eq!(selected, PictureMode::Fxaa);
    renderer.set_antialias(Antialias::from_picture(selected.code()));
    let from_press = draw(&mut window, &mut renderer, &world, &camera);
    renderer.set_antialias(Antialias::Fxaa);
    let explicit = draw(&mut window, &mut renderer, &world, &camera);
    assert_eq!(from_press, explicit, "the panel press did not select FXAA");
    assert_ne!(from_press, off, "FXAA left the unfiltered picture");

    let selected = press_mode(id::AA_OFF);
    assert_eq!(selected, PictureMode::Off);
    renderer.set_antialias(Antialias::from_picture(selected.code()));
    let restored = draw(&mut window, &mut renderer, &world, &camera);
    assert_eq!(restored, off, "off did not restore the unfiltered picture");

    let width = renderer.width();
    let height = renderer.height();
    let aspect = width as f32 / height as f32;
    let uv = viewport_uv(&camera, aspect, [-0.6, 1.35, 0.0]).expect("edge on screen");
    let x = (uv[0] * width as f32).round() as i32;
    let y = (uv[1] * height as f32).round() as i32;
    let mut changed = false;
    for dy in -12..12 {
        for dx in -12..12 {
            let px = x + dx;
            let py = y + dy;
            if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                continue;
            }
            let index = ((py as u32 * width + px as u32) * 4) as usize;
            if from_press[index..index + 4] != off[index..index + 4] {
                changed = true;
            }
        }
    }
    assert!(changed, "FXAA did not change the silhouette");

    let (overlay, panel) = panel_overlay(width as f32, height as f32);
    renderer.set_antialias(Antialias::Off);
    let off_ui = draw_overlay(&mut window, &mut renderer, &world, &camera, &overlay);
    renderer.set_antialias(Antialias::from_picture(PictureMode::Fxaa.code()));
    let fxaa_ui = draw_overlay(&mut window, &mut renderer, &world, &camera, &overlay);
    renderer.set_antialias(Antialias::from_picture(PictureMode::Ssaa.code()));
    let ssaa_ui = draw_overlay(&mut window, &mut renderer, &world, &camera, &overlay);
    let off_panel = rect_bytes(&off_ui, width, panel);
    assert_eq!(
        rect_bytes(&fxaa_ui, width, panel),
        off_panel,
        "FXAA changed the panel"
    );
    assert_eq!(
        rect_bytes(&ssaa_ui, width, panel),
        off_panel,
        "SSAA changed the panel"
    );
    assert!(
        off_panel.iter().any(|byte| *byte != 0),
        "the panel is missing"
    );
    println!("aa camera path press matched FXAA and off restored the picture");
}

fn panel_overlay(width: f32, height: f32) -> (Vec<ScreenRect>, genos_ui::Rect) {
    let mut state = State::default();
    let frame = lighting_frame(
        &mut state,
        [width, height],
        &Camera::opening(),
        Some([0.0, 4.0, 0.0]),
        Pointer {
            x: 0.0,
            y: 0.0,
            down: false,
        },
    );
    let panel = frame
        .shown
        .iter()
        .find(|item| item.id == id::PANEL)
        .expect("panel")
        .rect;
    let overlay = frame
        .paints
        .iter()
        .map(|paint| ScreenRect {
            x: paint.x,
            y: paint.y,
            w: paint.w,
            h: paint.h,
            color: paint.color,
        })
        .collect();
    (overlay, panel)
}

fn draw_overlay(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    overlay: &[ScreenRect],
) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw_with_overlay(world, camera, overlay, true, false)
        .expect("draw")
        .expect("readback")
}

fn rect_bytes(pixels: &[u8], width: u32, rect: genos_ui::Rect) -> Vec<u8> {
    let x0 = rect.x.round().max(0.0) as u32;
    let y0 = rect.y.round().max(0.0) as u32;
    let x1 = (rect.x + rect.w).round().max(0.0) as u32;
    let y1 = (rect.y + rect.h).round().max(0.0) as u32;
    let mut out = Vec::new();
    for y in y0..y1 {
        let start = ((y * width + x0) * 4) as usize;
        let end = ((y * width + x1) * 4) as usize;
        out.extend_from_slice(&pixels[start..end]);
    }
    out
}
