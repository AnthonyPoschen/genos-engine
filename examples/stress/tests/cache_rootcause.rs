#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! A wall texel stays on the face it was claimed on.
//!
//! Compact's lit-from offer used to write the cell centre along the normal.
//! That centre is up to half a 12.5 cm texel off the face, and behind it the
//! rays start inside the solid. The hall roof sits on a cell boundary, so its
//! centre is the face and it converges. A wall between boundaries does not.

use std::sync::Mutex;

use genos_render::{offer_origin, trace_ray, CacheQuery, Renderer};
use genos_scene::{look_direction, Camera, LightingPreset, LightingSettings, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FRAMES: u32 = 4;
const TEXEL: f32 = 0.125;

fn open() -> (MutexGuard, genos_window::Lifecycle<Renderer>) {
    static GPU: Mutex<()> = Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height).expect("renderer");
    (guard, genos_window::Lifecycle::new(window, renderer, permit))
}

type MutexGuard = std::sync::MutexGuard<'static, ()>;

fn f32_u32(v: u32) -> f32 {
    let mut acc = 0.0f32;
    let mut bit = 1.0f32;
    let mut n = v;
    while n != 0 {
        if n & 1 == 1 {
            acc += bit;
        }
        bit += bit;
        n >>= 1;
    }
    acc
}

fn hall() -> (Stage, Camera) {
    let mut stage = Stage::new(
        Scale::SMALL,
        1,
        LightMix {
            count: 5,
            dynamic_pct: 0,
            layout: Layout::Spread,
        },
        1.0,
        0.12,
        120.0,
    );
    stage.sun_frozen = true;
    stage.boxes_still = true;
    stage.lamps_still = true;
    stage.apply();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    (stage, camera)
}

fn ray_dir(camera: &Camera, x: u32, y: u32) -> [f32; 3] {
    let forward = look_direction(camera.yaw, camera.pitch);
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward).normalize();
    let width = f32_u32(WIDTH);
    let height = f32_u32(HEIGHT);
    let ndc_x = (f32_u32(x) + 0.5) / width * 2.0 - 1.0;
    let ndc_y = 1.0 - (f32_u32(y) + 0.5) / height * 2.0;
    let tan_half = 1.0 / 3.0_f32.sqrt();
    let aspect = width / height;
    let dir = (forward + right * (ndc_x * tan_half * aspect) + up * (ndc_y * tan_half)).normalize();
    [dir.x, dir.y, dir.z]
}

fn axis_of(normal: [f32; 3]) -> usize {
    let ax = normal[0].abs();
    let ay = normal[1].abs();
    let az = normal[2].abs();
    if ax >= ay && ax >= az {
        0
    } else if ay >= az {
        1
    } else {
        2
    }
}

struct Face {
    position: [f32; 3],
    normal: [f32; 3],
    axis: usize,
}

fn off_plane_wall(scene: &genos_scene::Scene, camera: &Camera) -> Option<Face> {
    let eye = [camera.position.x, camera.position.y, camera.position.z];
    let mut y = 8u32;
    while y < HEIGHT.saturating_sub(8) {
        let mut x = 8u32;
        while x < WIDTH.saturating_sub(8) {
            let dir = ray_dir(camera, x, y);
            if let Some(hit) = trace_ray(scene, eye, dir, 40.0) {
                let wall = hit.normal[1].abs() < 0.15 && hit.normal[0].abs().max(hit.normal[2].abs()) > 0.9;
                let white = hit.albedo[0] > 0.85 && hit.albedo[1] > 0.85 && hit.albedo[2] > 0.85;
                if wall && white {
                    let axis = axis_of(hit.normal);
                    let origin = offer_origin(hit.point, hit.normal, TEXEL);
                    if (origin[axis] - hit.point[axis]).abs() > 0.01 {
                        return Some(Face {
                            position: hit.point,
                            normal: hit.normal,
                            axis,
                        });
                    }
                }
            }
            x = x.saturating_add(16);
        }
        y = y.saturating_add(12);
    }
    None
}

/// After compact, a wall texel whose cell centre is behind the face is still
/// stored on the face. Before the fix the stored axis was the centre, 2.5 cm
/// inside the solid, and the texel's mean was driven to black.
#[test]
fn an_off_plane_wall_texel_stays_on_the_face() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    renderer.reset_lighting_cache();
    let (stage, camera) = hall();
    let face = off_plane_wall(&stage.world.scene, &camera).expect("off-plane wall");
    let query = [CacheQuery {
        position: face.position,
        normal: face.normal,
        size: TEXEL,
    }];
    for _ in 0..FRAMES {
        let _ = window.pump();
        renderer
            .draw(&stage.world, &camera, true)
            .expect("draw")
            .expect("pixels");
    }
    let cells = renderer.read_cache_cells(&query).expect("cache cell");
    let cell = cells.first().expect("one cell");
    assert!(cell.hit.slot.is_some(), "the wall texel was not claimed");
    let stored = cell.position[face.axis];
    let face_at = face.position[face.axis];
    let centre = offer_origin(face.position, face.normal, TEXEL)[face.axis];
    eprintln!(
        "face {face_at:.4} centre {centre:.4} stored {stored:.4} weight {:.2} normal {:?}",
        cell.weight, face.normal
    );
    assert!(
        (stored - face_at).abs() < 1.0e-3,
        "stored axis {stored:.4} left the face {face_at:.4} (cell centre {centre:.4})"
    );
    assert!(
        (stored - centre).abs() > 1.0e-3,
        "stored axis {stored:.4} is the cell centre, not the face {face_at:.4}"
    );
}
