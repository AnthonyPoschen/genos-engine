//! `genos-single-frame-cascade`: a CPU prototype of the sparse cascade traced
//! in one frame with split rays (Freeman, Sannikov and Pitt-Francis, arXiv 2607.20384).
//!
//! It answers two questions with numbers and draws nothing on a GPU:
//! how many rays one frame needs for the cascade to converge with no temporal
//! mean, and how far that frame is from a path-traced reference. Every ray
//! goes through [`Surfaces`], the tracer interface, and is counted there, so
//! the count holds for any tracer behind that interface.
//! See `debug/single-frame-cascade/README.md`.

#![expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    reason = "prototype: index and grid arithmetic between float, i32, u32 and usize"
)]

mod gpu;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};

use genos_debug::Image;
use genos_render::{identity_pose, GltfSurface, SceneMesh, Surfaces, TraceHit};
use genos_scene::{look_direction, Floor, Light, Scene, Sky, Vec3};
use genos_scene_lib::Kind;
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};

const HELP: &str = "\
genos-single-frame-cascade: the sparse cascade in one frame, split rays, on the CPU.

Traces one frame of the cascade (and --frames of them with new jitter) from a
camera pose, counts the rays, and compares the bounce irradiance at a grid of
pixels with a path-traced reference. Prints the numbers and writes pictures.

  --scene NAME         scene under debug/scenes (default lighting-stress):
                       a `kind stress` scene, or a `kind gltf` scene such as
                       sponza with the ray tracer
  --checkpoint NAME    stand at a checkpoint of the scene (default its first)
  --eye X,Y,Z          eye position, metres (overrides the checkpoint)
  --yaw R --pitch R    view direction, radians (with --eye)
  --size WxH           picture size (default 1280x720)
  --lamps N            point lamps in the building (default the scene's)
  --mode MODE          nee: a ray hit is shaded by every lamp with a shadow
                       ray, the cascade carries one bounce (the paper).
                       emissive: lamps are glowing spheres, surfaces are black,
                       the cascade carries the lamps' own light (default nee)
  --lamp-radius M      sphere radius in emissive mode (default 0.1)
  --tracer T           ray: each cascade ray is traced against the scene.
                       cone: each cascade ray is a cone marched through a
                       voxel hierarchy of surface points; no scene trace
                       (default ray)
  --voxels SOURCE      what the cone tracer's voxels are built from. screen:
                       the points the picture shows. world: every surface of
                       the building (default world)
  --cone-width F       cone width over the cascade's direction spacing
                       (default 1.0)
  --probe-pixel X,Y    with --tracer cone: at that pixel, compare a ray and a
                       cone in 1024 cosine directions and print the directions
                       where they differ most
  --voxel-check        print, by surface orientation, the finest voxels' stored
                       light over the exact direct light of the pixels in them
  --refine-level L     when a cone step at a coarser level meets cover, redo that
                       step at level L (finest 2 = 6.25 cm), so the read near a
                       surface cannot reach past a wall; 9 is off (default 3)
  --max-level L        coarsest voxel level a cone reads, 2 (6.25 cm) to 9 (8 m)
                       (default 9)
  --voxel-order O      none: a voxel's surfaces count wherever they lie in it.
                       front: a voxel keeps the centroid of the area that
                       faces each way, and that area counts only when the
                       cone's start is on the side it faces (default front)
  --probe-px P         cascade-0 probe spacing in pixels (default 4)
  --theta T            cascade-0 angular resolution; 2*T*T directions
                       (default 4: 32 directions)
  --cascades N         cascade count (default 4)
  --t0 F               cascade-0 ray length in probe spacings (default 1.6)
  --spp N              rays per pixel per frame (default 1)
  --frames N           frames, each with new jitter (default 8)
  --stride N           compare every Nth pixel each way (default 16)
  --ref-samples N      reference rays per compared block (default 1024)
  --block B            compare B x B pixel means, as capture.compare does; the
                       reference rays are split over the block (default 4)
  --dump               also write samples.csv: one row per compared block
  --bin-cosine C       how the shade weighs a cascade-0 direction. centre: the
                       cosine of the direction's centre (the paper). mean: the
                       cosine of the luma-weighted mean direction of the light
                       that direction holds, carried through the merge
                       (default mean)
  --threads N          worker threads (default the machine's)
  --out DIR            output directory (default ./genos-debug/single-frame-cascade)
  -h, --help           this text

No environment variable is read.
";

type V3 = [f32; 3];
const REACH: f32 = 200.0;
const LIFT: f32 = 0.02;
/// The engine's diffuse scale: 0.8 / pi (genos-debug `reference.rs`).
const LAMBERT: f32 = 0.254_647_9;
const LAMP_UNIT: f32 = 72.0;
const FOV_Y: f32 = 60.0;
/// Finest probe spacing, metres. A level of detail doubles it.
const BASE_SPACING: f32 = 1.0 / 64.0;
const PI: f32 = std::f32::consts::PI;

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V3, b: V3) -> V3 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}
fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn normalize(a: V3) -> V3 {
    let len = dot(a, a).sqrt().max(1.0e-12);
    scale(a, 1.0 / len)
}
fn luma(c: V3) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Nee,
    Emissive,
}

struct Options {
    scene: String,
    checkpoint: Option<String>,
    eye: Option<V3>,
    yaw: f32,
    pitch: f32,
    width: usize,
    height: usize,
    lamps: Option<usize>,
    mode: Mode,
    lamp_radius: f32,
    cone: bool,
    world_voxels: bool,
    cone_width: f32,
    /// Count a voxel's area only when the cone starts on the side it faces.
    front_order: bool,
    max_level: u8,
    refine_level: u8,
    voxel_check: bool,
    probe_pixel: Option<(usize, usize)>,
    gpu_bench: bool,
    probe_px: f32,
    theta: u32,
    cascades: usize,
    t0: f32,
    spp: u32,
    frames: u32,
    stride: usize,
    ref_samples: u32,
    block: usize,
    dump: bool,
    mean_cosine: bool,
    threads: usize,
    out: PathBuf,
}

