//! Lighting debug binary. Scenes come from `debug/scenes/`. Arguments choose the
//! scene, the checkpoint, the lighting preset, and where pictures land.
//! `./genos-debug/<kind>/` is the default output. `--out` overrides it.
//! An environment variable applies only when the matching argument was omitted.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use genos_debug::{png, CheckpointPose, Host, Knob};
use genos_input::{character_controller, InputCode, InputSystem};
use genos_render::{identity_pose, DrawKind, FixedPart, Renderer, ScreenRect, ViewMode, World};
use genos_scene::{
    look_direction, parse_enabled, parse_rate, update, Actions, Camera, DebugSettings, Floor,
    FpsCounterSettings, Light, Scene, Sky, Vec3, PITCH_LIMIT,
};
use genos_scene_lib::{Entry, Kind, Lamp, Sun};
use genos_stress::building::{Layout, LightMix};
use genos_stress::knobs::{apply_lighting_knob, lighting_knobs};
use genos_stress::stage::{Scale, Stage};
use genos_ui::{
    button_panel, Action, FpsOverlay, PanelCheck, PanelDropdown, PanelRow, PanelSlider, Pointer,
    SliderStep, State,
};
use genos_window::{extent_changed, FocusGate, Window};

const USAGE: &str = "\
genos-lighting — lighting scenes, checkpoints, and the debug picture

  --scene NAME           scene file name (debug/scenes). Falls back to GENOS_SCENE,
                         then stress
  --checkpoint NAME      jump here at launch
  --preset NAME          potato, low, medium, or high. Sets GENOS_LIGHTING_PRESET
  --gpu-times N          print GPU_MS every N frames (GENOS_GPU_TIMES)
  --lighting-debug       print the lighting probe line (GENOS_LIGHTING_DEBUG)
  --unfilled             show the texels view (slots with fewer than 4 sets are dim)
  --full-effort          preset high, unless --preset is set, and compose every frame
                         (GENOS_LIGHTING_COMPOSE_ALL)
  --headless             proof window, no panel
  --frames N             quit after N frames
  --fps-counter [on|off] frame-rate plate (default off). A bare flag turns it on.
                         GENOS_FPS_COUNTER applies only when this is omitted
  --fps-hz N             plate update rate in Hz (default 4, range 0.25 to 30).
                         GENOS_FPS_HZ applies only when this is omitted
  --out DIR              screenshots, dumps, and logs (default ./genos-debug)
  --size WxH             window size (default 1280x720; 320x180 for --check)
  --check                every scene at its check checkpoint, four frames,
                         mean luma against that scene's own floor

Page Up and Page Down switch scenes. Keys 1-9 jump checkpoints. Home and 0 reset
to the active checkpoint. Arrow keys and the wheel step the slider under the
pointer, or move the highlight in an open dropdown. Enter picks the highlighted
choice. Escape closes the list. F3 toggles the frame-rate plate. WASD fly, Space
up, C or Ctrl down, Shift is faster. The plate shows FPS and frame ms, averaged
over each 1/rate window (default 4 Hz). It is off until the checkbox, F3, MCP,
or --fps-counter turns it on.

Screenshots go to <out>/shots, the log to <out>/logs, lighting dumps to <out>/dumps.
The binary does not write under $HOME.
";

const FLY_SPEED: f32 = 5.0;
const STICK_LOOK: f32 = 2.0;
const MAX_FRAME_SECONDS: f32 = 0.25;
const STEP: f32 = 1.0 / 60.0;
const ID_SCENE: u32 = 2000;
const ID_CP: u32 = 2200;
const ID_VIEW: u32 = 2400;
const ID_DROP: u32 = 2600;
const ID_FPS: u32 = 2800;
const ID_FPS_HZ: u32 = 2801;
const ID_KNOB: u32 = 3000;
const ID_SWITCH: u32 = 4000;
const CHECKS_PER_ROW: usize = 3;
/// Ordered grades and numeric knobs. An on/off switch is not in this list.
const SLIDER_KNOBS: &[&str] = &[
    "lighting_preset",
    "lighting_bounces",
    "lighting_direct_lights",
    "lighting_direct_refresh",
    "lighting_direct_scale",
    "lighting_lamp_pick_samples",
    "lighting_lamp_pick_candidates",
    "lighting_lamp_pick_radius",
    "lighting_lamp_pick_history",
    "lighting_gi_refresh",
    "lighting_budget_shift",
    "lighting_probe_rays",
    "lighting_probe_stale",
    "lighting_world_stale",
    "lighting_denoise_iters",
    "lighting_denoise_variance",
    "lighting_screen_verify",
    "lighting_world_verify",
];
/// Unordered choices. A long list of scenes uses the same dropdown.
const DROP_KNOBS: &[&str] = &[
    "lighting_shadow_filter",
    "lighting_ambient",
    "lighting_lamp_pick",
];
const VIEWS: [&str; 7] = [
    "full", "direct", "bounce", "albedo", "normal", "depth", "texels",
];
const PRESETS: [&str; 4] = ["potato", "low", "medium", "high"];

