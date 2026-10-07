//! Lighting stress test: a big, fairly open building with windows, skylights and wide
//! doorways, still and moving lamps, a sun on a day/night cycle, and coloured boxes
//! that slide and spin. `--scale small|big` tiles one 25 m section 1 or 16 times so the
//! cost of size can be read as a delta; `--bench` sweeps lamp count × dynamic share ×
//! scale from a fixed viewpoint on a fixed timeline.
//!
//! Every option also reads an environment variable (`GENOS_STRESS_SCALE`,
//! `GENOS_STRESS_LIGHTS`, `GENOS_STRESS_DYNAMIC`, `GENOS_STRESS_LAYOUT`,
//! `GENOS_STRESS_DAY_SECONDS`); a flag
//! wins over the variable. The panel switches lamps, dynamic share, scale and the sun
//! live.

mod bench;
mod building;
mod png;
mod stage;

use std::path::PathBuf;
use std::time::Instant;

use genos_input::{character_controller, InputCode, InputSystem};
use genos_render::{Renderer, ScreenRect};
use genos_scene::{look_direction, update, Actions, Camera, Scene, Vec3, PITCH_LIMIT};
use genos_ui::{button_panel, Action, PanelButton, PanelRow, Pointer, State};
use genos_window::{extent_changed, FocusGate, Window};

use building::{Layout, LightMix, View};
use stage::{Scale, Stage};

const USAGE: &str = "genos-stress [options]
  --scale small|big|N|NxM   building size: small = 1 section (25 m), big = 4x4 (100 m)
  --lights N                lamps in the whole building (panel: 5/25/50/100)
  --dynamic PCT             share of lamps that move, 0-100 (panel: 0/25/50/75/100)
  --layout spread|first     lamps spread over every section, or all in section 0
                            where the benchmark camera stands (default spread;
                            first for --bench, so every scale sees the same lamps)
  --power X                 lamp total multiplier (default 1)
  --day-seconds S           seconds per day/night cycle (default 120)
  --time F                  start time as a fraction of a day: 0 sunrise, 0.25 noon,
                            0.5 sunset (default 0.12)
  --sun-speed X             day clock multiplier (default 1)
  --freeze-sun              hold the sun at --time
  --no-sky                  no sky light: only the sun and the lamps
  --still-boxes             hold the coloured boxes
  --seed N                  lamp and box layout seed (default 1)
  --view hall|roomA         start (and benchmark) viewpoint (default hall)
  --eye X Y Z YAW PITCH     start the camera here instead of --view (free look)
  --size WxH                window size (default 1280x720; 2560x1440 for --bench)
  --proof                   open the proof window (floats, takes no focus; --bench
                            always uses it, so the compositor keeps its size)
  --frames N                quit after N frames; frames step 1/60 s of scene time
  --shot PATH               settle the light on the last frame and write a PNG
  --no-panel                hide the panel
  --bench SECONDS           sweep and print tables; per configuration seconds
  --warmup FRAMES           benchmark warm-up frames per configuration (default 30)
  --sweep-lights LIST       benchmark lamp counts (default 5,25,50,100)
  --sweep-dynamic LIST      benchmark dynamic shares (default 0,25,100)
  --sweep-scales LIST       benchmark scales (default small,big)
  --bench-out PATH          also write the tables to PATH";

const LIGHT_STEPS: [usize; 4] = [5, 25, 50, 100];
const DYNAMIC_STEPS: [u32; 5] = [0, 25, 50, 75, 100];
const SPEED_STEPS: [f32; 3] = [1.0, 4.0, 16.0];
/// Panel ids. Each row's buttons count up from its base.
const ID_LIGHTS: u32 = 100;
const ID_DYNAMIC: u32 = 200;
const ID_SCALE: u32 = 300;
const ID_SUN: u32 = 400;
const ID_SPEED: u32 = 410;
const ID_BOXES: u32 = 500;
const ID_LAYOUT: u32 = 600;
const ID_SKY: u32 = 700;
/// A hitch longer than this does not replay the missed time.
const MAX_FRAME_SECONDS: f32 = 0.25;

