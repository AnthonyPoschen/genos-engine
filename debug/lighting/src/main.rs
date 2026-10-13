//! Lighting debug binary. Scenes come from `debug/scenes/`. Arguments choose the
//! scene, the checkpoint, the lighting preset, and where pictures land.
//! `./genos-debug/<kind>/` is the default output. `--out` overrides it.
//! An environment variable applies only when the matching argument was omitted.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use genos_debug::{png, CheckpointPose, Host, Knob};
use genos_input::{character_controller, InputCode, InputSystem};
use genos_render::{
    identity_pose, DrawKind, FixedPart, Renderer, ScreenRect, ViewMode, World,
};
use genos_scene::{
    look_direction, update, Actions, Camera, Floor, Light, Scene, Sky, Vec3, PITCH_LIMIT,
};
use genos_scene_lib::{Entry, Kind, Lamp, Sun};
use genos_stress::building::{Layout, LightMix};
use genos_stress::knobs::{apply_lighting_knob, lighting_knobs};
use genos_stress::stage::{Scale, Stage};
use genos_ui::{button_panel, Action, PanelButton, PanelRow, PanelSlider, Pointer, State};
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
  --out DIR              screenshots, dumps, and logs (default ./genos-debug)
  --size WxH             window size (default 1280x720; 320x180 for --check)
  --check                load every scene headless, require a non-black picture,
                         and write shots/<scene>.png

Page Up and Page Down switch scenes. Keys 1-9 jump checkpoints. Home and 0 reset
to the active checkpoint. WASD fly, Space up, C or Ctrl down, Shift is faster.

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
const ID_PRESET: u32 = 2600;
const ID_KNOB: u32 = 1000;
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
    let mut app = App::open(entries, root, log.try_clone().map_err(|err| err.to_string())?)?;
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
    jump_camera(
        &mut app,
        opts.checkpoint.as_deref(),
        &mut camera,
    )?;
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
    let _ = writeln!(
        log,
        "mcp http://127.0.0.1:{}/",
        genos_mcp::listen_port()
    );
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
            let rows = panel_rows(&app, tools.view_mode());
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
                },
            );
            for action in &ui_frame.actions {
                if let Action::Press(id) = *action {
                    press(&mut app, &mut camera, &mut tools, &mut renderer, id)?;
                }
                if let Action::Slide { id, value } = *action {
                    slide(&mut app, id, value);
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
                let path = opts.out.join("shots").join(format!("{}.png", app.entry().name));
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
    let mut failed = Vec::new();
    let names: Vec<String> = app.entries.iter().map(|e| e.name.clone()).collect();
    for name in &names {
        let _ = writeln!(log, "check {name}");
        eprintln!("genos-lighting: check {name}");
        app.load_scene(name)?;
        jump_camera(app, None, &mut camera)?;
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
        let _ = writeln!(
            log,
            "check {name} mean {mean:.1} spread {spread:.1} {}",
            path.display()
        );
        if mean < 8.0 {
            failed.push(format!("{name} is black (mean {mean:.1})"));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
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

    fn preset_label(&self) -> &'static str {
        match &self.show {
            Show::Stress(stage) => stage.world.scene.lighting.preset_label(),
            Show::Still(world) => world.scene.lighting.preset_label(),
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
            let model = genos_load::load_gltf_file(&full)
                .map_err(|err| format!("{}: {err}", full.display()))?;
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
    camera.pitch = (camera.pitch + actions.look_y * STICK_LOOK * dt).clamp(-PITCH_LIMIT, PITCH_LIMIT);
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

fn panel_rows(app: &App, view: ViewMode) -> Vec<PanelRow> {
    let scene_buttons = app
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| PanelButton {
            id: ID_SCENE + i as u32,
            text: e.name.clone(),
            selected: i == app.index,
        })
        .collect();
    let cp_buttons = app
        .entry()
        .checkpoints
        .iter()
        .enumerate()
        .map(|(i, c)| PanelButton {
            id: ID_CP + i as u32,
            text: c.name.clone(),
            selected: i == app.checkpoint,
        })
        .collect();
    let view_buttons = VIEWS
        .iter()
        .enumerate()
        .map(|(i, name)| PanelButton {
            id: ID_VIEW + i as u32,
            text: (*name).to_string(),
            selected: view_name(view) == *name,
        })
        .collect();
    let preset = app.preset_label();
    let preset_buttons = PRESETS
        .iter()
        .enumerate()
        .map(|(i, name)| PanelButton {
            id: ID_PRESET + i as u32,
            text: (*name).to_string(),
            selected: preset == *name,
        })
        .collect();
    let mut rows = vec![
        PanelRow {
            label: "Scene".into(),
            buttons: scene_buttons,
            slider: None,
        },
        PanelRow {
            label: "Checkpoint".into(),
            buttons: cp_buttons,
            slider: None,
        },
        PanelRow {
            label: "View".into(),
            buttons: view_buttons,
            slider: None,
        },
        PanelRow {
            label: "Preset".into(),
            buttons: preset_buttons,
            slider: None,
        },
    ];
    if let Some(knob) = app.knobs().into_iter().find(|k| k.name == "lighting_bounces") {
        rows.push(PanelRow {
            label: knob.label,
            buttons: Vec::new(),
            slider: Some(PanelSlider {
                id: ID_KNOB,
                value: knob.value,
                min: knob.min,
                max: knob.max,
                step: knob.step,
                text: knob.text,
            }),
        });
    }
    rows
}

fn press(
    app: &mut App,
    camera: &mut Camera,
    tools: &mut genos_debug::Tools,
    renderer: &mut Renderer,
    id: u32,
) -> Result<(), String> {
    if (ID_SCENE..ID_SCENE + app.entries.len() as u32).contains(&id) {
        switch_to(
            app,
            (id - ID_SCENE) as usize,
            None,
            camera,
            tools,
            renderer,
        )?;
    } else if (ID_CP..ID_CP + 64).contains(&id) {
        let i = (id - ID_CP) as usize;
        if let Some(name) = app.entry().checkpoints.get(i).map(|c| c.name.clone()) {
            let pose = app.jump_checkpoint(&name)?;
            camera.set_pose(
                Vec3::new(pose.position[0], pose.position[1], pose.position[2]),
                pose.yaw,
                pose.pitch,
            );
        }
    } else if (ID_VIEW..ID_VIEW + VIEWS.len() as u32).contains(&id) {
        let mode = match VIEWS[(id - ID_VIEW) as usize] {
            "direct" => ViewMode::Direct,
            "bounce" => ViewMode::Bounce,
            "albedo" => ViewMode::Albedo,
            "normal" => ViewMode::Normal,
            "depth" => ViewMode::Depth,
            "texels" => ViewMode::Texels,
            _ => ViewMode::Full,
        };
        tools.set_view(mode);
    } else if (ID_PRESET..ID_PRESET + PRESETS.len() as u32).contains(&id) {
        let _ = app.set_knob("lighting_preset", (id - ID_PRESET) as f64);
    }
    Ok(())
}

fn slide(app: &mut App, id: u32, value: f64) {
    if id == ID_KNOB {
        let _ = app.set_knob("lighting_bounces", value);
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
    args.next()
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn options() -> Result<Options, String> {
    let mut args = std::env::args().skip(1);
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
                let (w, h) = text
                    .split_once('x')
                    .ok_or("size is WxH")?;
                size = (
                    w.parse().map_err(|_| "bad width".to_string())?,
                    h.parse().map_err(|_| "bad height".to_string())?,
                );
            }
            "--check" => check = true,
            other => return Err(format!("unknown argument {other}")),
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