struct Options {
    scene: String,
    checkpoint: Option<String>,
    preset: Option<String>,
    gpu_times: Option<u32>,
    lighting_debug: bool,
    unfilled: bool,
    full_effort: bool,
    headless: bool,
    frames: Option<u32>,
    out: PathBuf,
    out_set: bool,
    size: (u32, u32),
    check: bool,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("genos-lighting: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let opts = options()?;
    apply_intent(&opts)?;
    std::fs::create_dir_all(opts.out.join("shots")).map_err(|err| err.to_string())?;
    std::fs::create_dir_all(opts.out.join("logs")).map_err(|err| err.to_string())?;
    std::fs::create_dir_all(opts.out.join("dumps")).map_err(|err| err.to_string())?;
    if std::env::var_os("GENOS_LIGHTING_DUMP").is_some() {
        let dump_set = std::env::var_os("GENOS_LIGHTING_DUMP_DIR").is_some();
        if opts.out_set || !dump_set {
            std::env::set_var("GENOS_LIGHTING_DUMP_DIR", opts.out.join("dumps"));
        }
    }
    let mut log = std::fs::File::create(opts.out.join("logs").join("run.log"))
        .map_err(|err| err.to_string())?;
    let root = genos_scene_lib::repo_root();
    let entries = genos_scene_lib::load_dir(&genos_scene_lib::scenes_dir())?;
    if entries.is_empty() {
        return Err("no scenes in debug/scenes".into());
    }
    let mut app = App::open(
        entries,
        root,
        log.try_clone().map_err(|err| err.to_string())?,
    )?;
    if opts.check {
        return check_all(&mut app, &opts, &mut log);
    }
    let index = app
        .entries
        .iter()
        .position(|e| e.name == opts.scene)
        .ok_or_else(|| format!("unknown scene {}", opts.scene))?;
    app.load_scene(&app.entries[index].name.clone())?;
    let mut camera = Camera::opening();
    jump_camera(&mut app, opts.checkpoint.as_deref(), &mut camera)?;
    let (width, height) = opts.size;
    let mut window = if opts.headless {
        Window::open_proof(width, height)?
    } else {
        Window::open_named(width, height, "genos-lighting", "Genos lighting")?
    };
    let first = window.pump();
    let mut renderer = Renderer::open(window.display, window.surface, first.width, first.height)?;
    let mcp_host = genos_mcp::Host::new(app.scene().clone(), camera.clone());
    let _server = genos_mcp::Server::start_on(mcp_host, genos_mcp::listen_port())?;
    let _ = writeln!(log, "mcp http://127.0.0.1:{}/", genos_mcp::listen_port());
    let mut tools = genos_debug::Tools::new();
    tools.set_out_dir(opts.out.join("shots"));
    if opts.unfilled {
        tools.set_view(ViewMode::Texels);
    }
    if opts.frames.is_some() {
        tools.set_fixed_step(Some(STEP));
    }
    let mut size = (renderer.width(), renderer.height());
    let mut focus = FocusGate::default();
    let mut ui = State::default();
    let mut input = InputSystem::new();
    let controls = character_controller();
    let mut drawn = 0u32;
    let mut clock: Option<Instant> = None;
    let mut fps_overlay = FpsOverlay::new();
    loop {
        let now = Instant::now();
        let real_dt = clock
            .map(|then| now.saturating_duration_since(then).as_secs_f32())
            .unwrap_or(STEP);
        let dt = tools.scene_dt(if opts.frames.is_some() {
            STEP
        } else {
            real_dt.min(MAX_FRAME_SECONDS)
        });
        clock = Some(now);
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
        hotkeys(&mut app, &input, &mut camera, &mut tools, &mut renderer)?;
        let move_axis = controls.axis_2d(&input, "move");
        let look_axis = controls.axis_2d(&input, "look");
        let (paints, look_capture) = if opts.headless {
            (Vec::new(), true)
        } else {
            let fps_menu = app.scene().fps_counter.effective();
            let rows = panel_rows(&app, tools.view_mode(), &fps_menu);
            let ui_frame = button_panel(
                &mut ui,
                &panel_title(&app),
                &rows,
                [size.0 as f32, size.1 as f32],
                &camera,
                Pointer {
                    x: frame.pointer_x,
                    y: frame.pointer_y,
                    down: !camera.captured && (gated.mouse_left || gated.capture_click),
                    wheel_y: if camera.captured { 0.0 } else { gated.wheel_y },
                    key_dec: !camera.captured
                        && (input.keyboard.pressed(InputCode::key_left)
                            || input.keyboard.pressed(InputCode::key_down)),
                    key_inc: !camera.captured
                        && (input.keyboard.pressed(InputCode::key_right)
                            || input.keyboard.pressed(InputCode::key_up)),
                    key_activate: !camera.captured && input.keyboard.pressed(InputCode::key_enter),
                    key_dismiss: !camera.captured && input.keyboard.pressed(InputCode::key_escape),
                },
            );
            for action in &ui_frame.actions {
                match *action {
                    Action::Slide { id, value } => slide(&mut app, id, value)?,
                    Action::Select { id, value } => {
                        select(&mut app, &mut camera, &mut tools, &mut renderer, id, value)?;
                    }
                    Action::Toggle { id, on } => toggle(&mut app, id, on)?,
                    _ => {}
                }
            }
            (ui_frame.paints, ui_frame.look_capture)
        };
        let mut overlay: Vec<ScreenRect> = paints
            .iter()
            .map(|p| ScreenRect {
                x: p.x,
                y: p.y,
                w: p.w,
                h: p.h,
                color: p.color,
            })
            .collect();
        app.advance(dt);
        fly(
            &mut camera,
            app.scene(),
            &input,
            &Actions {
                forward: move_axis.y,
                strafe: move_axis.x,
                mouse_dx: input.mouse.dx,
                mouse_dy: input.mouse.dy,
                look_x: look_axis.x,
                look_y: look_axis.y,
                capture_click: look_capture
                    && (gated.capture_click || controls.down(&input, "capture")),
                escape: controls.down(&input, "release"),
            },
            dt,
        );
        if camera.captured != frame.pointer_locked {
            if let Err(err) = window.set_pointer_capture(camera.captured) {
                eprintln!("genos-lighting: {err}");
                camera.captured = false;
            }
        }
        let tools_read = tools.before_draw(&mut genos_debug::Ctx {
            renderer: &mut renderer,
            camera: &mut camera,
            host: &mut app,
        });
        overlay.extend_from_slice(tools.overlay());
        let fps_now = app.scene().fps_counter.effective();
        fps_overlay.note(
            fps_now.enabled,
            fps_now.rate_hz,
            real_dt,
            [size.0 as f32, size.1 as f32],
        );
        if fps_now.enabled {
            overlay.extend(fps_overlay.paints().iter().map(|paint| ScreenRect {
                x: paint.x,
                y: paint.y,
                w: paint.w,
                h: paint.h,
                color: paint.color,
            }));
        }
        drawn += 1;
        let last = opts.frames.is_some_and(|limit| drawn >= limit);
        let want_read = tools_read || last;
        let draw_start = Instant::now();
        let pixels =
            renderer.draw_with_overlay(app.world(), &camera, &overlay, want_read, false)?;
        tools.after_draw(
            &mut renderer,
            pixels.as_deref(),
            genos_debug::FrameTiming {
                frame_ms: real_dt as f64 * 1000.0,
                draw_ms: draw_start.elapsed().as_secs_f64() * 1000.0,
                gpu_ms: None,
            },
        );
        if last {
            if let Some(pixels) = pixels {
                let path = opts
                    .out
                    .join("shots")
                    .join(format!("{}.png", app.entry().name));
                png::write_png(&path, renderer.width(), renderer.height(), &pixels)?;
                let (mean, spread) = picture_stats(&pixels);
                let _ = writeln!(
                    log,
                    "shot {} mean {mean:.1} spread {spread:.1}",
                    path.display()
                );
            }
        }
        if last || tools.quit_code().is_some() {
            break;
        }
    }
    if let Some(code) = tools.quit_code().filter(|c| *c != 0) {
        std::process::exit(code);
    }
    Ok(())
}

fn check_all(app: &mut App, opts: &Options, log: &mut std::fs::File) -> Result<(), String> {
    let mut window = Window::open_proof(opts.size.0, opts.size.1)?;
    let first = window.pump();
    let mut renderer = Renderer::open(window.display, window.surface, first.width, first.height)?;
    let mut camera = Camera::opening();
    let mut passed = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();
    let names: Vec<String> = app.entries.iter().map(|e| e.name.clone()).collect();
    for name in &names {
        let _ = writeln!(log, "check {name}");
        eprintln!("genos-lighting: check {name}");
        if let Err(err) = app.load_scene(name) {
            if gltf_scene(&app.entries, name) && gltf_bytes_absent(&err) {
                let line = skip_line(name, &err);
                skipped.push(name.clone());
                eprintln!("genos-lighting: {line}");
                let _ = writeln!(log, "{line}");
                continue;
            }
            return Err(err);
        }
        let check = app.entry().check.clone();
        jump_camera(app, Some(&check.checkpoint), &mut camera)?;
        renderer.reset_lighting_cache();
        let mut pixels = None;
        for frame in 0..4 {
            let _ = window.pump();
            app.advance(STEP);
            let read = frame == 3;
            pixels = renderer.draw(app.world(), &camera, read)?;
        }
        let pixels = pixels.ok_or_else(|| format!("{name}: no picture"))?;
        let (mean, spread) = picture_stats(&pixels);
        let path = opts.out.join("shots").join(format!("{name}.png"));
        png::write_png(&path, renderer.width(), renderer.height(), &pixels)?;
        let verdict = if mean >= check.min_mean {
            passed.push(name.clone());
            "pass"
        } else {
            failed.push(format!(
                "{name} at {} mean {mean:.1} is under {:.1}",
                check.checkpoint, check.min_mean
            ));
            "fail"
        };
        let line = format!(
            "check {name} {verdict} {} mean {mean:.1} spread {spread:.1} min {:.1} {}",
            check.checkpoint,
            check.min_mean,
            path.display()
        );
        eprintln!("genos-lighting: {line}");
        let _ = writeln!(log, "{line}");
    }
    let summary = check_summary(&passed, &skipped, &failed);
    let _ = writeln!(log, "{summary}");
    if failed.is_empty() {
        eprintln!("genos-lighting: {summary}");
        Ok(())
    } else {
        Err(summary)
    }
}

/// A missing glTF is a skip. The tally does not count it as a pass.
fn skip_line(name: &str, err: &str) -> String {
    format!("skip {name} (not a pass): {err}")
}

fn check_summary(passed: &[String], skipped: &[String], failed: &[String]) -> String {
    let mut line = format!(
        "{} passed, {} skipped, {} failed",
        passed.len(),
        skipped.len(),
        failed.len()
    );
    if !skipped.is_empty() {
        line.push_str(" (skipped ");
        line.push_str(&skipped.join(", "));
        line.push(')');
    }
    if !failed.is_empty() {
        line.push_str(": ");
        line.push_str(&failed.join("; "));
    }
    line
}

struct App {
    entries: Vec<Entry>,
    index: usize,
    checkpoint: usize,
    show: Show,
    root: PathBuf,
    log: std::fs::File,
}

enum Show {
    Stress(Stage),
    Still(World),
}

impl App {
    fn open(entries: Vec<Entry>, root: PathBuf, log: std::fs::File) -> Result<Self, String> {
        let show = build_entry(&root, &entries[0])?;
        Ok(Self {
            entries,
            index: 0,
            checkpoint: 0,
            show,
            root,
            log,
        })
    }