fn parse_args() -> Result<Option<Options>, String> {
    let mut o = Options {
        scene: genos_scene_lib::DEFAULT_SCENE.to_string(),
        checkpoint: None,
        eye: None,
        yaw: 0.0,
        pitch: 0.0,
        width: 1280,
        height: 720,
        lamps: None,
        mode: Mode::Nee,
        lamp_radius: 0.1,
        cone: false,
        world_voxels: true,
        cone_width: 1.0,
        front_order: true,
        max_level: VOXEL_COARSEST,
        refine_level: 3,
        voxel_check: false,
        probe_pixel: None,
        gpu_bench: false,
        probe_px: 4.0,
        theta: 4,
        cascades: 4,
        t0: 1.6,
        spp: 1,
        frames: 8,
        stride: 16,
        ref_samples: 1024,
        block: 4,
        dump: false,
        mean_cosine: true,
        threads: std::thread::available_parallelism().map_or(4, std::num::NonZero::get),
        out: PathBuf::from("genos-debug/single-frame-cascade"),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--scene" => o.scene = value()?,
            "--checkpoint" => o.checkpoint = Some(value()?),
            "--eye" => {
                let text = value()?;
                let parts: Vec<f32> = text
                    .split(',')
                    .map(|p| num("--eye", p))
                    .collect::<Result<_, _>>()?;
                let [x, y, z] = parts[..] else {
                    return Err(format!("--eye takes X,Y,Z, not {text}"));
                };
                o.eye = Some([x, y, z]);
            }
            "--yaw" => o.yaw = num(&arg, &value()?)?,
            "--pitch" => o.pitch = num(&arg, &value()?)?,
            "--size" => {
                let text = value()?;
                let (w, h) = text
                    .split_once('x')
                    .ok_or_else(|| format!("--size takes WxH, not {text}"))?;
                o.width = num("--size", w)?;
                o.height = num("--size", h)?;
            }
            "--lamps" => o.lamps = Some(num(&arg, &value()?)?),
            "--mode" => {
                o.mode = match value()?.as_str() {
                    "nee" => Mode::Nee,
                    "emissive" => Mode::Emissive,
                    other => return Err(format!("--mode is nee or emissive, not {other}")),
                }
            }
            "--lamp-radius" => o.lamp_radius = num(&arg, &value()?)?,
            "--tracer" => {
                o.cone = match value()?.as_str() {
                    "ray" => false,
                    "cone" => true,
                    other => return Err(format!("--tracer is ray or cone, not {other}")),
                }
            }
            "--voxels" => {
                o.world_voxels = match value()?.as_str() {
                    "screen" => false,
                    "world" => true,
                    other => return Err(format!("--voxels is screen or world, not {other}")),
                }
            }
            "--cone-width" => o.cone_width = num(&arg, &value()?)?,
            "--voxel-check" => o.voxel_check = true,
            "--refine-level" => o.refine_level = num::<u8>(&arg, &value()?)?.clamp(VOXEL_FINEST, VOXEL_COARSEST),
            "--probe-pixel" => {
                let text = value()?;
                let (x, y) = text.split_once(',').ok_or_else(|| format!("bad --probe-pixel: {text}"))?;
                o.probe_pixel = Some((num(&arg, x)?, num(&arg, y)?));
            }
            "--max-level" => o.max_level = num::<u8>(&arg, &value()?)?.clamp(VOXEL_FINEST, VOXEL_COARSEST),
            "--voxel-order" => {
                o.front_order = match value()?.as_str() {
                    "none" => false,
                    "front" => true,
                    other => return Err(format!("bad --voxel-order: {other}")),
                }
            }
            "--gpu-bench" => o.gpu_bench = true,
            "--probe-px" => o.probe_px = num(&arg, &value()?)?,
            "--theta" => o.theta = num(&arg, &value()?)?,
            "--cascades" => o.cascades = num(&arg, &value()?)?,
            "--t0" => o.t0 = num(&arg, &value()?)?,
            "--spp" => o.spp = num(&arg, &value()?)?,
            "--frames" => o.frames = num(&arg, &value()?)?,
            "--stride" => o.stride = num(&arg, &value()?)?,
            "--ref-samples" => o.ref_samples = num(&arg, &value()?)?,
            "--block" => o.block = num(&arg, &value()?)?,
            "--dump" => o.dump = true,
            "--bin-cosine" => {
                o.mean_cosine = match value()?.as_str() {
                    "centre" => false,
                    "mean" => true,
                    other => return Err(format!("bad --bin-cosine: {other}")),
                }
            }
            "--threads" => o.threads = num(&arg, &value()?)?,
            "--out" => o.out = PathBuf::from(value()?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if o.width == 0 || o.height == 0 || o.theta == 0 || o.cascades == 0 || o.cascades > 6 {
        return Err("--size, --theta and --cascades (1 to 6) must be above 0".into());
    }
    o.spp = o.spp.max(1);
    o.frames = o.frames.max(1);
    o.stride = o.stride.max(1);
    o.block = o.block.clamp(1, o.stride);
    o.threads = o.threads.max(1);
    Ok(Some(o))
}

/// A glTF model's triangles as the scene's surfaces.
struct GltfWalls(GltfSurface);

impl Surfaces for GltfWalls {
    fn first_hit(&self, origin: V3, dir: V3, reach: f32) -> Option<TraceHit> {
        self.0.trace(origin, dir, reach)
    }
    fn blocked(&self, from: V3, to: V3) -> bool {
        self.0.occluded(from, to)
    }
}

/// The scene to trace: its lamps and sky, and its surfaces.
fn load_scene(root: &std::path::Path, entry: &genos_scene_lib::Entry, o: &Options) -> Result<(Scene, Option<GltfWalls>), String> {
    match &entry.kind {
        Kind::Stress(spec) => {
            let layout = Layout::parse(&spec.layout).ok_or_else(|| format!("unknown layout {}", spec.layout))?;
            let mut stage = Stage::new(
                Scale::parse(&spec.scale)?,
                1,
                LightMix { count: o.lamps.unwrap_or(spec.lights), dynamic_pct: spec.dynamic, layout },
                1.0,
                spec.time,
                120.0,
            );
            stage.sun_frozen = spec.freeze_sun;
            stage.boxes_still = spec.still_boxes;
            stage.lamps_still = spec.still_lamps;
            stage.sky_on = spec.sky;
            stage.apply();
            Ok((stage.world.scene.clone(), None))
        }
        Kind::Gltf { path, sun, sky, lamps } => {
            let full = root.join(path);
            let model = genos_load::load_gltf_file(&full).map_err(|e| format!("{}: {e}", full.display()))?;
            // The analytic floor is not part of the model: park it far away, as
            // the scene program does.
            let mut scene = Scene::from_lists(
                Floor { position: Vec3::new(1000.0, 0.0, 0.0), half_x: 0.01, half_z: 0.01, color: [0.2, 0.2, 0.2] },
                Vec::new(),
                Vec::new(),
                Vec::new(),
            );
            scene.ceiling = None;
            scene.sky = sky.map(|color| Sky { color });
            if let Some(sun) = sun {
                let d = normalize(sun.direction);
                scene.push_light(Light {
                    position: Vec3::new(0.0, 7.0, 0.0),
                    color: sun.color,
                    direction: Vec3::new(d[0], d[1], d[2]),
                });
            }
            for lamp in lamps {
                scene.push_light(Light {
                    position: Vec3::new(lamp.position[0], lamp.position[1], lamp.position[2]),
                    color: lamp.color,
                    direction: Vec3::ZERO,
                });
            }
            Ok((scene, Some(GltfWalls(GltfSurface::from_gltf(&model, &identity_pose())))))
        }
        Kind::Rhai { .. } | Kind::Mesh { .. } => {
            Err(format!("{} is neither a `kind stress` nor a `kind gltf` scene", entry.name))
        }
    }
}

/// The tracer interface with a ray count: every ray the cascade asks for.
struct Counted<'a> {
    inner: &'a dyn Surfaces,
    rays: AtomicU64,
}

impl Surfaces for Counted<'_> {
    fn first_hit(&self, origin: V3, dir: V3, reach: f32) -> Option<TraceHit> {
        self.rays.fetch_add(1, Ordering::Relaxed);
        self.inner.first_hit(origin, dir, reach)
    }
    fn blocked(&self, from: V3, to: V3) -> bool {
        self.inner.blocked(from, to)
    }
    fn inside(&self, p: V3) -> bool {
        self.inner.inside(p)
    }
}

/// The scene with each point lamp as a glowing sphere.
struct Glowing<'a> {
    scene: &'a dyn Surfaces,
    /// Centre and emitted radiance.
    lamps: Vec<(V3, V3)>,
    radius: f32,
}

impl Surfaces for Glowing<'_> {
    fn first_hit(&self, origin: V3, dir: V3, reach: f32) -> Option<TraceHit> {
        let wall = self.scene.first_hit(origin, dir, reach);
        let mut nearest = wall.map_or(reach, |hit| dot(sub(hit.point, origin), dir));
        let mut best = wall;
        for (centre, glow) in &self.lamps {
            let oc = sub(origin, *centre);
            let b = dot(oc, dir);
            let c = dot(oc, oc) - self.radius * self.radius;
            let disc = b * b - c;
            if disc <= 0.0 {
                continue;
            }
            let t = -b - disc.sqrt();
            if t > 1.0e-3 && t < nearest {
                nearest = t;
                let point = add(origin, scale(dir, t));
                best = Some(TraceHit {
                    point,
                    normal: normalize(sub(point, *centre)),
                    albedo: [0.0; 3],
                    emissive: *glow,
                });
            }
        }
        best
    }
    fn blocked(&self, from: V3, to: V3) -> bool {
        self.scene.blocked(from, to)
    }
    fn inside(&self, p: V3) -> bool {
        self.scene.inside(p)
    }
}

/// The six axis directions a voxel keeps its surfaces by.
const AXES: [V3; 6] = [
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];
/// Finest and coarsest voxel level: 6.25 cm and 8 m (`BASE_SPACING * 2^level`).
const VOXEL_FINEST: u8 = 2;
const VOXEL_COARSEST: u8 = 9;
/// Voxel level one direct-light sample is shared over: 6.25 cm. At 25 cm a
/// patch of floor or roof that runs under a wall took one sample for both
/// rooms, and the dark room's edges glowed with the lit room's light.
const LIGHT_LEVEL: u8 = 2;

/// What one voxel knows of the surfaces inside it: per axis direction, the
/// area that faces that way and that area's outgoing light. A lamp is kept
/// apart, because a sphere shows the same area to every direction.
#[derive(Clone, Copy, Default)]
struct Voxel {
    area: [f32; 6],
    light: [V3; 6],
    /// Area-weighted sum of the positions of the area that faces each way.
    at: [V3; 6],
    lamp_area: f32,
    lamp_light: V3,
    /// Area-weighted sum of the lamp points' positions.
    lamp_at: V3,
}

/// The cone tracer's scene: a sparse voxel hierarchy of surface points.
struct Voxels {
    cells: HashMap<(u8, [i32; 3]), Voxel>,
    lookups: AtomicU64,
    /// Shadow rays cast to light the surface points (once, not per frame, while
    /// nothing moves).
    shadow_rays: u64,
    points: usize,
}

fn voxel_size(level: u8) -> f32 {
    BASE_SPACING * 2f32.powi(i32::from(level))
}

impl Voxels {
    fn new() -> Self {
        Self { cells: HashMap::new(), lookups: AtomicU64::new(0), shadow_rays: 0, points: 0 }
    }

    /// A patch of surface: its point, the way it faces, its area and the
    /// radiance it sends out. It goes into the voxel behind the surface at
    /// every level, so a point over the surface is never in its own voxel
    /// when the surface lies on a grid plane.
    fn add_surface(&mut self, point: V3, normal: V3, area: f32, radiance: V3) {
        self.points += 1;
        for level in VOXEL_FINEST..=VOXEL_COARSEST {
            let size = voxel_size(level);
            let inside = sub(point, scale(normal, 0.25 * size.min(0.25)));
            let voxel = self.cells.entry((level, nearest_cell(inside, size))).or_default();
            for (d, axis) in AXES.iter().enumerate() {
                let facing = dot(normal, *axis).max(0.0) * area;
                voxel.area[d] += facing;
                voxel.light[d] = add(voxel.light[d], scale(radiance, facing));
                voxel.at[d] = add(voxel.at[d], scale(point, facing));
            }
        }
    }