struct Options {
    scale: Scale,
    lights: usize,
    dynamic: u32,
    layout: Layout,
    power: f32,
    day_seconds: f32,
    time: f32,
    sun_speed: f32,
    freeze_sun: bool,
    no_sky: bool,
    still_boxes: bool,
    seed: u64,
    view: View,
    eye: Option<[f32; 5]>,
    size: (u32, u32),
    proof: bool,
    frames: Option<u32>,
    shot: Option<PathBuf>,
    panel: bool,
    bench: Option<bench::Plan>,
    bench_out: Option<PathBuf>,
}

fn parse<T: std::str::FromStr>(name: &str, text: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("bad value {text} for {name}"))
}

fn list<T: std::str::FromStr>(name: &str, text: &str) -> Result<Vec<T>, String> {
    text.split(',')
        .map(|item| parse(name, item.trim()))
        .collect()
}

fn env<T: std::str::FromStr>(name: &str) -> Result<Option<T>, String> {
    match std::env::var(name) {
        Ok(text) => parse(name, &text).map(Some),
        Err(_) => Ok(None),
    }
}

fn options() -> Result<Options, String> {
    let mut opts = Options {
        scale: match std::env::var("GENOS_STRESS_SCALE") {
            Ok(text) => Scale::parse(&text)?,
            Err(_) => Scale::SMALL,
        },
        lights: env("GENOS_STRESS_LIGHTS")?.unwrap_or(5),
        dynamic: env("GENOS_STRESS_DYNAMIC")?.unwrap_or(50),
        layout: match std::env::var("GENOS_STRESS_LAYOUT") {
            Ok(text) => Layout::parse(&text).ok_or(format!("unknown layout {text}"))?,
            Err(_) => Layout::Spread,
        },
        power: 1.0,
        day_seconds: env("GENOS_STRESS_DAY_SECONDS")?.unwrap_or(120.0),
        time: 0.12,
        sun_speed: 1.0,
        freeze_sun: false,
        no_sky: false,
        still_boxes: false,
        seed: 1,
        view: View::Hall,
        eye: None,
        size: (1280, 720),
        proof: false,
        frames: None,
        shot: None,
        panel: true,
        bench: None,
        bench_out: None,
    };
    let mut seconds = None;
    let mut size_set = false;
    let mut layout_set = std::env::var("GENOS_STRESS_LAYOUT").is_ok();
    let mut warmup = 30;
    let mut sweep_lights = LIGHT_STEPS.to_vec();
    let mut sweep_dynamic = vec![0, 25, 100];
    let mut sweep_scales = vec![Scale::SMALL, Scale::BIG];
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--scale" => opts.scale = Scale::parse(&value()?)?,
            "--lights" => opts.lights = parse(&arg, &value()?)?,
            "--dynamic" => opts.dynamic = parse(&arg, &value()?)?,
            "--layout" => {
                let text = value()?;
                opts.layout = Layout::parse(&text).ok_or(format!("unknown layout {text}"))?;
                layout_set = true;
            }
            "--power" => opts.power = parse(&arg, &value()?)?,
            "--day-seconds" => opts.day_seconds = parse(&arg, &value()?)?,
            "--time" => opts.time = parse(&arg, &value()?)?,
            "--sun-speed" => opts.sun_speed = parse(&arg, &value()?)?,
            "--freeze-sun" => opts.freeze_sun = true,
            "--no-sky" => opts.no_sky = true,
            "--still-boxes" => opts.still_boxes = true,
            "--seed" => opts.seed = parse(&arg, &value()?)?,
            "--view" => {
                let text = value()?;
                opts.view = View::parse(&text).ok_or(format!("unknown view {text}"))?;
            }
            "--eye" => {
                let mut eye = [0.0; 5];
                for slot in &mut eye {
                    *slot = parse(&arg, &value()?)?;
                }
                opts.eye = Some(eye);
            }
            "--size" => {
                let text = value()?;
                let (w, h) = text.split_once('x').ok_or(format!("bad size {text}"))?;
                opts.size = (parse(&arg, w)?, parse(&arg, h)?);
                size_set = true;
            }
            "--proof" => opts.proof = true,
            "--frames" => opts.frames = Some(parse(&arg, &value()?)?),
            "--shot" => opts.shot = Some(PathBuf::from(value()?)),
            "--no-panel" => opts.panel = false,
            "--bench" => seconds = Some(parse::<f32>(&arg, &value()?)?),
            "--warmup" => warmup = parse(&arg, &value()?)?,
            "--sweep-lights" => sweep_lights = list(&arg, &value()?)?,
            "--sweep-dynamic" => sweep_dynamic = list(&arg, &value()?)?,
            "--sweep-scales" => {
                sweep_scales = value()?
                    .split(',')
                    .map(|s| Scale::parse(s.trim()))
                    .collect::<Result<_, _>>()?
            }
            "--bench-out" => opts.bench_out = Some(PathBuf::from(value()?)),
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    opts.dynamic = opts.dynamic.min(100);
    if let Some(seconds) = seconds {
        // A benchmark draws at a fixed size in a window the compositor does not tile,
        // and with the same lamps near the camera at every scale.
        opts.proof = true;
        if !size_set {
            opts.size = (2560, 1440);
        }
        if !layout_set {
            opts.layout = Layout::First;
        }
        opts.bench = Some(bench::Plan {
            seconds,
            warmup_frames: warmup,
            lights: sweep_lights,
            dynamic: sweep_dynamic.into_iter().map(|d| d.min(100)).collect(),
            scales: sweep_scales,
            day: opts.time,
            layout: opts.layout,
            size: opts.size,
        });
    }
    Ok(opts)
}