    fn entry(&self) -> &Entry {
        &self.entries[self.index]
    }

    fn world(&self) -> &World {
        match &self.show {
            Show::Stress(stage) => &stage.world,
            Show::Still(world) => world,
        }
    }

    fn advance(&mut self, dt: f32) {
        if let Show::Stress(stage) = &mut self.show {
            stage.advance(dt);
        }
    }

    fn note(&mut self, line: &str) {
        eprintln!("genos-lighting: {line}");
        let _ = writeln!(self.log, "{line}");
    }
}

impl Host for App {
    fn scene(&mut self) -> &mut Scene {
        match &mut self.show {
            Show::Stress(stage) => &mut stage.world.scene,
            Show::Still(world) => &mut world.scene,
        }
    }

    fn knobs(&self) -> Vec<Knob> {
        match &self.show {
            Show::Stress(stage) => Host::knobs(stage),
            Show::Still(world) => lighting_knobs(&world.scene.lighting),
        }
    }

    fn set_knob(&mut self, name: &str, value: f64) -> Result<(), String> {
        match &mut self.show {
            Show::Stress(stage) => Host::set_knob(stage, name, value),
            Show::Still(world) => {
                if apply_lighting_knob(&mut world.scene.lighting, name, value)? {
                    Ok(())
                } else {
                    Err(format!("unknown knob {name}"))
                }
            }
        }
    }

    fn list_scenes(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.name.clone()).collect()
    }

