use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use genos_input::{character_controller, InputCode, InputSystem};
use genos_render::{Antialias, Renderer, ScreenRect, Simulation, World};

/// The flame, the smoke, and the flame lamp stay off while the radiance field is under study.
const SHOW_PARTICLES: bool = false;
use genos_scene::{load_path, update, Actions, Camera, Codimation, Easing, Scene, Vec3};
use genos_ui::{
    apply_frame_action, lighting_frame, Action, OpenFrame, Pointer, ProfileGraph, ProfileStream,
    State,
};
use genos_window::{extent_changed, FocusGate, Window};

/// Seconds for one leg of the red box path.
const BOX_LEG_SECONDS: f32 = 8.0;
/// Seconds for the first frame, before a previous frame exists.
const FIRST_FRAME_SECONDS: f32 = 1.0 / 60.0;
/// A hitch longer than this does not replay the missed time.
const MAX_FRAME_SECONDS: f32 = 0.25;

/// Elapsed seconds since `previous`. The first frame uses [`FIRST_FRAME_SECONDS`].
fn frame_seconds(previous: Option<Instant>, now: Instant) -> f32 {
    let elapsed = match previous {
        Some(start) => now.saturating_duration_since(start).as_secs_f32(),
        None => FIRST_FRAME_SECONDS,
    };
    if elapsed.is_finite() && elapsed > 0.0 {
        elapsed.min(MAX_FRAME_SECONDS)
    } else {
        0.0
    }
}

/// On-screen cost of the camera profiler.
///
/// `Basic` is the user default. `Detailed` adds GPU timestamps and the profile
/// file. Those queries cost frame time, so detailed stays off until a flag or
/// a `--proof` check asks for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProfileMode {
    Off,
    Basic,
    Detailed,
}

fn parse_profile_mode(explicit: Option<&str>, proof: bool) -> Result<ProfileMode, String> {
    match explicit {
        Some("off") => Ok(ProfileMode::Off),
        Some("basic") => Ok(ProfileMode::Basic),
        Some("detailed") => Ok(ProfileMode::Detailed),
        Some(other) => Err(format!("unknown profile mode {other}")),
        None if proof => Ok(ProfileMode::Detailed),
        None => Ok(ProfileMode::Basic),
    }
}

struct PendingProfile {
    open: OpenFrame,
    record: bool,
}

/// The red square in `scene.rhai`.
fn red_box_index(scene: &Scene) -> Option<usize> {
    scene
        .solids
        .iter()
        .position(|solid| solid.color[0] > 0.9 && solid.color[1] < 0.05 && solid.color[2] < 0.05)
}

/// Ease in and out between the wall corner and the far floor edge.
///
/// The loop starts on the home position, which sits on that diagonal.
fn red_box_path() -> Option<Codimation> {
    // Inner corner of the L. The long wall's south face is z = 4.8.
    // The east wall's west face is x = 5.8. The box is 1.5 m wide.
    let near = Vec3::new(5.8 - 0.75 - 0.08, 0.0, 4.8 - 0.75 - 0.08);
    let span = near.length();
    if span < 1.0e-4 {
        return None;
    }
    // Floor center (0, 1.4), half extents 8 and 9.36. Keep the box on the floor.
    let outward = near * (-1.0 / span);
    let min_x = -8.0 + 0.75;
    let min_z = 1.4 - 9.36 + 0.75;
    let tx = if outward.x < 0.0 {
        min_x / outward.x
    } else {
        f32::MAX
    };
    let tz = if outward.z < 0.0 {
        min_z / outward.z
    } else {
        f32::MAX
    };
    let far = outward * tx.min(tz);
    // The first leg runs from the floor edge to the corner, so Run from home
    // approaches the wall before it travels back out.
    let leg = near - far;
    let along = (Vec3::ZERO - far).dot(leg) / leg.length_squared().max(1.0e-6);
    let mut motion = Codimation::new(vec![far, near], Easing::EaseInOut, BOX_LEG_SECONDS)?;
    motion.seek(along.clamp(0.0, 1.0) * BOX_LEG_SECONDS);
    Some(motion)
}

