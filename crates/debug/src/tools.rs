#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::cast_lossless, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_sign_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::explicit_iter_loop, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::manual_pattern_char_comparison, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::many_single_char_names, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::missing_errors_doc, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::must_use_candidate, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::too_many_lines, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails"))]
#![expect(clippy::trivially_copy_pass_by_ref, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::wildcard_enum_match_arm, reason = "TODO(lint-2026-10-14): pre-existing wildcard match. docs/LINT_RULES.md")]

//! The frame side of the debug tools: runs queued commands against the renderer,
//! the scene and the camera, holds edits the example's own update would undo, and
//! answers waits and captures after the frames they need.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use genos_load::MaterialSettings;
use genos_mcp::json::Value;
use genos_render::{DebugView, MaterialStats, Renderer, SceneRead, ScreenRect, ViewMode};
use genos_scene::{
    Camera, DebugSettings, EventLoopSettings, FogSettings, FpsCounterSettings, HdrSettings, Light,
    LightingPreset, LightingSettings, MenuSettings, OutputMode, ReflectionSettings, Scene, Vec3,
    PITCH_LIMIT,
};

use crate::image::{Flicker, Image, Region};
use crate::knobs::{CheckpointPose, Host, Knob, LoadPace};
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
}

/// What a picture shows: a debug view of the faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub view: DebugView,
}