    fn load_scene(&mut self, name: &str) -> Result<(), String> {
        let index = self
            .entries
            .iter()
            .position(|e| e.name == name)
            .ok_or_else(|| format!("unknown scene {name}"))?;
        let show = build_entry(&self.root, &self.entries[index])?;
        self.index = index;
        self.checkpoint = 0;
        self.show = show;
        self.note(&format!("loaded {name}"));
        Ok(())
    }

    fn list_checkpoints(&self) -> Vec<String> {
        self.entry()
            .checkpoints
            .iter()
            .map(|c| c.name.clone())
            .collect()
    }

    fn jump_checkpoint(&mut self, name: &str) -> Result<CheckpointPose, String> {
        let index = self
            .entry()
            .checkpoints
            .iter()
            .position(|c| c.name == name)
            .ok_or_else(|| format!("unknown checkpoint {name}"))?;
        self.checkpoint = index;
        let cp = &self.entry().checkpoints[index];
        Ok(CheckpointPose {
            position: cp.eye,
            yaw: cp.yaw,
            pitch: cp.pitch,
        })
    }
}

fn gltf_scene(entries: &[Entry], name: &str) -> bool {
    entries
        .iter()
        .any(|entry| entry.name == name && matches!(entry.kind, Kind::Gltf { .. }))
}

/// A glTF scene whose bytes are not in this checkout. `--check` skips it.
/// `--scene` still returns the error.
fn gltf_bytes_absent(err: &str) -> bool {
    err.contains("Git LFS pointer") || err.contains("file not found")
}

fn build_entry(root: &Path, entry: &Entry) -> Result<Show, String> {
    match &entry.kind {
        Kind::Stress(spec) => {
            let scale = Scale::parse(&spec.scale)?;
            let layout = Layout::parse(&spec.layout)
                .ok_or_else(|| format!("unknown layout {}", spec.layout))?;
            let mut stage = Stage::new(
                scale,
                1,
                LightMix {
                    count: spec.lights,
                    dynamic_pct: spec.dynamic,
                    layout,
                },
                1.0,
                spec.time,
                120.0,
            );
            stage.sun_frozen = spec.freeze_sun;
            stage.boxes_still = spec.still_boxes;
            stage.lamps_still = spec.still_lamps;
            stage.sky_on = spec.sky;
            stage.apply();
            Ok(Show::Stress(stage))
        }
        Kind::Rhai { path } => {
            let full = root.join(path);
            let scene = genos_scene::load_path(&full)?;
            Ok(Show::Still(World::from_scene(scene)))
        }
        Kind::Gltf {
            path,
            sun,
            sky,
            lamps,
        } => {
            let full = root.join(path);
            eprintln!("genos-lighting: loading {}", full.display());
            let model = genos_load::load_gltf_file(&full).map_err(|err| match &err {
                genos_load::LoadError::LfsPointer { .. }
                | genos_load::LoadError::NotFound { .. } => err.to_string(),
                _ => format!("{}: {err}", full.display()),
            })?;
            let mut scene = Scene {
                floor: Floor {
                    position: Vec3::new(1000.0, 0.0, 0.0),
                    half_x: 0.01,
                    half_z: 0.01,
                    color: [0.2, 0.2, 0.2],
                },
                walls: Vec::new(),
                solids: Vec::new(),
                lights: Vec::new(),
                ceiling: None,
                sky: sky.map(|color| Sky { color }),
                lighting: genos_scene::LightingSettings::default(),
                debug: DebugSettings::default(),
                fps_counter: FpsCounterSettings::default(),
            };
            if let Some(sun) = sun {
                scene.lights.push(sun_light(sun));
            }
            for lamp in lamps {
                scene.lights.push(point_light(lamp));
            }
            let mut world = World::from_scene(scene);
            for object in &mut world.objects {
                if let DrawKind::Fixed(FixedPart::Floor) = object.kind {
                    object.hidden = true;
                    object.affects_light = false;
                }
            }
            let mut bake = genos_render::bake::BakeSettings::default();
            if std::env::var_os("GENOS_BAKE").is_none() {
                bake.mode = genos_render::bake::BakeMode::OnLoad;
            }
            world
                .add_gltf_traced(&model, &identity_pose(), &bake)
                .map_err(|err| format!("{}: bake: {err:?}", full.display()))?;
            Ok(Show::Still(world))
        }
    }
}

fn sun_light(sun: &Sun) -> Light {
    let d = unit(sun.direction);
    Light {
        position: Vec3::new(0.0, 7.0, 0.0),
        color: sun.color,
        direction: Vec3::new(d[0], d[1], d[2]),
    }
}

fn point_light(lamp: &Lamp) -> Light {
    Light {
        position: Vec3::new(lamp.position[0], lamp.position[1], lamp.position[2]),
        color: lamp.color,
        direction: Vec3::ZERO,
    }
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1.0e-8 {
        v.map(|c| c / len)
    } else {
        v
    }
}

fn jump_camera(app: &mut App, name: Option<&str>, camera: &mut Camera) -> Result<(), String> {
    let chosen = match name {
        Some(name) => name.to_string(),
        None => app
            .entry()
            .checkpoints
            .first()
            .map(|c| c.name.clone())
            .unwrap_or_default(),
    };
    if chosen.is_empty() {
        return Ok(());
    }
    let pose = app.jump_checkpoint(&chosen)?;
    camera.set_pose(
        Vec3::new(pose.position[0], pose.position[1], pose.position[2]),
        pose.yaw,
        pose.pitch,
    );
    Ok(())
}