fn main() {
    if let Err(err) = run() {
        eprintln!("genos-camera: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut frames = None;
    let mut readback = None;
    let mut trace = false;
    let mut proof = false;
    let mut eye = None;
    let mut pitch = 0.0f32;
    let mut lamp = None;
    let mut profile_arg = None;
    let mut scene_name = String::from("scene.rhai");
    let mut size = (1280u32, 720u32);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--frames" => {
                let value = args.next().ok_or("missing frame count")?;
                frames = Some(value.parse::<u32>().map_err(|err| err.to_string())?);
            }
            "--readback" => {
                readback = Some(PathBuf::from(args.next().ok_or("missing readback path")?));
            }
            "--trace" => trace = true,
            "--proof" => proof = true,
            "--eye" => {
                let x = args.next().ok_or("missing eye x")?;
                let z = args.next().ok_or("missing eye z")?;
                let yaw = args.next().ok_or("missing eye yaw")?;
                eye = Some((
                    x.parse::<f32>().map_err(|err| err.to_string())?,
                    z.parse::<f32>().map_err(|err| err.to_string())?,
                    yaw.parse::<f32>().map_err(|err| err.to_string())?,
                ));
            }
            "--pitch" => {
                let value = args.next().ok_or("missing pitch")?;
                pitch = value.parse::<f32>().map_err(|err| err.to_string())?;
            }
            "--lamp" => {
                let x = args.next().ok_or("missing lamp x")?;
                let y = args.next().ok_or("missing lamp y")?;
                let z = args.next().ok_or("missing lamp z")?;
                lamp = Some((
                    x.parse::<f32>().map_err(|err| err.to_string())?,
                    y.parse::<f32>().map_err(|err| err.to_string())?,
                    z.parse::<f32>().map_err(|err| err.to_string())?,
                ));
            }
            "--profile" => {
                profile_arg = Some(args.next().ok_or("missing profile mode")?);
            }
            "--scene" => {
                scene_name = args.next().ok_or("missing scene file")?;
            }
            "--size" => {
                let text = args.next().ok_or("missing size")?;
                let (w, h) = text.split_once('x').ok_or(format!("bad size {text}, want WxH"))?;
                size = (
                    w.parse::<u32>().map_err(|err| err.to_string())?,
                    h.parse::<u32>().map_err(|err| err.to_string())?,
                );
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }

    let scene_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(&scene_name);
    let scene = load_path(&scene_path)?;
    let mut camera = match eye {
        Some((x, z, yaw)) => Camera::new(x, z, yaw),
        None => Camera::opening(),
    };
    camera.pitch = pitch;
    camera.attach_scene(&scene);
    let red_box = red_box_index(&scene);
    if let Some(index) = red_box {
        if let Some(motion) = red_box_path() {
            camera.codimate(&scene, index, motion);
            camera.set_codimation_running(index, false);
        }
    }
    let bench_camera = camera.clone();
    let host = genos_mcp::Host::new(scene, camera);
    if let Some((x, y, z)) = lamp {
        host.with_frame(|scene, _| {
            scene.lights = vec![genos_scene::Light {
                position: genos_scene::Vec3::new(x, y, z),
                color: [1.0, 1.0, 1.0],

                direction: genos_scene::Vec3::ZERO,
            }];
        });
    }
    let server = genos_mcp::Server::start(host.clone())?;
    eprintln!("genos-camera mcp {}", server.url());
    let mut world = World::from_scene(host.drawn_scene());
    let mut scene_revision = host.scene_revision();
    let mut fire = SHOW_PARTICLES.then(|| Simulation::from_scene(&world.scene));
    let mut window = if proof {
        Window::open_proof(size.0, size.1)?
    } else {
        Window::open(size.0, size.1)?
    };
    let first = window.pump();
    let mut renderer = Renderer::open(window.display, window.surface, first.width, first.height)?;
    let mut drawn = 0u32;
    let mut size = (renderer.width(), renderer.height());
    let mut focus = FocusGate::default();
    let mut ui = State::default();
    let mut input = InputSystem::new();
    let controls = character_controller();
    let profile_mode = parse_profile_mode(profile_arg.as_deref(), proof)?;
    let mut profile_stream = match profile_mode {
        ProfileMode::Detailed => {
            let path = std::env::current_dir()
                .map_err(|err| err.to_string())?
                .join("genos-camera.profile");
            let stream = ProfileStream::create(path)?;
            eprintln!("genos-camera profile detailed {}", stream.path().display());
            Some(stream)
        }
        ProfileMode::Basic => {
            eprintln!("genos-camera profile basic");
            None
        }
        ProfileMode::Off => {
            eprintln!("genos-camera profile off");
            None
        }
    };
    let mut profile = ProfileGraph::default();
    let mut profile_pending: VecDeque<PendingProfile> = VecDeque::new();
    let mut graph_rects: Vec<ScreenRect> = Vec::new();
    let mut drag_origin: Option<Duration> = None;
    let boot = Instant::now();
    let mut wireframe = false;
    let mut cascade_view = [false; 3];
    let mut frame_clock: Option<Instant> = None;
    let mut bench = Bench::from_env(&world.scene, bench_camera)?;
    if let Some(dir) = bench.as_ref().and_then(|bench| bench.shots.as_ref()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("GENOS_BENCH_SHOTS {}: {e}", dir.display()))?;
    }
    if bench.is_some() {
        // A bench reads the picture as it is drawn (shots, the flicker phase): a
        // readback that settled the light would measure the settled field, not what
        // a moving lamp shows, and would run every build due inside the frame.
        renderer.set_live_readback(true);
    }
    loop {
        let now = Instant::now();
        let dt = frame_seconds(frame_clock, now);
        frame_clock = Some(now);
        // window pump
        let pump_at = Instant::now();
        let frame = window.pump();
        if frame.closing {
            break;
        }
        if extent_changed(size.0, size.1, frame.width, frame.height) || frame.resized {
            renderer.resize(frame.width, frame.height)?;
            size = (renderer.width(), renderer.height());
        }
        let pump_cpu = pump_at.elapsed();
        // input
        let input_at = Instant::now();
        let gated = focus.gate(&frame);
        input.begin_frame();
        input.apply_evdev_keys(&gated.keys_down);
        input.set_mouse_button(InputCode::mouse_left, gated.mouse_left);
        input.add_mouse_delta(gated.mouse_dx, gated.mouse_dy);
        input.poll_gamepads();
        let move_axis = controls.axis_2d(&input, "move");
        let look_axis = controls.axis_2d(&input, "look");
        let input_cpu = input_at.elapsed();
        let graph_on = profile_mode != ProfileMode::Off;
        let detailed = profile_mode == ProfileMode::Detailed;
        if detailed {
            let stream = profile_stream
                .as_mut()
                .ok_or("detailed profile is missing its file")?;
            publish_profile(
                &mut renderer,
                &mut profile_pending,
                &mut profile,
                stream,
                false,
            )?;
        }
        if graph_on && input.keyboard.pressed(InputCode::key_p) && !profile.is_paused() {
            // Wait for GPU times that were submitted before this key, then freeze.
            // A time that arrives after pause() stays out of the hold.
            let times = if detailed {
                renderer.finish_gpu_times()?
            } else {
                Vec::new()
            };
            hold_ready_frames(
                &mut profile,
                &mut profile_pending,
                times,
                profile_stream.as_mut(),
            )?;
        }
        if graph_on && input.keyboard.pressed(InputCode::key_r) {
            profile.reset();
            drag_origin = None;
        }
        let paused = graph_on && profile.is_paused();
        // camera/scene update, first half: the panel reads the lamp after a reload
        let scene_at = Instant::now();
        if host.scene_revision() != scene_revision {
            scene_revision = host.scene_revision();
            world = World::from_scene(host.drawn_scene());
        }
        let view = host.camera();
        wireframe = toggle_wireframe(wireframe, &input, view.captured);
        cascade_view = toggle_cascade_view(cascade_view, &input);
        let lamp = world
            .scene
            .lights
            .first()
            .map(|light| [light.position.x, light.position.y, light.position.z]);
        let mut scene_cpu = scene_at.elapsed();
        // ui
        let ui_at = Instant::now();
        let ui_frame = lighting_frame(
            &mut ui,
            [size.0 as f32, size.1 as f32],
            &view,
            lamp,
            Pointer {
                x: frame.pointer_x,
                y: frame.pointer_y,
                // A click can press and release before the next pump. The latch still hits the UI.
                down: !view.captured && (gated.mouse_left || gated.capture_click),
            },
        );
        let mut overlay: Vec<ScreenRect> = ui_frame
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
        // The graph prints a live frame time. A readback file must stay the same on the next launch.
        if readback.is_none() && graph_on {
            let viewport = [size.0 as f32, size.1 as f32];
            if profile.overlay_due(viewport, detailed, now) {
                copy_graph(
                    &profile.overlay_at(viewport, detailed, now),
                    &mut graph_rects,
                );
            }
            if paused {
                let down = gated.mouse_left || gated.capture_click;
                if let Some(start) = drag_origin {
                    if let Some(time) = profile.cached_time_at_x(frame.pointer_x) {
                        profile.select(start, time);
                    }
                    if !down {
                        drag_origin = None;
                    }
                } else if down {
                    if let Some(time) = profile.cached_time_at(frame.pointer_x, frame.pointer_y) {
                        drag_origin = Some(time);
                        profile.select(time, time);
                    }
                }
            } else {
                drag_origin = None;
            }
            // A drag stores the interval now. The picture still waits for the period.
            if profile.overlay_due(viewport, detailed, now) {
                copy_graph(
                    &profile.overlay_at(viewport, detailed, now),
                    &mut graph_rects,
                );
            }
            overlay.extend_from_slice(&graph_rects);
        }
        let ui_cpu = ui_at.elapsed();
        // camera/scene update
        let scene_at = Instant::now();
        let mut box_running = ui.box_running;
        for action in &ui_frame.actions {
            if matches!(action, Action::ToggleBoxRun) {
                box_running = !box_running;
            }
        }
        ui.box_running = box_running;
        let draw_camera = host.with_frame(|scene, camera| {
            if let Some(index) = red_box_index(scene) {
                camera.set_codimation_running(index, box_running);
            }
            for action in &ui_frame.actions {
                if let Some(mode) = apply_frame_action(scene, *action) {
                    renderer.set_antialias(Antialias::from_picture(mode.code()));
                }
            }
            if paused {
                camera.captured = false;
            }
            update(
                camera,
                scene,
                &Actions {
                    forward: if paused { 0.0 } else { move_axis.y },
                    strafe: if paused { 0.0 } else { move_axis.x },
                    mouse_dx: if paused { 0.0 } else { input.mouse.dx },
                    mouse_dy: if paused { 0.0 } else { input.mouse.dy },
                    look_x: if paused { 0.0 } else { look_axis.x },
                    look_y: if paused { 0.0 } else { look_axis.y },
                    capture_click: !paused
                        && ui_frame.look_capture
                        && (gated.capture_click || controls.down(&input, "capture")),
                    escape: controls.down(&input, "release"),
                },
                if paused { 0.0 } else { dt },
            );
            if paused {
                camera.captured = false;
            }
            camera.clone()
        });
        if host.scene_revision() != scene_revision {
            scene_revision = host.scene_revision();
            world = World::from_scene(host.drawn_scene());
        }
        if bench.is_none() && draw_camera.captured != frame.pointer_locked {
            match window.set_pointer_capture(draw_camera.captured) {
                Ok(()) => {}
                Err(err) => {
                    eprintln!("genos-camera: {err}");
                    host.with_frame(|_, camera| camera.captured = false);
                }
            }
        }
        scene_cpu += scene_at.elapsed();
        // simulation step
        let sim_at = Instant::now();
        if !paused {
            if let Some(fire) = fire.as_mut() {
                fire.advance(&mut world, dt);
            }
        }
        let sim_cpu = sim_at.elapsed();
        let draw_camera = match bench.as_mut() {
            Some(bench) => {
                if bench.step(&mut world.scene, renderer.tier_stats(), now) {
                    break;
                }
                bench.camera.clone()
            }
            None => draw_camera,
        };
        // gpu draw/present
        renderer.set_cascade_view(&world.scene, &draw_camera, cascade_view);
        let shot = bench.as_ref().and_then(|bench| {
            let name = bench.shot.as_ref()?;
            Some(bench.shots.as_ref()?.join(name))
        });
        let flicker_frame = bench.as_mut().is_some_and(|bench| bench.wants_frame(now));
        let want_read = readback.is_some() || shot.is_some() || flicker_frame;
        if bench.is_some() {
            // The panel and the live graph are not the scene: a bench draws without them.
            overlay.clear();
        }
        let draw_at = Instant::now();
        let (pixels, timing) = if detailed {
            let (pixels, timing) =
                renderer.draw_profiled(&world, &draw_camera, &overlay, want_read, wireframe)?;
            (pixels, Some(timing))
        } else {
            let pixels =
                renderer.draw_with_overlay(&world, &draw_camera, &overlay, want_read, wireframe)?;
            (pixels, None)
        };
        let draw_cpu = timing
            .map(|timing| timing.cpu)
            .unwrap_or_else(|| draw_at.elapsed());
        let draw_gpu = timing.and_then(|timing| timing.gpu);
        let duration = pump_cpu + input_cpu + ui_cpu + scene_cpu + sim_cpu + draw_cpu;
        let open = OpenFrame {
            time: boot.elapsed(),
            duration,
            cpu: [pump_cpu, input_cpu, ui_cpu, scene_cpu, sim_cpu, draw_cpu],
        };
        if detailed {
            let stream = profile_stream
                .as_mut()
                .ok_or("detailed profile is missing its file")?;
            // A paused present stays on screen and does not enter the held graph.
            let record = !paused;
            if let Some(gpu) = draw_gpu {
                publish_profile(
                    &mut renderer,
                    &mut profile_pending,
                    &mut profile,
                    stream,
                    false,
                )?;
                if !profile_pending.is_empty() {
                    return Err("profile gpu time arrived out of order".into());
                }
                if record {
                    let sample = open.finish(gpu);
                    stream.append(&sample)?;
                    profile.remember(sample);
                }
            } else {
                profile_pending.push_back(PendingProfile { open, record });
                publish_profile(
                    &mut renderer,
                    &mut profile_pending,
                    &mut profile,
                    stream,
                    false,
                )?;
            }
        } else if graph_on && !paused {
            profile.remember(open.finish(Duration::ZERO));
        }
        if flicker_frame {
            if let (Some(pixels), Some(bench)) = (pixels.as_ref(), bench.as_mut()) {
                bench.take_frame(pixels, renderer.width(), renderer.height(), now);
            }
        }
        if want_read && (readback.is_some() || shot.is_some()) {
            if let Some(pixels) = pixels {
                write_png(
                    shot.as_ref().or(readback.as_ref()).unwrap(),
                    renderer.width(),
                    renderer.height(),
                    &pixels,
                )?;
            }
        }
        if trace {
            println!(
                "trace size={}x{} focused={} captured={} locked={} yaw={:.4} pitch={:.4} x={:.3} y={:.3} z={:.3} move={:.2},{:.2} look={:.2},{:.2}",
                renderer.width(),
                renderer.height(),
                frame.focused as u8,
                draw_camera.captured as u8,
                frame.pointer_locked as u8,
                draw_camera.yaw,
                draw_camera.pitch,
                draw_camera.position.x,
                draw_camera.position.y,
                draw_camera.position.z,
                move_axis.x,
                move_axis.y,
                look_axis.x,
                look_axis.y,
            );
        }
        drawn += 1;
        if frames.is_some_and(|limit| drawn >= limit) {
            break;
        }
    }
    if profile_mode == ProfileMode::Detailed {
        let stream = profile_stream
            .as_mut()
            .ok_or("detailed profile is missing its file")?;
        publish_profile(
            &mut renderer,
            &mut profile_pending,
            &mut profile,
            stream,
            true,
        )?;
        if !profile_pending.is_empty() {
            return Err("profile frames are missing gpu time".into());
        }
    }
    if let Some(bench) = bench {
        bench.report(renderer.width(), renderer.height(), renderer.antialias(), renderer.tier_weights());
    }
    println!("genos-camera frames={drawn}");
    Ok(())
}

