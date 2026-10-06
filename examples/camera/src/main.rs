use std::path::PathBuf;

use genos_input::{character_controller, InputCode, InputSystem};
use genos_render::{Renderer, ScreenRect, World};
use genos_scene::{load_path, update, Actions, Camera};
use genos_ui::{apply_lamp, lighting_frame, Pointer, State};
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
            other => return Err(format!("unknown argument {other}")),
        }
    }

    let scene_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scene.rhai");
    let scene = load_path(&scene_path)?;
    let mut camera = Camera::opening();
    camera.attach_scene(&scene);
    let mut world = World::from_scene(scene);
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
    loop {
        let frame = window.pump();
        if frame.closing {
            break;
        }
        if extent_changed(size.0, size.1, frame.width, frame.height) || frame.resized {
            renderer.resize(frame.width, frame.height)?;
            size = (renderer.width(), renderer.height());
        }
        let gated = focus.gate(&frame);
        input.begin_frame();
        input.apply_evdev_keys(&gated.keys_down);
        input.set_mouse_button(InputCode::mouse_left, gated.mouse_left);
        input.add_mouse_delta(gated.mouse_dx, gated.mouse_dy);
        input.poll_gamepads();
        let move_axis = controls.axis_2d(&input, "move");
        let look_axis = controls.axis_2d(&input, "look");
        let lamp = world
            .scene
            .lights
            .first()
            .map(|light| [light.position.x, light.position.y, light.position.z]);
        let ui_frame = lighting_frame(
            &mut ui,
            [size.0 as f32, size.1 as f32],
            &camera,
            lamp,
            Pointer {
                x: frame.pointer_x,
                y: frame.pointer_y,
                // A click can press and release before the next pump. The latch still hits the UI.
                down: !camera.captured && (gated.mouse_left || gated.capture_click),
            },
        );
        for action in &ui_frame.actions {
            apply_lamp(&mut world.scene, *action);
        }
        update(
            &mut camera,
            &mut world.scene,
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
        if camera.captured != frame.pointer_locked {
            match window.set_pointer_capture(camera.captured) {
                Ok(()) => {}
                Err(err) => {
                    eprintln!("genos-camera: {err}");
                    camera.captured = false;
                }
            }
        }
        let want_read = readback.is_some();
        let overlay: Vec<ScreenRect> = ui_frame
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
        let pixels = renderer.draw_with_overlay(&world, &camera, &overlay, want_read)?;
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
                camera.captured as u8,
                frame.pointer_locked as u8,
                camera.yaw,
                camera.pitch,
                camera.position.x,
                camera.position.y,
                camera.position.z,
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
    println!("genos-camera frames={drawn}");
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
