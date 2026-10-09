//! The frame side of the debug tools: runs queued commands against the renderer,
//! the scene and the camera, holds edits the example's own update would undo, and
//! answers waits and captures after the frames they need.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use genos_mcp::json::Value;
use genos_render::{
    Bounces, BrickReport, BrickState, DebugView, LightBuildTimes, Renderer, ScreenRect, ViewMode,
};
use genos_scene::{Camera, Light, Scene, Vec3, PITCH_LIMIT};

use crate::image::{Flicker, Image, Region};
use crate::knobs::{lighting_knobs, set_lighting_knob, Host, Knob};
use crate::reference::{self, RefSetup, Reference};
use crate::value::{list, num, obj, text, vec3, Args};

/// Scene seconds per frame when time runs on a fixed step.
pub const DEFAULT_STEP: f32 = 1.0 / 60.0;

/// What the example hands the tools before it draws.
pub struct Ctx<'a> {
    pub renderer: &'a mut Renderer,
    pub camera: &'a mut Camera,
    pub host: &'a mut dyn Host,
}

/// Where the frame's time went, as the example measured it.
#[derive(Clone, Debug, Default)]
pub struct FrameTiming {
    /// Wall time since the last frame began.
    pub frame_ms: f64,
    /// CPU time of the draw call.
    pub draw_ms: f64,
    /// GPU time of the picture, when the example profiles it.
    pub gpu_ms: Option<f64>,
    /// Light builds that finished this frame.
    pub light: Vec<LightBuildTimes>,
}

/// How probe markers are coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeColor {
    /// Unlit grey, changing red, refining yellow, steady green.
    State,
    /// Green when just updated, red at 4 s or older.
    Age,
    /// Blue to red by priority, against the highest on screen.
    Priority,
    /// Probe spacing: blue fine to red coarse.
    Spacing,
    /// Red while changing; everything else dim.
    Changing,
}

impl ProbeColor {
    const ALL: [(&'static str, ProbeColor); 5] = [
        ("state", Self::State),
        ("age", Self::Age),
        ("priority", Self::Priority),
        ("spacing", Self::Spacing),
        ("changing", Self::Changing),
    ];

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().find(|(n, _)| *n == text).map(|(_, c)| *c)
    }

    fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, c)| *c == self)
            .map_or("?", |(n, _)| n)
    }
}

/// What a picture shows: a debug view of the faces and, optionally, probe markers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub view: DebugView,
    pub probes: Option<ProbeColor>,
}

impl Look {
    /// `full`, `direct`, `bounce`, `nobounce`, `bounce:N` / `full:N` (bounce limit,
    /// `inf` for all), `near`, `far`, `albedo`, `normal`, `depth`, `light:I`,
    /// `probes:<state|age|priority|spacing|changing>`.
    pub fn parse(spec: &str, base: Bounces) -> Result<Self, String> {
        let (head, arg) = match spec.split_once(':') {
            Some((h, a)) => (h.trim(), Some(a.trim())),
            None => (spec.trim(), None),
        };
        let mut look = Look {
            view: DebugView {
                bounces: base,
                ..DebugView::default()
            },
            probes: None,
        };
        let limit = |a: &str| -> Result<Bounces, String> {
            match a {
                "inf" | "all" => Ok(Bounces::All),
                n => n
                    .parse::<u32>()
                    .map(|n| Bounces::from_limit(Some(n)))
                    .map_err(|_| format!("bounce limit is 0, 1, 2 or inf, got {n}")),
            }
        };
        match head {
            "nobounce" => look.view.bounces = Bounces::Zero,
            "probes" => {
                look.probes = Some(
                    ProbeColor::parse(arg.unwrap_or("state"))
                        .ok_or_else(|| format!("unknown probe colouring {}", arg.unwrap_or("")))?,
                );
                return Ok(look);
            }
            _ => {
                look.view.mode = ViewMode::parse(head).ok_or_else(|| {
                    format!(
                        "unknown view {head}: use full, direct, bounce, nobounce, near, far, albedo, normal, depth, light:I, probes:COLOR"
                    )
                })?;
                if let Some(a) = arg {
                    if look.view.mode == ViewMode::Light {
                        look.view.lamp = a
                            .parse()
                            .map_err(|_| format!("light:{a} needs a lamp index"))?;
                    } else {
                        look.view.bounces = limit(a)?;
                    }
                }
            }
        }
        Ok(look)
    }

    pub fn name(&self) -> String {
        if let Some(p) = self.probes {
            return format!("probes:{}", p.name());
        }
        match self.view.mode {
            ViewMode::Light => format!("light:{}", self.view.lamp),
            mode => match self.view.bounces.limit() {
                None => mode.name().to_string(),
                Some(n) => format!("{}:{n}", mode.name()),
            },
        }
    }
}

#[derive(Clone, Debug)]
struct Path {
    points: Vec<Vec3>,
    frames: u32,
    start: u64,
    looped: bool,
}

impl Path {
    fn at(&self, frame: u64) -> Vec3 {
        let n = self.points.len();
        if n == 1 {
            return self.points[0];
        }
        let mut t = (frame.saturating_sub(self.start)) as f32 / self.frames.max(1) as f32;
        t = if self.looped { t.fract() } else { t.min(1.0) };
        let f = t * (n - 1) as f32;
        let i = (f.floor() as usize).min(n - 2);
        let s = f - i as f32;
        self.points[i] + (self.points[i + 1] - self.points[i]) * s
    }
}

#[derive(Clone, Debug, Default)]
struct LightEdit {
    off: bool,
    color: Option<[f32; 3]>,
    intensity: Option<f32>,
    position: Option<Vec3>,
    path: Option<Path>,
}