/// `GENOS_BENCH=<seconds>`: an unattended benchmark. The camera stays where it starts
/// (the opening view, or `--eye`/`--pitch`); input is ignored. After a short warm-up the scene stands still for a third
/// of the time, then the first lamp circles the boxes for a third, then it drops to
/// beside the boxes and rises back (the settling clip), each time until the tier has
/// no work left. Then the lamp orbits for the flicker phase, and the red box (or the
/// first solid) is dragged across the floor: `Drag` times the frames while it moves,
/// `Rest` how long its light takes to settle once it stops, and `DragFlicker` reads
/// back every frame as it moves a fixed step a frame. Last, `Switch` moves every solid
/// off the probe grid at once (every brick places its probes again, like loading
/// another room) and times how the picture converges. One `BENCH` line per phase,
/// then the program exits.
struct Bench {
    camera: Camera,
    phase: BenchPhase,
    phase_start: Instant,
    span: Duration,
    home: Vec3,
    last: Option<Instant>,
    frames: Vec<f32>,
    /// When a drop or a rise had taken all its change passes: the visible change.
    visible: Option<Duration>,
    /// When no brick on screen was still changing (its passes issued), the most bricks
    /// on screen a batch took at once, and the slowest batch pick in microseconds.
    view: Option<Duration>,
    critical: usize,
    batch_us: u32,
    lines: Vec<String>,
    /// `GENOS_BENCH_SHOTS=<dir>`: live pictures at fixed times into each drop and rise.
    shots: Option<PathBuf>,
    /// The next of `BENCH_SHOT_MS` to take, and the name of the picture due this frame.
    next_shot: usize,
    shot: Option<String>,
    /// The tier as of the last frame: bricks in it, and those still changing or due.
    tier: genos_render::TierStats,
    flicker: Flicker,
    /// The solid the drag phases move, and where it started.
    drag: Option<(usize, Vec3)>,
    drag_flicker: Flicker,
    /// Where every solid started, for `Switch`.
    solids: Vec<Vec3>,
    /// Pictures of the phase that settles (`Rest`, `Switch`), against its last.
    converge: Converge,
}