fn hotkeys(
    app: &mut App,
    input: &InputSystem,
    camera: &mut Camera,
    tools: &mut genos_debug::Tools,
    renderer: &mut Renderer,
) -> Result<(), String> {
    let n = app.entries.len();
    if input.keyboard.pressed(InputCode::key_page_down) {
        switch_to(app, (app.index + 1) % n, None, camera, tools, renderer)?;
    } else if input.keyboard.pressed(InputCode::key_page_up) {
        let index = if app.index == 0 { n - 1 } else { app.index - 1 };
        switch_to(app, index, None, camera, tools, renderer)?;
    }
    let keys = [
        InputCode::key_1,
        InputCode::key_2,
        InputCode::key_3,
        InputCode::key_4,
        InputCode::key_5,
        InputCode::key_6,
        InputCode::key_7,
        InputCode::key_8,
        InputCode::key_9,
    ];
    for (i, code) in keys.iter().enumerate() {
        if input.keyboard.pressed(*code) {
            if let Some(name) = app.entry().checkpoints.get(i).map(|c| c.name.clone()) {
                let pose = app.jump_checkpoint(&name)?;
                camera.set_pose(
                    Vec3::new(pose.position[0], pose.position[1], pose.position[2]),
                    pose.yaw,
                    pose.pitch,
                );
            }
        }
    }
    if input.keyboard.pressed(InputCode::key_f3) {
        let on = app.scene().fps_counter.effective().enabled;
        let _ = app
            .scene()
            .fps_counter
            .set_key("enabled", if on { "0" } else { "1" });
    }
    if (input.keyboard.pressed(InputCode::key_home) || input.keyboard.pressed(InputCode::key_0))
        && app.checkpoint < app.entry().checkpoints.len()
    {
        let name = app.entry().checkpoints[app.checkpoint].name.clone();
        let pose = app.jump_checkpoint(&name)?;
        camera.set_pose(
            Vec3::new(pose.position[0], pose.position[1], pose.position[2]),
            pose.yaw,
            pose.pitch,
        );
    }
    Ok(())
}

fn switch_to(
    app: &mut App,
    index: usize,
    checkpoint: Option<&str>,
    camera: &mut Camera,
    tools: &mut genos_debug::Tools,
    renderer: &mut Renderer,
) -> Result<(), String> {
    let name = app.entries[index].name.clone();
    app.load_scene(&name)?;
    tools.drop_scene_edits();
    renderer.reset_lighting_cache();
    jump_camera(app, checkpoint, camera)
}

fn fly(camera: &mut Camera, scene: &mut Scene, input: &InputSystem, actions: &Actions, dt: f32) {
    update(camera, scene, actions, 0.0);
    camera.yaw += actions.look_x * STICK_LOOK * dt;
    camera.pitch =
        (camera.pitch + actions.look_y * STICK_LOOK * dt).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    let forward = look_direction(camera.yaw, 0.0);
    let right = look_direction(camera.yaw + std::f32::consts::FRAC_PI_2, 0.0);
    let mut lift = 0.0;
    if input.keyboard.down(InputCode::key_space) {
        lift += 1.0;
    }
    if input.keyboard.down(InputCode::key_c)
        || input.keyboard.down(InputCode::key_control_left)
        || input.keyboard.down(InputCode::key_control_right)
    {
        lift -= 1.0;
    }
    let boost = if input.keyboard.down(InputCode::key_shift_left)
        || input.keyboard.down(InputCode::key_shift_right)
    {
        4.0
    } else {
        1.0
    };
    let step = (forward * actions.forward + right * actions.strafe + Vec3::Y * lift)
        * (FLY_SPEED * boost * dt);
    camera.set_pose(camera.position + step, camera.yaw, camera.pitch);
}

fn view_name(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::Direct => "direct",
        ViewMode::Bounce => "bounce",
        ViewMode::Albedo => "albedo",
        ViewMode::Normal => "normal",
        ViewMode::Depth => "depth",
        ViewMode::Texels => "texels",
        ViewMode::Light => "light",
        ViewMode::Full => "full",
    }
}

fn panel_title(app: &App) -> String {
    let review = app.entry().review.as_str();
    let review = if review.len() > 140 {
        format!("{}…", &review[..140])
    } else {
        review.to_string()
    };
    format!("{} — {review}", app.entry().name)
}

fn index_steps(names: impl IntoIterator<Item = impl Into<String>>) -> Vec<SliderStep> {
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| SliderStep {
            name: name.into(),
            value: index as f64,
        })
        .collect()
}

fn named_slider(id: u32, value: f64, names: Vec<String>) -> PanelSlider {
    let mut slider = PanelSlider::named(id, value, index_steps(names));
    // Tick names carry the step. The side readout would repeat that name.
    slider.text.clear();
    slider
}

fn drop_row(label: &str, id: u32, value: f64, names: Vec<String>) -> Option<PanelRow> {
    if names.is_empty() {
        return None;
    }
    Some(PanelRow {
        label: label.into(),
        buttons: Vec::new(),
        slider: None,
        dropdown: Some(PanelDropdown::new(id, value, index_steps(names))),
        checks: Vec::new(),
    })
}

fn switch_knobs(app: &App) -> Vec<Knob> {
    app.knobs()
        .into_iter()
        .filter(|knob| knob.choices == ["off", "on"])
        .collect()
}

fn switch_rows(app: &App) -> Vec<PanelRow> {
    let switches = switch_knobs(app);
    switches
        .chunks(CHECKS_PER_ROW)
        .enumerate()
        .map(|(row_index, chunk)| PanelRow {
            label: String::new(),
            buttons: Vec::new(),
            slider: None,
            dropdown: None,
            checks: chunk
                .iter()
                .enumerate()
                .map(|(column, knob)| PanelCheck {
                    id: ID_SWITCH + (row_index * CHECKS_PER_ROW + column) as u32,
                    text: knob.label.clone(),
                    on: knob.value >= 0.5,
                })
                .collect(),
        })
        .collect()
}

fn knob_row(app: &App, index: usize, name: &str) -> Option<PanelRow> {
    let knob = app.knobs().into_iter().find(|knob| knob.name == name)?;
    if knob.choices == ["off", "on"] {
        return None;
    }
    let label = if name == "lighting_preset" {
        "Preset".to_string()
    } else {
        knob.label.clone()
    };
    let slider = if knob.choices.is_empty() {
        PanelSlider::range(
            ID_KNOB + index as u32,
            knob.value,
            knob.min,
            knob.max,
            knob.step,
            knob.text,
        )
    } else {
        named_slider(ID_KNOB + index as u32, knob.value, knob.choices)
    };
    Some(PanelRow {
        label,
        buttons: Vec::new(),
        slider: Some(slider),
        dropdown: None,
        checks: Vec::new(),
    })
}