impl LightEdit {
    fn apply(&self, light: &mut Light, frame: u64) {
        if let Some(path) = &self.path {
            light.position = path.at(frame);
        } else if let Some(p) = self.position {
            light.position = p;
        }
        if let Some(c) = self.color {
            light.color = c;
        }
        if let Some(k) = self.intensity {
            light.color = light.color.map(|c| c * k);
        }
        if self.off {
            light.color = [0.0; 3];
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ObjectEdit {
    position: Option<Vec3>,
    yaw: Option<f32>,
    /// Radians per second added on top of the example's turn.
    spin: f32,
    spun: f32,
    frozen: Option<(Vec3, f32)>,
    path: Option<Path>,
}

/// A picture a job wants this frame.
#[derive(Clone, Copy, Debug)]
struct Shot {
    look: Look,
    /// Wait for the light to settle before reading (a picture the screen never shows).
    settle: bool,
}

enum Then {
    /// Save each picture; reply with paths and luminance.
    Save {
        paths: Vec<Option<PathBuf>>,
        names: Vec<Option<String>>,
        size: Option<(u32, u32)>,
        region: Region,
    },
    /// Diff the two pictures.
    Diff {
        out: Option<PathBuf>,
        threshold: u8,
        gain: f32,
        region: Region,
    },
}

enum Job {
    Frames {
        left: u32,
    },
    Settle {
        left: u32,
        quiet: u32,
        need: u32,
        strict: bool,
        frames: u32,
    },
    Shots {
        shots: Vec<Shot>,
        got: Vec<Image>,
        then: Then,
    },
    /// Trace a ground-truth reference off the frame thread, then (optionally)
    /// compare a live picture with it.
    Reference(Box<RefJob>),
    Flicker {
        left: u32,
        shot: Shot,
        flicker: Flicker,
        heatmap: Option<PathBuf>,
        frames_dir: Option<PathBuf>,
        index: u32,
    },
}

struct RefJob {
    /// Where traces are kept for reuse (None: always trace).
    cache: Option<PathBuf>,
    spp: u32,
    max_spp: u32,
    noise_target: f32,
    seconds: f32,
    block: u32,
    max_bounces: u32,
    /// Trace the scene's triangles instead of its analytic shapes.
    triangles: bool,
    size: (u32, u32),
    worker: Option<std::thread::JoinHandle<Reference>>,
    reference: Option<Reference>,
    /// Frames the live picture must wait after the trace starts.
    frames_left: u32,
    /// None: only trace (the `reference` command).
    shot: Option<Shot>,
    live: Option<Image>,
    name: String,
    prefix: Option<String>,
    grid: (u32, u32),
    region: Region,
    paused: bool,
}

struct Running {
    id: u64,
    job: Job,
}

/// The debug tools for one picture. See `docs/systems/debugging.md`.
pub struct Tools {
    frame: u64,
    paused: bool,
    fixed_dt: Option<f32>,
    steps_left: u32,
    dt: f32,
    look: Look,
    probes_visible_only: bool,
    lights: HashMap<usize, LightEdit>,
    added: Vec<(Light, LightEdit)>,
    solo: Option<usize>,
    objects: HashMap<usize, ObjectEdit>,
    camera_path: Option<(Path, Vec<(f32, f32)>)>,
    jobs: Vec<Running>,
    /// Shot read this frame, if any.
    reading: Option<Shot>,
    captures: HashMap<String, Image>,
    references: HashMap<String, Reference>,
    timing: VecDeque<(u64, FrameTiming)>,
    overlay: Vec<ScreenRect>,
    quit: Option<i32>,
    out_dir: PathBuf,
    base_lights: usize,
}

impl Default for Tools {
    fn default() -> Self {
        Self::new()
    }
}

impl Tools {
    /// Register the commands on the MCP port and start with the picture as it is.
    pub fn new() -> Self {
        genos_mcp::command::register(crate::commands::specs());
        Self {
            frame: 0,
            paused: false,
            fixed_dt: None,
            steps_left: 0,
            dt: 0.0,
            look: Look {
                view: DebugView::default(),
                probes: None,
            },
            probes_visible_only: true,
            lights: HashMap::new(),
            added: Vec::new(),
            solo: None,
            objects: HashMap::new(),
            camera_path: None,
            jobs: Vec::new(),
            reading: None,
            captures: HashMap::new(),
            references: HashMap::new(),
            timing: VecDeque::new(),
            overlay: Vec::new(),
            quit: None,
            out_dir: genos_mcp::discovery_dir().join("shots"),
            base_lights: 0,
        }
    }

    /// Where captures without a path go.
    pub fn set_out_dir(&mut self, dir: PathBuf) {
        self.out_dir = dir;
    }

    /// Run scene time on a fixed step from the start (a script or a test).
    pub fn set_fixed_step(&mut self, dt: Option<f32>) {
        self.fixed_dt = dt;
    }

    /// Scene seconds this frame should advance: 0 while paused (unless stepping),
    /// the fixed step when set, else `real_dt`.
    pub fn scene_dt(&mut self, real_dt: f32) -> f32 {
        let step = self.fixed_dt.unwrap_or(real_dt);
        self.dt = if self.steps_left > 0 {
            self.steps_left -= 1;
            self.fixed_dt.unwrap_or(DEFAULT_STEP)
        } else if self.paused {
            0.0
        } else {
            step
        };
        self.dt
    }

    /// A `quit` command asked the example to stop, with this exit code.
    pub fn quit_code(&self) -> Option<i32> {
        self.quit
    }

    /// Ask the example to stop.
    pub fn request_quit(&mut self, code: i32) {
        self.quit = Some(code);
    }

    /// Rectangles the tools draw over the picture (probe markers).
    pub fn overlay(&self) -> &[ScreenRect] {
        &self.overlay
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Run queued commands, put back the edits the example's update undid, and set
    /// the view. True when the example must read the picture back this frame.
    pub fn before_draw(&mut self, ctx: &mut Ctx) -> bool {
        for cmd in genos_mcp::command::take() {
            let args = Args(&cmd.args);
            match self.run(&cmd.name, &args, ctx) {
                Ok(Step::Reply(value)) => genos_mcp::command::reply(cmd.id, Ok(value)),
                Ok(Step::Job(job)) => self.jobs.push(Running { id: cmd.id, job }),
                Err(err) => genos_mcp::command::reply(cmd.id, Err(err)),
            }
        }
        self.apply_edits(ctx.host.scene());
        self.apply_camera_path(ctx.camera);
        for running in &mut self.jobs {
            if let Job::Reference(job) = &mut running.job {
                if job.worker.is_none() && job.reference.is_none() {
                    let setup = RefSetup {
                        max_bounces: job.max_bounces,
                        triangles: job.triangles,
                        max_spp: job.max_spp.max(job.spp),
                        noise_target: job.noise_target,
                        seconds: job.seconds,
                        ..RefSetup::from_camera(
                            ctx.host.scene(),
                            ctx.camera,
                            job.size.0,
                            job.size.1,
                            job.spp,
                        )
                    };
                    let cache = job.cache.clone();
                    job.worker = Some(std::thread::spawn(move || match cache {
                        Some(dir) => reference::render_cached(&setup, &dir),
                        None => reference::render(&setup),
                    }));
                }
            }
        }
        self.reading = self.jobs.iter().find_map(|r| match &r.job {
            Job::Shots { shots, got, .. } => shots.get(got.len()).copied(),
            Job::Flicker { shot, .. } => Some(*shot),
            Job::Reference(job)
                if job.reference.is_some() && job.frames_left == 0 && job.live.is_none() =>
            {
                job.shot
            }
            _ => None,
        });
        let look = self.reading.map_or(self.look, |s| s.look);
        let mut view = look.view;
        // The bounce limit is a lighting setting; a look keeps it unless it names one.
        if self.reading.is_none() {
            view.bounces = ctx.renderer.lighting_config().bounces;
        }
        ctx.renderer.set_debug_view(view);
        ctx.renderer
            .set_live_readback(!self.reading.is_some_and(|s| s.settle));
        self.overlay = match look.probes {
            Some(color) => probe_markers(
                &ctx.renderer.probe_report(),
                ctx.camera,
                (ctx.renderer.width(), ctx.renderer.height()),
                color,
                self.probes_visible_only,
            ),
            None => Vec::new(),
        };
        self.reading.is_some()
    }

    /// Feed the frame's picture and stats to the waiting jobs and answer the done ones.
    pub fn after_draw(
        &mut self,
        renderer: &mut Renderer,
        pixels: Option<&[u8]>,
        timing: FrameTiming,
    ) {
        self.frame += 1;
        self.timing.push_back((self.frame, timing));
        while self.timing.len() > 240 {
            self.timing.pop_front();
        }
        let image = match (self.reading, pixels) {
            (Some(_), Some(px)) => Some(Image::from_bgra(renderer.width(), renderer.height(), px)),
            _ => None,
        };
        let stats = renderer.tier_stats();
        // Every brick in view settled; bricks out of view wait their turn.
        let settled_now = stats.seen_bricks > 0 && stats.seen_settled == stats.seen_bricks;
        let idle = stats.pending_bricks == 0;
        let mut image_used = false;
        let mut done = Vec::new();
        for (index, running) in self.jobs.iter_mut().enumerate() {
            let finished: Option<Result<Value, String>> = match &mut running.job {
                Job::Frames { left } => {
                    *left = left.saturating_sub(1);
                    (*left == 0).then(|| Ok(obj(vec![("frame", num(self.frame as f64))])))
                }
                Job::Settle {
                    left,
                    quiet,
                    need,
                    strict,
                    frames,
                } => {
                    *frames += 1;
                    let calm = if *strict { idle } else { settled_now };
                    *quiet = if calm { *quiet + 1 } else { 0 };
                    *left = left.saturating_sub(1);
                    if *quiet >= *need || *left == 0 {
                        Some(Ok(obj(vec![
                            ("settled", Value::Bool(*quiet >= *need)),
                            ("frames", num(*frames as f64)),
                            ("pending", num(stats.pending_bricks as f64)),
                            ("changing", num(stats.changing_bricks as f64)),
                            ("seen", num(stats.seen_bricks as f64)),
                            ("seen_settled", num(stats.seen_settled as f64)),
                        ])))
                    } else {
                        None
                    }
                }
                Job::Shots { shots, got, then } => {
                    if !image_used && got.len() < shots.len() {
                        if let Some(img) = &image {
                            got.push(img.clone());
                            image_used = true;
                        }
                    }
                    (got.len() == shots.len())
                        .then(|| finish_shots(shots, got, then, &mut self.captures))
                }
                Job::Reference(job) => {
                    job.frames_left = job.frames_left.saturating_sub(1);
                    if job.worker.as_ref().is_some_and(|w| w.is_finished()) {
                        job.reference = job.worker.take().and_then(|w| w.join().ok());
                        if job.reference.is_none() {
                            job.shot = None;
                        }
                    }
                    if !image_used
                        && job.reference.is_some()
                        && job.frames_left == 0
                        && job.live.is_none()
                        && job.shot.is_some()
                    {
                        if let Some(img) = &image {
                            job.live = Some(img.clone());
                            image_used = true;
                        }
                    }
                    let ready =
                        job.reference.is_some() && (job.shot.is_none() || job.live.is_some());
                    let failed =
                        job.worker.is_none() && job.reference.is_none() && job.shot.is_none();
                    if failed {
                        Some(Err("the reference trace panicked".to_string()))
                    } else if ready {
                        Some(finish_reference(
                            job,
                            &self.out_dir,
                            &mut self.captures,
                            &mut self.references,
                        ))
                    } else {
                        None
                    }
                }
                Job::Flicker {
                    left,
                    flicker,
                    heatmap,
                    frames_dir,
                    index,
                    ..
                } => {
                    if let Some(img) = &image {
                        flicker.add(img);
                        if let Some(dir) = frames_dir {
                            let _ = img.save(&dir.join(format!("frame-{index:04}.png")));
                        }
                        *index += 1;
                        *left = left.saturating_sub(1);
                    }
                    (*left == 0).then(|| {
                        let report = flicker.report();
                        if let Some(path) = heatmap {
                            flicker.heatmap(0.1).save(path)?;
                        }
                        Ok(obj(vec![
                            ("frames", num(report.frames as f64)),
                            ("mean_delta", num(report.mean_delta as f64)),
                            ("max_delta", num(report.max_delta as f64)),
                            ("pops", num(report.pops as f64)),
                            ("popped_share", num(report.popped_share as f64)),
                            ("mean_range", num(report.mean_range as f64)),
                            (
                                "means",
                                list(report.means.iter().map(|m| num(*m as f64)).collect()),
                            ),
                            (
                                "heatmap",
                                heatmap
                                    .as_ref()
                                    .map_or(Value::Null, |p| text(p.display().to_string())),
                            ),
                        ]))
                    })
                }
            };
            if let Some(result) = finished {
                done.push((index, result));
            }
        }
        for (index, result) in done.into_iter().rev() {
            let running = self.jobs.remove(index);
            genos_mcp::command::reply(running.id, result);
        }
        if self.reading.is_some() {
            renderer.set_live_readback(false);
        }
    }

    fn apply_edits(&mut self, scene: &mut Scene) {
        self.base_lights = scene.lights.len();
        for (index, edit) in &self.lights {
            if let Some(light) = scene.lights.get_mut(*index) {
                edit.apply(light, self.frame);
            }
        }
        for (light, edit) in &self.added {
            let mut light = light.clone();
            edit.apply(&mut light, self.frame);
            scene.lights.push(light);
        }
        if let Some(keep) = self.solo {
            for (i, light) in scene.lights.iter_mut().enumerate() {
                if i != keep {
                    light.color = [0.0; 3];
                }
            }
        }
        let dt = self.dt;
        for (index, edit) in self.objects.iter_mut() {
            let Some(solid) = scene.solids.get_mut(*index) else {
                continue;
            };
            if let Some((p, yaw)) = edit.frozen {
                solid.position = p;
                solid.yaw = yaw;
                continue;
            }
            if let Some(path) = &edit.path {
                solid.position = path.at(self.frame);
            } else if let Some(p) = edit.position {
                solid.position = p;
            }
            if let Some(yaw) = edit.yaw {
                solid.yaw = yaw;
            }
            edit.spun += edit.spin * dt;
            solid.yaw += edit.spun;
        }
    }

    fn apply_camera_path(&mut self, camera: &mut Camera) {
        let Some((path, turns)) = &self.camera_path else {
            return;
        };
        let p = path.at(self.frame);
        let n = turns.len();
        let t =
            ((self.frame.saturating_sub(path.start)) as f32 / path.frames.max(1) as f32).min(1.0);
        let f = t * (n.saturating_sub(1)) as f32;
        let i = (f.floor() as usize).min(n.saturating_sub(2));
        let s = f - i as f32;
        let (yaw, pitch) = if n >= 2 {
            (
                turns[i].0 + (turns[i + 1].0 - turns[i].0) * s,
                turns[i].1 + (turns[i + 1].1 - turns[i].1) * s,
            )
        } else {
            turns[0]
        };
        camera.set_pose(p, yaw, pitch);
        if t >= 1.0 {
            self.camera_path = None;
        }
    }

    /// All knobs: the example's, then the lighting ones.
    pub fn knobs(&self, renderer: &Renderer, host: &dyn Host) -> Vec<Knob> {
        let mut out = host.knobs();
        out.extend(lighting_knobs(&renderer.lighting_config()));
        out
    }

    /// Set one knob by name, snapped to its step.
    pub fn set_knob(
        &mut self,
        renderer: &mut Renderer,
        host: &mut dyn Host,
        name: &str,
        value: f64,
    ) -> Result<f64, String> {
        let all = self.knobs(renderer, host);
        let knob = all
            .iter()
            .find(|k| k.name == name)
            .ok_or_else(|| format!("unknown knob {name}"))?;
        let v = knob.snap(value);
        if name.starts_with("lighting.") {
            let mut config = renderer.lighting_config();
            set_lighting_knob(&mut config, name, v)?;
            renderer.set_lighting_config(config);
        } else {
            host.set_knob(name, v)?;
        }
        Ok(v)
    }

    fn shot_from(&self, args: &Args, ctx: &Ctx) -> Result<Shot, String> {
        let base = ctx.renderer.lighting_config().bounces;
        let look = match args.str("mode")? {
            Some(m) => Look::parse(&m, base)?,
            None => self.look,
        };
        Ok(Shot {
            look,
            settle: args.bool("settle")?.unwrap_or(false),
        })
    }

    fn path_arg(
        &self,
        args: &Args,
        key: &str,
        default_name: Option<&str>,
    ) -> Result<Option<PathBuf>, String> {
        Ok(match args.str(key)? {
            Some(p) => {
                let p = PathBuf::from(p);
                Some(if p.is_absolute() {
                    p
                } else {
                    self.out_dir.join(p)
                })
            }
            None => default_name.map(|n| self.out_dir.join(n)),
        })
    }

    fn run(&mut self, name: &str, args: &Args, ctx: &mut Ctx) -> Result<Step, String> {
        let reply = |v: Value| Ok(Step::Reply(v));
        match name {
            "status" => {
                let stats = ctx.renderer.tier_stats();
                reply(obj(vec![
                    ("frame", num(self.frame as f64)),
                    ("paused", Value::Bool(self.paused)),
                    (
                        "fixed_dt",
                        self.fixed_dt.map_or(Value::Null, |d| num(d as f64)),
                    ),
                    ("view", text(self.look.name())),
                    (
                        "size",
                        list(vec![
                            num(ctx.renderer.width() as f64),
                            num(ctx.renderer.height() as f64),
                        ]),
                    ),
                    ("camera", camera_value(ctx.camera)),
                    (
                        "stable",
                        Value::Bool(stats.pending_bricks == 0 && stats.changing_bricks == 0),
                    ),
                    ("bricks", num(stats.bricks as f64)),
                    ("changing", num(stats.changing_bricks as f64)),
                    ("pending", num(stats.pending_bricks as f64)),
                    ("seen", num(stats.seen_bricks as f64)),
                    ("seen_settled", num(stats.seen_settled as f64)),
                ]))
            }
            "lighting" => {
                for knob in lighting_knobs(&ctx.renderer.lighting_config()) {
                    let key = knob.name.trim_start_matches("lighting.").to_string();
                    if let Some(v) = args.f64(&key)? {
                        self.set_knob(ctx.renderer, ctx.host, &knob.name, v)?;
                    } else if let Some(t) = args.str(&key)? {
                        let v = match (key.as_str(), t.as_str()) {
                            ("bounces", "inf") => 3.0,
                            ("spacing", "auto") => 0.0,
                            ("near_rays", "shader") => -1.0,
                            _ => knob.parse(&t)?,
                        };
                        self.set_knob(ctx.renderer, ctx.host, &knob.name, v)?;
                    }
                }
                let c = ctx.renderer.lighting_config();
                reply(Value::Object(
                    lighting_knobs(&c)
                        .into_iter()
                        .map(|k| {
                            (
                                k.name.trim_start_matches("lighting.").to_string(),
                                knob_value(&k),
                            )
                        })
                        .collect(),
                ))
            }
            "knobs" => {
                let all = self.knobs(ctx.renderer, ctx.host);
                if let (Some(n), Some(v)) = (args.str("name")?, args.0.get("value")) {
                    let knob = all
                        .iter()
                        .find(|k| k.name == n)
                        .ok_or_else(|| format!("unknown knob {n}"))?;
                    let value = match v.as_f64() {
                        Some(f) => f,
                        None => knob.parse(v.as_str().unwrap_or(""))?,
                    };
                    self.set_knob(ctx.renderer, ctx.host, &n, value)?;
                }
                reply(list(
                    self.knobs(ctx.renderer, ctx.host)
                        .iter()
                        .map(|k| {
                            obj(vec![
                                ("name", text(k.name.clone())),
                                ("value", num(k.value)),
                                ("text", text(k.text.clone())),
                                ("min", num(k.min)),
                                ("max", num(k.max)),
                                ("step", num(k.step)),
                                (
                                    "choices",
                                    list(k.choices.iter().map(|c| text(c.clone())).collect()),
                                ),
                            ])
                        })
                        .collect(),
                ))
            }
            "probes" => reply(probes_value(
                &ctx.renderer.probe_report(),
                args,
                ctx.renderer,
            )?),
            "lights" => {
                let report = ctx.renderer.lamp_report(ctx.host.scene(), ctx.camera);
                reply(list(
                    report
                        .iter()
                        .map(|l| {
                            let edit = self.lights.get(&l.index);
                            obj(vec![
                                ("index", num(l.index as f64)),
                                ("added", Value::Bool(l.index >= self.base_lights)),
                                (
                                    "on",
                                    Value::Bool(
                                        !edit.is_some_and(|e| e.off)
                                            && self.solo.is_none_or(|s| s == l.index),
                                    ),
                                ),
                                ("directional", Value::Bool(l.directional)),
                                ("position", vec3(l.position)),
                                ("color", vec3(l.color)),
                                ("range", num(l.range as f64)),
                                ("distance", num(l.distance as f64)),
                                ("in_view", Value::Bool(l.in_view)),
                                ("screen_share", num(l.screen_share as f64)),
                                ("impact", num(l.impact as f64)),
                            ])
                        })
                        .collect(),
                ))
            }
            "light" => self.light_cmd(args, ctx).map(Step::Reply),
            "objects" => reply(list(
                ctx.host
                    .scene()
                    .solids
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        obj(vec![
                            ("index", num(i as f64)),
                            ("position", vec3([s.position.x, s.position.y, s.position.z])),
                            ("yaw", num(s.yaw as f64)),
                            ("size", num(s.size as f64)),
                            ("height", num(s.height as f64)),
                            ("color", vec3(s.color)),
                            ("edited", Value::Bool(self.objects.contains_key(&i))),
                        ])
                    })
                    .collect(),
            )),
            "object" => {
                let index = args.u32("index")?.ok_or("object needs index")? as usize;
                let solid = ctx
                    .host
                    .scene()
                    .solids
                    .get(index)
                    .ok_or_else(|| format!("no object {index}"))?
                    .clone();
                if args.bool("reset")? == Some(true) {
                    self.objects.remove(&index);
                    return reply(obj(vec![("index", num(index as f64))]));
                }
                let edit = self.objects.entry(index).or_default();
                if let Some(p) = args.vec3("position")? {
                    edit.position = Some(p);
                    edit.path = None;
                }
                if let Some(points) = args.points("path")? {
                    edit.path = Some(Path {
                        points,
                        frames: args.u32("frames")?.unwrap_or(60),
                        start: self.frame,
                        looped: args.bool("loop")?.unwrap_or(false),
                    });
                }
                if let Some(y) = args.f32("yaw")? {
                    edit.yaw = Some(y);
                    edit.spun = 0.0;
                }
                if let Some(s) = args.f32("spin")? {
                    edit.spin = s;
                }
                match args.bool("freeze")? {
                    Some(true) => edit.frozen = Some((solid.position, solid.yaw)),
                    Some(false) => edit.frozen = None,
                    None => {}
                }
                reply(obj(vec![
                    ("index", num(index as f64)),
                    (
                        "position",
                        vec3([solid.position.x, solid.position.y, solid.position.z]),
                    ),
                    ("yaw", num(solid.yaw as f64)),
                ]))
            }
            "camera" => {
                let cam = &mut *ctx.camera;
                let mut pos = cam.position;
                let mut yaw = cam.yaw;
                let mut pitch = cam.pitch;
                if let Some(p) = args.vec3("position")? {
                    pos = p;
                }
                if let Some(y) = args.f32("yaw")? {
                    yaw = y;
                }
                if let Some(p) = args.f32("pitch")? {
                    pitch = p;
                }
                if let Some(target) = args.vec3("look_at")? {
                    (yaw, pitch) = yaw_pitch(target - pos);
                }
                if let Some(points) = args.0.get("path").and_then(Value::as_array) {
                    // [[x, y, z, yaw, pitch], ...] over `frames`.
                    let mut ps = Vec::new();
                    let mut turns = Vec::new();
                    for item in points {
                        let c: Vec<f32> = item
                            .as_array()
                            .unwrap_or(&[])
                            .iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect();
                        if c.len() != 5 {
                            return Err("camera path is a list of [x, y, z, yaw, pitch]".into());
                        }
                        ps.push(Vec3::new(c[0], c[1], c[2]));
                        turns.push((c[3], c[4]));
                    }
                    if ps.is_empty() {
                        return Err("camera path is empty".into());
                    }
                    self.camera_path = Some((
                        Path {
                            points: ps,
                            frames: args.u32("frames")?.unwrap_or(60),
                            start: self.frame,
                            looped: false,
                        },
                        turns,
                    ));
                } else {
                    cam.set_pose(pos, yaw, pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT));
                }
                reply(camera_value(cam))
            }
            "time" => {
                if let Some(p) = args.bool("pause")? {
                    self.paused = p;
                }
                if let Some(d) = args.f64("fixed_dt")? {
                    self.fixed_dt = (d > 0.0).then_some(d as f32);
                }
                if args.bool("real_time")? == Some(true) {
                    self.fixed_dt = None;
                }
                reply(obj(vec![
                    ("paused", Value::Bool(self.paused)),
                    (
                        "fixed_dt",
                        self.fixed_dt.map_or(Value::Null, |d| num(d as f64)),
                    ),
                    ("frame", num(self.frame as f64)),
                ]))
            }
            "step" => {
                let n = args.u32("frames")?.unwrap_or(1).max(1);
                self.steps_left += n;
                Ok(Step::Job(Job::Frames { left: n }))
            }
            "wait" => Ok(Step::Job(Job::Frames {
                left: args.u32("frames")?.unwrap_or(1).max(1),
            })),
            "wait_settled" => Ok(Step::Job(Job::Settle {
                left: args.u32("timeout_frames")?.unwrap_or(600).max(1),
                quiet: 0,
                need: args.u32("quiet_frames")?.unwrap_or(3).max(1),
                strict: args.bool("strict")?.unwrap_or(false),
                frames: 0,
            })),
            "view" => {
                if let Some(m) = args.str("mode")? {
                    self.look = Look::parse(&m, ctx.renderer.lighting_config().bounces)?;
                    if self.look.view.bounces != ctx.renderer.lighting_config().bounces
                        && m.contains(':')
                        && self.look.view.mode != ViewMode::Light
                    {
                        let mut c = ctx.renderer.lighting_config();
                        c.bounces = self.look.view.bounces;
                        ctx.renderer.set_lighting_config(c);
                    }
                    if m == "nobounce" {
                        let mut c = ctx.renderer.lighting_config();
                        c.bounces = Bounces::Zero;
                        ctx.renderer.set_lighting_config(c);
                    }
                }
                if let Some(v) = args.bool("visible_only")? {
                    self.probes_visible_only = v;
                }
                reply(obj(vec![("view", text(self.look.name()))]))
            }
            "capture" => {
                let shot = self.shot_from(args, ctx)?;
                let name = args.str("name")?;
                let default = format!(
                    "frame-{:05}-{}.png",
                    self.frame,
                    shot.look.name().replace(':', "-")
                );
                let path = if args.bool("keep_only")? == Some(true) {
                    None
                } else {
                    self.path_arg(args, "path", Some(&default))?
                };
                let size = match (args.u32("width")?, args.u32("height")?) {
                    (Some(w), Some(h)) => Some((w.max(1), h.max(1))),
                    (Some(w), None) => {
                        let h = (w as f32 * ctx.renderer.height() as f32
                            / ctx.renderer.width().max(1) as f32)
                            .round() as u32;
                        Some((w.max(1), h.max(1)))
                    }
                    _ => None,
                };
                Ok(Step::Job(Job::Shots {
                    shots: vec![shot],
                    got: Vec::new(),
                    then: Then::Save {
                        paths: vec![path],
                        names: vec![name],
                        size,
                        region: region_arg(args)?,
                    },
                }))
            }
            "captures" => {
                // Several looks in consecutive frames, saved as <prefix>-<look>.png.
                let modes: Vec<String> = args
                    .0
                    .get("modes")
                    .and_then(Value::as_array)
                    .ok_or("captures needs modes: [\"full\", \"direct\", ...]")?
                    .iter()
                    .filter_map(|m| m.as_str().map(str::to_string))
                    .collect();
                let prefix = args
                    .str("prefix")?
                    .unwrap_or_else(|| format!("frame-{:05}", self.frame));
                let settle = args.bool("settle")?.unwrap_or(false);
                let base = ctx.renderer.lighting_config().bounces;
                let mut shots = Vec::new();
                let mut paths = Vec::new();
                let mut names = Vec::new();
                for m in &modes {
                    shots.push(Shot {
                        look: Look::parse(m, base)?,
                        settle,
                    });
                    let file = format!("{prefix}-{}.png", m.replace(':', "-"));
                    paths.push(self.path_arg(&Args(&Value::Null), "", Some(&file))?);
                    names.push(Some(format!("{prefix}-{m}")));
                }
                let size = args.u32("width")?.zip(args.u32("height")?);
                Ok(Step::Job(Job::Shots {
                    shots,
                    got: Vec::new(),
                    then: Then::Save {
                        paths,
                        names,
                        size,
                        region: region_arg(args)?,
                    },
                }))
            }
            "luminance" => {
                let shot = self.shot_from(args, ctx)?;
                Ok(Step::Job(Job::Shots {
                    shots: vec![shot],
                    got: Vec::new(),
                    then: Then::Save {
                        paths: vec![None],
                        names: vec![None],
                        size: None,
                        region: region_arg(args)?,
                    },
                }))
            }
            "diff" => {
                let out = self.path_arg(args, "out", None)?;
                let threshold = args.u32("threshold")?.unwrap_or(8).min(255) as u8;
                let gain = args.f32("gain")?.unwrap_or(4.0);
                let region = region_arg(args)?;
                // Two stored captures or files, or two looks drawn now.
                if let (Some(a), Some(b)) = (args.str("a")?, args.str("b")?) {
                    let a = self.image_named(&a)?;
                    let b = self.image_named(&b)?;
                    return reply(diff_value(&a, &b, out, threshold, gain, region)?);
                }
                let base = ctx.renderer.lighting_config().bounces;
                let settle = args.bool("settle")?.unwrap_or(false);
                let a = Look::parse(
                    &args
                        .str("mode_a")?
                        .ok_or("diff needs a and b, or mode_a and mode_b")?,
                    base,
                )?;
                let b = Look::parse(&args.str("mode_b")?.ok_or("diff needs mode_b")?, base)?;
                Ok(Step::Job(Job::Shots {
                    shots: vec![Shot { look: a, settle }, Shot { look: b, settle }],
                    got: Vec::new(),
                    then: Then::Diff {
                        out,
                        threshold,
                        gain,
                        region,
                    },
                }))
            }
            "reference" | "compare" => {
                let base = ctx.renderer.lighting_config().bounces;
                let shot = if name == "compare" {
                    let look =
                        Look::parse(&args.str("mode")?.unwrap_or_else(|| "full".into()), base)?;
                    Some(Shot {
                        look,
                        settle: args.bool("settle")?.unwrap_or(false),
                    })
                } else {
                    None
                };
                let (rw, rh) = (ctx.renderer.width().max(1), ctx.renderer.height().max(1));
                let width = args.u32("width")?.unwrap_or(rw.min(320)).max(1);
                let height = args
                    .u32("height")?
                    .unwrap_or_else(|| (width as f32 * rh as f32 / rw as f32).round() as u32)
                    .max(1);
                let max_bounces = match args.str("bounces")?.as_deref() {
                    Some("inf") | None => args.u32("bounces")?.unwrap_or(64),
                    Some(other) => other
                        .parse()
                        .map_err(|_| format!("bounces must be a number or inf, not {other}"))?,
                };
                let triangles = match args.str("geometry")?.as_deref() {
                    None | Some("shapes") => false,
                    Some("triangles") => true,
                    Some(other) => return Err(format!("geometry must be shapes or triangles, not {other}")),
                };
                let grid = args
                    .floats("grid", 2)?
                    .map_or((3, 3), |g| (g[0].max(1.0) as u32, g[1].max(1.0) as u32));
                let ref_name = args.str("name")?.unwrap_or_else(|| "reference".into());
                let cache = match args.str("cache")?.as_deref() {
                    Some("off") => None,
                    Some(dir) => Some(PathBuf::from(dir)),
                    None => Some(
                        std::env::var_os("GENOS_REFERENCE_CACHE")
                            .map_or_else(|| PathBuf::from("target/reference-cache"), PathBuf::from),
                    ),
                };
                let mut job = RefJob {
                    cache,
                    spp: args.u32("spp")?.unwrap_or(64).max(1),
                    max_spp: args.u32("max_spp")?.unwrap_or(16384),
                    noise_target: args.f32("noise")?.unwrap_or(0.03),
                    seconds: args.f32("seconds")?.unwrap_or(300.0),
                    block: args.u32("block")?.unwrap_or(2).max(1),
                    max_bounces,
                    triangles,
                    size: (width, height),
                    worker: None,
                    reference: None,
                    frames_left: args.u32("frames")?.unwrap_or(0),
                    shot,
                    live: None,
                    name: ref_name,
                    prefix: args.str("prefix")?.or_else(|| Some(name.to_string())),
                    grid,
                    region: region_arg(args)?,
                    paused: self.paused,
                };
                // Reuse a stored reference instead of tracing again.
                if let Some(stored) = args.str("reference")? {
                    let r = self.references.get(&stored).ok_or_else(|| {
                        format!("no stored reference {stored}; run reference first")
                    })?;
                    job.reference = Some(r.clone());
                }
                if let Some(live) = args.str("live")? {
                    job.live = Some(self.image_named(&live)?);
                }
                Ok(Step::Job(Job::Reference(Box::new(job))))
            }
            "flicker" => {
                let frames = args.u32("frames")?.unwrap_or(60).max(2);
                let shot = self.shot_from(args, ctx)?;
                Ok(Step::Job(Job::Flicker {
                    left: frames,
                    shot,
                    flicker: Flicker::new(
                        region_arg(args)?,
                        args.f32("threshold")?.unwrap_or(0.03),
                    ),
                    heatmap: self.path_arg(args, "heatmap", None)?,
                    frames_dir: self.path_arg(args, "frames_dir", None)?,
                    index: 0,
                }))
            }
            "timing" => reply(self.timing_value(args.u32("frames")?.unwrap_or(60) as usize)),
            "quit" => {
                self.quit = Some(args.f64("code")?.unwrap_or(0.0) as i32);
                reply(obj(vec![("quit", Value::Bool(true))]))
            }
            _ => Err(format!("unknown command {name}")),
        }
    }

    fn light_cmd(&mut self, args: &Args, ctx: &mut Ctx) -> Result<Value, String> {
        if args.bool("solo_off")? == Some(true) {
            self.solo = None;
        }
        if args.bool("add")? == Some(true) {
            let position = args.vec3("position")?.ok_or("a new light needs position")?;
            let color = args.rgb("color")?.unwrap_or([1.0, 0.9, 0.8]);
            let k = args.f32("intensity")?.unwrap_or(1.0);
            let light = Light {
                position,
                color: color.map(|c| c * k),
                direction: Vec3::ZERO,
            };
            self.added.push((light, LightEdit::default()));
            let index = self.base_lights + self.added.len() - 1;
            return Ok(obj(vec![("index", num(index as f64))]));
        }
        let index = args
            .u32("index")?
            .ok_or("light needs index (or add: true)")? as usize;
        let total = self.base_lights + self.added.len();
        if index >= total {
            return Err(format!("no light {index}; there are {total}"));
        }
        if args.bool("remove")? == Some(true) {
            if index >= self.base_lights {
                self.added.remove(index - self.base_lights);
            } else {
                self.lights.entry(index).or_default().off = true;
            }
            return Ok(obj(vec![("removed", num(index as f64))]));
        }
        if args.bool("reset")? == Some(true) {
            self.lights.remove(&index);
        }
        if args.bool("solo")? == Some(true) {
            self.solo = Some(index);
        }
        let edit = if index >= self.base_lights {
            &mut self.added[index - self.base_lights].1
        } else {
            self.lights.entry(index).or_default()
        };
        if let Some(on) = args.bool("on")? {
            edit.off = !on;
        }
        if let Some(c) = args.rgb("color")? {
            edit.color = Some(c);
        }
        if let Some(k) = args.f32("intensity")? {
            edit.intensity = Some(k);
        }
        if let Some(p) = args.vec3("position")? {
            edit.position = Some(p);
            edit.path = None;
        }
        if let Some(points) = args.points("path")? {
            edit.path = Some(Path {
                points,
                frames: args.u32("frames")?.unwrap_or(60),
                start: self.frame,
                looped: args.bool("loop")?.unwrap_or(false),
            });
        }
        let _ = ctx;
        Ok(obj(vec![("index", num(index as f64))]))
    }

    fn image_named(&self, name: &str) -> Result<Image, String> {
        if let Some(img) = self.captures.get(name) {
            return Ok(img.clone());
        }
        let path = PathBuf::from(name);
        let path = if path.is_absolute() {
            path
        } else {
            self.out_dir.join(path)
        };
        Image::load(&path).map_err(|err| format!("{name} is not a capture name or a PNG: {err}"))
    }

    fn timing_value(&self, frames: usize) -> Value {
        let recent: Vec<&(u64, FrameTiming)> =
            self.timing.iter().rev().take(frames.max(1)).collect();
        let n = recent.len().max(1) as f64;
        let mean =
            |f: &dyn Fn(&FrameTiming) -> f64| recent.iter().map(|(_, t)| f(t)).sum::<f64>() / n;
        let worst =
            |f: &dyn Fn(&FrameTiming) -> f64| recent.iter().map(|(_, t)| f(t)).fold(0.0, f64::max);
        let light = |t: &FrameTiming| t.light.iter().map(LightBuildTimes::total_ms).sum::<f64>();
        let gpu: Vec<f64> = recent.iter().filter_map(|(_, t)| t.gpu_ms).collect();
        let mut passes = [0.0f64; 8];
        let mut builds = 0usize;
        let mut rays = 0u64;
        for (_, t) in &recent {
            for b in &t.light {
                builds += 1;
                rays += b.probe_rays;
                for (i, ms) in b.passes_ms.iter().enumerate().take(8) {
                    passes[i] += ms;
                }
            }
        }
        let frame_ms = mean(&|t| t.frame_ms);
        obj(vec![
            ("frames", num(recent.len() as f64)),
            (
                "fps",
                num(if frame_ms > 0.0 {
                    1000.0 / frame_ms
                } else {
                    0.0
                }),
            ),
            ("frame_ms", num(frame_ms)),
            ("frame_ms_worst", num(worst(&|t| t.frame_ms))),
            ("draw_cpu_ms", num(mean(&|t| t.draw_ms))),
            (
                "gpu_ms",
                if gpu.is_empty() {
                    Value::Null
                } else {
                    num(gpu.iter().sum::<f64>() / gpu.len() as f64)
                },
            ),
            ("light_ms_per_frame", num(mean(&light))),
            ("light_builds", num(builds as f64)),
            ("probe_rays", num(rays as f64)),
            (
                "light_passes_ms_per_build",
                list(
                    passes
                        .iter()
                        .take_while(|_| builds > 0)
                        .map(|p| num(p / builds.max(1) as f64))
                        .collect(),
                ),
            ),
            (
                "light_pass_names",
                text("copy, world direct, world bounce, tier (see LightBuildTimes)"),
            ),
        ])
    }
}

enum Step {
    Reply(Value),
    Job(Job),
}

fn knob_value(k: &Knob) -> Value {
    if k.choices.is_empty() && k.text != "auto" && k.text != "shader" {
        num(k.value)
    } else {
        text(k.text.clone())
    }
}

fn region_arg(args: &Args) -> Result<Region, String> {
    Ok(args
        .floats("region", 4)?
        .map(|r| Region([r[0], r[1], r[2], r[3]]))
        .unwrap_or_default())
}

fn yaw_pitch(d: Vec3) -> (f32, f32) {
    let len = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt().max(1.0e-6);
    let pitch = (d.y / len).clamp(-1.0, 1.0).asin();
    let yaw = d.x.atan2(-d.z);
    (yaw, pitch)
}

fn camera_value(c: &Camera) -> Value {
    obj(vec![
        ("position", vec3([c.position.x, c.position.y, c.position.z])),
        ("yaw", num(c.yaw as f64)),
        ("pitch", num(c.pitch as f64)),
    ])
}

fn finish_reference(
    job: &mut RefJob,
    out_dir: &std::path::Path,
    captures: &mut HashMap<String, Image>,
    references: &mut HashMap<String, Reference>,
) -> Result<Value, String> {
    let r = job.reference.take().ok_or("no reference")?;
    let prefix = job.prefix.clone().unwrap_or_else(|| "reference".into());
    let path = |what: &str| out_dir.join(format!("{prefix}-{what}.png"));
    r.image.save(&path("reference"))?;
    captures.insert(job.name.clone(), r.image.clone());
    let mut fields = vec![
        ("reference", text(path("reference").display().to_string())),
        ("spp", num(r.spp as f64)),
        ("bounces", num(job.max_bounces as f64)),
        ("geometry", text(if job.triangles { "triangles" } else { "shapes" })),
        ("width", num(r.image.width as f64)),
        ("height", num(r.image.height as f64)),
        ("trace_seconds", num(r.seconds as f64)),
        ("noise", num(r.noise as f64)),
        ("paused", Value::Bool(job.paused)),
        (
            "coverage",
            num(r.hit.iter().filter(|h| **h).count() as f64 / r.hit.len().max(1) as f64),
        ),
    ];
    if let Some(live) = &job.live {
        let (c, heat) = reference::compare(live, &r, job.region, job.grid, job.block);
        live.resized(r.image.width, r.image.height)
            .save(&path("live"))?;
        heat.save(&path("heatmap"))?;
        let regions = c
            .regions
            .iter()
            .map(|(e, b)| obj(vec![("mean_rel", num(*e as f64)), ("bias", num(*b as f64))]))
            .collect();
        fields.extend([
            ("live", text(path("live").display().to_string())),
            ("heatmap", text(path("heatmap").display().to_string())),
            ("mean_abs", num(c.mean_abs as f64)),
            ("mean_rel", num(c.mean_rel as f64)),
            ("p95_abs", num(c.p95_abs as f64)),
            ("p95_rel", num(c.p95_rel as f64)),
            ("bias", num(c.bias as f64)),
            ("ref_mean", num(c.ref_mean as f64)),
            ("live_mean", num(c.live_mean as f64)),
            (
                "grid",
                list(vec![num(c.grid.0 as f64), num(c.grid.1 as f64)]),
            ),
            ("regions", list(regions)),
        ]);
    }
    references.insert(job.name.clone(), r);
    Ok(obj(fields))
}

fn finish_shots(
    shots: &[Shot],
    got: &[Image],
    then: &Then,
    captures: &mut HashMap<String, Image>,
) -> Result<Value, String> {
    match then {
        Then::Save {
            paths,
            names,
            size,
            region,
        } => {
            let mut out = Vec::new();
            for (i, img) in got.iter().enumerate() {
                let img = match size {
                    Some((w, h)) => img.resized(*w, *h),
                    None => img.clone(),
                };
                let lum = img.luminance(*region);
                let path = paths.get(i).cloned().flatten();
                if let Some(p) = &path {
                    img.save(p)?;
                }
                if let Some(Some(name)) = names.get(i) {
                    captures.insert(name.clone(), img.clone());
                }
                out.push(obj(vec![
                    ("mode", text(shots[i].look.name())),
                    (
                        "path",
                        path.map_or(Value::Null, |p| text(p.display().to_string())),
                    ),
                    ("width", num(img.width as f64)),
                    ("height", num(img.height as f64)),
                    ("luminance", num(lum.mean as f64)),
                    ("min", num(lum.min as f64)),
                    ("max", num(lum.max as f64)),
                    ("black_share", num(lum.black_share as f64)),
                ]));
            }
            Ok(if out.len() == 1 {
                out.remove(0)
            } else {
                list(out)
            })
        }
        Then::Diff {
            out,
            threshold,
            gain,
            region,
        } => diff_value(&got[0], &got[1], out.clone(), *threshold, *gain, *region),
    }
}

fn crop(img: &Image, region: Region) -> Image {
    let (x0, y0, x1, y1) = region.pixels(img.width, img.height);
    let mut rgb = Vec::new();
    for y in y0..y1 {
        let a = ((y * img.width + x0) * 3) as usize;
        let b = ((y * img.width + x1) * 3) as usize;
        rgb.extend_from_slice(&img.rgb[a..b]);
    }
    Image {
        width: x1 - x0,
        height: y1 - y0,
        rgb,
    }
}

fn diff_value(
    a: &Image,
    b: &Image,
    out: Option<PathBuf>,
    threshold: u8,
    gain: f32,
    region: Region,
) -> Result<Value, String> {
    let b = b.resized(a.width, a.height);
    let (a, b) = (crop(a, region), crop(&b, region));
    let (d, picture) = a.diff(&b, threshold, gain);
    if let Some(p) = &out {
        picture.save(p)?;
    }
    let la = a.luminance(Region::default()).mean;
    let lb = b.luminance(Region::default()).mean;
    Ok(obj(vec![
        ("mean_abs", num(d.mean_abs as f64)),
        ("max_abs", num(d.max_abs as f64)),
        ("over_share", num(d.over_share as f64)),
        ("luminance_a", num(la as f64)),
        ("luminance_b", num(lb as f64)),
        ("mean_luminance_delta", num(d.mean_luminance_delta as f64)),
        (
            "out",
            out.map_or(Value::Null, |p| text(p.display().to_string())),
        ),
    ]))
}

fn brick_value(b: &BrickReport) -> Value {
    obj(vec![
        (
            "brick",
            list(b.brick.iter().map(|v| num(*v as f64)).collect()),
        ),
        ("center", vec3(b.center)),
        ("state", text(b.state.label())),
        ("replace", Value::Bool(b.replace)),
        ("change_left", num(b.change_left as f64)),
        ("samples", num(b.samples as f64)),
        ("probes", num(b.probes as f64)),
        ("age", num(b.age as f64)),
        ("seen", num(b.seen as f64)),
        ("depth", num(b.depth as f64)),
        ("near_term", num(b.near_term as f64)),
        ("stale_term", num(b.stale_term as f64)),
        ("priority", num(b.priority as f64)),
        ("spacing", num(b.spacing as f64)),
        ("skip", text(b.skip)),
    ])
}

fn probes_value(report: &[BrickReport], args: &Args, renderer: &Renderer) -> Result<Value, String> {
    let count = |s: BrickState| report.iter().filter(|b| b.state == s).count() as f64;
    let on_screen: Vec<&BrickReport> = report.iter().filter(|b| b.seen > 0.0).collect();
    let mut pick: Vec<&BrickReport> = report.iter().collect();
    if let Some(state) = args.str("state")? {
        pick.retain(|b| b.state.label() == state);
    }
    if args.bool("visible")? == Some(true) {
        pick.retain(|b| b.seen > 0.0);
    }
    if let (Some(at), Some(r)) = (args.vec3("near")?, args.f32("radius")?) {
        pick.retain(|b| {
            let d = [b.center[0] - at.x, b.center[1] - at.y, b.center[2] - at.z];
            d.iter().map(|v| v * v).sum::<f32>().sqrt() <= r
        });
    }
    match args.str("sort")?.as_deref().unwrap_or("priority") {
        "age" => pick.sort_by(|a, b| b.age.total_cmp(&a.age)),
        "depth" => pick.sort_by(|a, b| a.depth.total_cmp(&b.depth)),
        _ => pick.sort_by(|a, b| b.priority.total_cmp(&a.priority)),
    }
    let limit = args.u32("limit")?.unwrap_or(20) as usize;
    let values = args.bool("values")? == Some(true);
    let oldest_seen = on_screen.iter().map(|b| b.age).fold(0.0f32, f32::max);
    Ok(obj(vec![
        ("bricks", num(report.len() as f64)),
        ("unlit", num(count(BrickState::Unlit))),
        ("changing", num(count(BrickState::Changing))),
        ("refining", num(count(BrickState::Refining))),
        ("steady", num(count(BrickState::Steady))),
        ("on_screen", num(on_screen.len() as f64)),
        (
            "on_screen_changing",
            num(on_screen
                .iter()
                .filter(|b| b.state == BrickState::Changing)
                .count() as f64),
        ),
        ("oldest_on_screen_age", num(oldest_seen as f64)),
        ("matched", num(pick.len() as f64)),
        (
            "list",
            list(
                pick.into_iter()
                    .take(limit)
                    .map(|b| {
                        let mut v = brick_value(b);
                        if values {
                            if let Value::Object(fields) = &mut v {
                                fields.push(("values".into(), probe_values(renderer, b.brick)));
                            }
                        }
                        v
                    })
                    .collect(),
            ),
        ),
    ]))
}

/// The stored light of each live probe of `brick`: position, samples, and the face
/// luminances of its top cube (every bounce) and first cube (one bounce).
fn probe_values(renderer: &Renderer, brick: [i32; 3]) -> Value {
    let faces = |f: [f32; 6]| list(f.iter().map(|v| num(f64::from(*v))).collect());
    list(
        renderer
            .probe_values(brick)
            .into_iter()
            .map(|p| {
                obj(vec![
                    (
                        "position",
                        list(p.position.iter().map(|v| num(f64::from(*v))).collect()),
                    ),
                    ("samples", num(f64::from(p.samples))),
                    ("top", faces(p.top)),
                    ("first", faces(p.first)),
                ])
            })
            .collect(),
    )
}

/// One square per brick at its projected centre, coloured by `color`.
fn probe_markers(
    report: &[BrickReport],
    camera: &Camera,
    size: (u32, u32),
    color: ProbeColor,
    visible_only: bool,
) -> Vec<ScreenRect> {
    let (w, h) = (size.0 as f32, size.1 as f32);
    let m = genos_scene::view_proj(camera, w / h.max(1.0));
    let top = report
        .iter()
        .filter(|b| b.seen > 0.0)
        .map(|b| b.priority)
        .fold(1.0e-6f32, f32::max);
    let mut out = Vec::new();
    for b in report {
        if visible_only && b.seen <= 0.0 {
            continue;
        }
        let p = b.center;
        let clip =
            [0usize, 1, 3].map(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r]);
        if clip[2] <= 0.05 {
            continue;
        }
        let (x, y) = (clip[0] / clip[2], clip[1] / clip[2]);
        if x.abs() > 1.0 || y.abs() > 1.0 {
            continue;
        }
        let px = (x * 0.5 + 0.5) * w;
        let py = (y * 0.5 + 0.5) * h;
        let s = (h * 0.25 * b.spacing / clip[2].max(0.5)).clamp(3.0, 14.0);
        let heat = |t: f32| {
            let t = t.clamp(0.0, 1.0);
            [t.min(0.5) * 2.0, 1.0 - (t - 0.5).max(0.0) * 2.0, 0.15]
        };
        let rgb = match color {
            ProbeColor::State => match b.state {
                BrickState::Unlit => [0.5, 0.5, 0.5],
                BrickState::Changing => [1.0, 0.15, 0.1],
                BrickState::Refining => [1.0, 0.85, 0.1],
                BrickState::Steady => [0.1, 0.9, 0.3],
            },
            ProbeColor::Age => heat((1.0 + b.age).log2() / 2.3),
            ProbeColor::Priority => {
                let t = b.priority / top;
                [t, 0.2, 1.0 - t]
            }
            ProbeColor::Spacing => {
                let t = (b.spacing.log2() + 2.0) / 4.0;
                [t.clamp(0.0, 1.0), 0.4, 1.0 - t.clamp(0.0, 1.0)]
            }
            ProbeColor::Changing => match b.state {
                BrickState::Changing | BrickState::Unlit => [1.0, 0.1, 0.1],
                _ => [0.15, 0.25, 0.15],
            },
        };
        out.push(ScreenRect {
            x: px - s * 0.5 - 1.0,
            y: py - s * 0.5 - 1.0,
            w: s + 2.0,
            h: s + 2.0,
            color: [0.0; 3],
        });
        out.push(ScreenRect {
            x: px - s * 0.5,
            y: py - s * 0.5,
            w: s,
            h: s,
            color: rgb,
        });
    }
    out
}