/// How fast the drag phases move the solid, in metres a second, and for how long
/// `Drag` moves it.
const BENCH_DRAG_SPEED: f32 = 1.0;
const BENCH_DRAG_TIME: Duration = Duration::from_millis(1500);
/// The solid's step per frame in `DragFlicker`: the drag speed at 600 frames a second.
const BENCH_DRAG_STEP: f32 = BENCH_DRAG_SPEED / 600.0;

/// Frames the flicker phase measures (`GENOS_BENCH_FLICKER=<frames>`, 0 skips it).
const BENCH_FLICKER_FRAMES: u32 = 240;
/// Frames of orbit before it measures, so the drop from the rise is gone.
const BENCH_FLICKER_LEAD: u32 = 60;
/// The lamp's turn per frame while it measures flicker: one turn in 3600 frames, about
/// the orbit's pace (one turn in six seconds) at 600 frames a second. A faster step
/// moves a lit floor pixel several 8-bit codes a frame, and its rounding alone then
/// reads as flicker.
const BENCH_FLICKER_STEP: f32 = std::f32::consts::TAU / 3600.0;
/// The flicker grid: 4 x 4 screen tiles.
const FLICKER_TILES: usize = 4;

/// The flicker phase: the lamp orbits a fixed step per frame and every frame is read
/// back. A pixel flickers when its 8-bit luminance turns back: it rises then falls, or
/// falls then rises, by the smaller of the two steps. A moving lamp or a moving shadow
/// edge changes a pixel one way at a time, rounding and all, so it reads zero. Each
/// tile takes the mean of its pixels (a small flickering patch still counts), averaged
/// over the frames.
#[derive(Default)]
struct Flicker {
    frames: u32,
    drawn: u32,
    measured: u32,
    last: Vec<Vec<f32>>,
    tiles: [f64; FLICKER_TILES * FLICKER_TILES],
}

impl Flicker {
    fn take(&mut self, bgra: &[u8], width: u32, height: u32) {
        self.drawn += 1;
        if self.drawn <= BENCH_FLICKER_LEAD {
            return;
        }
        let (w, h) = (width as usize, height as usize);
        let luma: Vec<f32> = bgra
            .chunks_exact(4)
            .take(w * h)
            .map(|p| 0.0722 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.2126 * f32::from(p[2]))
            .collect();
        if luma.len() < w * h {
            return;
        }
        self.last.push(luma);
        if self.last.len() < 3 {
            return;
        }
        let (a, b, c) = (&self.last[0], &self.last[1], &self.last[2]);
        for ty in 0..FLICKER_TILES {
            for tx in 0..FLICKER_TILES {
                let mut sum = 0.0f64;
                let mut count = 0usize;
                for y in ty * h / FLICKER_TILES..(ty + 1) * h / FLICKER_TILES {
                    for x in tx * w / FLICKER_TILES..(tx + 1) * w / FLICKER_TILES {
                        let i = y * w + x;
                        let (before, after) = (b[i] - a[i], c[i] - b[i]);
                        if before * after < 0.0 {
                            sum += f64::from(before.abs().min(after.abs()));
                        }
                        count += 1;
                    }
                }
                if count > 0 {
                    self.tiles[ty * FLICKER_TILES + tx] += sum / count as f64;
                }
            }
        }
        self.last.remove(0);
        self.measured += 1;
    }

    fn summary(&self) -> String {
        let n = f64::from(self.measured.max(1));
        let tiles: Vec<f64> = self.tiles.iter().map(|t| t / n).collect();
        let mean = tiles.iter().sum::<f64>() / tiles.len() as f64;
        let (worst, max) = tiles
            .iter()
            .enumerate()
            .fold((0, 0.0f64), |best, (i, &v)| if v > best.1 { (i, v) } else { best });
        let all: Vec<String> = tiles.iter().map(|t| format!("{t:.3}")).collect();
        format!(
            " flicker_frames={} flicker_mean={mean:.3} flicker_max={max:.3} worst_tile={},{} tiles={}",
            self.measured,
            worst % FLICKER_TILES,
            worst / FLICKER_TILES,
            all.join(","),
        )
    }
}

