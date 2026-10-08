//! Camera orbit, moving lamp, and overlay churn, timed with the shipped profiler.
//!
//! The camera stays at eye height and looks across the room, so the walls stay
//! in the background. The lamp moves with the orbit. Lit-color reads happen
//! outside the timed loops.

use std::time::{Duration, Instant};

use genos_render::{probe_spacing, Renderer, ScreenRect, Simulation, World, FIELD_PLACE};
use genos_scene::{load_path, viewport_uv, Camera, Shape, Vec3};
use genos_ui::{FrameSample, OpenFrame, STAGE_COUNT, STAGE_LABELS};
use genos_window::Window;

const RADIUS: f32 = 4.5;
const START: f32 = -std::f32::consts::FRAC_PI_2;
const WARMUP: u32 = 20;
const STILL_FRAMES: u32 = 36;
const OVERLAY_FRAMES: u32 = 36;
const LERP_STEPS: u32 = 96;

fn open() -> (std::sync::MutexGuard<'static, ()>, Window, Renderer) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    let mut window = Window::open_proof(1280, 720).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, window, renderer)
}

fn scene() -> genos_scene::Scene {
    let path = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    ));
    load_path(path).expect("camera scene")
}

fn red_box(scene: &genos_scene::Scene) -> Vec3 {
    scene
        .solids
        .iter()
        .find(|solid| {
            solid.shape == Shape::Square
                && solid.color[0] > 0.8
                && solid.color[1] < 0.2
                && solid.color[2] < 0.2
        })
        .expect("red square solid")
        .position
}

/// Level view around the red box. Pitch stays at 0 so the far walls stay in frame.
fn orbit(center: Vec3, angle: f32) -> Camera {
    let x = center.x + RADIUS * angle.cos();
    let z = center.z + RADIUS * angle.sin();
    let dx = center.x - x;
    let dz = center.z - z;
    Camera::new(x, z, dx.atan2(-dz))
}

fn place_lamp(world: &mut World, angle: f32) {
    let Some(light) = world.scene.lights.first_mut() else {
        return;
    };
    light.position.x = 3.5 * angle.cos();
    light.position.y = 6.5 + angle.sin();
    light.position.z = 3.5 * angle.sin();
}

fn fixed_overlay() -> Vec<ScreenRect> {
    vec![ScreenRect {
        x: 16.0,
        y: 16.0,
        w: 48.0,
        h: 20.0,
        color: [0.15, 0.16, 0.18],
    }]
}

fn moving_overlay(frame: u32) -> Vec<ScreenRect> {
    let shift = frame as f32 * 3.0;
    vec![
        ScreenRect {
            x: 16.0 + shift,
            y: 16.0,
            w: 36.0 + (frame % 5) as f32,
            h: 18.0,
            color: [0.2, 0.7, 0.3],
        },
        ScreenRect {
            x: 20.0,
            y: 40.0 + (frame % 7) as f32,
            w: 22.0,
            h: 14.0,
            color: [0.85, 0.4, 0.15],
        },
    ]
}

fn floor_point(center: Vec3) -> [f32; 3] {
    // Just outside the red box. A point farther east drops off the bottom of a level view.
    [center.x + 1.0, 0.0, center.z]
}

struct Timed {
    frames: Vec<OpenFrame>,
    gpus: Vec<Option<Duration>>,
}

fn take_ready(renderer: &mut Renderer, gpus: &mut [Option<Duration>]) {
    let ready = renderer.poll_gpu_times().expect("gpu times");
    let mut ready = ready.into_iter();
    for slot in gpus.iter_mut() {
        if slot.is_none() {
            if let Some(time) = ready.next() {
                *slot = Some(time);
            } else {
                break;
            }
        }
    }
}

fn finish_gpus(renderer: &mut Renderer, gpus: &mut [Option<Duration>]) {
    let ready = renderer.finish_gpu_times().expect("finish gpu times");
    let mut ready = ready.into_iter();
    for slot in gpus.iter_mut() {
        if slot.is_none() {
            if let Some(time) = ready.next() {
                *slot = Some(time);
            } else {
                break;
            }
        }
    }
}

fn samples_of(timed: &Timed) -> Vec<FrameSample> {
    timed
        .frames
        .iter()
        .zip(timed.gpus.iter())
        .map(|(open, gpu)| open.finish(gpu.unwrap_or(Duration::ZERO)))
        .collect()
}