    /// A lamp: a sphere of `radius` that sends out `glow` every way. Its disc
    /// is shared over points of the sphere, so a lamp wider than a voxel is
    /// spread over the voxels it fills and none of them is over-full.
    fn add_lamp(&mut self, centre: V3, radius: f32, glow: V3) {
        const POINTS: u32 = 256;
        let share = PI * radius * radius / POINTS as f32;
        for i in 0..POINTS {
            let (u, v) = r2(u64::from(i));
            let at = add(centre, scale(decode_dir(u, v), radius));
            for level in VOXEL_FINEST..=VOXEL_COARSEST {
                let voxel = self.cells.entry((level, nearest_cell(at, voxel_size(level)))).or_default();
                voxel.lamp_area += share;
                voxel.lamp_light = add(voxel.lamp_light, scale(glow, share));
                voxel.lamp_at = add(voxel.lamp_at, scale(at, share));
            }
        }
    }

    /// What a cone of one voxel's width meets at `at`, travelling along `dir`:
    /// the share of its cross-section that surfaces facing it cover, and the
    /// light of those surfaces over the whole cross-section. Trilinear over
    /// the eight voxels around the point. With `start`, an area counts only
    /// when the cone's start is on the side it faces, so a surface behind
    /// another one's plane (the sunlit top of the roof over a ceiling) never
    /// shows.
    fn sample(&self, level: u8, at: V3, dir: V3, start: Option<V3>) -> (f32, V3) {
        let size = voxel_size(level);
        let g = [at[0] / size - 0.5, at[1] / size - 0.5, at[2] / size - 0.5];
        let base = [g[0].floor(), g[1].floor(), g[2].floor()];
        let f = [g[0] - base[0], g[1] - base[1], g[2] - base[2]];
        let mut cover = 0.0;
        let mut light = [0.0; 3];
        self.lookups.fetch_add(8, Ordering::Relaxed);
        for corner in 0..8u32 {
            let o = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
            let w = (0..3).map(|a| if o[a] == 1 { f[a] } else { 1.0 - f[a] }).product::<f32>();
            if w <= 0.0 {
                continue;
            }
            let cell = [base[0] as i32 + o[0] as i32, base[1] as i32 + o[1] as i32, base[2] as i32 + o[2] as i32];
            let Some(voxel) = self.cells.get(&(level, cell)) else { continue };
            // In this step, and ahead of the cone's start.
            // Area facing `facing` counts when the start is on that side.
            let here = |sum: V3, area: f32, facing: Option<V3>| -> bool {
                let Some(start) = start else { return true };
                if area <= 0.0 {
                    return false;
                }
                facing.is_none_or(|n| dot(sub(start, scale(sum, 1.0 / area)), n) > 0.0)
            };
            let mut facing = 0.0;
            let mut sent = [0.0; 3];
            if here(voxel.lamp_at, voxel.lamp_area, None) {
                facing += voxel.lamp_area;
                sent = voxel.lamp_light;
            }
            for (d, axis) in AXES.iter().enumerate() {
                // A surface shows itself to a cone that travels against its normal.
                let toward = (-dot(dir, *axis)).max(0.0);
                if toward <= 0.0 || !here(voxel.at[d], voxel.area[d], Some(*axis)) {
                    continue;
                }
                facing += voxel.area[d] * toward;
                sent = add(sent, scale(voxel.light[d], toward));
            }
            cover += w * facing;
            light = add(light, scale(sent, w));
        }
        let cross = size * size;
        let raw = cover / cross;
        if raw <= 0.0 {
            return (0.0, [0.0; 3]);
        }
        let cover = raw.min(1.0);
        (cover, scale(light, cover / raw / cross))
    }
}

/// A probe: level of detail, cascade, and grid cell.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    lod: i8,
    cascade: u8,
    cell: [i32; 3],
}

fn spacing(lod: i8, cascade: usize) -> f32 {
    BASE_SPACING * 2f32.powi(i32::from(lod) + cascade as i32)
}

fn nearest_cell(x: V3, spacing: f32) -> [i32; 3] {
    [
        (x[0] / spacing).floor() as i32,
        (x[1] / spacing).floor() as i32,
        (x[2] / spacing).floor() as i32,
    ]
}

/// Algorithm 2 of the paper: the unit square to the sphere, equal areas.
fn decode_dir(u: f32, v: f32) -> V3 {
    let phi = u * 2.0 * PI;
    let z = 2.0 * v - 1.0;
    let r = (1.0 - z * z).max(0.0).sqrt();
    [r * phi.cos(), r * phi.sin(), z]
}

/// Parse one argument value.
fn num<T: std::str::FromStr>(name: &str, text: &str) -> Result<T, String> {
    text.parse().map_err(|_| format!("bad {name}: {text}"))
}

/// One deposit: what a ray or a cone learned about one cascade's interval:
/// the radiance it met there and the share of light that passes through.
struct Deposit {
    pixel: u32,
    dir: V3,
    cascade: u8,
    radiance: V3,
    through: f32,
}

/// The direction bin of `dir` at angular resolution `theta` (2 theta by theta).
fn dir_bin(dir: V3, theta: u32) -> usize {
    let u = (dir[1].atan2(dir[0]) / (2.0 * PI)).rem_euclid(1.0);
    let v = f32::midpoint(dir[2], 1.0).clamp(0.0, 1.0);
    let x = ((u * (2 * theta) as f32) as u32).min(2 * theta - 1);
    let y = ((v * theta as f32) as u32).min(theta - 1);
    (y * 2 * theta + x) as usize
}

fn bin_dir(bin: usize, theta: u32) -> V3 {
    let x = u32::try_from(bin).unwrap_or(u32::MAX) % (2 * theta);
    let y = u32::try_from(bin).unwrap_or(u32::MAX) / (2 * theta);
    decode_dir((x as f32 + 0.5) / (2 * theta) as f32, (y as f32 + 0.5) / theta as f32)
}

/// The R2 low-discrepancy sequence on the unit square.
fn r2(index: u64) -> (f32, f32) {
    const G: f64 = 1.324_717_957_244_746;
    let i = index as f64;
    (
        (0.5 + i / G).fract() as f32,
        (0.5 + i / (G * G)).fract() as f32,
    )
}

/// One cascade: its probes and, per probe and direction, the deposited sums
/// and the merged cone.
struct Cascade {
    theta: u32,
    keys: Vec<Key>,
    radiance: Vec<V3>,
    /// Sum of each deposit's luma times its direction.
    moment: Vec<V3>,
    through: Vec<f32>,
    count: Vec<u32>,
    cone: Vec<V3>,
    /// The merged cone's luma-weighted direction sum, merged as `cone` is.
    cone_moment: Vec<V3>,
    filled: Vec<bool>,
}

impl Cascade {
    fn dirs(&self) -> usize {
        (2 * self.theta * self.theta) as usize
    }
}

struct Pixel {
    point: V3,
    normal: V3,
    albedo: V3,
    /// Area of surface the pixel covers, square metres.
    area: f32,
    lod: i8,
}

struct Frame {
    /// Bounce irradiance per pixel; zero where the picture shows nothing.
    irradiance: Vec<V3>,
    rays: u64,
    shadow_rays: u64,
    voxel_lookups: u64,
    probes: Vec<usize>,
    /// Share of cascade-0 directions a pixel needed and no probe had.
    holes: f32,
}

struct Setup<'a> {
    scene: &'a Scene,
    /// The scene's surfaces alone: what a shadow ray is tested against.
    walls: &'a dyn Surfaces,
    /// What the cascade's rays hit.
    surfaces: &'a dyn Surfaces,
    /// What the cascade's cones march through, with `--tracer cone`.
    voxels: Option<&'a Voxels>,
    pixels: &'a [Option<Pixel>],
    options: &'a Options,
}

/// Direct irradiance at a point: every lamp that faces it, one shadow ray
/// each to the lamp's centre. Returns the light and the shadow rays cast. The
/// cascade and the reference both shade a hit with this, so they agree on
/// what a hit sends back and differ only in how the bounce is gathered.
fn direct_light(scene: &Scene, walls: &dyn Surfaces, point: V3, normal: V3) -> (V3, u64) {
    let origin = add(point, scale(normal, LIFT));
    let mut sum = [0.0; 3];
    let mut rays = 0;
    for index in 0..scene.light_len() {
        let light = scene.light(index);
        let along = [light.direction.x, light.direction.y, light.direction.z];
        let (toward, d2, target) = if dot(along, along) > 1.0e-8 {
            // The sun: the engine's lamp unit at a fixed 7 m.
            let toward = scale(normalize(along), -1.0);
            (toward, 49.0, add(origin, scale(toward, 80.0)))
        } else {
            let at = [light.position.x, light.position.y, light.position.z];
            let to = sub(at, origin);
            (normalize(to), dot(to, to).max(0.01), at)
        };
        let cosine = dot(toward, normal);
        if cosine <= 0.0 {
            continue;
        }
        rays += 1;
        if !walls.blocked(origin, target) {
            sum = add(sum, scale(light.color, LAMP_UNIT * cosine / d2));
        }
    }
    (sum, rays)
}

/// Radiance a ray carries back from its hit.
fn hit_radiance(setup: &Setup, hit: &TraceHit, shadow_rays: &AtomicU64) -> V3 {
    if setup.options.mode == Mode::Emissive {
        return hit.emissive;
    }
    let (direct, rays) = direct_light(setup.scene, setup.walls, hit.point, hit.normal);
    shadow_rays.fetch_add(rays, Ordering::Relaxed);
    add(hit.emissive, scale(mul(hit.albedo, direct), LAMBERT))
}

