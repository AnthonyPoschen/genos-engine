use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use genos_input::{character_controller, InputCode, InputSystem};
use genos_render::{Renderer, ScreenRect, Simulation, World, STEP_DT};
use genos_scene::{load_path, update, Actions, Camera};
use genos_ui::{
    apply_lamp, lighting_frame, profile_overlay, profiler_enabled, FrameSample, OpenFrame, Pointer,
    ProfileStream, State,
};
use genos_window::{extent_changed, FocusGate, Window};

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
            }];
        });
    }
    let server = genos_mcp::Server::start(host.clone())?;
    eprintln!("genos-camera mcp {}", server.url());
    let mut world = World::from_scene(host.drawn_scene());
    let mut scene_revision = host.scene_revision();
    let mut fire = Simulation::from_scene(&world.scene);
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
    let profile = profiler_enabled(proof, frames);
    let mut profile_stream = if profile {
        let path = std::env::current_dir()
            .map_err(|err| err.to_string())?
            .join("genos-camera.profile");
        let stream = ProfileStream::create(path)?;
        eprintln!("genos-camera profile {}", stream.path().display());
        Some(stream)
    } else {
        None
    };
    let mut profile_history = Vec::new();
    let mut profile_pending: VecDeque<OpenFrame> = VecDeque::new();
    loop {
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
        let pump_cpu = stage_cpu(profile, pump_at);
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
        let input_cpu = stage_cpu(profile, input_at);
        if profile {
            publish_profile(
                &mut renderer,
                &mut profile_pending,
                &mut profile_history,
                profile_stream.as_mut().ok_or("profile stream is missing")?,
                false,
            )?;
        }
        // camera/scene update, first half: the panel reads the lamp after a reload
        let scene_at = Instant::now();
        if host.scene_revision() != scene_revision {
            scene_revision = host.scene_revision();
            world = World::from_scene(host.drawn_scene());
        }
        let view = host.camera();
        let lamp = world
            .scene
            .lights
            .first()
            .map(|light| [light.position.x, light.position.y, light.position.z]);
        let mut scene_cpu = stage_cpu(profile, scene_at);
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
        if profile && readback.is_none() {
            let graph = profile_overlay(&profile_history, [size.0 as f32, size.1 as f32]);
            overlay.extend(graph.paints.iter().map(|paint| ScreenRect {
                x: paint.x,
                y: paint.y,
                w: paint.w,
                h: paint.h,
                color: paint.color,
            }));
        }
        let ui_cpu = stage_cpu(profile, ui_at);
        // camera/scene update
        let scene_at = Instant::now();
        let draw_camera = host.with_frame(|scene, camera| {
            for action in &ui_frame.actions {
                apply_lamp(scene, *action);
            }
            update(
                camera,
                scene,
                &Actions {
                    forward: move_axis.y,
                    strafe: move_axis.x,
                    mouse_dx: input.mouse.dx,
                    mouse_dy: input.mouse.dy,
                    look_x: look_axis.x,
                    look_y: look_axis.y,
                    capture_click: ui_frame.look_capture
                        && (gated.capture_click || controls.down(&input, "capture")),
                    escape: controls.down(&input, "release"),
                },
                1.0 / 60.0,
            );
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
        scene_cpu += stage_cpu(profile, scene_at);
        // simulation step
        let sim_at = Instant::now();
        fire.advance(&mut world, STEP_DT);
        let sim_cpu = stage_cpu(profile, sim_at);
        // gpu draw/present
        let want_read = readback.is_some();
        let (pixels, draw_cpu, draw_gpu) = if profile {
            let (pixels, timing) =
                renderer.draw_profiled(&world, &draw_camera, &overlay, want_read)?;
            (pixels, timing.cpu, timing.gpu)
        } else {
            let pixels = renderer.draw_with_overlay(&world, &draw_camera, &overlay, want_read)?;
            (pixels, Duration::ZERO, None)
        };
        if profile {
            let duration = pump_cpu + input_cpu + ui_cpu + scene_cpu + sim_cpu + draw_cpu;
            let open = OpenFrame {
                duration,
                cpu: [pump_cpu, input_cpu, ui_cpu, scene_cpu, sim_cpu, draw_cpu],
            };
            let stream = profile_stream.as_mut().ok_or("profile stream is missing")?;
            if let Some(gpu) = draw_gpu {
                publish_profile(
                    &mut renderer,
                    &mut profile_pending,
                    &mut profile_history,
                    stream,
                    false,
                )?;
                if !profile_pending.is_empty() {
                    return Err("profile gpu time arrived out of order".into());
                }
                let sample = open.finish(gpu);
                stream.append(&sample)?;
                profile_history.push(sample);
            } else {
                profile_pending.push_back(open);
                publish_profile(
                    &mut renderer,
                    &mut profile_pending,
                    &mut profile_history,
                    stream,
                    false,
                )?;
            }
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
    if profile {
        publish_profile(
            &mut renderer,
            &mut profile_pending,
            &mut profile_history,
            profile_stream.as_mut().ok_or("profile stream is missing")?,
            true,
        )?;
        if !profile_pending.is_empty() {
            return Err("profile frames are missing gpu time".into());
        }
    }
    println!("genos-camera frames={drawn}");
    Ok(())
}

fn stage_cpu(enabled: bool, start: Instant) -> Duration {
    if enabled {
        start.elapsed()
    } else {
        Duration::ZERO
    }
}

fn publish_profile(
    renderer: &mut Renderer,
    pending: &mut VecDeque<OpenFrame>,
    history: &mut Vec<FrameSample>,
    stream: &mut ProfileStream,
    wait: bool,
) -> Result<(), String> {
    let times = if wait {
        renderer.finish_gpu_times()?
    } else {
        renderer.poll_gpu_times()?
    };
    for gpu in times {
        let open = pending
            .pop_front()
            .ok_or("profile gpu time arrived without a frame")?;
        let sample = open.finish(gpu);
        stream.append(&sample)?;
        history.push(sample);
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