fn one_frame(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &mut World,
    sim: &mut Simulation,
    boot: Instant,
    camera: &Camera,
    overlay: &[ScreenRect],
    timestamps: bool,
    scene_cpu: Duration,
    ui_cpu: Duration,
) -> Result<(OpenFrame, Option<Duration>), String> {
    let pump_at = Instant::now();
    let pumped = window.pump();
    if pumped.closing {
        return Err("proof window closed".into());
    }
    let pump_cpu = pump_at.elapsed();
    let input_at = Instant::now();
    let _input = (pumped.pointer_x, pumped.pointer_y);
    let input_cpu = input_at.elapsed();
    let sim_at = Instant::now();
    sim.advance(world, 1.0 / 120.0);
    let sim_cpu = sim_at.elapsed();
    let draw_at = Instant::now();
    // A frame whose fence has already signaled hands back its span here; the others
    // come later through `take_ready`.
    let (draw_cpu, gpu) = if timestamps {
        let (_pixels, profile) = renderer.draw_profiled(world, camera, overlay, false, false)?;
        (profile.cpu, profile.gpu)
    } else {
        renderer.draw_with_overlay(world, camera, overlay, false, false)?;
        (draw_at.elapsed(), None)
    };
    let cpu = [pump_cpu, input_cpu, ui_cpu, scene_cpu, sim_cpu, draw_cpu];
    let duration = cpu.iter().copied().sum();
    Ok((
        OpenFrame {
            time: boot.elapsed(),
            duration,
            cpu,
        },
        gpu,
    ))
}

fn record(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &mut World,
    sim: &mut Simulation,
    boot: Instant,
    camera: &Camera,
    overlay: &[ScreenRect],
    timestamps: bool,
    scene_cpu: Duration,
    ui_cpu: Duration,
    timed: &mut Timed,
) -> Result<(), String> {
    let (open, gpu) = one_frame(
        window, renderer, world, sim, boot, camera, overlay, timestamps, scene_cpu, ui_cpu,
    )?;
    if timestamps {
        take_ready(renderer, &mut timed.gpus);
    }
    timed.frames.push(open);
    timed.gpus.push(gpu);
    Ok(())
}

fn median(values: &mut [Duration]) -> Duration {
    values.sort();
    values[values.len() / 2]
}

fn stage_medians(frames: &[FrameSample]) -> [Duration; STAGE_COUNT] {
    let mut medians = [Duration::ZERO; STAGE_COUNT];
    for stage in 0..STAGE_COUNT {
        let mut times: Vec<Duration> = frames.iter().map(|frame| frame.stages[stage].cpu).collect();
        medians[stage] = median(&mut times);
    }
    medians
}

fn report(name: &str, frames: &[FrameSample], timestamps: bool) -> (Duration, String) {
    let medians = stage_medians(frames);
    let slowest = (0..STAGE_COUNT)
        .max_by_key(|index| medians[*index])
        .unwrap_or(0);
    let mut durations: Vec<Duration> = frames.iter().map(|frame| frame.duration).collect();
    let mid = median(&mut durations);
    let peak = durations.iter().copied().max().unwrap_or_default();
    let slow_2ms = durations
        .iter()
        .filter(|time| **time > Duration::from_millis(2))
        .count();
    let slow_5ms = durations
        .iter()
        .filter(|time| **time > Duration::from_millis(5))
        .count();
    let mut gpu_times: Vec<Duration> = frames
        .iter()
        .map(|frame| frame.stages[5].gpu)
        .filter(|time| *time > Duration::ZERO)
        .collect();
    let gpu_peak = frames
        .iter()
        .map(|frame| frame.stages[5].gpu)
        .max()
        .unwrap_or_default();
    let gpu_line = if !timestamps {
        "Vulkan timestamp queries are not available on this device".to_string()
    } else if gpu_times.is_empty() {
        "gpu draw/present device time missing".to_string()
    } else {
        let gpu_mid = median(&mut gpu_times);
        format!(
            "gpu draw/present device time median {gpu_mid:?} count {}",
            gpu_times.len()
        )
    };
    let mut lines = format!(
        "{name} frames {} median {mid:?} max {peak:?}\n",
        frames.len()
    );
    for stage in 0..STAGE_COUNT {
        lines.push_str(&format!(
            "{name} {} median {:?}\n",
            STAGE_LABELS[stage], medians[stage]
        ));
    }
    lines.push_str(&format!(
        "{name} slowest stage: {}\n{name} {gpu_line}\n{name} gpu max {gpu_peak:?} frames over 2ms {slow_2ms} over 5ms {slow_5ms}\n",
        STAGE_LABELS[slowest]
    ));
    (mid, lines)
}

fn read_color(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw_with_overlay(world, camera, &fixed_overlay(), true, false)
        .expect("readback draw")
        .expect("readback pixels")
}

