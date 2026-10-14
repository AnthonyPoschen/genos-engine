//! Lighting debug binary. Scenes come from `debug/scenes/`. Arguments choose the
//! scene, the checkpoint, the lighting preset, and where pictures land.
//! `./genos-debug/<kind>/` is the default output. `--out` overrides it.
//! An environment variable applies only when the matching argument was omitted.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use genos_debug::{png, CheckpointPose, Host, Knob};
use genos_input::{character_controller, debug_keys, InputCode, InputSystem, KeyBindings};
use genos_render::{identity_pose, DrawKind, FixedPart, Renderer, ScreenRect, ViewMode, World};
use genos_scene::{
    look_direction, parse_enabled, parse_menu_visible, parse_rate, update, Actions, Camera,
    DebugSettings, Floor, FocusPolicy, FpsCounterSettings, Light, MenuSettings, Scene, Sky, Vec3,
    PITCH_LIMIT,
};
use genos_scene_lib::{Entry, Kind, Lamp, Sun};
use genos_stress::building::{Layout, LightMix};
use genos_stress::knobs::{apply_lighting_knob, lighting_knobs};
use genos_stress::stage::{Scale, Stage};
use genos_ui::{
    button_panel_blocks, show_menu_tab, Action, FpsOverlay, InputPolicy, PanelBlock, PanelCheck,
    PanelDropdown, PanelRow, PanelSection, PanelSlider, PanelTab, PanelTabs, Pointer, SliderStep,
    State, MENU_HIDE, MENU_SHOW, PANEL_ID,
};
use genos_window::{extent_changed, FocusGate, Window};

const USAGE: &str = "\
genos-lighting — lighting scenes, checkpoints, and the debug picture

  --scene NAME           scene file name (debug/scenes). Falls back to GENOS_SCENE,
                         then stress
  --checkpoint NAME      jump here at launch
  --eye X,Y,Z            camera position, metres. --eye=X,Y,Z is the same flag.
                         Keeps the checkpoint aim unless --look is set
  --look X,Y,Z           world point to aim at. --look=X,Y,Z is the same flag.
                         Yaw 0 looks down -Z
  --preset NAME          potato, low, medium, or high. Sets GENOS_LIGHTING_PRESET
  --gpu-times N          print GPU_MS every N frames (GENOS_GPU_TIMES)
  --lighting-debug       print the lighting probe line (GENOS_LIGHTING_DEBUG)
  --unfilled             show the texels view (slots with fewer than 4 sets are dim)
  --full-effort          preset high, unless --preset is set, and compose every frame
                         (GENOS_LIGHTING_COMPOSE_ALL)
  --denoise-launch hash|screen
                         picture wavelet (GENOS_LIGHTING_DENOISE_LAUNCH). Default hash.
                         screen is the opt-in speed path
  --denoise-screen on|off alias of --denoise-launch (GENOS_LIGHTING_DENOISE_SCREEN)
  --headless             proof window, no panel
  --frames N             quit after N frames
  --fps-counter [on|off] frame-rate plate (default off). A bare flag turns it on.
                         GENOS_FPS_COUNTER applies only when this is omitted
  --fps-hz N             plate update rate in Hz (default 4, range 0.25 to 30).
                         GENOS_FPS_HZ applies only when this is omitted
  --menu shown|hidden    debug menu (default shown). GENOS_MENU applies only
                         when this is omitted
  --mcp-port N           loopback MCP port (default 8766). GENOS_MCP_PORT applies
                         only when this is omitted. genos-stress stays on 8765
  --out DIR              screenshots, dumps, and logs (default ./genos-debug)
  --size WxH             window size (default 1280x720; 320x180 for --check)
  --check                every scene at its check checkpoint, four frames,
                         mean luma against that scene's own floor

Page Up and Page Down switch scenes. Keys 1-9 jump checkpoints. Home and 0 reset
to the active checkpoint. The wheel scrolls the menu while the pointer is over
it. Perf can set that wheel policy to hover, modal, click, or passive. A click
gives the menu the keys. Arrow keys then move the focused control,
and a focused slider steps. Enter picks the highlighted choice or presses the
focused control. Escape closes an open list, then releases the keys. A click
outside releases them too. Grave releases the keys on the first press and
leaves the menu up. The next grave hides it. F1 hides it either way. Perf sets
Wheel focus
and Key focus to hover, modal, click, or passive. The title bar hide button
hides it. A menu tab in the top-left corner shows it. --menu hidden starts it
hidden. GENOS_MENU applies only when that argument was omitted.
get_menu_settings and set_menu_settings read and write shown or hidden, the
page, the folds, and those two focus policies. ] and [ change the page while
the menu has the keys. Tab and shift-Tab do that too. F toggles the frame-rate
plate. F3 is the same key. A section title folds that group.
The fold is remembered in <out>/menu/collapsed.txt. The open page is remembered
in <out>/menu/tab.txt.
The groups are debug/lighting/menu.txt. A section key of - has no fold.
GENOS_MENU_LAYOUT points at another file. The panel stays within a third of
the window. The pages are Scene/View, Pipeline, Direct/Lamps, GI, Denoise,
Shadows/Ambient, Perf, and Info. Direct and GI keep folds because those pages
are tall.
WASD fly, Space
up, C or Ctrl down, Shift is faster. The plate shows FPS and frame ms, averaged
over each 1/rate window (default 4 Hz). It is off until the checkbox, F or F3, MCP,
or --fps-counter turns it on. GENOS_DEBUG_KEYS replaces those keys.

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
const ID_MENU_WHEEL: u32 = 2820;
const ID_MENU_KEYS: u32 = 2821;
const ID_KNOB: u32 = 3000;
const ID_SWITCH: u32 = 4000;

mod menu_layout;

use menu_layout::{load_menu_layout, menu_folds, section_id, strip_id, tab_id, MenuFold, MenuLine};
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
    eye: Option<[f32; 3]>,
    look: Option<[f32; 3]>,
    preset: Option<String>,
    gpu_times: Option<u32>,
    lighting_debug: bool,
    unfilled: bool,
    full_effort: bool,
    denoise_launch: Option<String>,
    denoise_screen: Option<String>,
    headless: bool,
    frames: Option<u32>,
    out: PathBuf,
    out_set: bool,
    size: (u32, u32),
    check: bool,
    /// Set by `--mcp-port`. Empty means `GENOS_MCP_PORT`, then [`MCP_PORT`].
    mcp_port: Option<u16>,
}

