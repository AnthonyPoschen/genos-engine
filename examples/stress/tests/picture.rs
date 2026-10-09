//! The small stress picture against an independent ray trace.
//!
//! The tracer never reads the probe field. Samples come from the presented picture.

use std::sync::Mutex;

use genos_render::{
    agreement, linear_from_display, luminance, radiance, sees, Renderer,
};
use genos_scene::{viewport_uv, Camera, Scene, Shape, Solid, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

fn open() -> (MutexGuard, Window, Renderer) {
    static GPU: Mutex<()> = Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    let mut window = Window::open_proof(1280, 720).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, window, renderer)
}

type MutexGuard = std::sync::MutexGuard<'static, ()>;

fn stage() -> Stage {
    Stage::new(
        Scale::SMALL,
        1,
        LightMix {
            count: 5,
            dynamic_pct: 50,
            layout: Layout::Spread,
        },
        1.0,
        0.12,
        120.0,
    )
}

fn aim(eye: [f32; 3], target: [f32; 3]) -> Camera {
    let d = [
        target[0] - eye[0],
        target[1] - eye[1],
        target[2] - eye[2],
    ];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1.0e-6);
    let pitch = (d[1] / len).clamp(-1.0, 1.0).asin();
    let yaw = d[0].atan2(-d[2]);
    let mut camera = Camera::new(eye[0], eye[2], yaw);
    camera.set_pose(Vec3::new(eye[0], eye[1], eye[2]), yaw, pitch);
    camera
}

fn draw(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &genos_render::World,
    camera: &Camera,
    settle: bool,
) -> Vec<u8> {
    let _ = window.pump();
    if settle {
        renderer.set_live_readback(false);
        renderer.draw(world, camera, true).expect("draw").expect("pixels")
    } else {
        renderer.set_live_readback(true);
        renderer.draw(world, camera, false).expect("draw");
        renderer.read_picture().expect("pixels")
    }
}

fn sample_y(pixels: &[u8], width: u32, height: u32, camera: &Camera, world: [f32; 3]) -> f32 {
    let aspect = width as f32 / height as f32;
    let uv = viewport_uv(camera, aspect, world).unwrap_or_else(|| panic!("off screen {world:?}"));
    assert!(
        (0.0..1.0).contains(&uv[0]) && (0.0..1.0).contains(&uv[1]),
        "uv {uv:?} for {world:?}"
    );
    let x = (uv[0] * width as f32).round() as i32;
    let y = (uv[1] * height as f32).round() as i32;
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for dy in -2..=2 {
        for dx in -2..=2 {
            let px = x + dx;
            let py = y + dy;
            if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                continue;
            }
            let i = ((py as u32 * width + px as u32) * 4) as usize;
            sum[0] += linear_from_display(pixels[i + 2] as f32 / 255.0);
            sum[1] += linear_from_display(pixels[i + 1] as f32 / 255.0);
            sum[2] += linear_from_display(pixels[i] as f32 / 255.0);
            count += 1.0;
        }
    }
    luminance([sum[0] / count, sum[1] / count, sum[2] / count])
}

fn on_foot(solid: &Solid, x: f32, z: f32) -> bool {
    let dx = x - solid.position.x;
    let dz = z - solid.position.z;
    let (s, c) = solid.yaw.sin_cos();
    let lx = c * dx - s * dz;
    let lz = s * dx + c * dz;
    let half = solid.size * 0.5 + 0.2;
    match solid.shape {
        Shape::Square => lx.abs() <= half && lz.abs() <= half,
        Shape::Circle => lx * lx + lz * lz <= half * half,
    }
}

fn clear_floor(scene: &Scene, x: f32, z: f32) -> bool {
    scene.solids.iter().all(|solid| !on_foot(solid, x, z))
}

fn red_box(scene: &Scene) -> &Solid {
    scene
        .solids
        .iter()
        .find(|solid| {
            (solid.color[0] - 0.9).abs() < 0.05
                && (solid.color[1] - 0.15).abs() < 0.05
                && (solid.color[2] - 0.1).abs() < 0.05
        })
        .expect("red moving box")
}