fn sample(pixels: &[u8], width: u32, height: u32, camera: &Camera, world: [f32; 3]) -> [f32; 3] {
    let aspect = width as f32 / height as f32;
    let uv = viewport_uv(camera, aspect, world).expect("floor point is on screen");
    assert!(
        (0.02..0.98).contains(&uv[0]) && (0.02..0.98).contains(&uv[1]),
        "floor point left the frame: {uv:?}"
    );
    let x = (uv[0] * width as f32).round() as i32;
    let y = (uv[1] * height as f32).round() as i32;
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let px = x + dx;
            let py = y + dy;
            if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                continue;
            }
            let index = ((py as u32 * width + px as u32) * 4) as usize;
            sum[0] += pixels[index + 2] as f32;
            sum[1] += pixels[index + 1] as f32;
            sum[2] += pixels[index] as f32;
            count += 1.0;
        }
    }
    [sum[0] / count, sum[1] / count, sum[2] / count]
}

fn summed(color: [f32; 3]) -> f32 {
    (color[0] + color[1] + color[2]) / 255.0
}

fn flicker(colors: &[[f32; 3]]) -> Option<(usize, f32, f32)> {
    for (index, window) in colors.windows(3).enumerate() {
        let a = summed(window[0]);
        let b = summed(window[1]);
        let c = summed(window[2]);
        let gap = (a - b).abs().min((c - b).abs());
        // 0.45 is three times the old 0.15. The lamp unit is 72 instead of 24.
        // A one-step shadow graze stays under this gap. A frame that drops to black does not.
        if (a - c).abs() <= 0.05 && gap >= 0.45 {
            return Some((index, a, b));
        }
    }
    None
}