impl Look {
    /// `full`, `direct`, `bounce`, `nobounce` (the same as `direct`), `albedo`,
    /// `normal`, `depth`, `light:I`, `texels`.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let (head, arg) = match spec.split_once(':') {
            Some((h, a)) => (h.trim(), Some(a.trim())),
            None => (spec.trim(), None),
        };
        let mut look = Look {
            view: DebugView::default(),
        };
        look.view.mode = if head == "nobounce" {
            ViewMode::Direct
        } else {
            ViewMode::parse(head).ok_or_else(|| {
                format!(
                    "unknown view {head}: use full, direct, bounce, nobounce, albedo, normal, depth, light:I, texels, variance, path"
                )
            })?
        };
        if let Some(a) = arg {
            if look.view.mode != ViewMode::Light {
                return Err(format!(
                    "{head} takes no argument (bounce limits went with the old lighting)"
                ));
            }
            look.view.lamp = a
                .parse()
                .map_err(|_| format!("light:{a} needs a lamp index"))?;
        }
        Ok(look)
    }

    pub fn name(&self) -> String {
        match self.view.mode {
            ViewMode::Light => format!("light:{}", self.view.lamp),
            mode => mode.name().to_string(),
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
    /// Material override, 0 to 1. Only the reference tracer reads these today:
    /// solids have no roughness or metallic in the live renderer until the
    /// specular pass (ADR 0024) lands, so the picture ignores them.
    roughness: Option<f32>,
    metallic: Option<f32>,
}

/// A picture a job wants this frame.
///
/// `ui` keeps the debug menu, the menu tab, the frame-rate plate, and tool
/// overlays in the readback. It is false unless the command set it.
#[derive(Clone, Copy, Debug)]
struct Shot {
    look: Look,
    ui: bool,
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

/// A picture saved from one known present.
///
/// `path` is already resolved against the shot directory. `size` resamples
/// that present; it does not draw another one.
#[derive(Clone)]
struct FrameCapture {
    path: Option<PathBuf>,
    size: Option<(u32, u32)>,
    look: Look,
    ui: bool,
}

/// Presents still owed after the settle present, and the ids frozen there.
struct SettleHold {
    /// Tail presents not yet drawn, including the one that saves.
    left: u32,
    /// Tail presents requested. The saved present is `settled_frame + total`.
    total: u32,
    settled_frame: u64,
    settled_still: u32,
    settled_frames: u32,
    settled_wall_ms: f64,
    capture: Option<FrameCapture>,
}

enum Job {
    Frames {
        left: u32,
        /// Present count when the job was taken, before any of its presents.
        accepted: u64,
    },
    Settle {
        left: u32,
        quiet: u32,
        need: u32,
        frames: u32,
        /// Wall milliseconds until settle. The tail does not add to this.
        wall_ms: f64,
        /// Presents to draw after the settle present. None replies on that present.
        then_frames: Option<u32>,
        capture: Option<FrameCapture>,
        hold: Option<SettleHold>,
    },
    Shots {
        shots: Vec<Shot>,
        got: Vec<Image>,
        then: Then,
        /// Present index these pixels must come from. None takes the next present.
        at: Option<u64>,
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
    /// A scene file is loading off the frame thread.
    SceneLoad {
        name: String,
        checkpoint: String,
    },
}

/// The layers a reference writes: `full` the picture alone, `specular` also
/// the primary specular lobe alone (`<prefix>-reference-spec.png`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Aov {
    Full,
    Specular,
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
    /// `aov`: which layers to write besides the full picture.
    aov: Aov,
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
    /// Scene-referred read of the live frame. None falls back to the display image.
    scene: Option<SceneRead>,
    /// The live picture used the legacy shoulder, so a missing scene image may undo it.
    legacy_display: bool,
    /// `measure: true` ([`RefTiming::Measure`]): see there.
    timing: RefTiming,
}

/// When a reference job takes its live picture and its trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefTiming {
    /// The frame after the trace (and `frames`); frames draw while it traces.
    Interactive,
    /// Scripted and headless measurement. Exactly one frame is drawn, so the
    /// lighting updates once per measurement: the draw after the request. It is the
    /// live picture for `compare`, after `frames`. `reference` has no live picture,
    /// so `frames` is ignored. The frame thread then joins the CPU trace, and window
    /// events are not handled until the trace ends.
    Measure,
}

/// The `measure` argument of `reference` and `compare` (default interactive).
fn ref_timing(args: &Args) -> Result<RefTiming, String> {
    Ok(if args.bool("measure")? == Some(true) {
        RefTiming::Measure
    } else {
        RefTiming::Interactive
    })
}

impl RefJob {
    /// Whether this draw's picture is the live one.
    fn live_due(&self) -> bool {
        self.shot.is_some()
            && self.live.is_none()
            && self.frames_left == 0
            && (self.timing == RefTiming::Measure || self.reference.is_some())
    }

    /// One drawn frame: counts down `frames`, takes the trace once it is done
    /// (blocking in measure mode once the live picture is held), and holds `image`
    /// as the live picture when it is due. True when `image` was taken.
    fn on_draw(&mut self, image: Option<&Image>) -> bool {
        self.frames_left = self.frames_left.saturating_sub(1);
        let mut took = false;
        if self.live_due() {
            if let Some(img) = image {
                self.live = Some(img.clone());
                took = true;
            }
        }
        let wait = self.timing == RefTiming::Measure && (self.live.is_some() || self.shot.is_none());
        if self.worker.as_ref().is_some_and(|w| wait || w.is_finished()) {
            self.reference = self.worker.take().and_then(|w| w.join().ok());
            if self.reference.is_none() {
                self.shot = None;
            }
        }
        if !took && self.live_due() {
            if let Some(img) = image {
                self.live = Some(img.clone());
                took = true;
            }
        }
        took
    }
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
    /// `<out>/gi`. Live `frames.csv` and a written scenario summary land here.
    gi_dir: PathBuf,
    /// Last sample sequence already appended to `frames.csv`.
    gi_flushed: u64,
    base_lights: usize,
    /// Last reflection pass: ray count, view count, GPU time, image bytes.
    reflection_stats: Option<genos_render::ReflectionGrade>,
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
            },
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
            gi_dir: PathBuf::from("genos-debug").join("gi"),
            gi_flushed: 0,
            base_lights: 0,
            reflection_stats: None,
        }
    }

    /// Where captures without a path go.
    pub fn set_out_dir(&mut self, dir: PathBuf) {
        self.out_dir = dir;
    }

    /// Where GI pass times and work counters are written. Default `genos-debug/gi`.
    pub fn set_gi_dir(&mut self, dir: PathBuf) {
        self.gi_dir = dir;
    }

    /// The view the next frame shows, unless a capture job asks for another.
    pub fn set_view(&mut self, mode: ViewMode) {
        self.look.view = DebugView { mode, lamp: 0 };
    }

    /// The view `set_view` stored. A panel uses it to mark the current picture.
    pub fn view_mode(&self) -> ViewMode {
        self.look.view.mode
    }

    /// Whether the capture being read this frame keeps debug overlays.
    /// A frame with no capture job uses `cli_ui` (`--ui`).
    pub fn show_capture_overlay(&self, cli_ui: bool) -> bool {
        match self.reading {
            Some(shot) => shot.ui,
            None => cli_ui,
        }
    }

    /// Forget light, object, and camera-path edits. A scene switch calls this so
    /// the previous scene's edits do not land on the new one.
    pub fn drop_scene_edits(&mut self) {
        self.lights.clear();
        self.added.clear();
        self.solo = None;
        self.objects.clear();
        self.camera_path = None;
    }

    /// Run scene time on a fixed step from the start (a script or a test).
    pub fn set_fixed_step(&mut self, dt: Option<f32>) {
        self.fixed_dt = dt;
    }

    /// Hold the scene clock. MCP `step`, or [`Self::request_steps`], advances it.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Queue scene frames. The next `scene_dt` calls consume them one at a time.
    pub fn request_steps(&mut self, frames: u32) {
        self.steps_left = self.steps_left.saturating_add(frames.max(1));
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
        self.accept_commands(Some(ctx));
        self.poll_scene_loads(ctx);
        self.apply_edits(ctx.host.scene());
        self.apply_camera_path(ctx.camera);
        let materials = self.materials(ctx.host.scene().solids.len());
        for running in &mut self.jobs {
            if let Job::Reference(job) = &mut running.job {
                if job.worker.is_none() && job.reference.is_none() {
                    let setup = RefSetup {
                        max_bounces: job.max_bounces,
                        triangles: job.triangles,
                        max_spp: job.max_spp.max(job.spp),
                        noise_target: job.noise_target,
                        seconds: job.seconds,
                        specular: job.aov == Aov::Specular,
                        materials: materials.clone(),
                        ..RefSetup::from_camera(
                            ctx.host.scene(),
                            ctx.camera,
                            job.size.0,
                            job.size.1,
                            job.spp,
                        )
                    };
                    let cache = job.cache.clone();
                    job.worker = Some(std::thread::spawn(move || {
                        use std::sync::atomic::Ordering;
                        reference::RENDERING.fetch_add(1, Ordering::SeqCst);
                        let r = match cache {
                            Some(dir) => reference::render_cached(&setup, &dir),
                            None => reference::render(&setup),
                        };
                        reference::RENDERING.fetch_sub(1, Ordering::SeqCst);
                        r
                    }));
                }
            }
        }
        let read = self.arm_readback();
        let look = self.reading.map_or(self.look, |s| s.look);
        ctx.renderer.set_debug_view(look.view);
        self.overlay.clear();
        read
    }

    /// `take` once, at the start of the present. A command that arrives after
    /// this returns is taken on a later present. `before_draw` is this plus the
    /// device. `ctx` is absent only when the command does not touch the device.
    fn accept_commands(&mut self, mut ctx: Option<&mut Ctx>) {
        for cmd in genos_mcp::command::take() {
            let args = Args(&cmd.args);
            let result = self.dispatch(&cmd.name, &args, ctx.as_deref_mut());
            match result {
                Ok(Step::Reply(value)) => genos_mcp::command::reply(cmd.id, Ok(value)),
                Ok(Step::Job(job)) => self.jobs.push(Running { id: cmd.id, job }),
                Err(err) => genos_mcp::command::reply(cmd.id, Err(err)),
            }
        }
    }

    fn dispatch(
        &mut self,
        name: &str,
        args: &Args,
        mut ctx: Option<&mut Ctx>,
    ) -> Result<Step, String> {
        let ui_default = ctx
            .as_deref_mut()
            .is_some_and(|ctx| ctx.host.scene().debug.ui);
        if let Some(step) = self.frame_job(name, args, ui_default)? {
            return Ok(step);
        }
        let Some(ctx) = ctx else {
            return Err(format!("{name} needs the frame device"));
        };
        self.run(name, args, ctx)
    }

    /// Whether this present's picture must be read. The present is still
    /// `self.frame`; the draw about to happen is `self.frame + 1`.
    fn arm_readback(&mut self) -> bool {
        self.reading = self
            .jobs
            .iter()
            .find_map(|running| read_this_draw(self.frame, &running.job));
        self.reading.is_some()
    }

    /// Feed the frame's picture and stats to the waiting jobs and answer the done ones.
    pub fn after_draw(
        &mut self,
        renderer: &mut Renderer,
        pixels: Option<&[u8]>,
        timing: FrameTiming,
    ) {
        let offered = match (self.reading, pixels) {
            (Some(_), Some(px)) => {
                Some(Image::from_bgra(renderer.width(), renderer.height(), px))
            }
            _ => None,
        };
        let stats = renderer.settle_stats();
        // The lighting has settled and a frame drawn from its held light is on screen.
        let settled_now = renderer.picture_settled() && stats.settled;
        self.apply_present(
            offered,
            settled_now,
            stats.still_frames,
            timing.frame_ms,
            Some(timing),
            Some(renderer),
        );
    }

    /// Number this present, keep `offered` only when [`Self::arm_readback`] asked
    /// for it, and run the jobs. `before_draw` armed the read on the count before
    /// this number. The saved pixels are this present or they are absent.
    fn apply_present(
        &mut self,
        offered: Option<Image>,
        settled_now: bool,
        still_frames: u32,
        frame_wall_ms: f64,
        timing: Option<FrameTiming>,
        mut renderer: Option<&mut Renderer>,
    ) {
        let image = self.reading.as_ref().and(offered);
        self.frame += 1;
        if let Some(renderer) = renderer.as_mut() {
            self.reflection_stats = renderer.reflection_grade().ok();
        }
        if let Some(timing) = timing {
            self.timing.push_back((self.frame, timing));
            while self.timing.len() > 240 {
                self.timing.pop_front();
            }
        }
        let mut image_used = false;
        let mut done = Vec::new();
        for (index, running) in self.jobs.iter_mut().enumerate() {
            let finished: Option<Result<Value, String>> = match &mut running.job {
                Job::Frames { left, accepted } => frames_reply(self.frame, left, *accepted).map(Ok),
                Job::Settle {
                    left,
                    quiet,
                    need,
                    frames,
                    wall_ms,
                    then_frames,
                    capture,
                    hold,
                } => settle_on_present(
                    self.frame,
                    image.as_ref(),
                    settled_now,
                    still_frames,
                    frame_wall_ms,
                    left,
                    quiet,
                    *need,
                    frames,
                    wall_ms,
                    *then_frames,
                    capture,
                    hold,
                    &mut self.captures,
                ),
                Job::Shots {
                    shots,
                    got,
                    then,
                    at,
                } => {
                    let finished = shots_on_present(
                        self.frame,
                        image.as_ref(),
                        &mut image_used,
                        shots,
                        got,
                        then,
                        *at,
                        &mut self.captures,
                    );
                    finished
                }
                Job::Reference(job) => match renderer.as_deref_mut() {
                    None => Some(Err("reference needs the frame device".to_string())),
                    Some(renderer) => {
                        if job.on_draw(if image_used { None } else { image.as_ref() }) {
                            job.scene = renderer
                                .read_scene()
                                .ok()
                                .filter(|read| !read.rgb.is_empty());
                            job.legacy_display = renderer.hdr_legacy_display();
                            image_used = true;
                        }
                        let ready =
                            job.reference.is_some() && (job.shot.is_none() || job.live.is_some());
                        let failed =
                            job.worker.is_none() && job.reference.is_none() && job.shot.is_none();
                        if failed {
                            Some(Err("the reference trace panicked".to_string()))
                        } else if ready {
                            let legacy_display = job.legacy_display;
                            Some(finish_reference(
                                job,
                                &self.out_dir,
                                &mut self.captures,
                                &mut self.references,
                                legacy_display,
                            ))
                        } else {
                            None
                        }
                    }
                },
                Job::SceneLoad { .. } => None,
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
        if let Some(renderer) = renderer {
            self.flush_gi_frames(renderer);
        }
    }

    /// Append samples the renderer recorded after the last flush.
    ///
    /// The GPU timestamps are already in the sample. This write does not
    /// change that frame's pass time.
    fn flush_gi_frames(&mut self, renderer: &mut Renderer) {
        if !renderer.gi_metrics_enabled() {
            return;
        }
        let fresh: Vec<_> = renderer
            .gi_metrics_samples()
            .iter()
            .filter(|sample| sample.seq > self.gi_flushed)
            .copied()
            .collect();
        if let Some(last) = fresh.last() {
            self.gi_flushed = last.seq;
        }
        if let Err(err) = crate::gi_metrics::append_frames(&self.gi_dir, &fresh) {
            eprintln!("genos-debug: {err}");
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

    /// The `object` roughness and metallic overrides, per solid index. A solid
    /// with only one of the two set keeps paint's value for the other
    /// (roughness 1, metallic 0). Empty when no solid has one.
    fn materials(&self, solids: usize) -> Vec<Option<reference::SolidMaterial>> {
        let mut out = vec![None; solids];
        let mut any = false;
        for (index, edit) in &self.objects {
            if edit.roughness.is_none() && edit.metallic.is_none() {
                continue;
            }
            if let Some(slot) = out.get_mut(*index) {
                *slot = Some(reference::SolidMaterial {
                    roughness: edit.roughness.unwrap_or(1.0),
                    metallic: edit.metallic.unwrap_or(0.0),
                });
                any = true;
            }
        }
        if any {
            out
        } else {
            Vec::new()
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

    /// All knobs: the example's.
    pub fn knobs(&self, host: &dyn Host) -> Vec<Knob> {
        host.knobs()
    }

    /// Set one knob by name, snapped to its step.
    pub fn set_knob(&mut self, host: &mut dyn Host, name: &str, value: f64) -> Result<f64, String> {
        let all = self.knobs(host);
        let knob = all
            .iter()
            .find(|k| k.name == name)
            .ok_or_else(|| format!("unknown knob {name}"))?;
        let v = knob.snap(value);
        host.set_knob(name, v)?;
        Ok(v)
    }

    fn shot_from(&self, args: &Args, ui_default: bool) -> Result<Shot, String> {
        let look = match args.str("mode")? {
            Some(m) => Look::parse(&m)?,
            None => self.look,
        };
        Ok(Shot {
            look,
            ui: Self::shot_ui(args, ui_default)?,
        })
    }

    fn shot_ui(args: &Args, ui_default: bool) -> Result<bool, String> {
        Ok(args.bool("ui")?.unwrap_or(ui_default))
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

    /// `wait`, `step`, and `wait_settled`. None when `name` is some other command.
    /// These do not read the device. `run` handles the rest.
    fn frame_job(
        &mut self,
        name: &str,
        args: &Args,
        ui_default: bool,
    ) -> Result<Option<Step>, String> {
        match name {
            "step" => {
                let n = args.u32("frames")?.unwrap_or(1).max(1);
                self.steps_left += n;
                Ok(Some(Step::Job(Job::Frames {
                    left: n,
                    accepted: self.frame,
                })))
            }
            "wait" => Ok(Some(Step::Job(Job::Frames {
                left: args.u32("frames")?.unwrap_or(1).max(1),
                accepted: self.frame,
            }))),
            "wait_settled" => {
                let then_frames = args.u32("then_frames")?;
                let path = self.path_arg(args, "path", None)?;
                let size = match (args.u32("width")?, args.u32("height")?) {
                    (None, None) => None,
                    (Some(w), Some(h)) => Some((w.max(1), h.max(1))),
                    _ => return Err("wait_settled capture size needs width and height".into()),
                };
                if path.is_none() && (then_frames.unwrap_or(0) > 0 || size.is_some()) {
                    return Err("wait_settled capture needs a path".into());
                }
                if path.is_some() && then_frames.unwrap_or(0) == 0 {
                    return Err("wait_settled capture needs then_frames of at least 1".into());
                }
                let capture = if path.is_some() {
                    Some(FrameCapture {
                        path,
                        size,
                        look: self.look,
                        ui: Self::shot_ui(args, ui_default)?,
                    })
                } else {
                    None
                };
                Ok(Some(Step::Job(Job::Settle {
                    left: args.u32("timeout_frames")?.unwrap_or(600).max(1),
                    quiet: 0,
                    need: args.u32("quiet_frames")?.unwrap_or(3).max(1),
                    frames: 0,
                    wall_ms: 0.0,
                    then_frames,
                    capture,
                    hold: None,
                })))
            }
            _ => Ok(None),
        }
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "TODO(lint-2026-10-14): pass this settings value in from the caller. docs/LINT_RULES.md"
    )]
    fn run(&mut self, name: &str, args: &Args, ctx: &mut Ctx) -> Result<Step, String> {
        let ui_default = ctx.host.scene().debug.ui;
        let reply = |v: Value| Ok(Step::Reply(v));
        match name {
            "status" => {
                let stats = ctx.renderer.settle_stats();
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
                    ("stable", Value::Bool(stats.settled)),
                    ("still_frames", num(stats.still_frames as f64)),
                    (
                        "picture_settled",
                        Value::Bool(ctx.renderer.picture_settled()),
                    ),
                ]))
            }
            "knobs" => {
                let all = self.knobs(ctx.host);
                if let (Some(n), Some(v)) = (args.str("name")?, args.0.get("value")) {
                    let knob = all
                        .iter()
                        .find(|k| k.name == n)
                        .ok_or_else(|| format!("unknown knob {n}"))?;
                    let value = match v.as_f64() {
                        Some(f) => f,
                        None => knob.parse(v.as_str().unwrap_or(""))?,
                    };
                    self.set_knob(ctx.host, &n, value)?;
                }
                reply(list(
                    self.knobs(ctx.host)
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
                args.only(
                    "object",
                    &[
                        "index",
                        "reset",
                        "position",
                        "path",
                        "frames",
                        "loop",
                        "yaw",
                        "spin",
                        "freeze",
                        "roughness",
                        "metallic",
                    ],
                )?;
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
                if let Some(r) = args.f32("roughness")? {
                    edit.roughness = Some(unit("roughness", r)?);
                }
                if let Some(m) = args.f32("metallic")? {
                    edit.metallic = Some(unit("metallic", m)?);
                }
                let mut fields = vec![
                    ("index", num(index as f64)),
                    (
                        "position",
                        vec3([solid.position.x, solid.position.y, solid.position.z]),
                    ),
                    ("yaw", num(solid.yaw as f64)),
                ];
                if let Some(r) = edit.roughness {
                    fields.push(("roughness", num(r as f64)));
                }
                if let Some(m) = edit.metallic {
                    fields.push(("metallic", num(m as f64)));
                }
                reply(obj(fields))
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
            "view" => {
                if let Some(m) = args.str("mode")? {
                    self.look = Look::parse(&m)?;
                }
                reply(obj(vec![("view", text(self.look.name()))]))
            }
            "capture" => {
                let shot = self.shot_from(args, ui_default)?;
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
                let at = capture_at(self.frame, args.u32("frame")?)?;
                Ok(Step::Job(Job::Shots {
                    shots: vec![shot],
                    got: Vec::new(),
                    then: Then::Save {
                        paths: vec![path],
                        names: vec![name],
                        size,
                        region: region_arg(args)?,
                    },
                    at,
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
                let mut shots = Vec::new();
                let mut paths = Vec::new();
                let mut names = Vec::new();
                let ui = Self::shot_ui(args, ui_default)?;
                for m in &modes {
                    shots.push(Shot {
                        look: Look::parse(m)?,
                        ui,
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
                    at: None,
                }))
            }
            "luminance" => {
                let shot = self.shot_from(args, ui_default)?;
                Ok(Step::Job(Job::Shots {
                    shots: vec![shot],
                    got: Vec::new(),
                    then: Then::Save {
                        paths: vec![None],
                        names: vec![None],
                        size: None,
                        region: region_arg(args)?,
                    },
                    at: None,
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
                let a = Look::parse(
                    &args
                        .str("mode_a")?
                        .ok_or("diff needs a and b, or mode_a and mode_b")?,
                )?;
                let b = Look::parse(&args.str("mode_b")?.ok_or("diff needs mode_b")?)?;
                let ui = Self::shot_ui(args, ui_default)?;
                Ok(Step::Job(Job::Shots {
                    shots: vec![Shot { look: a, ui }, Shot { look: b, ui }],
                    got: Vec::new(),
                    then: Then::Diff {
                        out,
                        threshold,
                        gain,
                        region,
                    },
                    at: None,
                }))
            }
            "reference" | "compare" => {
                let shot = if name == "compare" {
                    let look = Look::parse(&args.str("mode")?.unwrap_or_else(|| "full".into()))?;
                    Some(Shot {
                        look,
                        ui: Self::shot_ui(args, ui_default)?,
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
                    Some(other) => {
                        return Err(format!("geometry must be shapes or triangles, not {other}"))
                    }
                };
                let aov = match args.str("aov")?.as_deref() {
                    None | Some("full") => Aov::Full,
                    Some("specular") => Aov::Specular,
                    Some(other) => return Err(format!("aov must be full or specular, not {other}")),
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
                    aov,
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
                    scene: None,
                    legacy_display: true,
                    timing: ref_timing(args)?,
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
                let shot = self.shot_from(args, ui_default)?;
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
            "gi_metrics" => {
                if !ctx.renderer.gi_metrics_enabled() {
                    return Err(crate::gi_metrics::disabled_error());
                }
                if args.bool("reset")? == Some(true) {
                    ctx.renderer.gi_metrics_clear();
                    self.gi_flushed = 0;
                    crate::gi_metrics::truncate_frames(&self.gi_dir)?;
                }
                let name = args.str("write")?;
                let samples = ctx.renderer.gi_metrics_samples().to_vec();
                if let Some(name) = name {
                    if samples.is_empty() {
                        return Err("GI metrics window is empty".into());
                    }
                    let path = crate::gi_metrics::write_summary(&self.gi_dir, &name, &samples)?;
                    reply(crate::gi_metrics::with_path(
                        crate::gi_metrics::report_value(true, &samples),
                        &path,
                    ))
                } else {
                    reply(crate::gi_metrics::report_value(true, &samples))
                }
            }
            "get_lighting_settings" => reply(lighting_report(
                &ctx.renderer.lighting_settings().to_text(),
                ctx.renderer,
            )),
            "lighting_under_trusted" => reply(obj(vec![(
                "pixels",
                num(f64::from(ctx.renderer.lighting_under_trusted_pixels())),
            )])),
            "set_lighting_settings" => {
                let scene = ctx.host.scene();
                apply_lighting_update(&mut scene.lighting, args.0)?;
                let written = scene.lighting.to_text();
                ctx.renderer.clear_lighting_override();
                reply(lighting_report(&written, ctx.renderer))
            }
            "get_material_settings" => {
                reply(obj(vec![(
                    "text",
                    text(ctx.renderer.material_settings().to_text()),
                )]))
            }
            "get_material_stats" => reply(material_stats_value(ctx.renderer.material_stats())),
            "set_material_settings" => {
                let mut settings = ctx.renderer.material_settings();
                apply_material_update(&mut settings, args.0)?;
                let written = settings.to_text();
                ctx.renderer.set_material_settings(settings);
                reply(obj(vec![("text", text(written))]))
            }
            "list_presets" => {
                let presets = LightingPreset::all()
                    .iter()
                    .map(|preset| {
                        obj(vec![
                            ("name", text(preset.name())),
                            ("text", text(LightingSettings::preset(*preset).to_text())),
                        ])
                    })
                    .collect();
                reply(obj(vec![("presets", list(presets))]))
            }
            "apply_preset" => {
                let name = args.str("name")?.ok_or("apply_preset needs name")?;
                let preset =
                    LightingPreset::parse(&name).ok_or_else(|| format!("unknown preset {name}"))?;
                ctx.host.scene().lighting = LightingSettings::preset(preset);
                let written = ctx.host.scene().lighting.to_text();
                ctx.renderer.clear_lighting_override();
                reply(lighting_report(&written, ctx.renderer))
            }
            "get_debug_settings" => {
                let settings = ctx.renderer.debug_settings(ctx.host.scene());
                reply(obj(vec![("text", text(settings.to_text()))]))
            }
            "set_debug_settings" => {
                apply_debug_update(&mut ctx.host.scene().debug, args.0)?;
                let cli = ctx.renderer.debug_out_cli().map(str::to_string);
                let settings = ctx.host.scene().debug.effective(cli.as_deref());
                reply(obj(vec![("text", text(settings.to_text()))]))
            }
            "get_fps_counter" => {
                reply(fps_counter_value(&ctx.host.scene().fps_counter.effective()))
            }
            "get_reflection_settings" => {
                reply(reflection_value(&ctx.host.scene().reflections.effective()))
            }
            "get_reflection_stats" => reply(reflection_stats_value(self.reflection_stats.as_ref())),
            "set_reflection_settings" => {
                apply_reflection_update(&mut ctx.host.scene().reflections, args.0)?;
                reply(reflection_value(&ctx.host.scene().reflections.effective()))
            }
            "set_fps_counter" => {
                apply_fps_update(&mut ctx.host.scene().fps_counter, args.0)?;
                reply(fps_counter_value(&ctx.host.scene().fps_counter.effective()))
            }
            "get_event_loop" => reply(event_loop_value(&ctx.host.scene().event_loop)),
            "set_event_loop" => {
                apply_event_loop_update(&mut ctx.host.scene().event_loop, args.0)?;
                reply(event_loop_value(&ctx.host.scene().event_loop))
            }
            "get_hdr_settings" => {
                let settings = ctx.host.scene().hdr.effective()?;
                reply(obj(vec![("text", text(settings.to_text()))]))
            }
            "set_hdr_settings" => {
                apply_hdr_update(&mut ctx.host.scene().hdr, args.0)?;
                let settings = ctx.host.scene().hdr.effective()?;
                reply(hdr_value(&settings, ctx.renderer))
            }
            "get_hdr" => {
                let settings = ctx.host.scene().hdr.effective()?;
                reply(hdr_value(&settings, ctx.renderer))
            }
            "get_fog_settings" => reply(fog_settings_value(&ctx.host.scene().fog)),
            "set_fog_settings" => {
                apply_fog_update(&mut ctx.host.scene().fog, args.0)?;
                reply(fog_settings_value(&ctx.host.scene().fog))
            }
            "get_fog_stats" => reply(fog_stats_value(ctx.host.scene(), ctx.renderer)),
            "get_menu_settings" => reply(menu_value(&ctx.host.scene().menu.effective())),
            "set_menu_settings" => {
                apply_menu_update(&mut ctx.host.scene().menu, args.0)?;
                reply(menu_value(&ctx.host.scene().menu.effective()))
            }
            "quit" => {
                self.quit = Some(args.f64("code")?.unwrap_or(0.0) as i32);
                reply(obj(vec![("quit", Value::Bool(true))]))
            }
            "list_scenes" => reply(list(ctx.host.list_scenes().into_iter().map(text).collect())),
            "load_scene" => {
                let name = args.str("name")?.ok_or("load_scene needs name")?;
                let checkpoint = args.str("checkpoint")?.unwrap_or_default();
                match ctx.host.begin_load(&name)? {
                    LoadPace::Ready => {
                        self.drop_scene_edits();
                        ctx.renderer.reset_lighting_cache();
                        reply(Self::scene_load_reply(ctx, &name, &checkpoint)?)
                    }
                    LoadPace::Pending => Ok(Step::Job(Job::SceneLoad { name, checkpoint })),
                }
            }
            "list_checkpoints" => reply(list(
                ctx.host.list_checkpoints().into_iter().map(text).collect(),
            )),
            "jump_checkpoint" => {
                let name = args.str("name")?.ok_or("jump_checkpoint needs name")?;
                let pose = ctx.host.jump_checkpoint(&name)?;
                Self::place_checkpoint(ctx.camera, pose);
                reply(obj(vec![
                    ("checkpoint", text(name)),
                    ("camera", camera_value(ctx.camera)),
                ]))
            }
            _ => Err(format!("unknown command {name}")),
        }
    }

    fn poll_scene_loads(&mut self, ctx: &mut Ctx) {
        let pending = self
            .jobs
            .iter()
            .any(|running| matches!(running.job, Job::SceneLoad { .. }));
        if !pending {
            return;
        }
        match ctx.host.poll_load() {
            Ok(LoadPace::Pending) => {}
            Ok(LoadPace::Ready) => {
                self.drop_scene_edits();
                ctx.renderer.reset_lighting_cache();
                let ready: Vec<(usize, u64, String, String)> = self
                    .jobs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, running)| match &running.job {
                        Job::SceneLoad { name, checkpoint } => {
                            Some((index, running.id, name.clone(), checkpoint.clone()))
                        }
                        _ => None,
                    })
                    .collect();
                for (index, id, name, checkpoint) in ready.into_iter().rev() {
                    let result = Self::scene_load_reply(ctx, &name, &checkpoint);
                    self.jobs.remove(index);
                    genos_mcp::command::reply(id, result);
                }
            }
            Err(err) => {
                let failed: Vec<(usize, u64)> = self
                    .jobs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, running)| match running.job {
                        Job::SceneLoad { .. } => Some((index, running.id)),
                        _ => None,
                    })
                    .collect();
                for (index, id) in failed.into_iter().rev() {
                    self.jobs.remove(index);
                    genos_mcp::command::reply(id, Err(err.clone()));
                }
            }
        }
    }

    fn scene_load_reply(ctx: &mut Ctx, name: &str, checkpoint: &str) -> Result<Value, String> {
        let checkpoint = if checkpoint.is_empty() {
            ctx.host
                .list_checkpoints()
                .into_iter()
                .next()
                .unwrap_or_default()
        } else {
            checkpoint.to_string()
        };
        if !checkpoint.is_empty() {
            let pose = ctx.host.jump_checkpoint(&checkpoint)?;
            Self::place_checkpoint(ctx.camera, pose);
        }
        Ok(obj(vec![
            ("scene", text(name.to_string())),
            ("checkpoint", text(checkpoint)),
            ("camera", camera_value(ctx.camera)),
        ]))
    }

    fn place_checkpoint(camera: &mut Camera, pose: CheckpointPose) {
        camera.set_pose(
            Vec3::new(pose.position[0], pose.position[1], pose.position[2]),
            pose.yaw,
            pose.pitch,
        );
    }

    /// `solo_off` with no index and no `add` only clears the solo view.
    fn light_solo_off_is_complete(args: &Args) -> Result<bool, String> {
        Ok(args.bool("solo_off")? == Some(true)
            && args.bool("add")? != Some(true)
            && args.u32("index")?.is_none())
    }

    fn light_cmd(&mut self, args: &Args, ctx: &mut Ctx) -> Result<Value, String> {
        if args.bool("solo_off")? == Some(true) {
            self.solo = None;
            if Self::light_solo_off_is_complete(args)? {
                return Ok(obj(vec![("solo", Value::Null)]));
            }
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
        let gpu: Vec<f64> = recent.iter().filter_map(|(_, t)| t.gpu_ms).collect();
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
        ])
    }
}

enum Step {
    Reply(Value),
    Job(Job),
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

/// A material value must lie in 0..=1.
fn unit(key: &str, v: f32) -> Result<f32, String> {
    if (0.0..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err(format!("{key} is 0 to 1, not {v}"))
    }
}

fn finish_reference(
    job: &mut RefJob,
    out_dir: &std::path::Path,
    captures: &mut HashMap<String, Image>,
    references: &mut HashMap<String, Reference>,
    legacy_display: bool,
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
        (
            "geometry",
            text(if job.triangles { "triangles" } else { "shapes" }),
        ),
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
    if job.aov == Aov::Specular {
        let spec = r
            .specular_image()
            .ok_or("aov specular asked, but the trace kept no specular layer")?;
        spec.save(&path("reference-spec"))?;
        fields.push((
            "reference_spec",
            text(path("reference-spec").display().to_string()),
        ));
    }
    if let Some(live) = &job.live {
        let (c, heat, grade) = if let Some(scene) = &job.scene {
            let (c, heat) = reference::compare_linear(scene, &r, job.region, job.grid, job.block);
            (c, heat, "scene")
        } else if legacy_display {
            let (c, heat) = reference::compare(live, &r, job.region, job.grid, job.block);
            (c, heat, "display")
        } else {
            return Err(
                "the scene image is missing, so this display transform is not graded".into(),
            );
        };
        live.resized(r.image.width, r.image.height)
            .save(&path("live"))?;
        heat.save(&path("heatmap"))?;
        let regions = c
            .regions
            .iter()
            .map(|(e, b)| obj(vec![("mean_rel", num(*e as f64)), ("bias", num(*b as f64))]))
            .collect();
        fields.extend([
            ("grade", text(grade)),
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

fn frames_reply(frame: u64, left: &mut u32, accepted: u64) -> Option<Value> {
    *left = left.saturating_sub(1);
    (*left == 0).then(|| {
        obj(vec![
            ("frame", num(frame as f64)),
            ("accepted", num(accepted as f64)),
        ])
    })
}

/// The picture this draw should read, if any.
///
/// `completed` is the present count before this draw. The draw itself will be
/// numbered `completed + 1`.
fn read_this_draw(completed: u64, job: &Job) -> Option<Shot> {
    let next = completed.saturating_add(1);
    match job {
        Job::Shots { shots, got, at, .. } => {
            let take = match at {
                Some(target) => *target == next && got.is_empty(),
                None => got.len() < shots.len(),
            };
            take.then(|| shots.get(got.len()).copied()).flatten()
        }
        Job::Settle {
            hold: Some(hold), ..
        } => (hold.left == 1)
            .then(|| {
                hold.capture.as_ref().map(|cap| Shot {
                    look: cap.look,
                    ui: cap.ui,
                })
            })
            .flatten(),
        Job::Flicker { shot, .. } => Some(*shot),
        Job::Reference(job) if job.live_due() => job.shot,
        _ => None,
    }
}

/// `requested` is the capture `frame` argument. A present that has already
/// been drawn cannot be saved later.
fn capture_at(now: u64, requested: Option<u32>) -> Result<Option<u64>, String> {
    let Some(target) = requested else {
        return Ok(None);
    };
    let target = u64::from(target);
    if target <= now {
        Err(format!("capture frame {target} already presented at {now}"))
    } else {
        Ok(Some(target))
    }
}

fn stamp_frame(result: Result<Value, String>, frame: u64) -> Result<Value, String> {
    let mut value = result?;
    if let Value::Object(pairs) = &mut value {
        pairs.push(("frame".to_string(), num(frame as f64)));
    }
    Ok(value)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the settle reply's fields stay in one place. docs/LINT_RULES.md"
)]
fn settle_reply(
    settled: bool,
    frames: u32,
    wall_ms: f64,
    still: u32,
    settled_frame: Option<u64>,
    saved: u64,
    captured: bool,
    picture: Option<Value>,
) -> Value {
    let mut pairs = vec![
        ("settled".to_string(), Value::Bool(settled)),
        ("frames".to_string(), num(f64::from(frames))),
        ("wall_ms".to_string(), num((wall_ms * 10.0).round() / 10.0)),
        ("still_frames".to_string(), num(f64::from(still))),
        ("frame".to_string(), num(saved as f64)),
        (
            "settled_frame".to_string(),
            settled_frame.map_or(Value::Null, |frame| num(frame as f64)),
        ),
        ("captured".to_string(), Value::Bool(captured)),
    ];
    if let Some(Value::Object(extra)) = picture {
        for (key, value) in extra {
            if key != "frame" {
                pairs.push((key, value));
            }
        }
    }
    Value::Object(pairs)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the settle job's fields stay in one place. docs/LINT_RULES.md"
)]
fn settle_on_present(
    frame: u64,
    image: Option<&Image>,
    settled_now: bool,
    still_frames: u32,
    frame_wall_ms: f64,
    left: &mut u32,
    quiet: &mut u32,
    need: u32,
    frames: &mut u32,
    wall_ms: &mut f64,
    then_frames: Option<u32>,
    capture: &mut Option<FrameCapture>,
    hold: &mut Option<SettleHold>,
    captures: &mut HashMap<String, Image>,
) -> Option<Result<Value, String>> {
    if let Some(hold) = hold {
        hold.left = hold.left.saturating_sub(1);
        if hold.left > 0 {
            return None;
        }
        let expected = hold.settled_frame.saturating_add(u64::from(hold.total));
        if frame != expected {
            return Some(Err(format!(
                "capture frame {frame} is not settled frame {} plus {}",
                hold.settled_frame, hold.total
            )));
        }
        let Some(cap) = hold.capture.clone() else {
            return Some(Ok(settle_reply(
                true,
                hold.settled_frames,
                hold.settled_wall_ms,
                hold.settled_still,
                Some(hold.settled_frame),
                frame,
                false,
                None,
            )));
        };
        let Some(image) = image else {
            return Some(Err(format!("missing capture at frame {frame}")));
        };
        let saved = save_frame(image, &cap, captures);
        return Some(saved.map(|picture| {
            settle_reply(
                true,
                hold.settled_frames,
                hold.settled_wall_ms,
                hold.settled_still,
                Some(hold.settled_frame),
                frame,
                true,
                Some(picture),
            )
        }));
    }
    *frames = frames.saturating_add(1);
    *wall_ms += frame_wall_ms;
    *quiet = if settled_now {
        quiet.saturating_add(1)
    } else {
        0
    };
    *left = left.saturating_sub(1);
    let settled = *quiet >= need;
    if settled {
        let tail = then_frames.unwrap_or(0);
        if tail == 0 {
            return Some(Ok(settle_reply(
                true,
                *frames,
                *wall_ms,
                still_frames,
                Some(frame),
                frame,
                false,
                None,
            )));
        }
        *hold = Some(SettleHold {
            left: tail,
            total: tail,
            settled_frame: frame,
            settled_still: still_frames,
            settled_frames: *frames,
            settled_wall_ms: *wall_ms,
            capture: capture.take(),
        });
        None
    } else if *left == 0 {
        Some(Ok(settle_reply(
            false,
            *frames,
            *wall_ms,
            still_frames,
            None,
            frame,
            false,
            None,
        )))
    } else {
        None
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the capture job's fields stay in one place. docs/LINT_RULES.md"
)]
fn shots_on_present(
    frame: u64,
    image: Option<&Image>,
    image_used: &mut bool,
    shots: &[Shot],
    got: &mut Vec<Image>,
    then: &Then,
    at: Option<u64>,
    captures: &mut HashMap<String, Image>,
) -> Option<Result<Value, String>> {
    if let Some(target) = at {
        if frame < target {
            return None;
        }
        if frame > target {
            return Some(Err(format!(
                "capture frame {frame} is not scheduled frame {target}"
            )));
        }
        if got.len() >= shots.len() {
            return Some(stamp_frame(finish_shots(shots, got, then, captures), frame));
        }
        let Some(image) = image.filter(|_| !*image_used) else {
            return Some(Err(format!("missing capture at frame {frame}")));
        };
        got.push(image.clone());
        *image_used = true;
        if got.len() == shots.len() {
            Some(stamp_frame(finish_shots(shots, got, then, captures), frame))
        } else {
            Some(Err(format!("missing capture at frame {frame}")))
        }
    } else {
        if !*image_used && got.len() < shots.len() {
            if let Some(image) = image {
                got.push(image.clone());
                *image_used = true;
            }
        }
        (got.len() == shots.len())
            .then(|| stamp_frame(finish_shots(shots, got, then, captures), frame))
    }
}

fn save_frame(
    image: &Image,
    cap: &FrameCapture,
    captures: &mut HashMap<String, Image>,
) -> Result<Value, String> {
    let pictures = [image.clone()];
    finish_shots(
        &[Shot {
            look: cap.look,
            ui: cap.ui,
        }],
        &pictures,
        &Then::Save {
            paths: vec![cap.path.clone()],
            names: vec![None],
            size: cap.size,
            region: Region::default(),
        },
        captures,
    )
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

fn lighting_report(body: &str, renderer: &Renderer) -> Value {
    let ms = renderer.lighting_pass_ms();
    let counts = renderer.lighting_dispatches();
    let names = Renderer::lighting_bucket_names();
    let passes = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            obj(vec![
                ("name", text(*name)),
                ("ms", num(ms.get(i).copied().unwrap_or(0.0))),
                (
                    "dispatches",
                    num(counts.get(i).copied().unwrap_or(0) as f64),
                ),
            ])
        })
        .collect();
    obj(vec![
        ("text", text(body)),
        ("passes", list(passes)),
        ("post_ms_device", num(renderer.post_ms_device())),
        ("post_vram_device", num(renderer.post_vram_device())),
    ])
}

fn lighting_value_text(value: &Value) -> Result<String, String> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    if let Some(on) = value.as_bool() {
        return Ok(if on { "on".into() } else { "off".into() });
    }
    if let Some(n) = value.as_f64() {
        if n.fract() == 0.0 && n.abs() < 1.0e15 {
            return Ok(format!("{n:.0}"));
        }
        return Ok(format!("{n:?}"));
    }
    if let Some(items) = value.as_array() {
        let parts = items
            .iter()
            .map(lighting_value_text)
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(parts.join(","));
    }
    Err("a lighting value is a string, number, bool, or list".into())
}

fn event_loop_value(settings: &EventLoopSettings) -> Value {
    obj(vec![
        ("text", text(settings.to_text())),
        ("longest_stall_ms", num(settings.longest_stall_ms)),
    ])
}

fn apply_event_loop_update(settings: &mut EventLoopSettings, args: &Value) -> Result<(), String> {
    let Value::Object(pairs) = args else {
        return Err("event-loop settings are an object".into());
    };
    if let Some(body) = args.get("text").and_then(|v| v.as_str()) {
        for line in body.split(|c| c == '\n' || c == ';') {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("event-loop line needs key=value, got {line}"));
            };
            settings.set_key(key, value)?;
        }
    }
    for (key, value) in pairs {
        if key == "text" {
            continue;
        }
        settings.set_key(key, &event_loop_value_text(value)?)?;
    }
    Ok(())
}

fn event_loop_value_text(value: &Value) -> Result<String, String> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    if let Some(n) = value.as_f64() {
        if n.fract() == 0.0 && n.abs() < 1.0e15 {
            return Ok(format!("{n:.0}"));
        }
        return Ok(format!("{n:?}"));
    }
    Err("an event-loop value is a string or a number".into())
}

fn reflection_stats_value(grade: Option<&genos_render::ReflectionGrade>) -> Value {
    let ran = grade.is_some_and(|grade| grade.ran);
    let rays = grade.map_or(0, |grade| grade.rays);
    let views = grade.map_or(0, |grade| grade.views);
    let gpu_ns = grade.map_or(0, |grade| grade.gpu_ns);
    let vram_bytes = grade.map_or(0, |grade| grade.vram_bytes);
    obj(vec![
        ("ran", Value::Bool(ran)),
        ("rays", num(f64::from(rays))),
        ("views", num(f64::from(views))),
        ("gpu_ns", num(gpu_ns as f64)),
        ("vram_bytes", num(vram_bytes as f64)),
        ("ms_frac", text("NOT-IMPLEMENTED")),
        ("vram_frac", text("NOT-IMPLEMENTED")),
    ])
}

fn reflection_value(settings: &ReflectionSettings) -> Value {
    obj(vec![
        ("text", text(settings.to_text())),
        ("enabled", Value::Bool(settings.enabled)),
        ("rough_cutoff", num(f64::from(settings.rough_cutoff))),
        ("mirror_cutoff", num(f64::from(settings.mirror_cutoff))),
        ("mirror_views", num(f64::from(settings.mirror_views))),
        ("rays", num(f64::from(settings.rays))),
        ("shore_cap", num(f64::from(settings.shore_cap))),
        ("caustics", text(settings.caustics.as_str())),
        ("tracer", text(settings.tracer.as_str())),
        ("water", Value::Bool(settings.water)),
    ])
}

fn apply_reflection_update(settings: &mut ReflectionSettings, args: &Value) -> Result<(), String> {
    let Value::Object(pairs) = args else {
        return Err("reflection settings are an object".into());
    };
    if let Some(body) = args.get("text").and_then(|v| v.as_str()) {
        for line in body.split(|c| c == '\n' || c == ';') {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("reflection line needs key=value, got {line}"));
            };
            settings.set_key(key, value)?;
        }
    }
    for (key, value) in pairs {
        if key == "text" {
            continue;
        }
        settings.set_key(key, &reflection_value_text(value)?)?;
    }
    Ok(())
}

fn reflection_value_text(value: &Value) -> Result<String, String> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    if let Some(on) = value.as_bool() {
        return Ok(if on { "on" } else { "off" }.to_string());
    }
    if let Some(number) = value.as_f64() {
        if number.fract() == 0.0 && number.abs() < 1.0e7 {
            return Ok(format!("{:.0}", number.round()));
        }
        return Ok(format!("{number:?}"));
    }
    Err("a reflection value is a string, number, or bool".into())
}

fn hdr_value(settings: &HdrSettings, renderer: &Renderer) -> Value {
    let (format, color_space) = renderer.hdr_surface();
    let chosen = match settings.output {
        OutputMode::Auto => "auto",
        OutputMode::Sdr => "sdr",
        OutputMode::Hdr10 => "hdr10",
        OutputMode::Scrgb => "scrgb",
    };
    obj(vec![
        ("text", text(settings.to_text())),
        ("ev100", num(renderer.hdr_ev100())),
        ("exposure", num(renderer.hdr_exposure())),
        (
            "scales",
            list(renderer.hdr_scales().into_iter().map(num).collect()),
        ),
        ("grade", text("scene")),
        ("still_pop", num(renderer.hdr_still_pop())),
        (
            "histogram",
            list(
                renderer
                    .hdr_histogram()
                    .into_iter()
                    .map(|count| num(count as f64))
                    .collect(),
            ),
        ),
        (
            "surfaces",
            list(vec![obj(vec![
                ("format", num(format as f64)),
                ("color_space", num(color_space as f64)),
            ])]),
        ),
        ("chosen", text(chosen)),
        ("resolved", text(renderer.hdr_resolved())),
        ("pq_white", num(renderer.hdr_pq_white())),
        ("scrgb_white", num(renderer.hdr_scrgb_white())),
        ("hdr_ms", num(renderer.hdr_ms())),
        ("hdr_vram_bytes", num(renderer.hdr_vram_bytes())),
        ("post_ms_device", num(renderer.post_ms_device())),
        ("post_vram_device", num(renderer.post_vram_device())),
    ])
}

fn apply_hdr_update(settings: &mut HdrSettings, args: &Value) -> Result<(), String> {
    let Value::Object(pairs) = args else {
        return Err("hdr settings are an object".into());
    };
    if let Some(body) = args.get("text").and_then(|v| v.as_str()) {
        for line in body.split(|c| c == '\n' || c == ';') {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("hdr line needs key=value, got {line}"));
            };
            settings.set_key(key, value)?;
        }
    }
    for (key, value) in pairs {
        if key == "text" {
            continue;
        }
        settings.set_key(key, &lighting_value_text(value)?)?;
    }
    Ok(())
}



fn fog_settings_value(settings: &FogSettings) -> Value {
    obj(vec![
        ("text", text(settings.to_text())),
        ("enabled", Value::Bool(settings.enabled)),
        ("density", num(f64::from(settings.density))),
        ("height", num(f64::from(settings.height))),
        ("thickness", num(f64::from(settings.thickness))),
        ("albedo", num(f64::from(settings.albedo))),
        ("phase_g", num(f64::from(settings.phase_g))),
        ("gi", Value::Bool(settings.gi)),
        ("history", Value::Bool(settings.history)),
        ("grid_width", num(f64::from(settings.grid_width))),
        ("grid_height", num(f64::from(settings.grid_height))),
        ("grid_slices", num(f64::from(settings.grid_slices))),
        ("samples", num(f64::from(settings.samples))),
        ("view", text(settings.inspect.name())),
        ("view_line", text(settings.inspect_line())),
    ])
}

fn apply_fog_update(settings: &mut FogSettings, args: &Value) -> Result<(), String> {
    let Value::Object(pairs) = args else {
        return Err("fog settings are an object".into());
    };
    if let Some(body) = args.get("text").and_then(|v| v.as_str()) {
        for line in body.split(|c| c == '\n' || c == ';') {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("fog line needs key=value, got {line}"));
            };
            settings.set_key(key, value)?;
        }
    }
    for (key, value) in pairs {
        if key == "text" {
            continue;
        }
        settings.set_key(key, &fog_value_text(value)?)?;
    }
    Ok(())
}

fn fog_value_text(value: &Value) -> Result<String, String> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    if let Some(on) = value.as_bool() {
        return Ok(if on { "on".into() } else { "off".into() });
    }
    if let Some(n) = value.as_f64() {
        if n.fract() == 0.0 && n.abs() < 1.0e15 {
            return Ok(format!("{n:.0}"));
        }
        return Ok(format!("{n:?}"));
    }
    Err("a fog value is a string, number, or bool".into())
}