/// How far `Switch` moves every solid, in metres along x and z: off the probe grid, so
/// every brick near a face places its probes again.
const BENCH_SWITCH_SHIFT: f32 = 0.37;
/// How often a settling phase reads the picture back for its convergence.
const CONVERGE_EVERY: Duration = Duration::from_millis(25);
/// A converge picture keeps every this-many-th pixel along x and y.
const CONVERGE_STEP: usize = 4;
/// A pixel has converged within this many 8-bit luminance codes of the last picture.
const CONVERGE_CODES: u8 = 6;
/// The share of pixels that have converged for the picture to count as converged.
const CONVERGE_SHARE: f64 = 0.95;

/// A settling phase's pictures (luminance, every [`CONVERGE_STEP`]-th pixel), read
/// back every [`CONVERGE_EVERY`] so the readbacks barely slow it. The last is the
/// settled picture; `conv95_ms` is when the share of pixels within
/// [`CONVERGE_CODES`] of it reached [`CONVERGE_SHARE`] for good, and `start_err` the
/// mean luminance error and `start_within` the share within tolerance of the first.
#[derive(Default)]
struct Converge {
    last: Option<Instant>,
    pictures: Vec<(f32, Vec<u8>)>,
}

impl Converge {
    fn wants(&mut self, now: Instant) -> bool {
        if self.last.is_some_and(|last| now - last < CONVERGE_EVERY) {
            return false;
        }
        self.last = Some(now);
        true
    }

    fn take(&mut self, bgra: &[u8], width: u32, height: u32, ms: f32) {
        let (w, h) = (width as usize, height as usize);
        if bgra.len() < w * h * 4 {
            return;
        }
        let mut luma = Vec::with_capacity((w / CONVERGE_STEP + 1) * (h / CONVERGE_STEP + 1));
        for y in (0..h).step_by(CONVERGE_STEP) {
            for x in (0..w).step_by(CONVERGE_STEP) {
                let p = &bgra[(y * w + x) * 4..(y * w + x) * 4 + 3];
                let l = 0.0722 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.2126 * f32::from(p[2]);
                luma.push(l.round().clamp(0.0, 255.0) as u8);
            }
        }
        self.pictures.push((ms, luma));
    }

    fn summary(&self) -> String {
        let Some((_, last)) = self.pictures.last() else {
            return " conv95_ms=-1".into();
        };
        let n = last.len().max(1) as f64;
        let within = |picture: &[u8]| {
            picture.iter().zip(last).filter(|(a, b)| a.abs_diff(**b) <= CONVERGE_CODES).count() as f64 / n
        };
        // The first picture from which every later one stays converged.
        let mut conv = None;
        for (ms, picture) in &self.pictures {
            if within(picture) < CONVERGE_SHARE {
                conv = None;
            } else if conv.is_none() {
                conv = Some(*ms);
            }
        }
        let first = &self.pictures[0].1;
        let err = first.iter().zip(last).map(|(a, b)| f64::from(a.abs_diff(*b))).sum::<f64>() / n;
        format!(
            " conv95_ms={:.0} start_err={err:.2} start_within={:.3} pictures={}",
            conv.unwrap_or(-1.0),
            within(first),
            self.pictures.len()
        )
    }
}

/// Milliseconds into a drop or a rise at which `GENOS_BENCH_SHOTS` takes a picture.
const BENCH_SHOT_MS: [u64; 12] = [0, 17, 33, 50, 100, 150, 200, 300, 500, 1000, 2000, 4000];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BenchPhase {
    Warmup,
    Still,
    Moving,
    Down,
    Up,
    Flicker,
    Drag,
    Rest,
    DragFlicker,
    Switch,
}

/// The longest a drop or a rise may take to settle before the bench gives up on it.
const BENCH_SETTLE_LIMIT: Duration = Duration::from_secs(10);
/// Seconds of warm-up before the first measured phase.
const BENCH_WARMUP: Duration = Duration::from_secs(3);

impl Bench {
    fn from_env(scene: &Scene, camera: Camera) -> Result<Option<Self>, String> {
        let Ok(value) = std::env::var("GENOS_BENCH") else {
            return Ok(None);
        };
        let seconds: f32 = value.parse().map_err(|_| format!("GENOS_BENCH={value} is not seconds"))?;
        if scene.lights.is_empty() {
            return Err("GENOS_BENCH needs a lamp in the scene".into());
        }
        let now = Instant::now();
        let flicker_frames = match std::env::var("GENOS_BENCH_FLICKER") {
            Ok(v) => v.parse().map_err(|_| format!("GENOS_BENCH_FLICKER={v} is not frames"))?,
            Err(_) => BENCH_FLICKER_FRAMES,
        };
        let drag = red_box_index(scene)
            .or((!scene.solids.is_empty()).then_some(0))
            .map(|i| (i, scene.solids[i].position));
        Ok(Some(Self {
            camera,
            phase: BenchPhase::Warmup,
            phase_start: now,
            span: Duration::from_secs_f32((seconds / 3.0).max(1.0)),
            home: scene.lights[0].position,
            last: None,
            frames: Vec::new(),
            visible: None,
            view: None,
            critical: 0,
            batch_us: 0,
            lines: Vec::new(),
            shots: std::env::var_os("GENOS_BENCH_SHOTS").map(PathBuf::from),
            next_shot: 0,
            shot: None,
            tier: genos_render::TierStats::default(),
            flicker: Flicker { frames: flicker_frames, ..Flicker::default() },
            drag,
            drag_flicker: Flicker { frames: flicker_frames, ..Flicker::default() },
            solids: scene.solids.iter().map(|s| s.position).collect(),
            converge: Converge::default(),
        }))
    }