#[test]
fn the_orbit_stays_in_the_still_frame_band() {
    let (_gpu, mut window, mut renderer) = open();
    let loaded = scene();
    let center = red_box(&loaded);
    let mut world = World::from_scene(loaded);
    let mut sim = Simulation::from_scene(&world.scene);
    let boot = Instant::now();
    let still_cam = orbit(center, START);
    let point = floor_point(center);
    let fixed = fixed_overlay();
    let timestamps = match renderer.draw_profiled(&world, &still_cam, &fixed, false, false) {
        Ok(_) => true,
        Err(err) if err.contains("timestamp") => {
            eprintln!("{err}");
            false
        }
        Err(err) => panic!("{err}"),
    };

    for _ in 0..WARMUP {
        let ui_at = Instant::now();
        let overlay = fixed_overlay();
        let ui_cpu = ui_at.elapsed();
        let scene_at = Instant::now();
        let camera = orbit(center, START);
        let scene_cpu = scene_at.elapsed();
        one_frame(
            &mut window,
            &mut renderer,
            &mut world,
            &mut sim,
            boot,
            &camera,
            &overlay,
            timestamps,
            scene_cpu,
            ui_cpu,
        )
        .expect("warmup");
        if timestamps {
            let _ = renderer.poll_gpu_times();
        }
    }

    let mut still = Timed {
        frames: Vec::new(),
        gpus: Vec::new(),
    };
    for _ in 0..STILL_FRAMES {
        let ui_at = Instant::now();
        let overlay = fixed_overlay();
        let ui_cpu = ui_at.elapsed();
        let scene_at = Instant::now();
        let camera = orbit(center, START);
        let scene_cpu = scene_at.elapsed();
        record(
            &mut window,
            &mut renderer,
            &mut world,
            &mut sim,
            boot,
            &camera,
            &overlay,
            timestamps,
            scene_cpu,
            ui_cpu,
            &mut still,
        )
        .expect("still frame");
    }
    if timestamps {
        finish_gpus(&mut renderer, &mut still.gpus);
    }
    let still_samples = samples_of(&still);
    let (still_mid, still_report) = report("still", &still_samples, timestamps);
    eprint!("{still_report}");

    let width = renderer.width();
    let height = renderer.height();
    let before = sample(
        &read_color(&mut window, &mut renderer, &world, &still_cam),
        width,
        height,
        &still_cam,
        point,
    );

    let mut overlay_run = Timed {
        frames: Vec::new(),
        gpus: Vec::new(),
    };
    for frame in 0..OVERLAY_FRAMES {
        let ui_at = Instant::now();
        let overlay = moving_overlay(frame);
        let ui_cpu = ui_at.elapsed();
        let scene_at = Instant::now();
        let camera = orbit(center, START);
        let scene_cpu = scene_at.elapsed();
        record(
            &mut window,
            &mut renderer,
            &mut world,
            &mut sim,
            boot,
            &camera,
            &overlay,
            timestamps,
            scene_cpu,
            ui_cpu,
            &mut overlay_run,
        )
        .expect("overlay frame");
    }
    if timestamps {
        finish_gpus(&mut renderer, &mut overlay_run.gpus);
    }
    let overlay_samples = samples_of(&overlay_run);
    let (overlay_mid, overlay_report) = report("overlay", &overlay_samples, timestamps);
    eprint!("{overlay_report}");
    let after = sample(
        &read_color(&mut window, &mut renderer, &world, &still_cam),
        width,
        height,
        &still_cam,
        point,
    );
    let overlay_gap = (summed(before) - summed(after)).abs();
    eprintln!(
        "overlay floor {:?} -> {:?} gap {overlay_gap:.4}",
        before, after
    );

    let home = renderer.field_player();
    let mut opposite = None;
    let mut lerp = Timed {
        frames: Vec::new(),
        gpus: Vec::new(),
    };
    for step in 0..LERP_STEPS {
        let angle = START + step as f32 / LERP_STEPS as f32 * std::f32::consts::TAU;
        let ui_at = Instant::now();
        let overlay = fixed_overlay();
        let ui_cpu = ui_at.elapsed();
        let scene_at = Instant::now();
        place_lamp(&mut world, angle);
        let camera = orbit(center, angle);
        let scene_cpu = scene_at.elapsed();
        record(
            &mut window,
            &mut renderer,
            &mut world,
            &mut sim,
            boot,
            &camera,
            &overlay,
            timestamps,
            scene_cpu,
            ui_cpu,
            &mut lerp,
        )
        .expect("lerp frame");
        if step + 1 == LERP_STEPS / 2 {
            opposite = Some((camera, renderer.field_player()));
        }
    }
    if timestamps {
        finish_gpus(&mut renderer, &mut lerp.gpus);
    }
    let lerp_samples = samples_of(&lerp);
    let (lerp_mid, lerp_report) = report("lerp", &lerp_samples, timestamps);
    eprint!("{lerp_report}");

    let mut colors = Vec::with_capacity(LERP_STEPS as usize);
    for step in 0..LERP_STEPS {
        let angle = START + step as f32 / LERP_STEPS as f32 * std::f32::consts::TAU;
        place_lamp(&mut world, angle);
        let camera = orbit(center, angle);
        let pixels = read_color(&mut window, &mut renderer, &world, &camera);
        colors.push(sample(&pixels, width, height, &camera, point));
    }
    if let Some((index, a, b)) = flicker(&colors) {
        panic!("floor color flickered at sample {index}: {a:.3} then {b:.3}");
    }

    let (opposite_cam, player) = opposite.expect("the orbit reached the far side");
    let dx = player[0] - home[0];
    let dz = player[1] - home[1];
    assert!(
        dx * dx + dz * dz > FIELD_PLACE * FIELD_PLACE,
        "the field stayed at the start {:?} instead of {:?}",
        home,
        player
    );
    let lag_x = player[0] - opposite_cam.position.x;
    let lag_z = player[1] - opposite_cam.position.z;
    assert!(
        lag_x * lag_x + lag_z * lag_z <= FIELD_PLACE * FIELD_PLACE,
        "the field player {:?} left the camera {:?}",
        player,
        opposite_cam.position
    );
    let eye = [player[0], opposite_cam.position.y, player[1]];
    let floor = &world.scene.floor;
    let far_x = if player[0] >= floor.position.x {
        floor.position.x - floor.half_x + 0.5
    } else {
        floor.position.x + floor.half_x - 0.5
    };
    let far_z = if player[1] >= floor.position.z {
        floor.position.z - floor.half_z + 0.5
    } else {
        floor.position.z + floor.half_z - 0.5
    };
    let near = probe_spacing(&world, eye, player[0], player[1]);
    let far = probe_spacing(&world, eye, far_x, far_z);
    eprintln!("probe spacing near {near:.3} far {far:.3} player {player:?}");
    assert!(
        (near - far).abs() < 1.0e-4,
        "spacing changed across the floor: near {near} far {far}"
    );

    assert!(
        overlay_gap < 0.05,
        "overlay changed the floor color by {overlay_gap:.3}: {before:?} -> {after:?}"
    );
    let limit = still_mid.saturating_mul(2);
    let spike = still_mid.saturating_mul(5);
    assert!(
        overlay_mid <= limit,
        "overlay median {overlay_mid:?} left the still band {still_mid:?}"
    );
    assert!(
        lerp_mid <= limit,
        "lerp median {lerp_mid:?} left the still band {still_mid:?}"
    );
    for (name, frames) in [("overlay", &overlay_samples), ("lerp", &lerp_samples)] {
        for (index, frame) in frames.iter().enumerate() {
            assert!(
                frame.duration <= spike,
                "{name} frame {index} took {:?} against still {still_mid:?} (5× is {spike:?})",
                frame.duration
            );
        }
    }
    if timestamps {
        let gpu_frames = lerp_samples
            .iter()
            .filter(|frame| frame.stages[5].gpu > Duration::ZERO)
            .count();
        assert!(
            gpu_frames > 0,
            "lerp gpu draw/present stored no device time"
        );
    }
}