fn fog_stats_value(scene: &Scene, renderer: &Renderer) -> Value {
    let probe = renderer.fog_frame_probe(&scene.fog, scene.lighting.preset);
    let bytes = u32::try_from(probe.vram_bytes).unwrap_or(u32::MAX);
    obj(vec![
        ("dispatches", num(f64::from(probe.dispatches))),
        ("samples_used", num(f64::from(probe.samples_used))),
        ("samples_capped", num(f64::from(probe.samples_capped))),
        ("grid_width", num(f64::from(probe.grid_width))),
        ("grid_height", num(f64::from(probe.grid_height))),
        ("grid_slices", num(f64::from(probe.grid_slices))),
        ("dispatched_width", num(f64::from(probe.dispatched_width))),
        ("dispatched_height", num(f64::from(probe.dispatched_height))),
        (
            "dispatched_slices",
            num(f64::from(probe.dispatched_slices)),
        ),
        ("history_accepts", num(f64::from(probe.history_accepts))),
        ("history_rejects", num(f64::from(probe.history_rejects))),
        ("vram_bytes", num(f64::from(bytes))),
        ("pass_ms", num(probe.pass_ms)),
        (
            "pass_ms_label",
            text("last frame lavapipe timestamp when fog dispatched; 0 before that frame. Not graded against an RTX 4070 ceiling. History stays off."),
        ),
        ("view", text(scene.fog.inspect.name())),
        ("view_line", text(scene.fog.inspect_line())),
        ("history_weight", num(0.0)),
    ])
}