/// Loopback port for this binary. `genos-stress` and `genos-camera` stay on 8765.
const MCP_PORT: u16 = 8766;

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
    place_camera(&opts, &mut camera)?;
    let (width, height) = opts.size;
    let mut window = if opts.headless {
        Window::open_proof(width, height)?
    } else {
        Window::open_named(width, height, "genos-lighting", "Genos lighting")?
    };
    let first = window.pump();
    let mut renderer = Renderer::open(window.display, window.surface, first.width, first.height)?;
    let mcp_host = genos_mcp::Host::new(app.scene().clone(), camera.clone());
    let mcp_port = opts.mcp_port.unwrap_or_else(mcp_port_from_env);
    let _server = genos_mcp::Server::start_on(mcp_host, mcp_port)?;
    let _ = writeln!(log, "mcp http://127.0.0.1:{mcp_port}/");
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
    let menu = load_menu_layout()?;
    let folds = menu_folds(&menu);
    let mut ui = State::default();
    let mut tracked_collapsed = app.scene().menu.collapsed.clone();
    let mut tracked_tab = app.scene().menu.tab.clone();
    if let Some(names) = load_menu_folds(&opts.out) {
        app.scene().menu.collapsed = names.clone();
        tracked_collapsed = names.clone();
        apply_collapsed(&mut ui, &folds, &names);
    }
    if let Some(name) = load_menu_tab(&opts.out) {
        app.scene().menu.tab = name.clone();
        tracked_tab = name.clone();
        ui.select_tab(strip_id(0), tab_id(&name));
    }
    let mut input = InputSystem::new();
    let controls = character_controller();
    let keys = debug_keys()?;
    let mut menu_open = app.scene().menu.effective().visible;
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
        pull_menu(
            &mut app,
            &mut ui,
            &folds,
            &mut menu_open,
            &mut tracked_collapsed,
            &mut tracked_tab,
        );
        hotkeys(
            &mut app,
            &input,
            &keys,
            &mut camera,
            &mut tools,
            &mut renderer,
        )?;
        let move_axis = controls.axis_2d(&input, "move");
        let look_axis = controls.axis_2d(&input, "look");
        let page_keys = !camera.captured && menu_open;
        let pointer = Pointer {
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
            key_blur: !camera.captured && input.keyboard.pressed(InputCode::key_grave),
            tab_prev: page_keys
                && (keys.pressed(&input, "tab_prev") || keys.pressed(&input, "tab_focus_prev")),
            tab_next: page_keys
                && (keys.pressed(&input, "tab_next") || keys.pressed(&input, "tab_focus_next")),
        };
        let mut drew_panel = false;
        let mut menu_blur = false;
        let (paints, look_capture) = if opts.headless {
            (Vec::new(), true)
        } else if !menu_open {
            let ui_frame = show_menu_tab(&mut ui, [size.0 as f32, size.1 as f32], pointer);
            if ui_frame
                .actions
                .iter()
                .any(|action| matches!(action, Action::Press(id) if *id == MENU_SHOW))
            {
                menu_open = true;
            }
            (ui_frame.paints, ui_frame.look_capture)
        } else {
            drew_panel = true;
            let fps_menu = app.scene().fps_counter.effective();
            let blocks = panel_blocks(&app, tools.view_mode(), &fps_menu, &menu);
            let ui_frame = button_panel_blocks(
                &mut ui,
                &panel_title(&app),
                &blocks,
                [size.0 as f32, size.1 as f32],
                &camera,
                pointer,
            );
            for action in &ui_frame.actions {
                match *action {
                    Action::Slide { id, value } => slide(&mut app, id, value)?,
                    Action::Select { id, value } => {
                        select(&mut app, &mut camera, &mut tools, &mut renderer, id, value)?;
                    }
                    Action::Toggle { id, on } => toggle(&mut app, id, on)?,
                    Action::Press(id) => {
                        if id == MENU_HIDE {
                            menu_open = false;
                        } else if folds.iter().any(|fold| fold.id == id) {
                            save_menu_folds(&opts.out, &ui, &folds)?;
                        } else if let Some(name) = tab_title(&menu, id) {
                            save_menu_tab(&opts.out, name)?;
                        }
                    }
                    _ => {}
                }
            }
            menu_blur = ui_frame.consumed.blur;
            (ui_frame.paints, ui_frame.look_capture)
        };
        if keys.pressed(&input, "menu") && !menu_blur {
            menu_open = !menu_open;
        }
        push_menu(
            &mut app,
            &ui,
            &folds,
            &menu,
            menu_open,
            drew_panel,
            &opts.out,
            &mut tracked_collapsed,
            &mut tracked_tab,
        )?;
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
        let menu = self.scene().menu.clone();
        let index = self
            .entries
            .iter()
            .position(|e| e.name == name)
            .ok_or_else(|| format!("unknown scene {name}"))?;
        let show = build_entry(&self.root, &self.entries[index])?;
        self.index = index;
        self.checkpoint = 0;
        self.show = show;
        self.scene().menu = menu;
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
                menu: MenuSettings::default(),
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