fn panel_rows(app: &App, view: ViewMode, fps: &FpsCounterSettings) -> Vec<PanelRow> {
    let view_value = VIEWS
        .iter()
        .position(|name| *name == view_name(view))
        .unwrap_or(0) as f64;
    let mut rows = Vec::new();
    if let Some(row) = drop_row(
        "Scene",
        ID_SCENE,
        app.index as f64,
        app.entries.iter().map(|entry| entry.name.clone()).collect(),
    ) {
        rows.push(row);
    }
    if let Some(row) = drop_row(
        "Checkpoint",
        ID_CP,
        app.checkpoint as f64,
        app.entry()
            .checkpoints
            .iter()
            .map(|checkpoint| checkpoint.name.clone())
            .collect(),
    ) {
        rows.push(row);
    }
    if let Some(row) = drop_row(
        "View",
        ID_VIEW,
        view_value,
        VIEWS.iter().map(|name| (*name).to_string()).collect(),
    ) {
        rows.push(row);
    }
    rows.extend(switch_rows(app));
    rows.push(fps_row(fps));
    for (index, name) in SLIDER_KNOBS.iter().enumerate() {
        if let Some(row) = knob_row(app, index, name) {
            rows.push(row);
        }
    }
    for (index, name) in DROP_KNOBS.iter().enumerate() {
        if let Some(row) = drop_knob_row(app, index, name) {
            rows.push(row);
        }
    }
    rows
}

fn drop_knob_row(app: &App, index: usize, name: &str) -> Option<PanelRow> {
    let knob = app.knobs().into_iter().find(|knob| knob.name == name)?;
    if knob.choices.is_empty() || knob.choices == ["off", "on"] {
        return None;
    }
    drop_row(
        &knob.label,
        ID_DROP + index as u32,
        knob.value,
        knob.choices,
    )
}

fn fps_row(fps: &FpsCounterSettings) -> PanelRow {
    PanelRow {
        label: "FPS".into(),
        buttons: Vec::new(),
        slider: Some(PanelSlider::range(
            ID_FPS_HZ,
            fps.rate_hz as f64,
            1.0,
            30.0,
            1.0,
            format!("{:.0} Hz", fps.rate_hz),
        )),
        dropdown: None,
        checks: vec![PanelCheck {
            id: ID_FPS,
            text: "Show".into(),
            on: fps.enabled,
        }],
    }
}

fn slide(app: &mut App, id: u32, value: f64) -> Result<(), String> {
    if id == ID_FPS_HZ {
        app.scene()
            .fps_counter
            .set_key("rate_hz", &format!("{value}"))?;
    } else if (ID_KNOB..ID_KNOB + SLIDER_KNOBS.len() as u32).contains(&id) {
        let name = SLIDER_KNOBS[(id - ID_KNOB) as usize];
        app.set_knob(name, value)?;
    }
    Ok(())
}

fn select(
    app: &mut App,
    camera: &mut Camera,
    tools: &mut genos_debug::Tools,
    renderer: &mut Renderer,
    id: u32,
    value: f64,
) -> Result<(), String> {
    let index = value.round().max(0.0) as usize;
    if id == ID_SCENE {
        if index != app.index && index < app.entries.len() {
            switch_to(app, index, None, camera, tools, renderer)?;
        }
    } else if id == ID_CP {
        if index != app.checkpoint {
            if let Some(name) = app.entry().checkpoints.get(index).map(|c| c.name.clone()) {
                let pose = app.jump_checkpoint(&name)?;
                camera.set_pose(
                    Vec3::new(pose.position[0], pose.position[1], pose.position[2]),
                    pose.yaw,
                    pose.pitch,
                );
            }
        }
    } else if id == ID_VIEW {
        if let Some(name) = VIEWS.get(index) {
            let mode = view_from_name(name);
            if tools.view_mode() != mode {
                tools.set_view(mode);
            }
        }
    } else if (ID_DROP..ID_DROP + DROP_KNOBS.len() as u32).contains(&id) {
        let name = DROP_KNOBS[(id - ID_DROP) as usize];
        app.set_knob(name, value)?;
    }
    Ok(())
}

fn toggle(app: &mut App, id: u32, on: bool) -> Result<(), String> {
    if id == ID_FPS {
        app.scene()
            .fps_counter
            .set_key("enabled", if on { "1" } else { "0" })?;
        return Ok(());
    }
    let index = id.wrapping_sub(ID_SWITCH) as usize;
    if let Some(name) = switch_knobs(app).get(index).map(|knob| knob.name.clone()) {
        app.set_knob(&name, if on { 1.0 } else { 0.0 })?;
    }
    Ok(())
}

fn view_from_name(name: &str) -> ViewMode {
    match name {
        "direct" => ViewMode::Direct,
        "bounce" => ViewMode::Bounce,
        "albedo" => ViewMode::Albedo,
        "normal" => ViewMode::Normal,
        "depth" => ViewMode::Depth,
        "texels" => ViewMode::Texels,
        _ => ViewMode::Full,
    }
}

fn picture_stats(bgra: &[u8]) -> (f32, f32) {
    let mut sum = 0.0f64;
    let mut sum2 = 0.0f64;
    let mut n = 0.0f64;
    for px in bgra.chunks_exact(4).step_by(8) {
        let l = (px[0] as f64 + px[1] as f64 + px[2] as f64) / 3.0;
        sum += l;
        sum2 += l * l;
        n += 1.0;
    }
    if n == 0.0 {
        return (0.0, 0.0);
    }
    let mean = sum / n;
    let var = (sum2 / n - mean * mean).max(0.0);
    (mean as f32, var.sqrt() as f32)
}

fn apply_intent(opts: &Options) -> Result<(), String> {
    if let Some(preset) = &opts.preset {
        if !PRESETS.contains(&preset.as_str()) {
            return Err(format!("unknown preset {preset}"));
        }
        std::env::set_var("GENOS_LIGHTING_PRESET", preset);
    } else if opts.full_effort {
        std::env::set_var("GENOS_LIGHTING_PRESET", "high");
    }
    if opts.full_effort {
        std::env::set_var("GENOS_LIGHTING_COMPOSE_ALL", "1");
    }
    if let Some(n) = opts.gpu_times {
        std::env::set_var("GENOS_GPU_TIMES", n.to_string());
    }
    if opts.lighting_debug {
        std::env::set_var("GENOS_LIGHTING_DEBUG", "1");
    }
    Ok(())
}