/// The reference: one bounce of irradiance at a point, from `samples` cosine
/// directions on the R2 sequence, each hit shaded by [`direct_light`].
fn bounce_reference(scene: &Scene, walls: &dyn Surfaces, point: V3, normal: V3, samples: u32, seed: u64) -> V3 {
    let sky = scene.sky.as_ref().map_or([0.0; 3], |s| s.color);
    let origin = add(point, scale(normal, LIFT));
    let side = normalize(cross(normal, if normal[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] }));
    let up = cross(normal, side);
    let (ju, jv) = r2(seed.wrapping_mul(2_654_435_761) % 100_003);
    let mut sum = [0.0; 3];
    for k in 0..samples.max(1) {
        let (u, v) = r2(u64::from(k));
        let (u, v) = ((u + ju).fract(), (v + jv).fract());
        let r = u.sqrt();
        let phi = 2.0 * PI * v;
        let dir = normalize(add(
            add(scale(side, r * phi.cos()), scale(up, r * phi.sin())),
            scale(normal, (1.0 - u).max(0.0).sqrt()),
        ));
        let l = match walls.first_hit(origin, dir, REACH) {
            None => sky,
            Some(hit) => {
                let (direct, _) = direct_light(scene, walls, hit.point, hit.normal);
                add(hit.emissive, scale(mul(hit.albedo, direct), LAMBERT))
            }
        };
        sum = add(sum, l);
    }
    // A cosine sample estimates the cosine-weighted mean radiance; irradiance is pi times it.
    scale(sum, PI / samples.max(1) as f32)
}