/// Point on the box face that looks at `eye`, just outside the surface, plus its normal and albedo.
fn box_face(solid: &Solid, eye: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let (s, c) = solid.yaw.sin_cos();
    let half = solid.size * 0.5;
    let y = solid.height * 0.5;
    let faces = [
        ([half, y, 0.0], [1.0, 0.0, 0.0]),
        ([-half, y, 0.0], [-1.0, 0.0, 0.0]),
        ([0.0, y, half], [0.0, 0.0, 1.0]),
        ([0.0, y, -half], [0.0, 0.0, -1.0]),
    ];
    let mut best_d = f32::MIN;
    let mut best = (faces[0].0, faces[0].1);
    for (local, normal) in faces {
        let wx = c * normal[0] + s * normal[2];
        let wz = -s * normal[0] + c * normal[2];
        let px = solid.position.x + c * local[0] + s * local[2];
        let py = local[1];
        let pz = solid.position.z - s * local[0] + c * local[2];
        let d = wx * (eye[0] - px) + normal[1] * (eye[1] - py) + wz * (eye[2] - pz);
        if d > best_d {
            best_d = d;
            best = ([px + wx * 0.01, py, pz + wz * 0.01], [wx, 0.0, wz]);
        }
    }
    best
}

fn floor_near(scene: &Scene, camera: &Camera, around: [f32; 2], aspect: f32) -> [f32; 3] {
    let eye = [
        camera.position.x,
        camera.position.y,
        camera.position.z,
    ];
    for ring in [1.3_f32, 1.8, 2.4, 3.2] {
        for step in 0..12 {
            let a = step as f32 * std::f32::consts::TAU / 12.0;
            let point = [around[0] + ring * a.cos(), 0.02, around[1] + ring * a.sin()];
            if !clear_floor(scene, point[0], point[2]) {
                continue;
            }
            let Some(uv) = viewport_uv(camera, aspect, point) else {
                continue;
            };
            if !(0.05..0.95).contains(&uv[0]) || !(0.05..0.95).contains(&uv[1]) {
                continue;
            }
            if sees(scene, eye, point) {
                return point;
            }
        }
    }
    panic!("no visible floor near {around:?}")
}

#[derive(Clone, Copy)]
struct Point {
    name: &'static str,
    at: [f32; 3],
    normal: [f32; 3],
    albedo: [f32; 3],
}

fn score(name: &str, pixels: &[u8], width: u32, height: u32, camera: &Camera, scene: &Scene, point: &Point) -> f32 {
    let engine = sample_y(pixels, width, height, camera, point.at);
    let reference = luminance(radiance(scene, point.at, point.normal, point.albedo));
    let score = agreement(engine, reference);
    println!(
        "point {name} engine_y={engine:.5} ref_y={reference:.5} score={score:.4} at={:.2?}",
        point.at
    );
    score
}

#[test]
fn the_small_stress_picture_matches_the_ray_trace() {
    let stage = stage();
    let scene = &stage.world.scene;
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    assert!(width >= 1280 && height >= 720, "{width}x{height}");

    let box_solid = red_box(scene);
    let hall_eye = [14.5, 1.7, 23.5];
    let (face_at, face_n) = box_face(box_solid, hall_eye);
    let face_albedo = box_solid.color;
    let hall = aim(hall_eye, face_at);
    let aspect = width as f32 / height as f32;
    let hall_floor = floor_near(
        scene,
        &hall,
        [box_solid.position.x, box_solid.position.z],
        aspect,
    );

    let room_eye = [6.4, 1.55, 1.4];
    let room = aim(room_eye, [4.0, 0.02, 4.5]);
    let room_floor = floor_near(scene, &room, [4.0, 4.5], aspect);

    // High and south-east so the north yard and the south shadow are both in sight.
    let outside = aim([36.0, 52.0, 46.0], [12.5, 1.0, 12.0]);
    let sun_yard = [12.5, 0.02, -4.0];
    // South of a solid wall, inside the sun's shadow. x=12.5 is the south doorway.
    let shadow_yard = [8.5, 0.02, 26.2];
    let eye = [outside.position.x, outside.position.y, outside.position.z];
    assert!(sees(scene, eye, sun_yard), "sun yard hidden");
    assert!(sees(scene, eye, shadow_yard), "shadow yard hidden");

    let hall_points = [
        Point {
            name: "hall_floor",
            at: hall_floor,
            normal: [0.0, 1.0, 0.0],
            albedo: [1.0, 1.0, 1.0],
        },
        Point {
            name: "box_face",
            at: face_at,
            normal: face_n,
            albedo: face_albedo,
        },
    ];
    let room_points = [Point {
        name: "room_a",
        at: room_floor,
        normal: [0.0, 1.0, 0.0],
        albedo: [1.0, 1.0, 1.0],
    }];
    let yard_points = [
        Point {
            name: "sun_yard",
            at: sun_yard,
            normal: [0.0, 1.0, 0.0],
            albedo: [1.0, 1.0, 1.0],
        },
        Point {
            name: "shadow_yard",
            at: shadow_yard,
            normal: [0.0, 1.0, 0.0],
            albedo: [1.0, 1.0, 1.0],
        },
    ];

    let started = std::time::Instant::now();
    let mut frames = 0u32;
    let mut live_hall = Vec::new();
    while started.elapsed() < std::time::Duration::from_millis(100) || frames < 1 {
        live_hall = draw(&mut window, &mut renderer, &stage.world, &hall, false);
        frames += 1;
        if frames > 30 {
            break;
        }
    }
    println!(
        "live_hall frames={frames} ms={:.1}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    for point in &hall_points {
        let _ = score(
            &format!("live_{}", point.name),
            &live_hall,
            width,
            height,
            &hall,
            scene,
            point,
        );
    }

    let settled_hall = draw(&mut window, &mut renderer, &stage.world, &hall, true);
    let mut scores = Vec::new();
    for point in &hall_points {
        scores.push(score(point.name, &settled_hall, width, height, &hall, scene, point));
    }
    let settled_room = draw(&mut window, &mut renderer, &stage.world, &room, true);
    for point in &room_points {
        scores.push(score(point.name, &settled_room, width, height, &room, scene, point));
    }
    let settled_out = draw(&mut window, &mut renderer, &stage.world, &outside, true);
    for point in &yard_points {
        scores.push(score(point.name, &settled_out, width, height, &outside, scene, point));
    }
    let mean = scores.iter().sum::<f32>() / scores.len() as f32;
    println!("mean={mean:.4} n={}", scores.len());
    assert!(
        scores.iter().all(|score| *score >= 0.80),
        "a point is under 0.80: {scores:?}"
    );
    assert!(mean >= 0.90, "mean {mean} is under 0.90");
}