    /// Set the lamp for this frame and record the last frame's time. True when done.
    fn step(&mut self, scene: &mut Scene, tier: genos_render::TierStats, now: Instant) -> bool {
        let pending = tier.pending_bricks;
        self.tier = tier;
        if let Some(last) = self.last {
            self.frames.push((now - last).as_secs_f32() * 1000.0);
        }
        self.last = Some(now);
        let elapsed = now - self.phase_start;
        let next = match self.phase {
            BenchPhase::Warmup => (elapsed >= BENCH_WARMUP).then_some(BenchPhase::Still),
            BenchPhase::Still => (elapsed >= self.span).then_some(BenchPhase::Moving),
            BenchPhase::Moving => (elapsed >= self.span).then_some(BenchPhase::Down),
            BenchPhase::Down => (pending == 0 || elapsed >= BENCH_SETTLE_LIMIT).then_some(BenchPhase::Up),
            BenchPhase::Up => (pending == 0 || elapsed >= BENCH_SETTLE_LIMIT).then_some(BenchPhase::Flicker),
            BenchPhase::Flicker => {
                (self.flicker.measured >= self.flicker.frames).then_some(BenchPhase::Drag)
            }
            BenchPhase::Drag => (elapsed >= BENCH_DRAG_TIME).then_some(BenchPhase::Rest),
            BenchPhase::Rest => {
                (pending == 0 || elapsed >= BENCH_SETTLE_LIMIT).then_some(BenchPhase::DragFlicker)
            }
            BenchPhase::DragFlicker => {
                (self.drag_flicker.measured >= self.drag_flicker.frames).then_some(BenchPhase::Switch)
            }
            BenchPhase::Switch => (pending == 0 || elapsed >= BENCH_SETTLE_LIMIT).then_some(BenchPhase::Switch),
        };
        // A drop, a rise or a rest counts from its first frame; its first frame always
        // has work.
        let settle = matches!(self.phase, BenchPhase::Down | BenchPhase::Up | BenchPhase::Rest | BenchPhase::Switch);
        if settle && self.frames.len() > 1 && tier.changing_bricks == 0 && self.visible.is_none() {
            self.visible = Some(elapsed);
        }
        if settle {
            self.critical = self.critical.max(tier.critical_bricks);
            self.batch_us = self.batch_us.max(tier.batch_us);
            if self.frames.len() > 1 && tier.critical_bricks == 0 && self.view.is_none() {
                self.view = Some(elapsed);
            }
        }
        let done = next.is_some() && (!settle || self.frames.len() > 1);
        if done {
            if self.phase != BenchPhase::Warmup {
                self.finish(elapsed, pending);
            }
            let last = match self.phase {
                BenchPhase::Up => self.flicker.frames == 0 && self.drag.is_none(),
                BenchPhase::Flicker => self.drag.is_none(),
                BenchPhase::Switch => true,
                _ => false,
            };
            if last {
                return true;
            }
            self.phase = next.unwrap_or(self.phase);
            if self.phase == BenchPhase::Flicker && self.flicker.frames == 0 {
                self.phase = BenchPhase::Drag;
            }
            self.phase_start = now;
            self.frames.clear();
            self.visible = None;
            self.view = None;
            self.critical = 0;
            self.batch_us = 0;
            self.next_shot = 0;
            self.converge = Converge::default();
            if self.phase == BenchPhase::DragFlicker && self.drag_flicker.frames == 0 {
                self.phase = BenchPhase::Switch;
            }
        }
        let t = (now - self.phase_start).as_secs_f32();
        self.shot = None;
        if self.shots.is_some() && matches!(self.phase, BenchPhase::Down | BenchPhase::Up) {
            let ms = (now - self.phase_start).as_millis() as u64;
            while self.next_shot < BENCH_SHOT_MS.len() && BENCH_SHOT_MS[self.next_shot] <= ms {
                self.next_shot += 1;
                self.shot = Some(format!("{:?}_{ms:05}ms.png", self.phase).to_lowercase());
            }
        }
        let lamp = &mut scene.lights[0].position;
        *lamp = match self.phase {
            BenchPhase::Warmup | BenchPhase::Still | BenchPhase::Up => self.home,
            // About one turn every six seconds, around the boxes.
            BenchPhase::Moving => Vec3::new(1.0 + 2.5 * t.cos(), 2.5, -1.0 + 2.5 * t.sin()),
            BenchPhase::Down => Vec3::new(0.8, 1.6, -1.6),
            BenchPhase::Flicker => {
                let a = self.flicker.drawn as f32 * BENCH_FLICKER_STEP;
                Vec3::new(1.0 + 2.5 * a.cos(), 2.5, -1.0 + 2.5 * a.sin())
            }
            BenchPhase::Drag | BenchPhase::Rest | BenchPhase::DragFlicker | BenchPhase::Switch => self.home,
        };
        if let Some((index, start)) = self.drag {
            // Out along x while `Drag` runs, held for `Rest`, then back a step a frame.
            let out = BENCH_DRAG_SPEED * BENCH_DRAG_TIME.as_secs_f32();
            let x = match self.phase {
                BenchPhase::Drag => BENCH_DRAG_SPEED * t.min(BENCH_DRAG_TIME.as_secs_f32()),
                BenchPhase::Rest => out,
                BenchPhase::DragFlicker => out - self.drag_flicker.drawn as f32 * BENCH_DRAG_STEP,
                _ => 0.0,
            };
            scene.solids[index].position = start + Vec3::new(x, 0.0, 0.0);
        }
        if self.phase == BenchPhase::Switch {
            let shift = Vec3::new(BENCH_SWITCH_SHIFT, 0.0, BENCH_SWITCH_SHIFT);
            for (solid, start) in scene.solids.iter_mut().zip(&self.solids) {
                solid.position = *start + shift;
            }
        }
        false
    }

    /// The flicker phases read back every frame, the settling ones every
    /// [`CONVERGE_EVERY`].
    fn wants_frame(&mut self, now: Instant) -> bool {
        match self.phase {
            BenchPhase::Flicker | BenchPhase::DragFlicker => true,
            BenchPhase::Rest | BenchPhase::Switch => self.converge.wants(now),
            _ => false,
        }
    }

    /// Hand this frame's readback to the phase that asked for it.
    fn take_frame(&mut self, bgra: &[u8], width: u32, height: u32, now: Instant) {
        match self.phase {
            BenchPhase::DragFlicker => self.drag_flicker.take(bgra, width, height),
            BenchPhase::Flicker => self.flicker.take(bgra, width, height),
            _ => {
                let ms = (now - self.phase_start).as_secs_f32() * 1000.0;
                self.converge.take(bgra, width, height, ms);
            }
        }
    }

    fn finish(&mut self, elapsed: Duration, pending: usize) {
        let mut sorted = self.frames.clone();
        sorted.sort_by(f32::total_cmp);
        let n = sorted.len().max(1);
        let mean = sorted.iter().sum::<f32>() / n as f32;
        let at = |q: f32| sorted.get(((n as f32 * q) as usize).min(n - 1)).copied().unwrap_or(0.0);
        let mut line = format!(
            "BENCH {:?} frames={} seconds={:.2} fps={:.0} ms_mean={:.3} ms_p50={:.3} ms_p99={:.3} ms_max={:.3}",
            self.phase,
            sorted.len(),
            elapsed.as_secs_f32(),
            1000.0 / mean.max(1.0e-6),
            mean,
            at(0.5),
            at(0.99),
            sorted.last().copied().unwrap_or(0.0),
        );
        if matches!(self.phase, BenchPhase::Down | BenchPhase::Up | BenchPhase::Rest | BenchPhase::Switch) {
            let visible = self.visible.map_or(-1.0, |v| v.as_secs_f32() * 1000.0);
            let view = self.view.map_or(-1.0, |v| v.as_secs_f32() * 1000.0);
            line.push_str(&format!(
                " view_ms={view:.0} critical={} batch_us={}",
                self.critical, self.batch_us
            ));
            line.push_str(&format!(
                " visible_ms={visible:.0} settle_ms={:.0} settled={} bricks={} changing={} pending={}",
                elapsed.as_secs_f32() * 1000.0,
                pending == 0,
                self.tier.bricks,
                self.tier.changing_bricks,
                pending,
            ));
        }
        if self.phase == BenchPhase::Flicker {
            line.push_str(&self.flicker.summary());
        }
        if self.phase == BenchPhase::DragFlicker {
            line.push_str(&self.drag_flicker.summary());
        }
        if matches!(self.phase, BenchPhase::Rest | BenchPhase::Switch) {
            line.push_str(&self.converge.summary());
        }
        eprintln!("{line}");
        self.lines.push(line);
    }