fn need_arg(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn options() -> Result<Options, String> {
    let mut args = std::env::args().skip(1).peekable();
    let mut scene = std::env::var("GENOS_SCENE").ok();
    let mut checkpoint = None;
    let mut preset = None;
    let mut gpu_times = None;
    let mut lighting_debug = false;
    let mut unfilled = false;
    let mut full_effort = false;
    let mut headless = false;
    let mut frames = None;
    let mut out = PathBuf::from("genos-debug");
    let mut out_set = false;
    let mut size = (1280u32, 720u32);
    let mut check = false;
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!("{USAGE}");
            std::process::exit(0);
        }
        match arg.as_str() {
            "--scene" => scene = Some(need_arg(&mut args, "--scene")?),
            "--checkpoint" => checkpoint = Some(need_arg(&mut args, "--checkpoint")?),
            "--preset" => preset = Some(need_arg(&mut args, "--preset")?),
            "--gpu-times" => {
                gpu_times = Some(
                    need_arg(&mut args, "--gpu-times")?
                        .parse()
                        .map_err(|_| "bad --gpu-times".to_string())?,
                );
            }
            "--lighting-debug" => lighting_debug = true,
            "--unfilled" => unfilled = true,
            "--full-effort" => full_effort = true,
            "--headless" => headless = true,
            "--frames" => {
                frames = Some(
                    need_arg(&mut args, "--frames")?
                        .parse()
                        .map_err(|_| "bad --frames".to_string())?,
                );
            }
            "--out" => {
                out = PathBuf::from(need_arg(&mut args, "--out")?);
                out_set = true;
            }
            "--size" => {
                let text = need_arg(&mut args, "--size")?;
                let (w, h) = text.split_once('x').ok_or("size is WxH")?;
                size = (
                    w.parse().map_err(|_| "bad width".to_string())?,
                    h.parse().map_err(|_| "bad height".to_string())?,
                );
            }
            "--check" => check = true,
            "--fps-counter" => {
                if args.peek().is_some_and(|next| !next.starts_with('-')) {
                    let value = args.next().unwrap();
                    parse_enabled(&value).map_err(|err| format!("bad --fps-counter: {err}"))?;
                }
            }
            "--fps-hz" => {
                let value = need_arg(&mut args, "--fps-hz")?;
                parse_rate(&value).map_err(|err| format!("bad --fps-hz: {err}"))?;
            }
            other => {
                if let Some(value) = other.strip_prefix("--fps-counter=") {
                    parse_enabled(value).map_err(|err| format!("bad --fps-counter: {err}"))?;
                } else if let Some(value) = other.strip_prefix("--fps-hz=") {
                    parse_rate(value).map_err(|err| format!("bad --fps-hz: {err}"))?;
                } else {
                    return Err(format!("unknown argument {other}"));
                }
            }
        }
    }
    if check {
        headless = true;
        size = (320, 180);
    }
    Ok(Options {
        scene: scene.unwrap_or_else(|| "stress".into()),
        checkpoint,
        preset,
        gpu_times,
        lighting_debug,
        unfilled,
        full_effort,
        headless,
        frames,
        out,
        out_set,
        size,
        check,
    })
}

#[cfg(test)]
mod tests {
    use super::{check_summary, gltf_bytes_absent, gltf_scene, skip_line};
    use genos_render::ViewMode;
    use genos_scene_lib::{Check, Entry, Kind};
    use genos_ui::Paint;

    #[test]
    fn a_missing_gltf_is_skipped_and_a_bad_scene_is_not() {
        let entries = vec![Entry {
            name: "sponza".into(),
            review: String::new(),
            kind: Kind::Gltf {
                path: "assets/built/sponza/sponza.gltf".into(),
                sun: None,
                sky: None,
                lamps: Vec::new(),
            },
            checkpoints: Vec::new(),
            check: Check {
                checkpoint: "atrium".into(),
                min_mean: 8.0,
            },
        }];
        assert!(gltf_scene(&entries, "sponza"));
        assert!(!gltf_scene(&entries, "room"));
        let pointer = "assets/built/sponza/sponza.gltf is a Git LFS pointer; the asset bytes are not in this checkout";
        assert!(gltf_bytes_absent(pointer));
        assert!(gltf_bytes_absent(
            "file not found: assets/built/sponza/sponza.gltf"
        ));
        assert!(!gltf_bytes_absent("sponza.gltf: unrecognized asset"));
        let line = skip_line("sponza", pointer);
        assert!(line.contains("not a pass"));
        let summary = check_summary(&["camera".into(), "hell".into()], &["sponza".into()], &[]);
        assert_eq!(summary, "2 passed, 1 skipped, 0 failed (skipped sponza)");
        assert!(!summary.contains("sponza passed"));
    }