fn main() {
    if let Err(err) = run() {
        eprintln!("genos-stress: {err}");
        std::process::exit(1);
    }
}

/// A fly camera at `view`. It has no colliders: the engine's physics world holds 32
/// bodies, far fewer than the big building's walls, so the stress test flies through
/// walls instead of walking.
fn camera_at(view: View) -> Camera {
    let (x, z, yaw, pitch) = view.pose();
    let mut camera = Camera::new(x, z, yaw);
    camera.pitch = pitch;
    camera
}

/// Fly speed in metres per second; Shift is four times faster.
const FLY_SPEED: f32 = 5.0;
/// Stick look in radians per second.
const STICK_LOOK: f32 = 2.0;

/// Move the fly camera: move keys along the view on the ground plane, Space up and C
/// down. Mouse look and capture go through [`update`] with no time step, which also
/// skips its physics.
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
    if input.keyboard.down(InputCode::key_c) {
        lift -= 1.0;
    }
    let boost = if input.keyboard.down(InputCode::key_shift_left) {
        4.0
    } else {
        1.0
    };
    let step = (forward * actions.forward + right * actions.strafe + Vec3::Y * lift)
        * (FLY_SPEED * boost * dt);
    let position = camera.position + step;
    camera.set_pose(position, camera.yaw, camera.pitch);
}

