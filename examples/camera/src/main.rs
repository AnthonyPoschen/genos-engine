use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use genos_input::{character_controller, InputCode, InputSystem};
use genos_render::{Antialias, Renderer, ScreenRect, Simulation, World};

/// The flame, the smoke, and the flame lamp stay off while the radiance field is under study.
const SHOW_PARTICLES: bool = false;
use genos_scene::{load_path, update, Actions, Camera};
use genos_ui::{
    apply_frame_action, lighting_frame, OpenFrame, Pointer, ProfileGraph, ProfileStream, State,
};
use genos_window::{extent_changed, FocusGate, Window};

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
            other => return Err(format!("unknown argument {other}")),
        }
    }

    let scene_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scene.rhai");
    let scene = load_path(&scene_path)?;
    let mut camera = match eye {
        Some((x, z, yaw)) => Camera::new(x, z, yaw),
        None => Camera::opening(),
    };
    camera.pitch = pitch;
    camera.attach_scene(&scene);
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
        Window::open_proof(1280, 720)?
    } else {
        Window::open(1280, 720)?
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
    let mut frame_clock: Option<Instant> = None;
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
        let draw_camera = host.with_frame(|scene, camera| {
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
        if draw_camera.captured != frame.pointer_locked {
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
        // gpu draw/present
        let want_read = readback.is_some();
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
        if want_read {
            if let Some(pixels) = pixels {
                write_png(
                    readback.as_ref().unwrap(),
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
    println!("genos-camera frames={drawn}");
    Ok(())
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

#[cfg(test)]
mod tests {
    use genos_input::InputSystem;

    use std::collections::VecDeque;
    use std::time::Duration;

    use genos_ui::{FrameSample, OpenFrame, ProfileGraph, STAGE_COUNT};

    use super::{
        hold_ready_frames, parse_profile_mode, toggle_wireframe, PendingProfile, ProfileMode,
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
}