fn fps_counter_value(settings: &FpsCounterSettings) -> Value {
    obj(vec![
        ("text", text(settings.to_text())),
        ("enabled", Value::Bool(settings.enabled)),
        ("rate_hz", num(settings.rate_hz as f64)),
    ])
}

fn menu_value(settings: &MenuSettings) -> Value {
    obj(vec![
        ("text", text(settings.to_text())),
        ("visible", Value::Bool(settings.visible)),
        ("tab", text(settings.tab.clone())),
        (
            "collapsed",
            list(settings.collapsed.iter().cloned().map(text).collect()),
        ),
        ("wheel", text(settings.wheel.as_str())),
        ("keys", text(settings.keys.as_str())),
    ])
}

fn apply_menu_update(settings: &mut MenuSettings, args: &Value) -> Result<(), String> {
    let Value::Object(pairs) = args else {
        return Err("menu settings are an object".into());
    };
    if let Some(body) = args.get("text").and_then(|v| v.as_str()) {
        for line in body.split(|c| c == '\n' || c == ';') {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("menu line needs key=value, got {line}"));
            };
            settings.set_key(key, value)?;
        }
    }
    for (key, value) in pairs {
        if key == "text" {
            continue;
        }
        settings.set_key(key, &menu_value_text(value)?)?;
    }
    Ok(())
}