fn run() -> Result<(), String> {
    let opts = options()?;
    let mix = LightMix {
        count: opts.lights,
        dynamic_pct: opts.dynamic,
        layout: opts.layout,
    };
    let mut stage = Stage::new(
        opts.scale,
        opts.seed,
        mix,
        opts.power,
        opts.time,
        opts.day_seconds,
    );
    stage.sun_speed = opts.sun_speed;
    stage.sun_frozen = opts.freeze_sun;
    stage.sky_on = !opts.no_sky;
    stage.boxes_still = opts.still_boxes;
    let mut camera = camera_at(opts.view);
    if let Some([x, y, z, yaw, pitch]) = opts.eye {
        camera.set_pose(genos_scene::Vec3::new(x, y, z), yaw, pitch);
    }
    let (width, height) = opts.size;
    let mut window = if opts.proof {
        Window::open_proof(width, height)?
    } else {
        Window::open_named(width, height, "genos-stress", "Genos lighting stress")?
    };
    let first = window.pump();
    let mut renderer = Renderer::open(window.display, window.surface, first.width, first.height)?;
    eprintln!(
        "genos-stress {} ({} sections), {} occluders, {} moving boxes, {} lamps ({} moving)",
        stage.scale.label(),
        stage.scale.sections(),
        stage.occluders(),
        stage.building.movers(),
        stage.lamp_count(),
        stage.mix.dynamic().min(stage.lamp_count()),
    );

    if let Some(plan) = &opts.bench {
        let text = bench::run(&mut renderer, &mut window, &mut stage, &camera, plan)?;
        println!("{text}");
        if let Some(path) = &opts.bench_out {
            std::fs::write(path, &text).map_err(|err| err.to_string())?;
        }
        return Ok(());
    }

    let fixed_step = opts.frames.is_some() || opts.shot.is_some();
    let mut size = (renderer.width(), renderer.height());
    let mut focus = FocusGate::default();
    let mut ui = State::default();
    let mut input = InputSystem::new();
    let controls = character_controller();
    let mut drawn = 0u32;
    let mut clock: Option<Instant> = None;
    let mut fps = FrameRate::default();
    loop {
        let now = Instant::now();
        let dt = if fixed_step {
            bench::STEP
        } else {
            clock
                .map(|then| now.saturating_duration_since(then).as_secs_f32())
                .unwrap_or(bench::STEP)
                .min(MAX_FRAME_SECONDS)
        };
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
        let move_axis = controls.axis_2d(&input, "move");
        let look_axis = controls.axis_2d(&input, "look");

        fps.note(
            dt,
            renderer
                .take_light_builds()
                .iter()
                .map(|b| b.total_ms())
                .sum(),
        );
        let (paints, look_capture) = if opts.panel {
            let ui_frame = button_panel(
                &mut ui,
                &panel_title(&stage, &fps),
                &panel_rows(&stage),
                [size.0 as f32, size.1 as f32],
                &camera,
                Pointer {
                    x: frame.pointer_x,
                    y: frame.pointer_y,
                    down: !camera.captured && (gated.mouse_left || gated.capture_click),
                },
            );
            for action in &ui_frame.actions {
                if let Action::Press(id) = action {
                    press(&mut stage, *id);
                }
            }
            (ui_frame.paints, ui_frame.look_capture)
        } else {
            (Vec::new(), true)
        };
        let overlay: Vec<ScreenRect> = paints
            .iter()
            .map(|p| ScreenRect {
                x: p.x,
                y: p.y,
                w: p.w,
                h: p.h,
                color: p.color,
            })
            .collect();

        stage.advance(dt);
        fly(
            &mut camera,
            &mut stage.world.scene,
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
                eprintln!("genos-stress: {err}");
                camera.captured = false;
            }
        }

        drawn += 1;
        let last = opts.frames.is_some_and(|limit| drawn >= limit);
        let want_read = last && opts.shot.is_some();
        let pixels =
            renderer.draw_with_overlay(&stage.world, &camera, &overlay, want_read, false)?;
        if let (Some(path), Some(pixels)) = (&opts.shot, pixels) {
            png::write_png(path, renderer.width(), renderer.height(), &pixels)?;
            let (hh, mm) = building::clock(stage.day);
            let eye = camera.position;
            eprintln!(
                "genos-stress wrote {} at {hh:02}:{mm:02}, eye {:.2} {:.2} {:.2} yaw {:.3} pitch {:.3}",
                path.display(),
                eye.x,
                eye.y,
                eye.z,
                camera.yaw,
                camera.pitch
            );
        }
        if last {
            break;
        }
    }
    println!("genos-stress frames={drawn}");
    Ok(())
}

/// Frame rate and light build time over the last half second, for the panel title.
#[derive(Default)]
struct FrameRate {
    seconds: f32,
    frames: u32,
    light_ms: f64,
    fps: f32,
    light_per_frame: f64,
}

impl FrameRate {
    fn note(&mut self, dt: f32, light_ms: f64) {
        self.seconds += dt;
        self.frames += 1;
        self.light_ms += light_ms;
        if self.seconds >= 0.5 {
            self.fps = self.frames as f32 / self.seconds;
            self.light_per_frame = self.light_ms / self.frames as f64;
            *self = Self {
                fps: self.fps,
                light_per_frame: self.light_per_frame,
                ..Self::default()
            };
        }
    }
}