fn warmup(window: &mut Window, renderer: &mut Renderer, world: &genos_render::World, camera: &Camera) {
    // The live frames stay inside the tier budget, so the first ones do not
    // finish the field. Settle once, then the timed cases start from that picture.
    let _ = draw(window, renderer, world, camera, true);
    for _ in 0..4 {
        let _ = draw(window, renderer, world, camera, false);
    }
}

fn over_time(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &genos_render::World,
    camera: &Camera,
    ms: u128,
) -> (Vec<u8>, u32, f64) {
    let started = std::time::Instant::now();
    let mut frames = 0u32;
    let mut pixels = Vec::new();
    while started.elapsed().as_millis() < ms || frames < 1 {
        pixels = draw(window, renderer, world, camera, false);
        frames += 1;
        if frames > 60 {
            break;
        }
    }
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    (pixels, frames, elapsed)
}

fn check_points(
    label: &str,
    pixels: &[u8],
    width: u32,
    height: u32,
    camera: &Camera,
    scene: &Scene,
    points: &[Point],
) {
    let mut scores = Vec::new();
    for point in points {
        scores.push(score(
            &format!("{label}_{}", point.name),
            pixels,
            width,
            height,
            camera,
            scene,
            point,
        ));
    }
    let mean = scores.iter().sum::<f32>() / scores.len() as f32;
    println!("{label} mean={mean:.4}");
    assert!(
        scores.iter().all(|score| *score >= 0.80),
        "{label} under 0.80: {scores:?}"
    );
    if points.len() >= 4 {
        assert!(mean >= 0.90, "{label} mean {mean} under 0.90");
    }
}