fn menu_value_text(value: &Value) -> Result<String, String> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    if let Some(on) = value.as_bool() {
        return Ok(if on { "shown".into() } else { "hidden".into() });
    }
    if let Some(items) = value.as_array() {
        let parts = items
            .iter()
            .map(menu_value_text)
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(parts.join(","));
    }
    if let Some(n) = value.as_f64() {
        if n.fract() == 0.0 && n.abs() < 1.0e15 {
            return Ok(format!("{}", n as i64));
        }
        return Ok(format!("{n:?}"));
    }
    Err("a menu value is a string, number, bool, or list".into())
}

fn apply_fps_update(settings: &mut FpsCounterSettings, args: &Value) -> Result<(), String> {
    let Value::Object(pairs) = args else {
        return Err("fps counter settings are an object".into());
    };
    if let Some(body) = args.get("text").and_then(|v| v.as_str()) {
        for line in body.split(|c| c == '\n' || c == ';') {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("fps counter line needs key=value, got {line}"));
            };
            settings.set_key(key, value)?;
        }
    }
    for (key, value) in pairs {
        if key == "text" {
            continue;
        }
        settings.set_key(key, &fps_value_text(value)?)?;
    }
    Ok(())
}

fn fps_value_text(value: &Value) -> Result<String, String> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    if let Some(on) = value.as_bool() {
        return Ok(if on { "on".into() } else { "off".into() });
    }
    if let Some(n) = value.as_f64() {
        if n.fract() == 0.0 && n.abs() < 1.0e15 {
            return Ok(format!("{}", n as i64));
        }
        return Ok(format!("{n:?}"));
    }
    Err("an fps counter value is a string, number, or bool".into())
}

fn apply_debug_update(settings: &mut DebugSettings, args: &Value) -> Result<(), String> {
    let Value::Object(pairs) = args else {
        return Err("debug settings are an object".into());
    };
    if let Some(text) = args.get("text").and_then(|v| v.as_str()) {
        for line in text.split(|c| c == '\n' || c == ';') {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("debug line needs key=value, got {line}"));
            };
            settings.set_key(key, value)?;
        }
    }
    for (key, value) in pairs {
        if key == "text" {
            continue;
        }
        let text = if let Some(text) = value.as_str() {
            text.to_string()
        } else if let Some(on) = value.as_bool() {
            if on {
                "1".into()
            } else {
                "0".into()
            }
        } else {
            return Err(format!("debug setting {key} is a string or a bool"));
        };
        settings.set_key(key, &text)?;
    }
    Ok(())
}

fn material_stats_value(stats: MaterialStats) -> Value {
    obj(vec![
        ("committed", num(stats.committed as f64)),
        ("budget", num(stats.budget as f64)),
        ("refused", num(f64::from(stats.refused))),
        ("evictions", num(f64::from(stats.evictions))),
        ("thrash", num(f64::from(stats.thrash))),
        ("holes", num(f64::from(stats.holes))),
        ("rate", num(stats.rate as f64)),
        ("stream_ms", num(f64::from(stats.stream_ms))),
        ("text", text(stats.line())),
    ])
}

fn apply_material_update(settings: &mut MaterialSettings, args: &Value) -> Result<(), String> {
    let Value::Object(pairs) = args else {
        return Err("material settings are an object".into());
    };
    if let Some(text) = args.get("text").and_then(|v| v.as_str()) {
        for line in text.split(|c| c == '\n' || c == ';') {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("material line needs key=value, got {line}"));
            };
            settings.set_key(key.trim(), value.trim())?;
        }
    }
    for (key, value) in pairs {
        if key == "text" {
            continue;
        }
        settings.set_key(key, &lighting_value_text(value)?)?;
    }
    Ok(())
}

