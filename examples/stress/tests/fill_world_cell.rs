#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! A fill ray lands in the world cell, not the 12.5 cm screen texel.
//!
//! The tile fill used to schedule both sizes into the same pick. A young
//! screen texel outranks its 25 cm parent, so the parent stays at weight 0
//! and compose shows a direct-only mean. The visit claims the parent and
//! returns; the next round rays that cell.

use std::sync::Mutex;

use genos_render::{offer_origin, trace_ray, CacheQuery, Renderer};
use genos_scene::{look_direction, Camera, LightingPreset, LightingSettings, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FRAMES: u32 = 40;
const TEXEL: f32 = 0.125;
const WORLD: f32 = 0.25;

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

struct Face {
    position: [f32; 3],
    normal: [f32; 3],
    axis: usize,
}

fn walls(scene: &genos_scene::Scene, camera: &Camera) -> Vec<Face> {
    let eye = [camera.position.x, camera.position.y, camera.position.z];
    let mut found = Vec::new();
    let mut y = 8u32;
    while y < HEIGHT.saturating_sub(8) {
        let mut x = 8u32;
        while x < WIDTH.saturating_sub(8) {
            let dir = ray_dir(camera, x, y);
            if let Some(hit) = trace_ray(scene, eye, dir, 40.0) {
                let wall = hit.normal[1].abs() < 0.15 && hit.normal[0].abs().max(hit.normal[2].abs()) > 0.9;
                let white = hit.albedo[0] > 0.85 && hit.albedo[1] > 0.85 && hit.albedo[2] > 0.85;
                if wall && white {
                    let axis = if hit.normal[0].abs() >= hit.normal[2].abs() { 0 } else { 2 };
                    found.push(Face {
                        position: hit.point,
                        normal: hit.normal,
                        axis,
                    });
                }
            }
            x = x.saturating_add(16);
        }
        y = y.saturating_add(16);
    }
    found
}

/// After a short still, on-screen wall rays sit in the 25 cm cell. The 12.5 cm
/// texel under the same point stays empty, and the cell stays on the face.
#[test]
fn a_screen_wall_fill_ray_lands_in_the_world_cell() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    renderer.reset_lighting_cache();
    let (stage, camera) = hall();
    let mut faces = walls(&stage.world.scene, &camera);
    assert!(faces.len() >= 8, "need on-screen walls, found {}", faces.len());
    // The probe copies at most 32 cells. Two sizes per wall, so sixteen walls.
    faces.truncate(16);
    for _ in 0..FRAMES {
        let _ = window.pump();
        renderer
            .draw(&stage.world, &camera, true)
            .expect("draw")
            .expect("pixels");
    }
    let mut queries = Vec::with_capacity(faces.len() * 2);
    for face in &faces {
        queries.push(CacheQuery {
            position: face.position,
            normal: face.normal,
            size: TEXEL,
        });
        queries.push(CacheQuery {
            position: face.position,
            normal: face.normal,
            size: WORLD,
        });
    }
    let cells = renderer.read_cache_cells(&queries).expect("cache cells");
    let mut world_lit = 0usize;
    let mut fine_lit = 0usize;
    let mut off_face = 0usize;
    for (i, face) in faces.iter().enumerate() {
        let fine = cells[i * 2];
        let world = cells[i * 2 + 1];
        if fine.weight >= 0.5 {
            fine_lit += 1;
        }
        let centre = offer_origin(face.position, face.normal, WORLD)[face.axis];
        let stored = world.position[face.axis];
        let face_at = face.position[face.axis];
        let on_face = world.hit.slot.is_some() && (stored - face_at).abs() < 1.0e-3;
        if world.hit.slot.is_some() && (stored - face_at).abs() >= 1.0e-3 {
            off_face += 1;
            eprintln!(
                "off face stored {stored:.4} face {face_at:.4} centre {centre:.4} weight {:.2}",
                world.weight
            );
        }
        if world.weight >= 1.0 && on_face {
            world_lit += 1;
        }
    }
    let n = faces.len();
    eprintln!("walls {n} fine_lit {fine_lit} world_lit {world_lit} off_face {off_face}");
    assert_eq!(
        fine_lit, 0,
        "{fine_lit} of {n} screen texels took fill rays; the world cell is the store"
    );
    assert!(
        world_lit * 4 >= n * 3,
        "only {world_lit} of {n} world cells hold a face sample"
    );
    assert_eq!(off_face, 0, "{off_face} world cells left the claimer's face");
}