#[test]
fn the_live_picture_tracks_the_ray_trace_within_100ms() {
    let mut stage = stage();
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    assert!(width >= 1280 && height >= 720, "{width}x{height}");
    renderer.set_live_readback(true);

    let hall_eye = [14.5, 1.7, 23.5];
    let (face_at, face_n) = box_face(red_box(&stage.world.scene), hall_eye);
    let face_albedo = red_box(&stage.world.scene).color;
    let hall = aim(hall_eye, face_at);
    let aspect = width as f32 / height as f32;
    let hall_floor = floor_near(
        &stage.world.scene,
        &hall,
        [red_box(&stage.world.scene).position.x, red_box(&stage.world.scene).position.z],
        aspect,
    );
    let hall_points = [
        Point {
            name: "hall_floor",
            at: hall_floor,
            normal: [0.0, 1.0, 0.0],
            albedo: [1.0, 1.0, 1.0],
        },
        Point {
            name: "box_face",
            at: face_at,
            normal: face_n,
            albedo: face_albedo,
        },
    ];
    warmup(&mut window, &mut renderer, &stage.world, &hall);

    let lamp = stage
        .world
        .scene
        .lights
        .iter()
        .position(|light| {
            light.direction.length_squared() < 1.0e-6
                && (light.position.x - 12.5).abs() < 1.0
                && (light.position.z - 21.5).abs() < 1.0
        })
        .expect("hall lamp");
    let kept = stage.world.scene.lights[lamp].color;
    stage.world.scene.lights[lamp].color = [0.0, 0.0, 0.0];
    let (pixels, frames, ms) = over_time(&mut window, &mut renderer, &stage.world, &hall, 100);
    println!("lamp_off frames={frames} ms={ms:.1}");
    assert!(ms < 250.0, "lamp off took {ms:.1} ms");
    check_points("lamp_off", &pixels, width, height, &hall, &stage.world.scene, &hall_points);

    stage.world.scene.lights[lamp].color = kept;
    let (pixels, frames, ms) = over_time(&mut window, &mut renderer, &stage.world, &hall, 100);
    println!("lamp_on frames={frames} ms={ms:.1}");
    assert!(ms < 250.0, "lamp on took {ms:.1} ms");
    check_points("lamp_on", &pixels, width, height, &hall, &stage.world.scene, &hall_points);

    for step in 1..=3 {
        stage.advance(0.4);
        let (face_at, face_n) = box_face(red_box(&stage.world.scene), hall_eye);
        let albedo = red_box(&stage.world.scene).color;
        let points = [
            Point {
                name: "hall_floor",
                at: [14.2, 0.02, 21.3],
                normal: [0.0, 1.0, 0.0],
                albedo: [1.0, 1.0, 1.0],
            },
            Point {
                name: "box_face",
                at: face_at,
                normal: face_n,
                albedo,
            },
        ];
        // One frame after the box jumps is a single noisy gather. The next frame
        // is the same pose with that gather blended in.
        let mut pixels = Vec::new();
        for _ in 0..2 {
            pixels = draw(&mut window, &mut renderer, &stage.world, &hall, false);
        }
        check_points(
            &format!("pose{step}"),
            &pixels,
            width,
            height,
            &hall,
            &stage.world.scene,
            &points,
        );
    }

    let outside = aim([36.0, 52.0, 46.0], [12.5, 1.0, 12.0]);
    let sun_yard = Point {
        name: "sun_yard",
        at: [12.5, 0.02, -4.0],
        normal: [0.0, 1.0, 0.0],
        albedo: [1.0, 1.0, 1.0],
    };
    let shadow_yard = Point {
        name: "shadow_yard",
        at: [8.5, 0.02, 26.2],
        normal: [0.0, 1.0, 0.0],
        albedo: [1.0, 1.0, 1.0],
    };
    let yards = [sun_yard, shadow_yard];
    stage.sun_frozen = true;
    let frozen = stage.world.scene.clone();
    let skipped = renderer.tier_stats().interior_skipped;
    stage.advance(1.5);
    let (pixels, frames, ms) = over_time(&mut window, &mut renderer, &stage.world, &outside, 100);
    assert!(
        renderer.tier_stats().interior_skipped > skipped,
        "interior motion relit probes while the camera was outside"
    );
    println!("outside_still frames={frames} ms={ms:.1}");
    check_points("outside_still", &pixels, width, height, &outside, &frozen, &yards);

    let before = luminance(radiance(
        &stage.world.scene,
        sun_yard.at,
        sun_yard.normal,
        sun_yard.albedo,
    ));
    stage.sun_frozen = false;
    stage.day = 0.22;
    stage.boxes_still = true;
    stage.apply();
    let after = luminance(radiance(
        &stage.world.scene,
        sun_yard.at,
        sun_yard.normal,
        sun_yard.albedo,
    ));
    assert!((before - after).abs() > 0.02, "sun did not move the yard: {before} {after}");
    let (pixels, frames, ms) = over_time(&mut window, &mut renderer, &stage.world, &outside, 100);
    println!("sun_move frames={frames} ms={ms:.1}");
    check_points("sun_move", &pixels, width, height, &outside, &stage.world.scene, &yards);

    stage.day = 0.75;
    stage.apply();
    let (pixels, frames, ms) = over_time(&mut window, &mut renderer, &stage.world, &outside, 100);
    println!("sun_gone frames={frames} ms={ms:.1}");
    let settled = draw(&mut window, &mut renderer, &stage.world, &outside, true);
    let _ = score("sun_gone_live_sun", &pixels, width, height, &outside, &stage.world.scene, &yards[0]);
    let _ = score("sun_gone_settled_sun", &settled, width, height, &outside, &stage.world.scene, &yards[0]);
    let _ = score("sun_gone_settled_shadow", &settled, width, height, &outside, &stage.world.scene, &yards[1]);
    check_points("sun_gone", &pixels, width, height, &outside, &stage.world.scene, &yards);
}