fn run_frame(setup: &Setup, jitter: (f32, f32)) -> Frame {
    let o = setup.options;
    let n = o.cascades;
    // Probes: the nearest probe of every cascade to every visible point.
    let mut index: HashMap<Key, u32> = HashMap::new();
    let mut cascades: Vec<Cascade> = (0..n)
        .map(|c| Cascade {
            theta: o.theta << c,
            keys: Vec::new(),
            radiance: Vec::new(),
            moment: Vec::new(),
            through: Vec::new(),
            count: Vec::new(),
            cone: Vec::new(),
            cone_moment: Vec::new(),
            filled: Vec::new(),
        })
        .collect();
    // The probe of each pixel in each cascade.
    let mut home: Vec<u32> = vec![u32::MAX; setup.pixels.len() * n];
    for (p, pixel) in setup.pixels.iter().enumerate() {
        let Some(pixel) = pixel else { continue };
        for (c, cascade) in cascades.iter_mut().enumerate() {
            let key = Key {
                lod: pixel.lod,
                cascade: u8::try_from(c).unwrap_or(u8::MAX),
                cell: nearest_cell(pixel.point, spacing(pixel.lod, c)),
            };
            let slot = *index.entry(key).or_insert_with(|| {
                cascade.keys.push(key);
                u32::try_from(cascade.keys.len() - 1).unwrap_or(u32::MAX)
            });
            home[p * n + c] = slot;
        }
    }
    for cascade in &mut cascades {
        let len = cascade.keys.len() * cascade.dirs();
        cascade.radiance = vec![[0.0; 3]; len];
        cascade.moment = vec![[0.0; 3]; len];
        cascade.through = vec![0.0; len];
        cascade.count = vec![0; len];
        cascade.cone = vec![[0.0; 3]; len];
        cascade.cone_moment = vec![[0.0; 3]; len];
        cascade.filled = vec![false; len];
    }
    // Ray generation (Algorithm 3): probes under one coarser probe take a
    // contiguous run of the R2 sequence, so each cascade's directions are
    // covered evenly.
    let mut rays_of: Vec<Vec<u64>> = cascades.iter().map(|c| vec![0; c.keys.len()]).collect();
    for (p, pixel) in setup.pixels.iter().enumerate() {
        if pixel.is_some() {
            rays_of[0][home[p * n] as usize] += u64::from(o.spp);
        }
    }
    let mut parent: Vec<Vec<u32>> = Vec::new();
    for c in 0..n.saturating_sub(1) {
        let up: Vec<u32> = cascades[c]
            .keys
            .iter()
            .map(|key| {
                let s = spacing(key.lod, c);
                let centre = [
                    (key.cell[0] as f32 + 0.5) * s,
                    (key.cell[1] as f32 + 0.5) * s,
                    (key.cell[2] as f32 + 0.5) * s,
                ];
                let above = Key {
                    lod: key.lod,
                    cascade: u8::try_from(c + 1).unwrap_or(u8::MAX),
                    cell: nearest_cell(centre, spacing(key.lod, c + 1)),
                };
                index.get(&above).copied().unwrap_or(0)
            })
            .collect();
        for (i, &up_i) in up.iter().enumerate() {
            let rays = rays_of[c][i];
            rays_of[c + 1][up_i as usize] += rays;
        }
        parent.push(up);
    }
    let mut offset: Vec<Vec<u64>> = rays_of.iter().map(|r| vec![0; r.len()]).collect();
    let mut running = 0u64;
    for (i, rays) in rays_of[n - 1].iter().enumerate() {
        offset[n - 1][i] = running;
        running += rays;
    }
    for c in (0..n.saturating_sub(1)).rev() {
        let mut next = offset[c + 1].clone();
        for (i, &up_i) in parent[c].iter().enumerate() {
            offset[c][i] = next[up_i as usize];
            next[up_i as usize] += rays_of[c][i];
        }
    }
    let mut sequence: Vec<u64> = vec![0; setup.pixels.len()];
    {
        let mut next = offset[0].clone();
        for (p, pixel) in setup.pixels.iter().enumerate() {
            if pixel.is_some() {
                let slot = home[p * n] as usize;
                sequence[p] = next[slot];
                next[slot] += u64::from(o.spp);
            }
        }
    }
    // Trace. Each ray is split: the cascade whose interval holds the hit takes
    // its radiance, and every cascade under it learns the way was clear.
    let counted = Counted { inner: setup.surfaces, rays: AtomicU64::new(0) };
    let shadow_rays = AtomicU64::new(0);
    let sky = if o.mode == Mode::Emissive {
        [0.0; 3]
    } else {
        setup.scene.sky.as_ref().map_or([0.0; 3], |s| s.color)
    };
    let chunk = setup.pixels.len().div_ceil(o.threads).max(1);
    let deposits: Vec<Deposit> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..setup.pixels.len())
            .step_by(chunk)
            .map(|start| {
                let counted = &counted;
                let shadow_rays = &shadow_rays;
                let sequence = &sequence;
                scope.spawn(move || {
                    let mut out = Vec::new();
                    let end = (start + chunk).min(setup.pixels.len());
                    for p in start..end {
                        let Some(pixel) = &setup.pixels[p] else { continue };
                        let origin = add(pixel.point, scale(pixel.normal, LIFT));
                        let t0 = o.t0 * spacing(pixel.lod, 0);
                        for s in 0..o.spp {
                            let (u, v) = r2(sequence[p] + u64::from(s));
                            let mut dir = decode_dir((u + jitter.0).fract(), (v + jitter.1).fract());
                            if dot(dir, pixel.normal) < 0.0 {
                                dir = scale(dir, -1.0);
                            }
                            if let Some(voxels) = setup.voxels {
                                for (c, radiance, through) in march_cone(voxels, o, pixel.point, pixel.normal, dir, t0, sky) {
                                    out.push(Deposit {
                                        pixel: u32::try_from(p).unwrap_or(u32::MAX),
                                        dir,
                                        cascade: u8::try_from(c).unwrap_or(u8::MAX),
                                        radiance,
                                        through,
                                    });
                                }
                                continue;
                            }
                            let (cascade, radiance) = match counted.first_hit(origin, dir, REACH) {
                                Some(hit) => {
                                    let t = dot(sub(hit.point, origin), dir);
                                    let mut c = 0usize;
                                    let mut reach = t0;
                                    while c + 1 < n && t > reach {
                                        c += 1;
                                        reach *= 4.0;
                                    }
                                    (c, hit_radiance(setup, &hit, shadow_rays))
                                }
                                // The sky is past every interval: the last
                                // cascade's rays take it on a miss.
                                None => (n - 1, sky),
                            };
                            // Split: every cascade under the hit learns the
                            // way was clear; the hit's cascade takes its light.
                            for c in 0..=cascade {
                                out.push(Deposit {
                                    pixel: u32::try_from(p).unwrap_or(u32::MAX),
                                    dir,
                                    cascade: u8::try_from(c).unwrap_or(u8::MAX),
                                    radiance: if c == cascade { radiance } else { [0.0; 3] },
                                    through: if c == cascade { 0.0 } else { 1.0 },
                                });
                            }
                        }
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    for d in &deposits {
        let c = usize::from(d.cascade);
        let cascade = &mut cascades[c];
        let slot = home[d.pixel as usize * n + c] as usize;
        let at = slot * cascade.dirs() + dir_bin(d.dir, cascade.theta);
        cascade.count[at] += 1;
        cascade.radiance[at] = add(cascade.radiance[at], d.radiance);
        cascade.moment[at] = add(cascade.moment[at], scale(d.dir, luma(d.radiance)));
        cascade.through[at] += d.through;
    }
    // Merge from the last cascade down (the paper's equation 7).
    for c in (0..n).rev() {
        let (lower, upper) = cascades.split_at_mut(c + 1);
        let cascade = &mut lower[c];
        let above = upper.first();
        let dirs = cascade.dirs();
        for slot in 0..cascade.keys.len() {
            let key = cascade.keys[slot];
            let s = spacing(key.lod, c);
            let centre = [
                (key.cell[0] as f32 + 0.5) * s,
                (key.cell[1] as f32 + 0.5) * s,
                (key.cell[2] as f32 + 0.5) * s,
            ];
            for bin in 0..dirs {
                let at = slot * dirs + bin;
                let count = cascade.count[at];
                if count == 0 {
                    continue;
                }
                let j = scale(cascade.radiance[at], 1.0 / count as f32);
                let beta = cascade.through[at] / count as f32;
                let mut cone = j;
                let mut cone_moment = scale(cascade.moment[at], 1.0 / count as f32);
                if let (Some(above), true) = (above, beta > 0.0) {
                    let x = u32::try_from(bin).unwrap_or(u32::MAX) % (2 * cascade.theta);
                    let y = u32::try_from(bin).unwrap_or(u32::MAX) / (2 * cascade.theta);
                    // The four finer directions of the next cascade that
                    // have data; one with none is left out, not read as dark.
                    let mut far = [0.0; 3];
                    let mut far_moment = [0.0; 3];
                    let mut found = 0.0;
                    for k in 0..4u32 {
                        let child = ((2 * y + (k >> 1)) * 2 * above.theta + 2 * x + (k & 1)) as usize;
                        if let Some((v, m)) = interpolate(above, &index, key.lod, c + 1, centre, child) {
                            far = add(far, v);
                            far_moment = add(far_moment, m);
                            found += 1.0;
                        }
                    }
                    if found > 0.0 {
                        cone = add(cone, scale(far, beta / found));
                        cone_moment = add(cone_moment, scale(far_moment, beta / found));
                    }
                }
                cascade.cone[at] = cone;
                cascade.cone_moment[at] = cone_moment;
                cascade.filled[at] = true;
            }
        }
    }
    // Shade: the cosine-weighted sum of the cascade-0 cones around the pixel.
    let first = &cascades[0];
    let dirs = first.dirs();
    let bin_dirs: Vec<V3> = (0..dirs).map(|b| bin_dir(b, first.theta)).collect();
    let mut irradiance = vec![[0.0; 3]; setup.pixels.len()];
    let mut wanted = 0.0f64;
    let mut missing = 0.0f64;
    for (p, pixel) in setup.pixels.iter().enumerate() {
        let Some(pixel) = pixel else { continue };
        let mut sum = [0.0; 3];
        let mut weight = 0.0;
        for (bin, dir) in bin_dirs.iter().enumerate() {
            let cosine = dot(*dir, pixel.normal);
            if cosine <= 0.0 {
                continue;
            }
            wanted += f64::from(cosine);
            match interpolate(first, &index, pixel.lod, 0, pixel.point, bin) {
                Some((v, m)) => {
                    // The light's own cosine when it is known: a bright patch
                    // that fills a small part of the direction is weighed where
                    // it is, not at the direction's centre. The weight still
                    // counts the direction's share of the hemisphere.
                    let held = dot(m, m).sqrt();
                    let lit = if o.mean_cosine && held > 1.0e-9 && luma(v) > 1.0e-9 {
                        let spread = (held / luma(v)).min(1.0);
                        let mean = dot(scale(m, 1.0 / held), pixel.normal).max(0.0);
                        // A spread-out direction (spread near 0) keeps the centre's cosine.
                        mean * spread + cosine * (1.0 - spread)
                    } else {
                        cosine
                    };
                    sum = add(sum, scale(v, lit));
                    weight += cosine;
                }
                None => missing += f64::from(cosine),
            }
        }
        if weight > 0.0 {
            irradiance[p] = scale(sum, PI / weight);
        }
    }
    Frame {
        irradiance,
        rays: counted.rays.load(Ordering::Relaxed),
        shadow_rays: shadow_rays.load(Ordering::Relaxed),
        voxel_lookups: setup.voxels.map_or(0, |v| v.lookups.swap(0, Ordering::Relaxed)),
        probes: cascades.iter().map(|c| c.keys.len()).collect(),
        holes: if wanted > 0.0 { (missing / wanted) as f32 } else { 0.0 },
    }
}

/// A cone marched through the voxels from `point`. Its width in each interval
/// is that cascade's direction spacing, so its footprint grows with the probe
/// spacing. Cover adds up: a surface that crosses several voxels is one
/// surface. Returns, for each cascade the cone reached, the radiance it met in
/// that interval over the light that entered it, and the share that passed.
fn march_cone(voxels: &Voxels, o: &Options, point: V3, normal: V3, dir: V3, t0: f32, sky: V3) -> Vec<(usize, V3, f32)> {
    let n = o.cascades;
    let finest = voxel_size(VOXEL_FINEST);
    let width0 = o.cone_width * (4.0 * PI / (2 * o.theta * o.theta) as f32).sqrt();
    let start = add(point, scale(normal, 0.75 * finest));
    let mut t = 0.75 * finest;
    let mut covered = 0.0f32;
    let mut reach = t0;
    let mut out = Vec::with_capacity(n);
    for c in 0..n {
        let end = if c + 1 == n { REACH } else { reach };
        let width = width0 / (1u32 << c) as f32;
        let open = 1.0 - covered;
        let mut met = [0.0; 3];
        while t < end && covered < 0.98 {
            let wide = (width * t).max(finest);
            let level = ((wide / BASE_SPACING).log2().ceil() as i32)
                .clamp(i32::from(VOXEL_FINEST), i32::from(o.max_level)) as u8;
            let step = voxel_size(level);
            let front = o.front_order.then_some(start);
            let (cover, light) = voxels.sample(level, add(start, scale(dir, t)), dir, front);
            if cover > 0.0 && level > o.refine_level {
                // Something is here: walk this step again at the finer level,
                // whose read cannot reach the far side of a wall.
                let fine = voxel_size(o.refine_level);
                let mut u = t - 0.5 * step + 0.5 * fine;
                while u < t + 0.5 * step && covered < 0.98 {
                    let (cover, light) = voxels.sample(o.refine_level, add(start, scale(dir, u)), dir, front);
                    if cover > 0.0 {
                        let seen = cover.min(1.0 - covered);
                        met = add(met, scale(light, seen / cover));
                        covered += seen;
                    }
                    u += fine;
                }
            } else if cover > 0.0 {
                let seen = cover.min(1.0 - covered);
                met = add(met, scale(light, seen / cover));
                covered += seen;
            }
            t += step;
        }
        let mut through = (1.0 - covered) / open;
        if c + 1 == n {
            met = add(met, scale(sky, 1.0 - covered));
            through = 0.0;
        }
        out.push((c, scale(met, 1.0 / open), through));
        if covered >= 0.98 {
            break;
        }
        reach *= 4.0;
    }
    out
}

/// One cone's whole radiance: each interval's light times the share that
/// reached it.
fn cone_total(intervals: &[(usize, V3, f32)]) -> V3 {
    let mut open = 1.0;
    let mut sum = [0.0; 3];
    for (_, radiance, through) in intervals {
        sum = add(sum, scale(*radiance, open));
        open *= through;
    }
    sum
}

/// Sparse trilinear read of one direction's cone: the probes that exist and
/// have that direction, renormalised.
fn interpolate(
    cascade: &Cascade,
    index: &HashMap<Key, u32>,
    lod: i8,
    level: usize,
    at: V3,
    bin: usize,
) -> Option<(V3, V3)> {
    let s = spacing(lod, level);
    let g = [at[0] / s - 0.5, at[1] / s - 0.5, at[2] / s - 0.5];
    let base = [g[0].floor(), g[1].floor(), g[2].floor()];
    let f = [g[0] - base[0], g[1] - base[1], g[2] - base[2]];
    let dirs = cascade.dirs();
    let mut sum = [0.0; 3];
    let mut moment = [0.0; 3];
    let mut weight = 0.0;
    for corner in 0..8u32 {
        let o = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
        let w = (0..3)
            .map(|a| if o[a] == 1 { f[a] } else { 1.0 - f[a] })
            .product::<f32>();
        if w <= 0.0 {
            continue;
        }
        let key = Key {
            lod,
            cascade: u8::try_from(level).unwrap_or(u8::MAX),
            cell: [
                base[0] as i32 + o[0] as i32,
                base[1] as i32 + o[1] as i32,
                base[2] as i32 + o[2] as i32,
            ],
        };
        let Some(&slot) = index.get(&key) else { continue };
        let i = slot as usize * dirs + bin;
        if cascade.filled[i] {
            sum = add(sum, scale(cascade.cone[i], w));
            moment = add(moment, scale(cascade.cone_moment[i], w));
            weight += w;
        }
    }
    (weight > 0.0).then(|| (scale(sum, 1.0 / weight), scale(moment, 1.0 / weight)))
}

/// Direct irradiance from the glowing spheres at a point: the lamp unit times
/// the visible share of each sphere's disc.
fn sphere_light(scene: &dyn Surfaces, lamps: &[(V3, V3)], radius: f32, point: V3, normal: V3) -> V3 {
    let origin = add(point, scale(normal, LIFT));
    let mut sum = [0.0; 3];
    for (centre, glow) in lamps {
        let to = sub(*centre, origin);
        let d2 = dot(to, to);
        let toward = normalize(to);
        let cosine = dot(toward, normal);
        if cosine <= 0.0 || d2 <= radius * radius {
            continue;
        }
        let side = normalize(cross(toward, if toward[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] }));
        let up = cross(toward, side);
        let grid = 8u32;
        let mut seen = 0u32;
        for i in 0..grid * grid {
            let a = ((i % grid) as f32 + 0.5) / grid as f32;
            let b = ((i / grid) as f32 + 0.5) / grid as f32;
            let r = radius * a.sqrt();
            let phi = 2.0 * PI * b;
            let target = add(*centre, add(scale(side, r * phi.cos()), scale(up, r * phi.sin())));
            if !scene.blocked(origin, target) {
                seen += 1;
            }
        }
        let visible = seen as f32 / (grid * grid) as f32;
        // The sphere's radiance times its solid angle: pi r^2 / d^2.
        sum = add(sum, scale(*glow, PI * radius * radius / d2 * cosine * visible));
    }
    sum
}

/// The cone tracer's voxels: surface points of the whole building, or of the
/// picture alone, each with the light it sends out, and the lamps.
fn build_voxels(scene: &Scene, mesh: &SceneMesh, pixels: &[Option<Pixel>], glow: &[(V3, V3)], o: &Options) -> Voxels {
    // Point, normal, area, albedo.
    let mut points: Vec<(V3, V3, f32, V3)> = Vec::new();
    if o.world_voxels {
        let step = 0.5 * voxel_size(VOXEL_FINEST);
        for tri in &mesh.triangles {
            let [a, b, c] = tri.positions;
            let (e1, e2) = (sub(b, a), sub(c, a));
            let face = cross(e1, e2);
            let area = 0.5 * dot(face, face).sqrt();
            if area <= 0.0 {
                continue;
            }
            let normal = normalize(tri.normals[0]);
            let albedo = mesh.materials.get(tri.material as usize).map_or([0.0; 3], |m| m.albedo);
            let n1 = (dot(e1, e1).sqrt() / step).ceil().max(1.0) as u32;
            let n2 = (dot(e2, e2).sqrt() / step).ceil().max(1.0) as u32;
            let first = points.len();
            for i in 0..n1 {
                for j in 0..n2 {
                    let u = (i as f32 + 0.5) / n1 as f32;
                    let v = (j as f32 + 0.5) / n2 as f32;
                    if u + v <= 1.0 {
                        points.push((add(a, add(scale(e1, u), scale(e2, v))), normal, 0.0, albedo));
                    }
                }
            }
            let each = area / (points.len() - first).max(1) as f32;
            let made = points.len();
            for point in &mut points[first..made] {
                point.2 = each;
            }
            if tri.two_sided {
                for k in first..made {
                    let (at, n, each, albedo) = points[k];
                    points.push((at, scale(n, -1.0), each, albedo));
                }
            }
        }
    } else {
        points.extend(pixels.iter().flatten().map(|p| (p.point, p.normal, p.area, p.albedo)));
    }
    let mut voxels = Voxels::new();
    if o.mode == Mode::Emissive {
        for (at, normal, area, _) in &points {
            voxels.add_surface(*at, *normal, *area, [0.0; 3]);
        }
        for (centre, glow) in glow {
            voxels.add_lamp(*centre, o.lamp_radius, *glow);
        }
        return voxels;
    }
    // One direct-light sample for each patch of LIGHT_LEVEL that faces one way.
    let size = voxel_size(LIGHT_LEVEL);
    let patch_of = |at: V3, normal: V3| {
        let axis = (0..6).max_by(|a, b| dot(normal, AXES[*a]).total_cmp(&dot(normal, AXES[*b]))).unwrap_or(0);
        (nearest_cell(sub(at, scale(normal, 0.01)), size), axis)
    };
    let mut patches: HashMap<([i32; 3], usize), (V3, V3, f32)> = HashMap::new();
    for (at, normal, _, _) in &points {
        let patch = patches.entry(patch_of(*at, *normal)).or_insert(([0.0; 3], [0.0; 3], 0.0));
        patch.0 = add(patch.0, *at);
        patch.1 = add(patch.1, *normal);
        patch.2 += 1.0;
    }
    let mut lit: HashMap<([i32; 3], usize), V3> = HashMap::new();
    for (key, (at, normal, count)) in &patches {
        let (light, rays) = direct_light(scene, mesh, scale(*at, 1.0 / count), normalize(*normal));
        voxels.shadow_rays += rays;
        lit.insert(*key, light);
    }
    for (at, normal, area, albedo) in &points {
        let direct = lit.get(&patch_of(*at, *normal)).copied().unwrap_or([0.0; 3]);
        voxels.add_surface(*at, *normal, *area, scale(mul(*albedo, direct), LAMBERT));
    }
    voxels
}

/// An f32 as an IEEE half float; too small is 0, too large is the largest half.
fn half(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    if exponent <= 0 {
        sign
    } else if exponent >= 31 {
        sign | 0x7bff
    } else {
        sign | ((exponent as u16) << 10) | ((bits & 0x007f_ffff) >> 13) as u16
    }
}

/// `--gpu-bench`: the cone march on the GPU over the same voxels, and the
/// hardware ray tracer on the same picture.
fn gpu_bench(
    o: &Options,
    scene: &Scene,
    mesh: &SceneMesh,
    pixels: &[Option<Pixel>],
    voxels: &Voxels,
    eye: V3,
    [forward, right, up]: [V3; 3],
) -> Result<(), String> {
    // A clip volume of each voxel level around the eye, the levels stacked on z.
    let side = gpu::SIDE as i32;
    let mut clip = [[0.0f32; 4]; 8];
    let mut corner = [[0i32; 3]; 8];
    for level in VOXEL_FINEST..=VOXEL_COARSEST {
        let size = voxel_size(level);
        let cell = nearest_cell(eye, size);
        let k = usize::from(level - VOXEL_FINEST);
        corner[k] = [cell[0] - side / 2, cell[1] - side / 2, cell[2] - side / 2];
        clip[k] = [corner[k][0] as f32 * size, corner[k][1] as f32 * size, corner[k][2] as f32 * size, size];
    }
    let texels = (side * side * side) as usize * gpu::LEVELS as usize * 4;
    let mut volumes: [Vec<u16>; 7] = std::array::from_fn(|_| vec![0u16; texels]);
    let mut inside = 0usize;
    for ((level, cell), voxel) in &voxels.cells {
        let k = usize::from(level - VOXEL_FINEST);
        let local = [cell[0] - corner[k][0], cell[1] - corner[k][1], cell[2] - corner[k][2]];
        if local.iter().any(|v| *v < 0 || *v >= side) {
            continue;
        }
        inside += 1;
        let at = (((local[2] + side * k as i32) * side + local[1]) * side + local[0]) as usize * 4;
        for d in 0..6 {
            for c in 0..3 {
                volumes[d][at + c] = half(voxel.light[d][c]);
            }
            volumes[d][at + 3] = half(voxel.area[d]);
        }
        for c in 0..3 {
            volumes[6][at + c] = half(voxel.lamp_light[c]);
        }
        volumes[6][at + 3] = half(voxel.lamp_area);
    }
    let surface: Vec<[f32; 8]> = pixels
        .iter()
        .map(|p| match p {
            Some(p) => [
                p.point[0], p.point[1], p.point[2], o.t0 * spacing(p.lod, 0),
                p.normal[0], p.normal[1], p.normal[2], 1.0,
            ],
            None => [0.0; 8],
        })
        .collect();
    let sky = if o.mode == Mode::Emissive { [0.0; 3] } else { scene.sky.as_ref().map_or([0.0; 3], |s| s.color) };
    let timing = gpu::time_cones(
        &gpu::ConeInput {
            surface: &surface,
            volumes: &volumes,
            clip,
            width0: o.cone_width * (4.0 * PI / (2 * o.theta * o.theta) as f32).sqrt(),
            cascades: u32::try_from(o.cascades).unwrap_or(u32::MAX),
            sky,
        },
        60,
    )?;
    println!("gpu bench on {}  {}x{}  mode {}", timing.device, o.width, o.height, if o.mode == Mode::Emissive { "emissive" } else { "nee" });
    println!(
        "cone march: {:.3} ms median, {:.3} ms least, {} cones, {:.1} steps and {:.1} filtered texture reads a cone, mean cover {:.3}",
        timing.ms_median, timing.ms_min, timing.cones, timing.steps_per_cone, timing.reads_per_cone, timing.covered
    );
    println!(
        "  voxel textures: 7 volumes of {side}x{side}x{} RGBA16F ({:.0} MiB), {inside} of {} voxels inside the clip volumes",
        side * gpu::LEVELS as i32,
        7.0 * texels as f64 * 2.0 / (1024.0 * 1024.0),
        voxels.cells.len(),
    );
    let tris: Vec<genos_gpuref::Tri> = mesh
        .triangles
        .iter()
        .map(|t| genos_gpuref::Tri {
            positions: t.positions,
            normals: t.normals,
            albedo: mesh.materials.get(t.material as usize).map_or([0.0; 3], |m| m.albedo),
            two_sided: t.two_sided,
        })
        .collect();
    let lamps: Vec<genos_gpuref::Lamp> = (0..scene.light_len())
        .map(|i| {
            let l = scene.light(i);
            genos_gpuref::Lamp {
                position: [l.position.x, l.position.y, l.position.z],
                direction: [l.direction.x, l.direction.y, l.direction.z],
                color: l.color,
            }
        })
        .collect();
    let lamp_count = lamps.len();
    let (path_ms, device) = gpu::time_rays(&genos_gpuref::Setup {
        tris,
        lamps,
        sky: scene.sky.as_ref().map_or([0.0; 3], |s| s.color),
        eye,
        forward,
        right,
        up,
        fov_y_deg: FOV_Y,
        width: u32::try_from(o.width).unwrap_or(u32::MAX),
        height: u32::try_from(o.height).unwrap_or(u32::MAX),
        max_bounces: 1,
        noise_target: 0.0,
        max_spp: 32,
        seconds: 600.0,
        paths_per_pass: 4,
    })?;
    println!(
        "hardware rays on {device}: {path_ms:.3} ms for one path a pixel (a camera ray, a bounce ray, and a shadow ray to each of {lamp_count} lamps at both hits)"
    );
    println!(
        "  one cascade frame is one ray a pixel and the shadow rays at its hit: about half a path, {:.3} ms",
        path_ms * 0.5
    );
    Ok(())
}

fn tone(c: V3, exposure: f32) -> [u8; 3] {
    c.map(|v| ((v * exposure).max(0.0).powf(1.0 / 2.2).min(1.0) * 255.0).round() as u8)
}

fn run(o: &Options) -> Result<(), String> {
    let root = genos_scene_lib::repo_root();
    let entries = genos_scene_lib::load_dir(&genos_scene_lib::scenes_dir())?;
    let name = genos_scene_lib::canonical(&o.scene);
    let entry = entries
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| format!("no scene {name} under {}", root.display()))?;
    let (scene, gltf) = load_scene(&root, entry, o)?;
    if gltf.is_some() && (o.cone || o.gpu_bench) {
        return Err(format!("{name}: the cone tracer voxelises the analytic building only so far"));
    }
    let scene = &scene;
    // The scene's triangles through the CPU BVH: the analytic tracer walks
    // every shape for every ray and is about a hundred times slower here.
    let mesh = SceneMesh::from_scene(scene);
    let walls: &dyn Surfaces = match &gltf {
        Some(model) => model,
        None => &mesh,
    };
    let (eye, yaw, pitch) = match (o.eye, &o.checkpoint) {
        (Some(eye), _) => (eye, o.yaw, o.pitch),
        (None, Some(cp)) => {
            let c = entry.checkpoint(cp).ok_or_else(|| format!("{name} has no checkpoint {cp}"))?;
            (c.eye, c.yaw, c.pitch)
        }
        (None, None) => {
            let c = entry.checkpoints.first().ok_or_else(|| format!("{name} has no checkpoint"))?;
            (c.eye, c.yaw, c.pitch)
        }
    };
    // The picture: one visible point per pixel, and its level of detail.
    let forward = {
        let d = look_direction(yaw, pitch);
        [d.x, d.y, d.z]
    };
    let right = normalize(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    let tan_y = (FOV_Y.to_radians() * 0.5).tan();
    let aspect = o.width as f32 / o.height as f32;
    let per_pixel = 2.0 * tan_y / o.height as f32;
    let pixels: Vec<Option<Pixel>> = (0..o.width * o.height)
        .map(|i| {
            let (x, y) = (i % o.width, i / o.width);
            let nx = (2.0 * (x as f32 + 0.5) / o.width as f32 - 1.0) * tan_y * aspect;
            let ny = (1.0 - 2.0 * (y as f32 + 0.5) / o.height as f32) * tan_y;
            let dir = normalize(add(add(forward, scale(right, nx)), scale(up, ny)));
            let hit = walls.first_hit(eye, dir, REACH)?;
            let away = sub(hit.point, eye);
            let reach = away[0].abs().max(away[1].abs()).max(away[2].abs());
            let wanted = o.probe_px * per_pixel * reach;
            let lod = (wanted / BASE_SPACING).log2().round().clamp(0.0, 12.0) as i8;
            let wide = per_pixel * dot(away, forward);
            let area = wide * wide / dot(hit.normal, scale(dir, -1.0)).max(0.05);
            Some(Pixel { point: hit.point, normal: hit.normal, albedo: hit.albedo, area, lod })
        })
        .collect();
    let shown = pixels.iter().filter(|p| p.is_some()).count();
    let glow: Vec<(V3, V3)> = (0..scene.light_len())
        .map(|i| scene.light(i))
        .filter(|l| l.direction.x == 0.0 && l.direction.y == 0.0 && l.direction.z == 0.0)
        .map(|l| {
            let centre = [l.position.x, l.position.y, l.position.z];
            (centre, scale(l.color, LAMP_UNIT / (PI * o.lamp_radius * o.lamp_radius)))
        })
        .collect();
    let glowing = Glowing { scene: walls, lamps: glow.clone(), radius: o.lamp_radius };
    let surfaces: &dyn Surfaces = if o.mode == Mode::Emissive { &glowing } else { walls };
    let voxels = (o.cone || o.gpu_bench).then(|| build_voxels(scene, &mesh, &pixels, &glow, o));
    if let (true, Some(voxels)) = (o.gpu_bench, &voxels) {
        return gpu_bench(o, scene, &mesh, &pixels, voxels, eye, [forward, right, up]);
    }
    if let (Some((px, py)), Some(voxels)) = (o.probe_pixel, &voxels) {
        let Some(pixel) = &pixels[py * o.width + px] else { return Err(format!("pixel {px},{py} shows nothing")) };
        let sky = if o.mode == Mode::Emissive { [0.0; 3] } else { scene.sky.as_ref().map_or([0.0; 3], |s| s.color) };
        let t0 = o.t0 * spacing(pixel.lod, 0);
        let side = normalize(cross(pixel.normal, if pixel.normal[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] }));
        let up = cross(pixel.normal, side);
        let origin = add(pixel.point, scale(pixel.normal, LIFT));
        let shadow = AtomicU64::new(0);
        let probe_setup = Setup { scene, walls, surfaces, voxels: None, pixels: &pixels, options: o };
        let mut rows = Vec::new();
        let (mut ray_sum, mut cone_sum) = (0.0f64, 0.0f64);
        for k in 0..1024u64 {
            let (u, v) = r2(k);
            let (r, phi) = (u.sqrt(), 2.0 * PI * v);
            let dir = normalize(add(add(scale(side, r * phi.cos()), scale(up, r * phi.sin())), scale(pixel.normal, (1.0 - u).max(0.0).sqrt())));
            let (ray, at) = match surfaces.first_hit(origin, dir, REACH) {
                Some(hit) => (hit_radiance(&probe_setup, &hit, &shadow), hit.point),
                None => (sky, add(origin, scale(dir, REACH))),
            };
            let cone = cone_total(&march_cone(voxels, o, pixel.point, pixel.normal, dir, t0, sky));
            ray_sum += f64::from(luma(ray));
            cone_sum += f64::from(luma(cone));
            rows.push((luma(cone) - luma(ray), dir, at, luma(ray), luma(cone)));
        }
        println!("probe pixel {px},{py} at {:?} facing {:?}: ray mean {:.5}, cone mean {:.5}", pixel.point, pixel.normal, ray_sum / 1024.0, cone_sum / 1024.0);
        rows.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (d, dir, at, ray, cone) in rows.iter().take(12) {
            println!("  dir [{:.2},{:.2},{:.2}] ray hit [{:.2},{:.2},{:.2}] ray {ray:.4} cone {cone:.4} (+{d:.4})", dir[0], dir[1], dir[2], at[0], at[1], at[2]);
        }
        return Ok(());
    }
    if let (true, Some(voxels)) = (o.voxel_check, &voxels) {
        // Stored light per unit area of the finest voxel a pixel's surface is
        // in, against the exact direct radiance at the pixel.
        let mut by: HashMap<usize, (f64, f64, u32)> = HashMap::new();
        for pixel in pixels.iter().step_by(97).flatten() {
            let axis = (0..6).max_by(|a, b| dot(pixel.normal, AXES[*a]).total_cmp(&dot(pixel.normal, AXES[*b]))).unwrap_or(0);
            let size = voxel_size(VOXEL_FINEST);
            let inside = sub(pixel.point, scale(pixel.normal, 0.25 * size));
            let Some(voxel) = voxels.cells.get(&(VOXEL_FINEST, nearest_cell(inside, size))) else { continue };
            if voxel.area[axis] <= 0.0 {
                continue;
            }
            let stored = luma(voxel.light[axis]) / voxel.area[axis];
            let (direct, _) = direct_light(scene, walls, pixel.point, pixel.normal);
            let exact = luma(scale(mul(pixel.albedo, direct), LAMBERT));
            let e = by.entry(axis).or_insert((0.0, 0.0, 0));
            e.0 += f64::from(stored);
            e.1 += f64::from(exact);
            e.2 += 1;
        }
        let mut keys: Vec<_> = by.keys().copied().collect();
        keys.sort_unstable();
        for axis in keys {
            let (stored, exact, n) = by[&axis];
            println!("voxel check axis {axis}: {n} pixels, stored {:.4}, exact {:.4}, ratio {:.3}", stored / f64::from(n), exact / f64::from(n), stored / exact.max(1.0e-9));
        }
    }
    let setup = Setup { scene, walls, surfaces, voxels: voxels.as_ref(), pixels: &pixels, options: o };

    let started = std::time::Instant::now();
    let frames: Vec<Frame> = (0..o.frames)
        .map(|f| {
            let (u, v) = r2(u64::from(f) * 7919 + 1);
            run_frame(&setup, if f == 0 { (0.0, 0.0) } else { (u, v) })
        })
        .collect();
    let cascade_seconds = started.elapsed().as_secs_f32();

    // The reference at a grid of pixels.
    // A block of `--block` pixels each way around each grid point. A block
    // is compared when at least half its pixels show a surface, as
    // capture.compare grades.
    let samples: Vec<usize> = (o.stride / 2..o.height)
        .step_by(o.stride)
        .flat_map(|y| (o.stride / 2..o.width).step_by(o.stride).map(move |x| y * o.width + x))
        .filter(|&i| pixels[i].is_some())
        .collect();
    let block_of = |i: usize| -> Vec<usize> {
        let (x, y) = (i % o.width, i / o.width);
        let x0 = x.saturating_sub(o.block / 2);
        let y0 = y.saturating_sub(o.block / 2);
        (y0..(y0 + o.block).min(o.height))
            .flat_map(|yy| (x0..(x0 + o.block).min(o.width)).map(move |xx| yy * o.width + xx))
            .filter(|&j| pixels[j].is_some())
            .collect()
    };
    let samples: Vec<usize> = samples.into_iter().filter(|&i| 2 * block_of(i).len() >= o.block * o.block).collect();
    let blocks: Vec<Vec<usize>> = samples.iter().map(|&i| block_of(i)).collect();
    let started = std::time::Instant::now();
    let chunk = samples.len().div_ceil(o.threads).max(1);
    let reference: Vec<V3> = std::thread::scope(|scope| {
        let handles: Vec<_> = samples
            .chunks(chunk)
            .map(|part| {
                let pixels = &pixels;
                let glow = &glow;
                let blocks = &blocks;
                let samples = &samples;
                scope.spawn(move || {
                    part.iter()
                        .map(|&i| {
                            let k = samples.binary_search(&i).unwrap_or(0);
                            let block = &blocks[k];
                            let each = (o.ref_samples / u32::try_from(block.len()).unwrap_or(1).max(1)).max(1);
                            let mut sum = [0.0; 3];
                            for &j in block {
                                let Some(pixel) = &pixels[j] else { continue };
                                let v = if o.mode == Mode::Emissive {
                                    sphere_light(walls, glow, o.lamp_radius, pixel.point, pixel.normal)
                                } else {
                                    bounce_reference(scene, walls, pixel.point, pixel.normal, each, j as u64)
                                };
                                sum = add(sum, v);
                            }
                            scale(sum, 1.0 / block.len().max(1) as f32)
                        })
                        .collect::<Vec<V3>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    let reference_seconds = started.elapsed().as_secs_f32();

    // Numbers.
    let ref_mean = reference.iter().map(|c| f64::from(luma(*c))).sum::<f64>() / samples.len().max(1) as f64;
    let got_at = |f: &Frame, k: usize| -> f64 {
        let block = &blocks[k];
        block.iter().map(|&j| f64::from(luma(f.irradiance[j]))).sum::<f64>() / block.len().max(1) as f64
    };
    let error_of = |take: usize| -> (f64, f64) {
        // Mean absolute error and signed error of the mean of `take` frames.
        let mut abs = 0.0;
        let mut signed = 0.0;
        for k in 0..samples.len() {
            let got = frames[..take].iter().map(|f| got_at(f, k)).sum::<f64>() / take as f64;
            let want = f64::from(luma(reference[k]));
            abs += (got - want).abs();
            signed += got - want;
        }
        let n = samples.len().max(1) as f64;
        (abs / n / ref_mean.max(1.0e-9), signed / n / ref_mean.max(1.0e-9))
    };
    let mut noise: Vec<f64> = (0..samples.len())
        .filter_map(|k| {
            let values: Vec<f64> = frames.iter().map(|f| got_at(f, k)).collect();
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            if mean <= 1.0e-6 || values.len() < 2 {
                return None;
            }
            let var = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / (values.len() - 1) as f64;
            Some(var.sqrt() / mean)
        })
        .collect();
    noise.sort_by(f64::total_cmp);
    let pick = |q: f64| noise.get(((noise.len() as f64 - 1.0) * q) as usize).copied().unwrap_or(0.0);

    let first = &frames[0];
    println!("scene {name}  eye {eye:?} yaw {yaw} pitch {pitch}  {}x{}  {} point lamps of {}", o.width, o.height, glow.len(), scene.light_len());
    println!("mode {}  tracer {}  theta {} ({} directions at cascade 0)  cascades {}  probe {} px  t0 {}  spp {}", if o.mode == Mode::Emissive { "emissive" } else { "nee" }, if o.cone { "cone" } else { "ray" }, o.theta, 2 * o.theta * o.theta, o.cascades, o.probe_px, o.t0, o.spp);
    println!("pixels shown {shown}  probes per cascade {:?}", first.probes);
    println!("rays per frame {}  ({:.2} per shown pixel)", first.rays, first.rays as f64 / shown.max(1) as f64);
    println!("shadow rays per frame {}  (one per lamp at every ray hit; 0 in emissive mode)", first.shadow_rays);
    if let Some(voxels) = &voxels {
        println!(
            "cone tracer: voxels from the {}, {} surface points in {} voxels, {} voxel reads per frame ({:.1} per cone), {} shadow rays to light the points (once while nothing moves)",
            if o.world_voxels { "whole building" } else { "picture" },
            voxels.points,
            voxels.cells.len(),
            first.voxel_lookups,
            first.voxel_lookups as f64 / (shown as f64 * f64::from(o.spp)).max(1.0),
            voxels.shadow_rays,
        );
    }
    println!("cascade-0 directions with no data {:.4}", first.holes);
    println!("compared blocks {} of {}x{} pixels  reference mean irradiance {ref_mean:.4}", samples.len(), o.block, o.block);
    for take in [1usize, 4, 8, 16] {
        if take <= frames.len() {
            let (abs, signed) = error_of(take);
            println!("mean of {take:2} frame(s): error {abs:.4} of the reference mean, bias {signed:+.4}");
        }
    }
    println!("single-frame noise (std over mean across {} frames): median {:.4}  p90 {:.4}", frames.len(), pick(0.5), pick(0.9));
    println!("cpu seconds: cascade {cascade_seconds:.1} for {} frames, reference {reference_seconds:.1}", frames.len());

    // Pictures: one frame, the mean of all frames, and the compared grid.
    std::fs::create_dir_all(&o.out).map_err(|e| format!("{}: {e}", o.out.display()))?;
    let exposure = (0.35 / ref_mean.max(1.0e-6)) as f32;
    let picture = |values: &dyn Fn(usize) -> V3, w: usize, h: usize| Image {
        width: u32::try_from(w).unwrap_or(u32::MAX),
        height: u32::try_from(h).unwrap_or(u32::MAX),
        rgb: (0..w * h).flat_map(|i| tone(values(i), exposure)).collect(),
    };
    picture(&|i| first.irradiance[i], o.width, o.height).save(&o.out.join("frame.png"))?;
    let take = frames.len() as f32;
    picture(
        &|i| scale(frames.iter().fold([0.0; 3], |acc, f| add(acc, f.irradiance[i])), 1.0 / take),
        o.width,
        o.height,
    )
    .save(&o.out.join("mean.png"))?;
    let (gw, gh) = (
        (o.stride / 2..o.width).step_by(o.stride).count(),
        (o.stride / 2..o.height).step_by(o.stride).count(),
    );
    let mut grid_ref = vec![[0.0; 3]; gw * gh];
    let mut grid_got = vec![[0.0; 3]; gw * gh];
    for (k, &i) in samples.iter().enumerate() {
        let (x, y) = (i % o.width / o.stride, i / o.width / o.stride);
        grid_ref[y * gw + x] = reference[k];
        grid_got[y * gw + x] = first.irradiance[i];
    }
    if o.dump {
        // One row per compared block: where it is, what faces it, and the two values.
        let mut csv = String::from("x,y,px,py,pz,nx,ny,nz,lod,distance,reference,frame\n");
        for (k, &i) in samples.iter().enumerate() {
            let Some(pixel) = &pixels[i] else { continue };
            let away = sub(pixel.point, eye);
            let _ = writeln!(
                csv,
                "{},{},{:.4},{:.4},{:.4},{:.3},{:.3},{:.3},{},{:.3},{:.6},{:.6}",
                i % o.width,
                i / o.width,
                pixel.point[0],
                pixel.point[1],
                pixel.point[2],
                pixel.normal[0],
                pixel.normal[1],
                pixel.normal[2],
                pixel.lod,
                dot(away, away).sqrt(),
                luma(reference[k]),
                got_at(first, k),
            );
        }
        std::fs::write(o.out.join("samples.csv"), csv).map_err(|e| format!("samples.csv: {e}"))?;
    }
    picture(&|i| grid_ref[i], gw, gh).save(&o.out.join("grid-reference.png"))?;
    picture(&|i| grid_got[i], gw, gh).save(&o.out.join("grid-frame.png"))?;
    println!("wrote frame.png, mean.png, grid-reference.png, grid-frame.png in {}", o.out.display());
    Ok(())
}

fn main() -> ExitCode {
    match parse_args() {
        Ok(None) => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        Ok(Some(options)) => match run(&options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("genos-single-frame-cascade: {err}");
                ExitCode::FAILURE
            }
        },
        Err(err) => {
            eprintln!("genos-single-frame-cascade: {err}\n\n{HELP}");
            ExitCode::from(2)
        }
    }
}