    fn report(&self, width: u32, height: u32, antialias: Antialias, weights: genos_render::TierWeights) {
        println!("BENCH size={width}x{height} antialias={antialias:?}");
        println!("BENCH tier_weights {weights}");
        for line in &self.lines {
            println!("{line}");
        }
    }
}

fn copy_graph(view: &genos_ui::ProfileView, rects: &mut Vec<ScreenRect>) {
    rects.clear();
    rects.extend(view.paints.iter().map(|paint| ScreenRect {
        x: paint.x,
        y: paint.y,
        w: paint.w,
        h: paint.h,
        color: paint.color,
    }));
}

/// Publish GPU times for frames that were already submitted, then freeze the graph.
///
/// `record` frames land in the hold. Frames remembered after this call do not.
fn hold_ready_frames(
    graph: &mut ProfileGraph,
    pending: &mut VecDeque<PendingProfile>,
    gpu_times: Vec<Duration>,
    mut stream: Option<&mut ProfileStream>,
) -> Result<(), String> {
    let mut finished = Vec::new();
    for gpu in gpu_times {
        let pending_frame = pending
            .pop_front()
            .ok_or("profile gpu time arrived without a frame")?;
        if pending_frame.record {
            finished.push(pending_frame.open.finish(gpu));
        }
    }
    graph.pause_after(finished.iter().cloned());
    if let Some(stream) = stream.as_mut() {
        for sample in &finished {
            stream.append(sample)?;
        }
    }
    Ok(())
}

fn publish_profile(
    renderer: &mut Renderer,
    pending: &mut VecDeque<PendingProfile>,
    graph: &mut ProfileGraph,
    stream: &mut ProfileStream,
    wait: bool,
) -> Result<(), String> {
    let times = if wait {
        renderer.finish_gpu_times()?
    } else {
        renderer.poll_gpu_times()?
    };
    for gpu in times {
        let pending_frame = pending
            .pop_front()
            .ok_or("profile gpu time arrived without a frame")?;
        if pending_frame.record {
            let sample = pending_frame.open.finish(gpu);
            stream.append(&sample)?;
            graph.remember(sample);
        }
    }
    Ok(())
}