    #[test]
    fn each_setting_uses_a_dropdown_a_slider_or_a_toggle() {
        use genos_scene::Camera;
        use genos_ui::{button_panel, Pointer, State};

        let root = genos_scene_lib::repo_root();
        let entries = genos_scene_lib::load_dir(&genos_scene_lib::scenes_dir()).expect("scenes");
        let log = std::fs::File::create(std::env::temp_dir().join("genos-lighting-menu.log"))
            .expect("log");
        let app = super::App::open(entries, root, log).expect("app");
        let fps = genos_scene::FpsCounterSettings::default();
        let rows = super::panel_rows(&app, ViewMode::Full, &fps);
        assert!(rows.iter().all(|row| row.buttons.is_empty()));
        let dropdowns: Vec<&str> = rows
            .iter()
            .filter(|row| row.dropdown.is_some())
            .map(|row| row.label.as_str())
            .collect();
        for label in [
            "Scene",
            "Checkpoint",
            "View",
            "Shadow filter",
            "Ambient",
            "Lamp pick",
        ] {
            assert!(dropdowns.contains(&label), "missing dropdown {label}");
        }
        let lamp = rows
            .iter()
            .find(|row| row.label == "Lamp pick")
            .expect("lamp pick dropdown")
            .dropdown
            .as_ref()
            .unwrap();
        let lamp_names: Vec<&str> = lamp
            .choices
            .iter()
            .map(|choice| choice.name.as_str())
            .collect();
        assert_eq!(
            lamp_names,
            ["off", "hits", "pixels", "both", "reuse (experimental)"]
        );
        assert_eq!(lamp.value, 1.0, "high samples hits");
        let sliders: Vec<&str> = rows
            .iter()
            .filter(|row| row.slider.is_some())
            .map(|row| row.label.as_str())
            .collect();
        for label in [
            "Preset",
            "Bounces",
            "Max lights",
            "Direct refresh",
            "Direct scale",
            "GI refresh",
            "Budget shift",
            "Probe rays",
            "Probe stale",
            "World stale",
            "Denoise iterations",
            "Denoise variance",
        ] {
            assert!(sliders.contains(&label), "missing slider {label}");
        }
        assert!(!sliders.contains(&"Shadow filter"));
        assert!(!sliders.contains(&"Ambient"));
        let checks: Vec<&str> = rows
            .iter()
            .flat_map(|row| row.checks.iter().map(|check| check.text.as_str()))
            .collect();
        for label in ["Direct", "Shadows", "GI", "Denoise", "Full effort", "Show"] {
            assert!(checks.contains(&label), "missing toggle {label}");
        }
        assert!(checks.iter().all(|label| !sliders.contains(label)));
        assert!(sliders.contains(&"FPS"));
        let fps_row = rows.iter().find(|row| row.label == "FPS").unwrap();
        let rate = fps_row.slider.as_ref().unwrap();
        assert_eq!(rate.id, super::ID_FPS_HZ);
        assert_eq!(rate.min, 1.0);
        assert_eq!(rate.max, 30.0);
        assert_eq!(rate.step, 1.0);
        assert!(fps_row.checks.iter().any(|check| check.id == super::ID_FPS));
        let preset = rows
            .iter()
            .find(|row| row.label == "Preset")
            .unwrap()
            .slider
            .as_ref()
            .unwrap();
        let names: Vec<&str> = preset.steps.iter().map(|step| step.name.as_str()).collect();
        assert!(names.contains(&"potato"));
        assert!(names.contains(&"day-cycle"));
        assert!(names.contains(&"gi-1bounce"));
        assert!(names.contains(&"high"));
        let rays = rows
            .iter()
            .find(|row| row.label == "Probe rays")
            .unwrap()
            .slider
            .as_ref()
            .unwrap();
        assert!(rays.steps.is_empty());
        assert_eq!(rays.step, 4.0);
        let denoise = rows
            .iter()
            .find(|row| row.label == "Denoise iterations")
            .unwrap()
            .slider
            .as_ref()
            .unwrap();
        let denoise_names: Vec<&str> = denoise
            .steps
            .iter()
            .map(|step| step.name.as_str())
            .collect();
        assert_eq!(denoise_names, ["1", "3", "5"]);

        let Some(path) = std::env::var_os("GENOS_MENU_SHOT") else {
            return;
        };
        let mut ui = State::default();
        let camera = Camera::opening();
        let viewport = [1280.0, 720.0];
        let idle = button_panel(
            &mut ui,
            "Menu",
            &rows,
            viewport,
            &camera,
            Pointer {
                x: -1.0,
                y: -1.0,
                ..Pointer::default()
            },
        );
        let hit = idle
            .shown
            .iter()
            .find(|shown| shown.id == super::ID_VIEW)
            .expect("view dropdown");
        let mut ui = State::default();
        let opened = button_panel(
            &mut ui,
            "Menu",
            &rows,
            viewport,
            &camera,
            Pointer::at(hit.rect.x + hit.rect.w * 0.5, hit.rect.y + 8.0, true),
        );
        let choice = opened
            .shown
            .iter()
            .find(|shown| shown.id != super::ID_VIEW && shown.text.as_deref() == Some("direct"))
            .expect("direct row");
        let frame = button_panel(
            &mut ui,
            "Menu",
            &rows,
            viewport,
            &camera,
            Pointer::at(
                choice.rect.x + 8.0,
                choice.rect.y + choice.rect.h * 0.5,
                false,
            ),
        );
        assert!(frame
            .shown
            .iter()
            .any(|shown| shown.id != super::ID_VIEW && shown.text.as_deref() == Some("bounce")));
        let (width, height, rgb) = raster_menu(&frame.paints);
        std::fs::write(path, genos_debug::png::encode_rgb(width, height, &rgb)).expect("png");
    }

    fn raster_menu(paints: &[Paint]) -> (u32, u32, Vec<u8>) {
        let mut max_x = 8.0f32;
        let mut max_y = 8.0f32;
        for paint in paints {
            max_x = max_x.max(paint.x + paint.w);
            max_y = max_y.max(paint.y + paint.h);
        }
        let margin = 12.0;
        let width = (max_x + margin).ceil() as u32;
        let height = (max_y + margin).ceil() as u32;
        let mut rgb = vec![8u8; (width * height * 3) as usize];
        for paint in paints {
            let color = [
                (paint.color[0].clamp(0.0, 1.0) * 255.0) as u8,
                (paint.color[1].clamp(0.0, 1.0) * 255.0) as u8,
                (paint.color[2].clamp(0.0, 1.0) * 255.0) as u8,
            ];
            let x0 = paint.x.floor().max(0.0) as u32;
            let y0 = paint.y.floor().max(0.0) as u32;
            let x1 = ((paint.x + paint.w).ceil() as u32).min(width);
            let y1 = ((paint.y + paint.h).ceil() as u32).min(height);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y * width + x) * 3) as usize;
                    rgb[i] = color[0];
                    rgb[i + 1] = color[1];
                    rgb[i + 2] = color[2];
                }
            }
        }
        (width, height, rgb)
    }

    #[test]
    fn help_lists_the_fps_counter() {
        assert!(super::USAGE.contains("--fps-counter"));
        assert!(super::USAGE.contains("--fps-hz"));
        assert!(super::USAGE.contains("F3"));
        assert!(super::USAGE.contains("GENOS_FPS_COUNTER"));
    }
}

#[cfg(test)]
mod fps_overlay;