fn panel_title(stage: &Stage, rate: &FrameRate) -> String {
    let (hh, mm) = building::clock(stage.day);
    format!(
        "Stress {} {:.0} fps light {:.1} ms {hh:02}:{mm:02}",
        stage.scale.label(),
        rate.fps,
        rate.light_per_frame
    )
}

fn row(label: &str, buttons: Vec<PanelButton>) -> PanelRow {
    PanelRow {
        label: label.into(),
        buttons,
    }
}

fn button(id: u32, text: String, selected: bool) -> PanelButton {
    PanelButton { id, text, selected }
}

fn panel_rows(stage: &Stage) -> Vec<PanelRow> {
    vec![
        row(
            "Lights",
            LIGHT_STEPS
                .iter()
                .enumerate()
                .map(|(i, n)| button(ID_LIGHTS + i as u32, n.to_string(), stage.mix.count == *n))
                .collect(),
        ),
        row(
            "Dynamic",
            DYNAMIC_STEPS
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    button(
                        ID_DYNAMIC + i as u32,
                        format!("{d}%"),
                        stage.mix.dynamic_pct == *d,
                    )
                })
                .collect(),
        ),
        row(
            "Lamps in",
            vec![
                button(ID_LAYOUT, "all".into(), stage.mix.layout == Layout::Spread),
                button(
                    ID_LAYOUT + 1,
                    "section 0".into(),
                    stage.mix.layout == Layout::First,
                ),
            ],
        ),
        row(
            "Scale",
            vec![
                button(ID_SCALE, "small".into(), stage.scale == Scale::SMALL),
                button(ID_SCALE + 1, "big".into(), stage.scale == Scale::BIG),
            ],
        ),
        row(
            "Sun",
            vec![
                button(ID_SUN, "run".into(), !stage.sun_frozen),
                button(ID_SUN + 1, "freeze".into(), stage.sun_frozen),
            ],
        ),
        row(
            "Sky",
            vec![
                button(ID_SKY, "on".into(), stage.sky_on),
                button(ID_SKY + 1, "off".into(), !stage.sky_on),
            ],
        ),
        row(
            "Sun speed",
            SPEED_STEPS
                .iter()
                .enumerate()
                .map(|(i, s)| button(ID_SPEED + i as u32, format!("x{s}"), stage.sun_speed == *s))
                .collect(),
        ),
        row(
            "Boxes",
            vec![
                button(ID_BOXES, "move".into(), !stage.boxes_still),
                button(ID_BOXES + 1, "still".into(), stage.boxes_still),
            ],
        ),
    ]
}

fn press(stage: &mut Stage, id: u32) {
    let pick = |base: u32, len: usize| {
        (base..base + len as u32)
            .contains(&id)
            .then(|| (id - base) as usize)
    };
    if let Some(i) = pick(ID_LIGHTS, LIGHT_STEPS.len()) {
        stage.mix.count = LIGHT_STEPS[i];
    } else if let Some(i) = pick(ID_DYNAMIC, DYNAMIC_STEPS.len()) {
        stage.mix.dynamic_pct = DYNAMIC_STEPS[i];
    } else if let Some(i) = pick(ID_SCALE, 2) {
        stage.set_scale([Scale::SMALL, Scale::BIG][i]);
    } else if let Some(i) = pick(ID_SUN, 2) {
        stage.sun_frozen = i == 1;
    } else if let Some(i) = pick(ID_SKY, 2) {
        stage.sky_on = i == 0;
    } else if let Some(i) = pick(ID_SPEED, SPEED_STEPS.len()) {
        stage.sun_speed = SPEED_STEPS[i];
    } else if let Some(i) = pick(ID_BOXES, 2) {
        stage.boxes_still = i == 1;
    } else if let Some(i) = pick(ID_LAYOUT, 2) {
        stage.mix.layout = [Layout::Spread, Layout::First][i];
    }
    stage.apply();
}