fn write_png(path: &std::path::Path, width: u32, height: u32, bgra: &[u8]) -> Result<(), String> {
    let mut raw = Vec::with_capacity(((width * 3 + 1) * height) as usize);
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            raw.push(bgra.get(i + 2).copied().unwrap_or(0));
            raw.push(bgra.get(i + 1).copied().unwrap_or(0));
            raw.push(bgra.get(i).copied().unwrap_or(0));
        }
    }
    let mut zlib = Vec::new();
    zlib.extend_from_slice(&[0x78, 0x01]);
    let mut adler_s1: u32 = 1;
    let mut adler_s2: u32 = 0;
    let mut rest = raw.as_slice();
    while !rest.is_empty() {
        let take = rest.len().min(65535);
        let last = take == rest.len();
        zlib.push(if last { 1 } else { 0 });
        zlib.extend_from_slice(&(take as u16).to_le_bytes());
        zlib.extend_from_slice(&(!take as u16).to_le_bytes());
        zlib.extend_from_slice(&rest[..take]);
        for byte in &rest[..take] {
            adler_s1 = (adler_s1 + *byte as u32) % 65521;
            adler_s2 = (adler_s2 + adler_s1) % 65521;
        }
        rest = &rest[take..];
    }
    zlib.extend_from_slice(&((adler_s2 << 16) | adler_s1).to_be_bytes());

    let mut png = Vec::new();
    png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    write_chunk(&mut png, b"IHDR", &{
        let mut hdr = Vec::new();
        hdr.extend_from_slice(&width.to_be_bytes());
        hdr.extend_from_slice(&height.to_be_bytes());
        hdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        hdr
    });
    write_chunk(&mut png, b"IDAT", &zlib);
    write_chunk(&mut png, b"IEND", &[]);
    std::fs::write(path, png).map_err(|err| err.to_string())
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = 0xffff_ffffu32;
    for byte in kind.iter().chain(data) {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg() & 0xedb8_8320;
            crc = (crc >> 1) ^ mask;
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

/// Digit-row 1 with either Control key toggles wireframe.
/// `pointer_captured` does not block the chord. Look capture still receives it.
fn toggle_wireframe(on: bool, input: &InputSystem, pointer_captured: bool) -> bool {
    let _ = pointer_captured;
    let control = input.keyboard.down(InputCode::key_control_left)
        || input.keyboard.down(InputCode::key_control_right);
    let one = input.keyboard.down(InputCode::key_1);
    let rising = input.keyboard.pressed(InputCode::key_control_left)
        || input.keyboard.pressed(InputCode::key_control_right)
        || input.keyboard.pressed(InputCode::key_1);
    if control && one && rising {
        return !on;
    }
    on
}

/// Control plus 2, 3, or 4 toggles one cascade layer. 2 is near, 3 is far, 4 is world.
fn toggle_cascade_view(mut show: [bool; 3], input: &InputSystem) -> [bool; 3] {
    let control = input.keyboard.down(InputCode::key_control_left)
        || input.keyboard.down(InputCode::key_control_right);
    if !control {
        return show;
    }
    let digits = [InputCode::key_2, InputCode::key_3, InputCode::key_4];
    let control_rising = input.keyboard.pressed(InputCode::key_control_left)
        || input.keyboard.pressed(InputCode::key_control_right);
    for (index, digit) in digits.iter().enumerate() {
        let rising = input.keyboard.pressed(*digit) || control_rising;
        if input.keyboard.down(*digit) && rising {
            show[index] = !show[index];
        }
    }
    show
}

#[cfg(test)]
mod tests {
    #[test]
    fn flicker_counts_a_pixel_that_turns_back_and_not_a_ramp() {
        use super::{Flicker, BENCH_FLICKER_LEAD};
        let (w, h) = (8u32, 8u32);
        let frame = |f: u32| -> Vec<u8> {
            let mut px = vec![0u8; (w * h * 4) as usize];
            for i in 0..(w * h) as usize {
                // Every pixel ramps; pixel 0 of the top-left tile also blinks by 4.
                let mut v = (10 + 3 * f) as u8;
                if i == 0 && f % 2 == 1 {
                    v += 4;
                }
                px[i * 4..i * 4 + 3].copy_from_slice(&[v, v, v]);
            }
            px
        };
        let mut flicker = Flicker { frames: 10, ..Flicker::default() };
        for f in 0..BENCH_FLICKER_LEAD + 10 {
            flicker.take(&frame(f), w, h);
        }
        let n = f64::from(flicker.measured);
        // The blink turns back by 1 (3 - 4 then 3 + 4) on every frame, one pixel of four.
        assert!((flicker.tiles[0] / n - 1.0 / 4.0).abs() < 1.0e-3, "{}", flicker.tiles[0] / n);
        assert!(flicker.tiles[1..].iter().all(|&t| t == 0.0));
    }
    use genos_input::InputSystem;

    use std::collections::VecDeque;
    use std::time::Duration;

    use genos_ui::{FrameSample, OpenFrame, ProfileGraph, STAGE_COUNT};

    use super::{
        hold_ready_frames, parse_profile_mode, toggle_cascade_view, toggle_wireframe,
        PendingProfile, ProfileMode,
    };

    const EVDEV_DIGIT_1: usize = 2;
    const EVDEV_CONTROL_LEFT: usize = 29;
    const EVDEV_CONTROL_RIGHT: usize = 97;

    /// Same key step as the camera frame: begin the frame, apply evdev keys, then the chord.
    fn frame_wireframe(
        on: bool,
        input: &mut InputSystem,
        keys_down: &[u8; 256],
        captured: bool,
    ) -> bool {
        input.begin_frame();
        input.apply_evdev_keys(keys_down);
        toggle_wireframe(on, input, captured)
    }

    #[test]
    fn pause_holds_the_frame_whose_gpu_time_just_arrived() {
        let mut graph = ProfileGraph::default();
        let mut pending = VecDeque::new();
        let mut cpu = [Duration::from_millis(1); STAGE_COUNT];
        cpu[5] = Duration::from_millis(15);
        pending.push_back(PendingProfile {
            open: OpenFrame {
                time: Duration::from_secs(2),
                duration: Duration::from_millis(20),
                cpu,
            },
            record: true,
        });
        hold_ready_frames(
            &mut graph,
            &mut pending,
            vec![Duration::from_millis(9)],
            None,
        )
        .expect("hold");
        graph.remember(FrameSample::at(
            Duration::from_secs(3),
            Duration::from_millis(8),
            [Duration::from_millis(1); STAGE_COUNT],
            Duration::from_millis(1),
        ));
        assert!(pending.is_empty());
        assert_eq!(graph.visible().len(), 1);
        assert_eq!(graph.visible()[0].time, Duration::from_secs(2));
        assert_eq!(graph.visible()[0].stages[5].cpu, Duration::from_millis(15));
        assert_eq!(graph.visible()[0].stages[5].gpu, Duration::from_millis(9));
        assert!(graph.is_paused());
    }

    #[test]
    fn the_user_profile_defaults_to_basic() {
        assert_eq!(
            parse_profile_mode(None, false).expect("default"),
            ProfileMode::Basic
        );
        assert_eq!(
            parse_profile_mode(Some("off"), false).expect("off"),
            ProfileMode::Off
        );
        assert_eq!(
            parse_profile_mode(Some("detailed"), false).expect("detailed"),
            ProfileMode::Detailed
        );
        assert_eq!(
            parse_profile_mode(None, true).expect("proof"),
            ProfileMode::Detailed
        );
        assert_eq!(
            parse_profile_mode(Some("basic"), true).expect("proof basic"),
            ProfileMode::Basic
        );
        assert!(parse_profile_mode(Some("nope"), false).is_err());
    }

    #[test]
    fn control_and_1_toggles_wireframe_while_look_is_captured() {
        let mut input = InputSystem::new();
        let mut keys = [0u8; 256];
        let mut on = false;

        keys[EVDEV_DIGIT_1] = 1;
        on = frame_wireframe(on, &mut input, &keys, false);
        assert!(!on, "digit 1 alone turned wireframe on");

        keys[EVDEV_DIGIT_1] = 0;
        on = frame_wireframe(on, &mut input, &keys, false);
        assert!(!on, "releasing digit 1 changed wireframe");

        keys[EVDEV_CONTROL_LEFT] = 1;
        on = frame_wireframe(on, &mut input, &keys, false);
        assert!(!on, "control alone turned wireframe on");

        keys[EVDEV_DIGIT_1] = 1;
        on = frame_wireframe(on, &mut input, &keys, false);
        assert!(on, "left control plus digit 1 did not turn wireframe on");

        on = frame_wireframe(on, &mut input, &keys, false);
        assert!(on, "holding the chord toggled wireframe again");

        keys = [0u8; 256];
        on = frame_wireframe(on, &mut input, &keys, true);
        assert!(
            on,
            "releasing the chord while captured turned wireframe off"
        );

        keys[EVDEV_CONTROL_RIGHT] = 1;
        keys[EVDEV_DIGIT_1] = 1;
        on = frame_wireframe(on, &mut input, &keys, true);
        assert!(
            !on,
            "right control plus digit 1 did not turn wireframe off while captured"
        );

        keys = [0u8; 256];
        on = frame_wireframe(on, &mut input, &keys, true);
        keys[EVDEV_CONTROL_RIGHT] = 1;
        keys[EVDEV_DIGIT_1] = 1;
        on = frame_wireframe(on, &mut input, &keys, true);
        assert!(
            on,
            "the chord did not turn wireframe on again while captured"
        );
    }

    #[test]
    fn control_and_2_toggles_the_near_cascade_only() {
        let mut input = InputSystem::new();
        let mut keys = [0u8; 256];
        let mut show = [false; 3];
        input.begin_frame();
        input.apply_evdev_keys(&keys);
        keys[3] = 1;
        input.begin_frame();
        input.apply_evdev_keys(&keys);
        show = toggle_cascade_view(show, &input);
        assert_eq!(show, [false, false, false]);
        keys[EVDEV_CONTROL_LEFT] = 1;
        input.begin_frame();
        input.apply_evdev_keys(&keys);
        show = toggle_cascade_view(show, &input);
        assert_eq!(show, [true, false, false]);
        input.begin_frame();
        input.apply_evdev_keys(&keys);
        show = toggle_cascade_view(show, &input);
        assert_eq!(show, [true, false, false]);
    }
}