fn apply_lighting_update(settings: &mut LightingSettings, args: &Value) -> Result<(), String> {
    let Value::Object(pairs) = args else {
        return Err("lighting settings are an object".into());
    };
    if let Some(text) = args.get("text").and_then(|v| v.as_str()) {
        for line in text.split(|c| c == '\n' || c == ';') {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("lighting line needs key=value, got {line}"));
            };
            settings.set_key(key, value)?;
        }
    }
    for (key, value) in pairs {
        if key == "text" {
            continue;
        }
        settings.set_key(key, &lighting_value_text(value)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use genos_mcp::json::Value;

    use super::*;

    /// Blocks this thread for ms (a receive that nothing sends to).
    fn wait_ms(ms: u64) {
        let (_tx, rx) = std::sync::mpsc::channel::<()>();
        let _ = rx.recv_timeout(std::time::Duration::from_millis(ms));
    }

    fn stub_reference() -> Reference {
        Reference {
            image: Image { width: 1, height: 1, rgb: vec![0, 0, 0] },
            linear: vec![[0.0; 3]],
            hit: vec![true],
            spp: 1,
            seconds: 0.0,
            noise: 0.0,
            specular: None,
        }
    }

    fn stub_job(measure: bool, delay_ms: u64) -> RefJob {
        RefJob {
            cache: None,
            spp: 1,
            max_spp: 1,
            noise_target: 0.0,
            seconds: 0.0,
            block: 1,
            max_bounces: 1,
            triangles: false,
            aov: Aov::Full,
            size: (1, 1),
            worker: Some(std::thread::spawn(move || {
                wait_ms(delay_ms);
                stub_reference()
            })),
            reference: None,
            frames_left: 0,
            shot: Some(Shot { look: Look::parse("full").unwrap(), ui: false }),
            live: None,
            name: "reference".into(),
            prefix: None,
            grid: (1, 1),
            region: Region([0.0, 0.0, 1.0, 1.0]),
            paused: true,
            scene: None,
            legacy_display: true,
            timing: if measure {
                RefTiming::Measure
            } else {
                RefTiming::Interactive
            },
        }
    }

    /// Draws frames (draw i's picture is the byte i) until the job holds its
    /// live picture and its trace. Returns the live byte and the frames drawn.
    fn drive(job: &mut RefJob) -> (u8, u32) {
        let mut draws = 0u32;
        while job.live.is_none() || job.reference.is_none() {
            let img = Image { width: 1, height: 1, rgb: vec![u8::try_from(draws).unwrap_or(u8::MAX); 3] };
            job.on_draw(Some(&img));
            draws += 1;
            if job.reference.is_none() {
                wait_ms(5);
            }
            assert!(draws < 1000, "the reference job never finished");
        }
        (job.live.as_ref().unwrap().rgb[0], draws)
    }

    #[test]
    fn a_measured_reference_holds_the_requested_frame_whatever_the_trace_takes() {
        for delay_ms in [0, 40, 160] {
            let mut job = stub_job(true, delay_ms);
            assert_eq!(drive(&mut job), (0, 1), "trace delay {delay_ms} ms");
        }
    }

    /// Draws until the trace is held; the number of frames drawn.
    fn drive_trace(job: &mut RefJob) -> u32 {
        let mut draws = 0u32;
        while job.reference.is_none() {
            job.on_draw(None);
            draws += 1;
            wait_ms(5);
            assert!(draws < 1000, "the trace never finished");
        }
        draws
    }

    #[test]
    fn a_measured_trace_draws_one_frame_whatever_the_trace_takes() {
        for delay_ms in [0, 40, 160] {
            let mut job = stub_job(true, delay_ms);
            job.shot = None;
            assert_eq!(drive_trace(&mut job), 1, "trace delay {delay_ms} ms");
        }
    }

    #[test]
    fn the_measure_argument_selects_measure_mode() {
        use genos_mcp::json::{object, Value};
        assert_eq!(ref_timing(&Args(&object([]))), Ok(RefTiming::Interactive));
        assert_eq!(
            ref_timing(&Args(&object([("measure", Value::Bool(true))]))),
            Ok(RefTiming::Measure)
        );
        assert_eq!(
            ref_timing(&Args(&object([("measure", Value::Bool(false))]))),
            Ok(RefTiming::Interactive)
        );
    }

    #[test]
    fn an_interactive_reference_still_shows_the_frame_after_the_trace() {
        let mut job = stub_job(false, 160);
        let (live, draws) = drive(&mut job);
        assert!(draws > 1 && u32::from(live) == draws - 1, "live {live} after {draws} draws");
    }

    #[test]
    fn pause_holds_the_clock_until_a_step_is_requested() {
        let mut tools = Tools::new();
        tools.set_fixed_step(Some(DEFAULT_STEP));
        tools.pause();
        assert_eq!(tools.scene_dt(1.0), 0.0);
        tools.request_steps(1);
        let stepped = tools.scene_dt(1.0);
        assert!((stepped - DEFAULT_STEP).abs() < 1.0e-6);
        assert_eq!(tools.scene_dt(1.0), 0.0);
    }

    #[test]
    fn a_capture_leaves_the_ui_out_unless_asked() {
        let tools = Tools::new();
        let plain = tools
            .shot_from(&Args(&genos_mcp::json::object([])), false)
            .unwrap();
        assert!(!plain.ui);
        assert!(
            tools
                .shot_from(&Args(&genos_mcp::json::object([])), true)
                .unwrap()
                .ui
        );
        let asked = tools
            .shot_from(
                &Args(&genos_mcp::json::object([(
                    "ui",
                    genos_mcp::json::bool(true),
                )])),
                false,
            )
            .unwrap();
        assert!(asked.ui);
        assert!(
            !tools
                .shot_from(
                    &Args(&genos_mcp::json::object([(
                        "ui",
                        genos_mcp::json::bool(false)
                    )])),
                    true,
                )
                .unwrap()
                .ui
        );
        assert!(Tools::shot_ui(
            &Args(&genos_mcp::json::object([(
                "ui",
                genos_mcp::json::string("nope"),
            )])),
            false,
        )
        .is_err());
        assert!(!tools.show_capture_overlay(false));
        assert!(tools.show_capture_overlay(true));
        let mut tools = tools;
        tools.reading = Some(Shot {
            look: tools.look,
            ui: false,
        });
        assert!(!tools.show_capture_overlay(true));
        tools.reading = Some(Shot {
            look: tools.look,
            ui: true,
        });
        assert!(tools.show_capture_overlay(false));
        let capture = crate::commands::specs()
            .into_iter()
            .find(|spec| spec.name == "capture")
            .unwrap();
        assert!(capture.description.contains("ui"));
        assert!(capture.description.contains("default false"));
        let captures = crate::commands::specs()
            .into_iter()
            .find(|spec| spec.name == "captures")
            .unwrap();
        assert!(captures.description.contains("ui"));
        assert!(captures.description.contains("default false"));
    }

    #[test]
    fn a_fold_sent_with_a_tab_stays_in_collapsed() {
        let mut settings = MenuSettings::default();
        let args = genos_mcp::json::parse(r#"{"fold":"direct","tab":"GI"}"#).expect("json");
        apply_menu_update(&mut settings, &args).expect("update");
        assert_eq!(settings.tab, "GI");
        assert_eq!(settings.collapsed, vec!["direct".to_string()]);
        let text = settings.effective().to_text();
        assert!(text.contains("collapsed=direct"), "reply text was {text}");
        assert!(text.contains("tab=GI"), "{text}");
    }

    #[test]
    fn solo_off_does_not_need_an_index() {
        let only = genos_mcp::json::parse(r#"{"solo_off":true}"#).expect("json");
        assert!(Tools::light_solo_off_is_complete(&Args(&only)).expect("solo_off"));
        let indexed = genos_mcp::json::parse(r#"{"solo_off":true,"index":1}"#).expect("json");
        assert!(!Tools::light_solo_off_is_complete(&Args(&indexed)).expect("index"));
        let added = genos_mcp::json::parse(r#"{"solo_off":true,"add":true}"#).expect("json");
        assert!(!Tools::light_solo_off_is_complete(&Args(&added)).expect("add"));
    }

    /// One present of the frame thread. `open` is the count `status` would
    /// read at the start of the present, before this present completes.
    fn present_wait(open: u64, left: &mut u32, accepted: u64) -> (u64, Option<Value>) {
        let frame = open + 1;
        (frame, frames_reply(frame, left, accepted))
    }

    #[test]
    fn a_status_bracket_counts_the_present_already_in_flight() {
        // The 4070 capture: status read 309, wait(2) replied 312, the next
        // status read 312. The old require `after == before + 2` is false.
        let before = 309_u64;
        let mut open = before;
        open += 1;
        let accepted = open;
        let mut left = 2_u32;
        let (frame, reply) = present_wait(open, &mut left, accepted);
        assert!(reply.is_none());
        open = frame;
        let (frame, reply) = present_wait(open, &mut left, accepted);
        let reply = reply.expect("wait finished");
        assert_eq!(reply.get("accepted").and_then(Value::as_u64), Some(310));
        assert_eq!(reply.get("frame").and_then(Value::as_u64), Some(312));
        assert_eq!(frame, 312);
        assert_ne!(frame, before + 2);
        assert_eq!(frame, accepted + 2);
    }

    #[test]
    fn an_extra_present_before_wait_is_accepted_widens_the_status_bracket() {
        let before = 309_u64;
        let mut open = before + 2;
        let accepted = open;
        let mut left = 2_u32;
        let (frame, reply) = present_wait(open, &mut left, accepted);
        assert!(reply.is_none());
        open = frame;
        let (frame, reply) = present_wait(open, &mut left, accepted);
        let reply = reply.expect("wait finished");
        assert_eq!(reply.get("frame").and_then(Value::as_u64), Some(before + 4));
        assert_eq!(
            reply.get("frame").and_then(Value::as_u64),
            reply.get("accepted").and_then(Value::as_u64).map(|n| n + 2)
        );
        assert_eq!(frame, before + 4);
    }

    fn pixel(frame: u64) -> Image {
        Image {
            width: 1,
            height: 1,
            rgb: vec![u8::try_from(frame).unwrap_or(255), 0, 0],
        }
    }

    fn full_look() -> Look {
        Look::parse("full").unwrap()
    }

    #[test]
    fn the_settle_tail_saves_the_present_two_after_settle() {
        let dir = std::env::temp_dir().join(format!("genos-settle-tail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("room-a-live.png");
        let mut left = 900_u32;
        let mut quiet = 0_u32;
        let mut frames = 0_u32;
        let mut wall = 0.0_f64;
        let mut capture = Some(FrameCapture {
            path: Some(path.clone()),
            size: None,
            look: full_look(),
            ui: false,
        });
        let mut hold = None;
        let mut captures = HashMap::new();
        let mut frame = 20_u64;
        let mut saved = None;
        for step in 0..8 {
            let settled_now = step >= 2;
            let still = if settled_now { step - 1 } else { 0usize };
            let due = hold
                .as_ref()
                .is_some_and(|hold: &SettleHold| hold.left == 1);
            frame += 1;
            let image = due.then(|| pixel(frame));
            let reply = settle_on_present(
                frame,
                image.as_ref(),
                settled_now,
                u32::try_from(still).unwrap_or(0),
                1.0,
                &mut left,
                &mut quiet,
                3,
                &mut frames,
                &mut wall,
                Some(2),
                &mut capture,
                &mut hold,
                &mut captures,
            );
            if let Some(reply) = reply {
                saved = Some((frame, reply.expect("tail")));
                break;
            }
        }
        let (saved_frame, reply) = saved.expect("the tail replies");
        assert_eq!(reply.get("settled").and_then(Value::as_bool), Some(true));
        assert_eq!(reply.get("frames").and_then(Value::as_u64), Some(5));
        assert_eq!(reply.get("settled_frame").and_then(Value::as_u64), Some(25));
        assert_eq!(reply.get("frame").and_then(Value::as_u64), Some(27));
        assert_eq!(saved_frame, 27);
        assert_eq!(reply.get("captured").and_then(Value::as_bool), Some(true));
        assert_eq!(reply.get("still_frames").and_then(Value::as_u64), Some(3));
        let loaded = Image::load(&path).expect("png");
        let byte = u8::try_from(saved_frame).unwrap();
        assert_eq!(loaded.rgb, vec![byte, 0, 0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_tail_pixel_is_not_replaced_by_the_next_present() {
        let mut left = 900_u32;
        let mut quiet = 0_u32;
        let mut frames = 0_u32;
        let mut wall = 0.0_f64;
        let mut capture = Some(FrameCapture {
            path: Some(std::path::PathBuf::from("missing.png")),
            size: None,
            look: full_look(),
            ui: false,
        });
        let mut hold = None;
        let mut captures = HashMap::new();
        let mut frame = 0_u64;
        let mut saw_missing = false;
        for _step in 0..6 {
            frame += 1;
            let due = hold
                .as_ref()
                .is_some_and(|hold: &SettleHold| hold.left == 1);
            let image = (!due).then(|| pixel(frame));
            let reply = settle_on_present(
                frame,
                image.as_ref(),
                true,
                3,
                1.0,
                &mut left,
                &mut quiet,
                1,
                &mut frames,
                &mut wall,
                Some(2),
                &mut capture,
                &mut hold,
                &mut captures,
            );
            if let Some(reply) = reply {
                let err = reply.expect_err("missing pixels");
                assert!(err.contains("missing capture"), "{err}");
                assert!(err.contains(&frame.to_string()), "{err}");
                saw_missing = true;
                break;
            }
        }
        assert!(saw_missing);
        assert!(!std::path::Path::new("missing.png").exists());
        frame += 1;
        let late = pixel(frame);
        let reply = settle_on_present(
            frame,
            Some(&late),
            true,
            3,
            1.0,
            &mut left,
            &mut quiet,
            1,
            &mut frames,
            &mut wall,
            Some(2),
            &mut capture,
            &mut hold,
            &mut captures,
        );
        let err = reply.expect("job still held").expect_err("no later save");
        assert!(err.contains("is not settled frame"), "{err}");
        assert!(!std::path::Path::new("missing.png").exists());
    }

    #[test]
    fn a_capture_rejects_a_past_present_and_a_later_stand_in() {
        assert!(capture_at(10, Some(10))
            .unwrap_err()
            .contains("already presented"));
        assert!(capture_at(10, Some(9))
            .unwrap_err()
            .contains("already presented"));
        assert_eq!(capture_at(10, Some(12)).unwrap(), Some(12));
        assert_eq!(capture_at(10, None).unwrap(), None);

        let dir = std::env::temp_dir().join(format!("genos-capture-at-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shot.png");
        let shot = Shot {
            look: full_look(),
            ui: false,
        };
        let mut got = Vec::new();
        let then = Then::Save {
            paths: vec![Some(path.clone())],
            names: vec![None],
            size: None,
            region: Region::default(),
        };
        let mut captures = HashMap::new();
        let mut used = false;
        let early = pixel(11);
        assert!(shots_on_present(
            11,
            Some(&early),
            &mut used,
            &[shot],
            &mut got,
            &then,
            Some(12),
            &mut captures,
        )
        .is_none());
        assert!(!path.exists());
        used = false;
        let due = read_this_draw(
            11,
            &Job::Shots {
                shots: vec![shot],
                got: Vec::new(),
                then: Then::Save {
                    paths: vec![Some(path.clone())],
                    names: vec![None],
                    size: None,
                    region: Region::default(),
                },
                at: Some(12),
            },
        );
        assert!(due.is_some());
        let exact = pixel(12);
        let saved = shots_on_present(
            12,
            Some(&exact),
            &mut used,
            &[shot],
            &mut got,
            &then,
            Some(12),
            &mut captures,
        )
        .expect("target present")
        .expect("saved");
        assert_eq!(saved.get("frame").and_then(Value::as_u64), Some(12));
        let loaded = Image::load(&path).unwrap();
        assert_eq!(loaded.rgb, vec![12, 0, 0]);

        let mut got = Vec::new();
        let mut used = false;
        let missing = shots_on_present(
            14,
            None,
            &mut used,
            &[shot],
            &mut got,
            &then,
            Some(14),
            &mut captures,
        )
        .expect("missing")
        .expect_err("no pixels");
        assert!(missing.contains("missing capture at frame 14"), "{missing}");
        let mut got = Vec::new();
        let mut used = false;
        let later = pixel(15);
        let wrong = shots_on_present(
            15,
            Some(&later),
            &mut used,
            &[shot],
            &mut got,
            &then,
            Some(14),
            &mut captures,
        )
        .expect("past the target")
        .expect_err("not the scheduled present");
        assert!(wrong.contains("is not scheduled frame 14"), "{wrong}");
        let loaded = Image::load(&path).unwrap();
        assert_eq!(loaded.rgb, vec![12, 0, 0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Queue `call` only after `opened` is sent. The frame thread has already
    /// taken this present, which is the gap a later `wait` cannot close.
    fn call_after(opened: &std::sync::mpsc::Receiver<()>, args: Value) -> Result<Value, String> {
        let _ = opened.recv();
        genos_mcp::command::call("wait", args, std::time::Duration::from_secs(5))
    }

    fn take_until_job(tools: &mut Tools) -> bool {
        for _ in 0..100_000 {
            let before = tools.jobs.len();
            tools.accept_commands(None);
            if tools.jobs.len() > before {
                return true;
            }
            std::thread::yield_now();
        }
        false
    }

    /// One present after its `take`. The offered byte is the present being
    /// drawn (`frame + 1`). `apply_present` keeps it only when `arm_readback`
    /// asked, then numbers that same present.
    fn draw(tools: &mut Tools, settled_now: bool, still: u32, device_read_back: bool) {
        let upcoming = tools.frame + 1;
        tools.arm_readback();
        let offered = device_read_back.then(|| pixel(upcoming));
        tools.apply_present(offered, settled_now, still, 1.0, None, None);
    }

    /// The command queue is process-wide. Tests that take it hold this for
    /// the whole run so one does not answer the other's call.
    fn script_queue() -> std::sync::MutexGuard<'static, ()> {
        static QUEUE: std::sync::Mutex<()> = std::sync::Mutex::new(());
        match QUEUE.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        }
    }

    /// The byte is the present the boundary is drawing. It is a synthetic
    /// fixture, not a photographed room and not a physical grade.
    #[test]
    fn the_present_boundary_pairs_the_read_back_with_the_saved_frame() {
        let _queue = script_queue();
        let mut tools = Tools::new();
        tools.frame = 309;
        let (open_tx, open_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            call_after(&open_rx, genos_mcp::json::object([("frames", genos_mcp::json::int(2))]))
        });
        tools.accept_commands(None);
        let before = tools.frame;
        let _ = open_tx.send(());
        draw(&mut tools, false, 0, true);
        assert_eq!(tools.frame, before + 1);
        assert!(take_until_job(&mut tools), "wait was not taken");
        assert_eq!(tools.frame, before + 1);
        draw(&mut tools, false, 0, true);
        tools.accept_commands(None);
        draw(&mut tools, false, 0, true);
        let reply = worker.join().unwrap().unwrap();
        assert_eq!(reply.get("accepted").and_then(Value::as_u64), Some(310));
        assert_eq!(reply.get("frame").and_then(Value::as_u64), Some(312));
        assert_ne!(
            reply.get("frame").and_then(Value::as_u64),
            Some(before + 2)
        );

        let dir = std::env::temp_dir().join(format!("genos-present-boundary-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let shot = dir.join("room-a-live.png");
        tools.frame = 0;
        tools.out_dir = dir.clone();
        let (open_tx, open_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = open_rx.recv();
            genos_mcp::command::call(
                "wait_settled",
                genos_mcp::json::object([
                    ("timeout_frames", genos_mcp::json::int(30)),
                    ("quiet_frames", genos_mcp::json::int(1)),
                    ("then_frames", genos_mcp::json::int(2)),
                    ("path", genos_mcp::json::string("room-a-live.png")),
                    ("width", genos_mcp::json::int(1)),
                    ("height", genos_mcp::json::int(1)),
                    ("ui", genos_mcp::json::bool(false)),
                ]),
                std::time::Duration::from_secs(5),
            )
        });
        let _ = open_tx.send(());
        assert!(take_until_job(&mut tools), "wait_settled was not taken");
        draw(&mut tools, true, 7, true);
        draw_taken(&mut tools, true, 7, true);
        draw_taken(&mut tools, true, 7, true);
        let saved = worker.join().unwrap().unwrap();
        let settled_frame = saved.get("settled_frame").and_then(Value::as_u64);
        let frame = saved.get("frame").and_then(Value::as_u64);
        assert_eq!(saved.get("captured").and_then(Value::as_bool), Some(true));
        assert_eq!(saved.get("frames").and_then(Value::as_u64), Some(1));
        assert_eq!(saved.get("still_frames").and_then(Value::as_u64), Some(7));
        assert_eq!(frame, settled_frame.map(|n| n + 2));
        let loaded = Image::load(&shot).unwrap();
        let byte = u8::try_from(frame.unwrap()).unwrap();
        assert_eq!(loaded.rgb, vec![byte, 0, 0]);
        assert_ne!(byte, u8::try_from(settled_frame.unwrap()).unwrap());

        let miss = dir.join("missing.png");
        tools.out_dir = dir.clone();
        let (open_tx, open_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = open_rx.recv();
            genos_mcp::command::call(
                "wait_settled",
                genos_mcp::json::object([
                    ("timeout_frames", genos_mcp::json::int(30)),
                    ("quiet_frames", genos_mcp::json::int(1)),
                    ("then_frames", genos_mcp::json::int(2)),
                    ("path", genos_mcp::json::string("missing.png")),
                    ("width", genos_mcp::json::int(1)),
                    ("height", genos_mcp::json::int(1)),
                    ("ui", genos_mcp::json::bool(false)),
                ]),
                std::time::Duration::from_secs(5),
            )
        });
        let _ = open_tx.send(());
        assert!(take_until_job(&mut tools), "missing wait_settled was not taken");
        draw(&mut tools, true, 7, false);
        draw_taken(&mut tools, true, 7, false);
        draw_taken(&mut tools, true, 7, false);
        let err = worker.join().unwrap().unwrap_err();
        assert!(err.contains("missing capture"), "{err}");
        assert!(!miss.exists());
        draw_taken(&mut tools, true, 7, true);
        assert!(!miss.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn draw_taken(tools: &mut Tools, settled_now: bool, still: u32, device_read_back: bool) {
        tools.accept_commands(None);
        draw(tools, settled_now, still, device_read_back);
    }

    /// Settings text the capture catalogue resolves. This is `LightingSettings`
    /// text, not a string written to satisfy the script's `contains` checks.
    fn capture_settings_text() -> String {
        let mut settings = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::High);
        settings.surface_cache.sh_cells = true;
        settings.experiments.toggles = genos_scene::LIGHTING_TOGGLE_CELLS_ONLY;
        settings.experiments.cache_async = false;
        settings.to_text()
    }

    /// `wait_settled` was taken at the start of this present. Draw that present,
    /// then later ones, until the real settle job replies. The offered byte is
    /// the present being drawn. It is a synthetic fixture, not a room photograph.
    fn drive_taken_settle(tools: &mut Tools) {
        let mut opening = true;
        for _ in 0..16 {
            if !opening {
                tools.accept_commands(None);
            }
            opening = false;
            let upcoming = tools.frame + 1;
            tools.arm_readback();
            tools.apply_present(Some(pixel(upcoming)), true, 7, 1.0, None, None);
            if tools.jobs.is_empty() {
                return;
            }
        }
        if let Some(running) = tools.jobs.pop() {
            genos_mcp::command::reply(
                running.id,
                Err("headless settle fixture ran out of presents".into()),
            );
        }
    }

    /// Headless answers for the commands this script sends. `wait_settled` is
    /// the production job. `camera` uses the production argument parser and
    /// `camera_value`. There is no renderer, so `status` and the lighting
    /// pass counters are fixture values. The settings body is not.
    fn serve_room_script(
        tools: &mut Tools,
        camera: &mut genos_scene::Camera,
        settings_text: &str,
        cmd: &genos_mcp::command::Pending,
    ) {
        let args = crate::value::Args(&cmd.args);
        let reply = match cmd.name.as_str() {
            "status" => Ok(crate::value::obj(vec![(
                "frame",
                crate::value::num(tools.frame as f64),
            )])),
            "time" => match (args.f64("fixed_dt"), args.bool("pause"), args.bool("real_time")) {
                (Ok(dt), Ok(_), Ok(_)) => Ok(crate::value::obj(vec![
                    ("paused", Value::Bool(tools.paused)),
                    ("fixed_dt", dt.map_or(Value::Null, crate::value::num)),
                    ("frame", crate::value::num(tools.frame as f64)),
                ])),
                (Err(err), _, _) | (_, Err(err), _) | (_, _, Err(err)) => Err(err),
            },
            "view" => match args.str("mode") {
                Ok(Some(mode)) => match Look::parse(&mode) {
                    Ok(look) => {
                        tools.look = look;
                        Ok(crate::value::obj(vec![(
                            "view",
                            crate::value::text(tools.look.name()),
                        )]))
                    }
                    Err(err) => Err(err),
                },
                Ok(None) => Ok(crate::value::obj(vec![(
                    "view",
                    crate::value::text(tools.look.name()),
                )])),
                Err(err) => Err(err),
            },
            "camera" => match (args.vec3("position"), args.f32("yaw"), args.f32("pitch")) {
                (Ok(position), Ok(yaw), Ok(pitch)) => {
                    let pos = position.unwrap_or(camera.position);
                    let yaw = yaw.unwrap_or(camera.yaw);
                    let pitch = pitch.unwrap_or(camera.pitch);
                    camera.set_pose(
                        pos,
                        yaw,
                        pitch.clamp(-genos_scene::PITCH_LIMIT, genos_scene::PITCH_LIMIT),
                    );
                    Ok(camera_value(camera))
                }
                (Err(err), _, _) | (_, Err(err), _) | (_, _, Err(err)) => Err(err),
            },
            "get_lighting_settings" => Ok(crate::value::obj(vec![
                ("text", crate::value::text(settings_text)),
                ("passes", crate::value::list(Vec::new())),
                ("post_ms_device", crate::value::num(0.0)),
                ("post_vram_device", crate::value::num(0.0)),
            ])),
            "wait" | "step" | "wait_settled" => match tools.frame_job(&cmd.name, &args, false) {
                Ok(Some(Step::Job(job))) => {
                    tools.jobs.push(Running { id: cmd.id, job });
                    drive_taken_settle(tools);
                    return;
                }
                Ok(Some(Step::Reply(value))) => Ok(value),
                Ok(None) => Err(format!("{} is not a frame job", cmd.name)),
                Err(err) => Err(err),
            },
            other => Err(format!(
                "headless room-script fixture has no {other}"
            )),
        };
        genos_mcp::command::reply(cmd.id, reply);
    }

    /// Pid-scoped directory for one room-script test. Drop removes it, so a
    /// failed assertion still clears the directory. `prepare` starts empty:
    /// a missing directory is fine, and a leftover that cannot be removed
    /// fails the test.
    struct RoomScriptDir {
        path: std::path::PathBuf,
    }

    impl RoomScriptDir {
        fn prepare(path: std::path::PathBuf) -> Self {
            if let Err(err) = std::fs::remove_dir_all(&path) {
                assert_eq!(
                    err.kind(),
                    std::io::ErrorKind::NotFound,
                    "could not empty room-script dir {}: {err}",
                    path.display()
                );
            }
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for RoomScriptDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    struct HostRoomScript {
        ok: bool,
        result: String,
        report: String,
        seen: Vec<(String, Value)>,
        dir: RoomScriptDir,
    }

    /// The checked-in script through `run_script` and the commands it registers.
    /// The settle job is the production one. Its saved byte is a fixture present,
    /// not a room photograph and not a blotch grade.
    fn run_room_script_on_host(label: &str, script: &std::path::Path) -> HostRoomScript {
        let _queue = script_queue();
        let dir = RoomScriptDir::prepare(std::env::temp_dir().join(format!(
            "genos-room-script-{label}-{}",
            std::process::id()
        )));
        let settings_text = capture_settings_text();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, Value)>::new()));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fixture_seen = std::sync::Arc::clone(&seen);
        let fixture_stop = std::sync::Arc::clone(&stop);
        let fixture_text = settings_text;
        let fixture_dir = dir.path.clone();
        let fixture = std::thread::spawn(move || {
            let mut tools = Tools::new();
            tools.out_dir = fixture_dir;
            let mut camera = genos_scene::Camera::new(0.0, 0.0, 0.0);
            while !fixture_stop.load(std::sync::atomic::Ordering::SeqCst) {
                for cmd in genos_mcp::command::take_wait(std::time::Duration::from_millis(20)) {
                    match fixture_seen.lock() {
                        Ok(mut seen) => seen.push((cmd.name.clone(), cmd.args.clone())),
                        Err(poison) => poison
                            .into_inner()
                            .push((cmd.name.clone(), cmd.args.clone())),
                    }
                    serve_room_script(&mut tools, &mut camera, &fixture_text, &cmd);
                }
            }
        });
        let ok = crate::script::run_script(&crate::script::ScriptRun {
            script: script.to_path_buf(),
            report_dir: dir.path.clone(),
            trace: false,
        });
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        fixture.join().unwrap();
        let result = std::fs::read_to_string(dir.path.join("result.json")).unwrap();
        let report = std::fs::read_to_string(dir.path.join("report.md")).unwrap();
        let seen = match seen.lock() {
            Ok(seen) => seen.clone(),
            Err(poison) => poison.into_inner().clone(),
        };
        HostRoomScript {
            ok,
            result,
            report,
            seen,
            dir,
        }
    }

    fn room_blotch_path() -> std::path::PathBuf {
        let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/stress/scripts/room_blotch.rhai");
        let script = script.canonicalize().unwrap();
        assert!(
            script.ends_with("examples/stress/scripts/room_blotch.rhai"),
            "{}",
            script.display()
        );
        script
    }

    fn replace_once(text: &str, from: &str, to: &str) -> String {
        assert_eq!(
            text.matches(from).count(),
            1,
            "expected one occurrence of {from}"
        );
        text.replacen(from, to, 1)
    }

    fn write_room_script(label: &str, text: &str) -> RoomScriptDir {
        let dir = RoomScriptDir::prepare(std::env::temp_dir().join(format!(
            "genos-room-script-src-{label}-{}",
            std::process::id()
        )));
        std::fs::write(dir.path.join("script.rhai"), text).unwrap();
        dir
    }

    fn assert_room_eye(args: &Value, yaw: f64) {
        let eyes = [0.664_f64, 1.699, 9.629];
        let position = args.get("position").and_then(Value::as_array).unwrap();
        assert_eq!(position.len(), eyes.len(), "eye {position:?}");
        for (got, want) in position.iter().zip(eyes) {
            let got = got.as_f64().unwrap();
            assert!((got - want).abs() < 1.0e-9, "eye {got} {want}");
        }
        let got_yaw = args.get("yaw").and_then(Value::as_f64).unwrap();
        let got_pitch = args.get("pitch").and_then(Value::as_f64).unwrap();
        assert!((got_yaw - yaw).abs() < 1.0e-9, "yaw {got_yaw} {yaw}");
        assert!((got_pitch - -0.12).abs() < 1.0e-9, "pitch {got_pitch}");
    }

    /// The checked-in room script, through `run_script`'s engine. Earlier
    /// harness tests read that file as text or drove a Rust helper, so a
    /// name the function cannot see stayed green. Rhai functions do not
    /// close over an outer `let` or a bare `const`.
    ///
    /// Both poses request the capture the script requires: a path, two frames
    /// after settle, and `captured=true`. The saved file is the fixture present.
    #[test]
    fn the_checked_in_room_script_runs_both_poses_through_the_host() {
        let script = room_blotch_path();
        let run = run_room_script_on_host("checked-in", &script);
        assert!(run.ok, "{}", run.result);
        let names: Vec<&str> = run.seen.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "status",
                "time",
                "view",
                "camera",
                "wait_settled",
                "get_lighting_settings",
                "camera",
                "wait_settled",
                "get_lighting_settings",
            ]
        );
        let cameras: Vec<&Value> = run
            .seen
            .iter()
            .filter(|(name, _)| name == "camera")
            .map(|(_, args)| args)
            .collect();
        let yaws = [-0.679_f64, -36.0 * 0.017_453_3];
        for (args, yaw) in cameras.iter().zip(yaws) {
            assert_room_eye(args, yaw);
        }
        for (_, args) in run.seen.iter().filter(|(name, _)| name == "wait_settled") {
            assert_eq!(args.get("timeout_frames").and_then(Value::as_i64), Some(900));
            assert_eq!(args.get("then_frames").and_then(Value::as_i64), Some(2));
            assert_eq!(args.get("width").and_then(Value::as_i64), Some(320));
            assert_eq!(args.get("height").and_then(Value::as_i64), Some(180));
        }
        let paths: Vec<&str> = run
            .seen
            .iter()
            .filter(|(name, _)| name == "wait_settled")
            .filter_map(|(_, args)| args.get("path").and_then(Value::as_str))
            .collect();
        assert_eq!(paths, ["room-a-live.png", "room-b-live.png"]);
        let settings_text = capture_settings_text();
        for name in ["room-a", "room-b"] {
            let resolved =
                std::fs::read_to_string(run.dir.path.join(format!("resolved-{name}.txt"))).unwrap();
            assert_eq!(resolved, settings_text);
            assert!(resolved.contains("sc_settle=0"));
            assert!(run.dir.path.join(format!("{name}-live.png")).is_file());
        }
        assert!(run.report.contains("schedule pose=room-a"));
        assert!(run.report.contains("schedule pose=room-b"));
        assert!(run.report.contains("captured=true"));
    }

    /// `shoot` still names `eye`, and the outer `let eye` is still there.
    /// The parameter and the call argument are gone, which is the miss that
    /// reached a device as `Variable not found: eye`.
    #[test]
    fn an_undeclared_eye_inside_shoot_fails_the_room_script() {
        let text = std::fs::read_to_string(room_blotch_path()).unwrap();
        let text = replace_once(
            &text,
            "fn shoot(name, yaw, eye, pitch)",
            "fn shoot(name, yaw, pitch)",
        );
        let text = replace_once(
            &text,
            "shoot(\"room-a\", -0.679, eye, pitch);",
            "shoot(\"room-a\", -0.679, pitch);",
        );
        let text = replace_once(
            &text,
            "shoot(\"room-b\", -36.0 * 0.0174533, eye, pitch);",
            "shoot(\"room-b\", -36.0 * 0.0174533, pitch);",
        );
        assert!(text.contains("let eye = [0.664, 1.699, 9.629];"));
        assert!(text.contains("position: eye"));
        let script = write_room_script("undeclared-eye", &text);
        let run = run_room_script_on_host("undeclared-eye", &script.path.join("script.rhai"));
        assert!(!run.ok, "{}", run.result);
        assert!(
            run.result.contains("Variable not found: eye"),
            "{}",
            run.result
        );
        let names: Vec<&str> = run.seen.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["status", "time", "view"]);
        assert!(!run.dir.path.join("room-a-live.png").is_file());
        assert!(!run.report.contains("captured=true"));
    }

    /// The tail and the size stay. The path the contract saves is gone, so
    /// the registered `wait_settled` rejects the call before any present is saved.
    #[test]
    fn a_room_script_without_its_capture_path_fails() {
        let text = std::fs::read_to_string(room_blotch_path()).unwrap();
        let text = replace_once(
            &text,
            "timeout_frames: 900,\n        then_frames: 2,\n        path: `${name}-live.png`,\n        width: 320,\n        height: 180",
            "timeout_frames: 900,\n        then_frames: 2,\n        width: 320,\n        height: 180",
        );
        assert!(text.contains("require(settled.captured == true, `missing-capture ${name}`)"));
        assert!(!text.contains("path: `${name}-live.png`"));
        let script = write_room_script("missing-capture", &text);
        let run = run_room_script_on_host("missing-capture", &script.path.join("script.rhai"));
        assert!(!run.ok, "{}", run.result);
        assert!(
            run.result.contains("wait_settled capture needs a path"),
            "{}",
            run.result
        );
        let names: Vec<&str> = run.seen.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            ["status", "time", "view", "camera", "wait_settled"]
        );
        let camera = run
            .seen
            .iter()
            .find(|(name, _)| name == "camera")
            .map(|(_, args)| args)
            .unwrap();
        assert_room_eye(camera, -0.679);
        let settle = run
            .seen
            .iter()
            .find(|(name, _)| name == "wait_settled")
            .map(|(_, args)| args)
            .unwrap();
        assert!(settle.get("path").is_none());
        assert_eq!(settle.get("then_frames").and_then(Value::as_i64), Some(2));
        assert_eq!(settle.get("width").and_then(Value::as_i64), Some(320));
        assert_eq!(settle.get("height").and_then(Value::as_i64), Some(180));
        assert!(!run.dir.path.join("room-a-live.png").is_file());
        assert!(!run.report.contains("captured=true"));
    }
}