/// `--eye=X,Y,Z` and `--look=X,Y,Z`. The spaced form is a separate argument.
fn placed_flag(arg: &str) -> Option<(&'static str, &str)> {
    if let Some(value) = arg.strip_prefix("--eye=") {
        Some(("--eye", value))
    } else if let Some(value) = arg.strip_prefix("--look=") {
        Some(("--look", value))
    } else {
        None
    }
}

fn parse_vec3(flag: &str, text: &str) -> Result<[f32; 3], String> {
    let parts: Vec<&str> = text.split(',').collect();
    if parts.len() != 3 {
        return Err(format!("{flag} is x,y,z"));
    }
    let mut out = [0.0; 3];
    for (i, part) in parts.iter().enumerate() {
        out[i] = part
            .trim()
            .parse()
            .map_err(|_| format!("{flag} has a bad number {part}"))?;
    }
    Ok(out)
}

/// `--eye` moves the camera. `--look` aims it, from that eye or from the
/// checkpoint. Yaw and pitch match `look_yaw_pitch` (yaw 0 looks down -Z).
fn place_camera(opts: &Options, camera: &mut Camera) -> Result<(), String> {
    let eye = match opts.eye {
        Some(eye) => eye,
        None => [camera.position.x, camera.position.y, camera.position.z],
    };
    if opts.eye.is_some() && opts.look.is_none() {
        camera.set_pose(Vec3::new(eye[0], eye[1], eye[2]), camera.yaw, camera.pitch);
    }
    if let Some(target) = opts.look {
        let (yaw, pitch) = genos_scene_lib::look_yaw_pitch(eye, target);
        camera.set_pose(Vec3::new(eye[0], eye[1], eye[2]), yaw, pitch);
    }
    Ok(())
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
    keys: &KeyBindings,
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
    let digits = [
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
    for (i, code) in digits.iter().enumerate() {
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
    if keys.pressed(input, "fps") {
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
    app.entry().name.clone()
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

fn knob_row(app: &App, index: usize, name: &str) -> Option<PanelRow> {
    let knob = app.knobs().into_iter().find(|knob| knob.name == name)?;
    if knob.choices == ["off", "on"] {
        return None;
    }
    let label = menu_label(name, &knob.label);
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

#[cfg(test)]
fn panel_rows(app: &App, view: ViewMode, fps: &FpsCounterSettings) -> Vec<PanelRow> {
    let menu = load_menu_layout().expect("layout");
    flatten_blocks(&panel_blocks(app, view, fps, &menu))
}

#[cfg(test)]
fn flatten_blocks(blocks: &[PanelBlock]) -> Vec<PanelRow> {
    let mut rows = Vec::new();
    for block in blocks {
        match block {
            PanelBlock::Section(section) => {
                rows.extend(section.rows.clone());
                rows.extend(flatten_blocks(&section.blocks));
            }
            PanelBlock::Tabs(tabs) => {
                for tab in &tabs.tabs {
                    rows.extend(flatten_blocks(&tab.blocks));
                }
            }
            PanelBlock::Rows(page) => rows.extend(page.clone()),
        }
    }
    rows
}

enum Piece {
    Row(PanelRow),
    Check(PanelCheck),
}

struct SectionAcc {
    tab: Option<String>,
    key: String,
    title: String,
    collapsed: bool,
    rows: Vec<PanelRow>,
    checks: Vec<PanelCheck>,
}

fn flush_checks(section: &mut SectionAcc) {
    if section.checks.is_empty() {
        return;
    }
    section.rows.push(PanelRow {
        label: String::new(),
        buttons: Vec::new(),
        slider: None,
        dropdown: None,
        checks: std::mem::take(&mut section.checks),
    });
}

fn panel_blocks(
    app: &App,
    view: ViewMode,
    fps: &FpsCounterSettings,
    lines: &[MenuLine],
) -> Vec<PanelBlock> {
    let mut sections = Vec::new();
    for line in lines {
        let same = sections.last().is_some_and(|section: &SectionAcc| {
            section.tab == line.tab && section.key == line.section
        });
        if !same {
            if let Some(section) = sections.last_mut() {
                flush_checks(section);
            }
            sections.push(SectionAcc {
                tab: line.tab.clone(),
                key: line.section.clone(),
                title: line.title.clone(),
                collapsed: line.collapsed,
                rows: Vec::new(),
                checks: Vec::new(),
            });
        }
        let Some(piece) = setting_piece(app, view, fps, &line.setting) else {
            continue;
        };
        let section = sections.last_mut().expect("section");
        match piece {
            Piece::Row(row) => {
                flush_checks(section);
                section.rows.push(row);
            }
            Piece::Check(check) => section.checks.push(check),
        }
    }
    if let Some(section) = sections.last_mut() {
        flush_checks(section);
    }
    let mut blocks = Vec::new();
    let mut strip: Option<PanelTabs> = None;
    let mut strip_index = 0usize;
    for section in sections {
        let block = if section.key == "-" {
            PanelBlock::Rows(section.rows)
        } else {
            PanelBlock::Section(PanelSection::new(
                section_id(&section.key),
                section.title,
                section.collapsed,
                section.rows,
            ))
        };
        match section.tab {
            None => {
                if let Some(tabs) = strip.take() {
                    blocks.push(PanelBlock::Tabs(tabs));
                }
                blocks.push(block);
            }
            Some(name) => {
                if strip.is_none() {
                    strip = Some(PanelTabs {
                        id: strip_id(strip_index),
                        tabs: Vec::new(),
                    });
                    strip_index += 1;
                }
                let tabs = strip.as_mut().expect("strip");
                if let Some(tab) = tabs.tabs.iter_mut().find(|tab| tab.title == name) {
                    tab.blocks.push(block);
                } else {
                    tabs.tabs.push(PanelTab {
                        id: tab_id(&name),
                        title: name,
                        blocks: vec![block],
                    });
                }
            }
        }
    }
    if let Some(tabs) = strip {
        blocks.push(PanelBlock::Tabs(tabs));
    }
    blocks
}

fn setting_piece(
    app: &App,
    view: ViewMode,
    fps: &FpsCounterSettings,
    setting: &str,
) -> Option<Piece> {
    let view_value = VIEWS
        .iter()
        .position(|name| *name == view_name(view))
        .unwrap_or(0) as f64;
    match setting {
        "review" => Some(Piece::Row(PanelRow {
            label: app.entry().review.clone(),
            buttons: Vec::new(),
            slider: None,
            dropdown: None,
            checks: Vec::new(),
        })),
        "scene" => drop_row(
            "Scene",
            ID_SCENE,
            app.index as f64,
            app.entries.iter().map(|entry| entry.name.clone()).collect(),
        )
        .map(Piece::Row),
        "checkpoint" => drop_row(
            "Checkpoint",
            ID_CP,
            app.checkpoint as f64,
            app.entry()
                .checkpoints
                .iter()
                .map(|checkpoint| checkpoint.name.clone())
                .collect(),
        )
        .map(Piece::Row),
        "view" => drop_row(
            "View",
            ID_VIEW,
            view_value,
            VIEWS.iter().map(|name| (*name).to_string()).collect(),
        )
        .map(Piece::Row),
        "fps" => Some(Piece::Row(fps_row(fps))),
        "menu_wheel" => focus_row("Wheel focus", ID_MENU_WHEEL, menu_settings(app).wheel).map(Piece::Row),
        "menu_keys" => focus_row("Key focus", ID_MENU_KEYS, menu_settings(app).keys).map(Piece::Row),
        name => {
            let knob = app.knobs().into_iter().find(|knob| knob.name == name)?;
            if knob.choices == ["off", "on"] {
                let index = switch_knobs(app)
                    .iter()
                    .position(|item| item.name == name)?;
                Some(Piece::Check(PanelCheck {
                    id: ID_SWITCH + index as u32,
                    text: menu_label(name, &knob.label),
                    on: knob.value >= 0.5,
                }))
            } else if let Some(index) = SLIDER_KNOBS.iter().position(|item| *item == name) {
                knob_row(app, index, name).map(Piece::Row)
            } else if let Some(index) = DROP_KNOBS.iter().position(|item| *item == name) {
                drop_knob_row(app, index, name).map(Piece::Row)
            } else {
                None
            }
        }
    }
}

fn menu_fold_path(out: &Path) -> PathBuf {
    out.join("menu").join("collapsed.txt")
}

fn menu_tab_path(out: &Path) -> PathBuf {
    out.join("menu").join("tab.txt")
}

fn tab_title<'a>(lines: &'a [MenuLine], id: u32) -> Option<&'a str> {
    lines.iter().find_map(|line| {
        let name = line.tab.as_deref()?;
        (tab_id(name) == id).then_some(name)
    })
}

fn load_menu_folds(out: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(menu_fold_path(out)).ok()?;
    Some(
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_string)
            .collect(),
    )
}

fn save_menu_folds(out: &Path, ui: &State, folds: &[MenuFold]) -> Result<(), String> {
    let names = folds
        .iter()
        .filter(|fold| ui.section_collapsed(fold.id))
        .map(|fold| fold.key.clone())
        .collect::<Vec<_>>();
    write_fold_names(out, &names)
}

fn write_fold_names(out: &Path, names: &[String]) -> Result<(), String> {
    let path = menu_fold_path(out);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let mut body = String::from("# collapsed menu sections\n");
    for name in names {
        body.push_str(name);
        body.push('\n');
    }
    std::fs::write(path, body).map_err(|err| err.to_string())
}

fn apply_collapsed(ui: &mut State, folds: &[MenuFold], names: &[String]) {
    for fold in folds {
        ui.fold_section(fold.id, names.iter().any(|name| name == &fold.key));
    }
}

/// Folds the panel has not laid out keep the names already stored.
fn merge_collapsed(previous: &[String], ui: &State, folds: &[MenuFold]) -> Vec<String> {
    let mut names = previous.to_vec();
    for fold in folds {
        if !ui.section_known(fold.id) {
            continue;
        }
        if ui.section_collapsed(fold.id) {
            if !names.iter().any(|name| name == &fold.key) {
                names.push(fold.key.clone());
            }
        } else {
            names.retain(|name| name != &fold.key);
        }
    }
    names
}

fn pull_menu(
    app: &mut App,
    ui: &mut State,
    folds: &[MenuFold],
    menu_open: &mut bool,
    tracked_collapsed: &mut Vec<String>,
    tracked_tab: &mut String,
) {
    let menu = app.scene().menu.clone();
    if menu.collapsed != *tracked_collapsed {
        apply_collapsed(ui, folds, &menu.collapsed);
        *tracked_collapsed = menu.collapsed;
    }
    if menu.tab != *tracked_tab {
        if !menu.tab.is_empty() {
            ui.select_tab(strip_id(0), tab_id(&menu.tab));
        }
        *tracked_tab = menu.tab;
    }
    *menu_open = app.scene().menu.effective().visible;
    if !*menu_open {
        ui.set_focused_panel(None);
    }
    let menu = app.scene().menu.clone();
    ui.set_input_policy(
        PANEL_ID,
        InputPolicy {
            wheel: menu.wheel,
            keys: menu.keys,
        },
    );
}

fn push_menu(
    app: &mut App,
    ui: &State,
    folds: &[MenuFold],
    lines: &[MenuLine],
    menu_open: bool,
    drew_panel: bool,
    out: &Path,
    tracked_collapsed: &mut Vec<String>,
    tracked_tab: &mut String,
) -> Result<(), String> {
    if drew_panel {
        let merged = merge_collapsed(tracked_collapsed, ui, folds);
        if merged != app.scene().menu.collapsed {
            app.scene().menu.collapsed = merged.clone();
            write_fold_names(out, &merged)?;
        }
        *tracked_collapsed = app.scene().menu.collapsed.clone();
        if let Some(id) = ui.selected_tab(strip_id(0)) {
            if let Some(name) = tab_title(lines, id) {
                if app.scene().menu.tab != name {
                    app.scene().menu.tab = name.to_string();
                    save_menu_tab(out, name)?;
                }
                *tracked_tab = app.scene().menu.tab.clone();
            }
        }
    }
    if app.scene().menu.visible != menu_open {
        app.scene().menu.visible = menu_open;
    }
    Ok(())
}

fn load_menu_tab(out: &Path) -> Option<String> {
    let text = std::fs::read_to_string(menu_tab_path(out)).ok()?;
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
}

fn save_menu_tab(out: &Path, name: &str) -> Result<(), String> {
    let path = menu_tab_path(out);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(path, format!("# selected menu tab\n{name}\n")).map_err(|err| err.to_string())
}

fn drop_knob_row(app: &App, index: usize, name: &str) -> Option<PanelRow> {
    let knob = app.knobs().into_iter().find(|knob| knob.name == name)?;
    if knob.choices.is_empty() || knob.choices == ["off", "on"] {
        return None;
    }
    drop_row(
        &menu_label(name, &knob.label),
        ID_DROP + index as u32,
        knob.value,
        knob.choices,
    )
}

/// Panel text from `docs/lighting/GLOSSARY.md`. The knob name stays the config key.
fn menu_label(name: &str, fallback: &str) -> String {
    match name {
        "lighting_preset" => "Preset",
        "lighting_bounces" => "Bounce count",
        "lighting_direct" => "Direct light",
        "lighting_direct_refresh" => "Direct refresh frames",
        "lighting_gi_refresh" => "GI refresh frames",
        "lighting_budget_shift" => "Ray budget shift",
        "lighting_lamp_pick" => "Lamp pick",
        "lighting_lamp_pick_samples" => "Samples",
        "lighting_lamp_pick_candidates" => "Candidates",
        "lighting_probe_rays" => "Screen probe rays",
        "lighting_probe_stale" => "Probe stale frames",
        "lighting_probes" => "Screen probes",
        "lighting_world_cache" => "Surface light cache",
        "lighting_world_stale" => "Cache stale frames",
        "lighting_denoise" => "Denoiser",
        "lighting_denoise_iters" => "Denoiser iterations",
        "lighting_denoise_variance" => "Denoiser variance",
        "lighting_denoise_launch" => "Denoiser launch",
        "lighting_denoise_adaptive" => "Adaptive denoise",
        "lighting_sky" => "Sky light",
        "lighting_prefill" => "Pre-fill",
        "lighting_ambient" => "Ambient fill",
        "lighting_shadow_filter" => "Shadow cell cache",
        "lighting_gi" => "GI",
        "lighting_emissive" => "Emissive",
        "lighting_compose_reuse" => "Compose reuse",
        "lighting_full_effort" => "Full effort",
        _ => fallback,
    }
    .to_string()
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
    } else if id == ID_MENU_WHEEL || id == ID_MENU_KEYS {
        let name = ["hover", "modal", "click", "passive"]
            .get(index)
            .copied()
            .unwrap_or("hover");
        let key = if id == ID_MENU_WHEEL { "wheel" } else { "keys" };
        app.scene().menu.set_key(key, name)?;
    } else if (ID_DROP..ID_DROP + DROP_KNOBS.len() as u32).contains(&id) {
        let name = DROP_KNOBS[(id - ID_DROP) as usize];
        app.set_knob(name, value)?;
    }
    Ok(())
}

fn menu_settings(app: &App) -> MenuSettings {
    match &app.show {
        Show::Stress(stage) => stage.world.scene.menu.clone(),
        Show::Still(world) => world.scene.menu.clone(),
    }
}

fn focus_index(policy: FocusPolicy) -> f64 {
    match policy {
        FocusPolicy::Hover => 0.0,
        FocusPolicy::Modal => 1.0,
        FocusPolicy::ClickToFocus => 2.0,
        FocusPolicy::Passive => 3.0,
    }
}

fn focus_row(label: &str, id: u32, policy: FocusPolicy) -> Option<PanelRow> {
    drop_row(
        label,
        id,
        focus_index(policy),
        ["hover", "modal", "click", "passive"]
            .into_iter()
            .map(str::to_string)
            .collect(),
    )
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
    if let Some(value) = &opts.denoise_screen {
        std::env::set_var("GENOS_LIGHTING_DENOISE_SCREEN", value);
    }
    if let Some(value) = &opts.denoise_launch {
        std::env::set_var("GENOS_LIGHTING_DENOISE_LAUNCH", value);
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
    let mut eye = None;
    let mut look = None;
    let mut preset = None;
    let mut gpu_times = None;
    let mut lighting_debug = false;
    let mut unfilled = false;
    let mut full_effort = false;
    let mut denoise_launch = None;
    let mut denoise_screen = None;
    let mut headless = false;
    let mut frames = None;
    let mut out = PathBuf::from("genos-debug");
    let mut out_set = false;
    let mut size = (1280u32, 720u32);
    let mut check = false;
    let mut mcp_port = None;
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!("{USAGE}");
            std::process::exit(0);
        }
        match arg.as_str() {
            "--scene" => scene = Some(need_arg(&mut args, "--scene")?),
            "--checkpoint" => checkpoint = Some(need_arg(&mut args, "--checkpoint")?),
            "--eye" => eye = Some(parse_vec3("--eye", &need_arg(&mut args, "--eye")?)?),
            "--look" => look = Some(parse_vec3("--look", &need_arg(&mut args, "--look")?)?),
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
            "--denoise-launch" => denoise_launch = Some(need_arg(&mut args, "--denoise-launch")?),
            "--denoise-screen" => denoise_screen = Some(need_arg(&mut args, "--denoise-screen")?),
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
            "--menu" => {
                let value = if args.peek().is_some_and(|next| !next.starts_with('-')) {
                    args.next().unwrap()
                } else {
                    return Err("--menu needs shown or hidden".into());
                };
                parse_menu_visible(&value).map_err(|err| format!("bad --menu: {err}"))?;
            }
            "--mcp-port" => {
                mcp_port = Some(parse_mcp_port(&need_arg(&mut args, "--mcp-port")?)?);
            }
            other => {
                if let Some((flag, value)) = placed_flag(other) {
                    let parsed = parse_vec3(flag, value)?;
                    if flag == "--eye" {
                        eye = Some(parsed);
                    } else {
                        look = Some(parsed);
                    }
                } else if let Some(value) = other.strip_prefix("--fps-counter=") {
                    parse_enabled(value).map_err(|err| format!("bad --fps-counter: {err}"))?;
                } else if let Some(value) = other.strip_prefix("--fps-hz=") {
                    parse_rate(value).map_err(|err| format!("bad --fps-hz: {err}"))?;
                } else if let Some(value) = other.strip_prefix("--menu=") {
                    parse_menu_visible(value).map_err(|err| format!("bad --menu: {err}"))?;
                } else if let Some(value) = other.strip_prefix("--mcp-port=") {
                    mcp_port = Some(parse_mcp_port(value)?);
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
        eye,
        look,
        preset,
        gpu_times,
        lighting_debug,
        unfilled,
        full_effort,
        denoise_launch,
        denoise_screen,
        headless,
        frames,
        out,
        out_set,
        size,
        check,
        mcp_port,
    })
}

fn parse_mcp_port(value: &str) -> Result<u16, String> {
    let port: u16 = value
        .trim()
        .parse()
        .map_err(|_| format!("--mcp-port needs 1 to 65535, got {value}"))?;
    if port == 0 {
        return Err("--mcp-port needs 1 to 65535, got 0".into());
    }
    Ok(port)
}

/// `GENOS_MCP_PORT` when it is a port, otherwise this binary's default.
fn mcp_port_from_env() -> u16 {
    std::env::var("GENOS_MCP_PORT")
        .ok()
        .and_then(|text| parse_mcp_port(&text).ok())
        .unwrap_or(MCP_PORT)
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
    fn eye_and_look_place_the_camera() {
        let mut opts = super::Options {
            scene: "stress".into(),
            checkpoint: None,
            eye: Some([1.0, 2.0, 3.0]),
            look: Some([1.0, 2.0, 1.0]),
            preset: None,
            gpu_times: None,
            lighting_debug: false,
            unfilled: false,
            full_effort: false,
            denoise_launch: None,
            denoise_screen: None,
            headless: true,
            frames: None,
            out: std::path::PathBuf::from("genos-debug"),
            out_set: false,
            size: (320, 180),
            check: false,
            mcp_port: None,
        };
        let mut camera = genos_scene::Camera::opening();
        super::place_camera(&opts, &mut camera).unwrap();
        assert!((camera.position.x - 1.0).abs() < 1.0e-5);
        assert!((camera.position.y - 2.0).abs() < 1.0e-5);
        assert!((camera.position.z - 3.0).abs() < 1.0e-5);
        // Target is 2 m down -Z from the eye. Yaw 0 looks down -Z.
        assert!(camera.yaw.abs() < 1.0e-4, "yaw {}", camera.yaw);
        assert!(camera.pitch.abs() < 1.0e-4, "pitch {}", camera.pitch);
        opts.look = None;
        camera.set_pose(genos_scene::Vec3::new(0.0, 0.0, 0.0), 0.4, -0.2);
        super::place_camera(&opts, &mut camera).unwrap();
        assert!((camera.position.x - 1.0).abs() < 1.0e-5);
        assert!((camera.yaw - 0.4).abs() < 1.0e-5);
        assert!((camera.pitch + 0.2).abs() < 1.0e-5);
        assert!(super::parse_vec3("--eye", "1, 2, 3").is_ok());
        assert!(super::parse_vec3("--eye", "1,2").is_err());
        let (flag, value) = super::placed_flag("--eye=4,5,6").unwrap();
        assert_eq!(super::parse_vec3(flag, value).unwrap(), [4.0, 5.0, 6.0]);
        let (flag, value) = super::placed_flag("--look=7,8,9").unwrap();
        assert_eq!(super::parse_vec3(flag, value).unwrap(), [7.0, 8.0, 9.0]);
        assert!(super::placed_flag("--eye").is_none());
    }

    #[test]
    fn each_setting_uses_a_dropdown_a_slider_or_a_toggle() {
        use genos_debug::Host;
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
            "Shadow cell cache",
            "Ambient fill",
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
            "Bounce count",
            "Max lights",
            "Direct refresh frames",
            "Direct scale",
            "GI refresh frames",
            "Ray budget shift",
            "Screen probe rays",
            "Probe stale frames",
            "Cache stale frames",
            "Denoiser iterations",
            "Denoiser variance",
        ] {
            assert!(sliders.contains(&label), "missing slider {label}");
        }
        assert!(!sliders.contains(&"Shadow cell cache"));
        assert!(!sliders.contains(&"Ambient fill"));
        let checks: Vec<&str> = rows
            .iter()
            .flat_map(|row| row.checks.iter().map(|check| check.text.as_str()))
            .collect();
        for label in [
            "Direct light",
            "Shadows",
            "GI",
            "Denoiser",
            "Screen probes",
            "Surface light cache",
            "Sky light",
            "Pre-fill",
            "Full effort",
            "Show",
        ] {
            assert!(checks.contains(&label), "missing toggle {label}");
        }
        let names: Vec<String> = app.knobs().into_iter().map(|knob| knob.name).collect();
        for name in [
            "lighting_bounces",
            "lighting_direct",
            "lighting_probes",
            "lighting_world_cache",
            "lighting_denoise",
            "lighting_sky",
            "lighting_prefill",
            "lighting_ambient",
            "lighting_shadow_filter",
        ] {
            assert!(names.iter().any(|knob| knob == name), "config name {name}");
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
            .find(|row| row.label == "Screen probe rays")
            .unwrap()
            .slider
            .as_ref()
            .unwrap();
        assert!(rays.steps.is_empty());
        assert_eq!(rays.step, 4.0);
        let denoise = rows
            .iter()
            .find(|row| row.label == "Denoiser iterations")
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
    fn sections_group_every_control_and_the_fold_file_round_trips() {
        let root = genos_scene_lib::repo_root();
        let entries = genos_scene_lib::load_dir(&genos_scene_lib::scenes_dir()).expect("scenes");
        let log = std::fs::File::create(std::env::temp_dir().join("genos-lighting-folds.log"))
            .expect("log");
        let app = super::App::open(entries, root, log).expect("app");
        let fps = genos_scene::FpsCounterSettings::default();
        let menu = super::load_menu_layout().expect("layout");
        let blocks = super::panel_blocks(&app, ViewMode::Full, &fps, &menu);
        let tabs = match &blocks[0] {
            genos_ui::PanelBlock::Tabs(tabs) => tabs,
            genos_ui::PanelBlock::Section(_) | genos_ui::PanelBlock::Rows(_) => {
                panic!("the menu is one tab strip")
            }
        };
        let titles: Vec<&str> = tabs.tabs.iter().map(|tab| tab.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Scene/View",
                "Pipeline",
                "Direct/Lamps",
                "GI",
                "Denoise",
                "Shadows/Ambient",
                "Perf",
                "Info"
            ]
        );
        let scene = tabs
            .tabs
            .iter()
            .find(|tab| tab.title == "Scene/View")
            .unwrap();
        assert!(matches!(scene.blocks[0], genos_ui::PanelBlock::Rows(_)));
        let direct = tabs
            .tabs
            .iter()
            .find(|tab| tab.title == "Direct/Lamps")
            .unwrap();
        assert!(direct.blocks.iter().any(|block| matches!(
            block,
            genos_ui::PanelBlock::Section(section)
                if section.title == "Lamps"
                    && !section.collapsed
                    && section.rows.iter().any(|row| row.label == "Max lights")
        )));
        let gi = tabs.tabs.iter().find(|tab| tab.title == "GI").unwrap();
        assert!(gi.blocks.iter().any(|block| matches!(
            block,
            genos_ui::PanelBlock::Section(section) if section.title == "Probes" && !section.collapsed
        )));
        assert!(gi.blocks.iter().any(|block| matches!(
            block,
            genos_ui::PanelBlock::Section(section)
                if section.title == "World cache" && section.collapsed
        )));
        let info = tabs.tabs.iter().find(|tab| tab.title == "Info").unwrap();
        assert!(matches!(
            &info.blocks[0],
            genos_ui::PanelBlock::Rows(rows) if rows[0].label == app.entry().review
        ));
        let flat = super::panel_rows(&app, ViewMode::Full, &fps);
        assert!(flat.iter().any(|row| row.label == "Bounce count"));
        assert!(flat.iter().any(|row| row.label == "Shadow cell cache"));
        assert!(flat.iter().all(|row| !row.label.contains("hides.")));
        let folds = super::menu_folds(&menu);
        assert!(folds.iter().all(|fold| fold.key != "-"));
        let mut ui = genos_ui::State::default();
        for fold in &folds {
            ui.fold_section(fold.id, fold.collapsed);
        }
        let dir = std::env::temp_dir().join(format!("genos-menu-{}", std::process::id()));
        super::save_menu_folds(&dir, &ui, &folds).expect("save");
        let names = super::load_menu_folds(&dir).expect("load");
        assert!(names.iter().any(|name| name == "world"));
        assert!(names.iter().all(|name| name != "direct" && name != "gi"));
        super::save_menu_tab(&dir, "GI").expect("tab");
        assert_eq!(super::load_menu_tab(&dir).as_deref(), Some("GI"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn help_lists_the_fps_counter() {
        assert!(super::USAGE.contains("--fps-counter"));
        assert!(super::USAGE.contains("--fps-hz"));
        assert!(super::USAGE.contains("Grave"));
        assert!(super::USAGE.contains("F1"));
        assert!(super::USAGE.contains("F toggles"));
        assert!(super::USAGE.contains("F3"));
        assert!(super::USAGE.contains("GENOS_DEBUG_KEYS"));
        assert!(super::USAGE.contains("] and ["));
        assert!(super::USAGE.contains("tab.txt"));
        assert!(super::USAGE.contains("GENOS_FPS_COUNTER"));
        assert!(super::USAGE.contains("--menu"));
        assert!(super::USAGE.contains("GENOS_MENU"));
        assert!(super::USAGE.contains("get_menu_settings"));
        assert!(super::USAGE.contains("--mcp-port"));
        assert!(super::USAGE.contains("8766"));
    }

    #[test]
    fn the_lighting_mcp_port_is_not_the_stress_port() {
        assert_eq!(super::MCP_PORT, 8766);
        assert_ne!(super::MCP_PORT, genos_mcp::PORT);
        assert_eq!(super::parse_mcp_port("8770").unwrap(), 8770);
        assert!(super::parse_mcp_port("0").is_err());
        assert!(super::parse_mcp_port("nope").is_err());
    }

    #[test]
    fn an_unseen_fold_stays_in_the_collapsed_list() {
        let mut ui = genos_ui::State::default();
        ui.fold_section(1, true);
        let folds = vec![
            super::MenuFold {
                id: 1,
                key: "direct".into(),
                collapsed: false,
            },
            super::MenuFold {
                id: 2,
                key: "world".into(),
                collapsed: true,
            },
        ];
        let merged = super::merge_collapsed(&["world".into(), "direct".into()], &ui, &folds);
        assert_eq!(merged, vec!["world".to_string(), "direct".to_string()]);
        ui.fold_section(1, false);
        let merged = super::merge_collapsed(&merged, &ui, &folds);
        assert_eq!(merged, vec!["world".to_string()]);
    }
}

#[cfg(test)]
mod fps_overlay;
