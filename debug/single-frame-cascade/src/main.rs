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

use std::collections::{HashMap, HashSet};
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
                       the cascade carries the lamps' own light.
                       full: lamps are glowing spheres, surfaces keep their
                       albedo, and after each frame every 25 cm patch gathers
                       its light through the voxels with 16 cones, so bounce
                       gains one bounce a frame; the sun is left out; the
                       reference is a multi-bounce path trace (default nee)
  --cone-start S       with --tracer cone, where each probe direction's cone
                       starts. voxel: at one of the cascade-0 voxel probes under
                       the probe, run through every interval and split into
                       that voxel's probes (default). probe: at the probe's own
                       seat, over its own interval only
  --cone-voxel P       with --cone-start voxel, which voxel probe a coarser
                       probe's cones start at. nearest: the one nearest its
                       seat, for every direction and frame (default). random:
                       a new one for each direction and frame
  --combine-visibility V  with --tracer cone: where a read between probes takes
                       only what passes from the reader to each probe's seat.
                       merge: a probe reading the coarser cascade. shade: a
                       pixel reading the voxel layer. none or both (default both)
  --gather-cones N     with --mode full: cones each 25 cm patch gathers its lamp
                       and sky light with each frame (default 16)
  --patch-check        with --mode full: after the frames, the solved light of
                       40 patches in view against a path-traced reference
                       there, and their D against the exact lamp light
  --sun on|off         with --mode full: the scene's sun as an emitter (phase
                       2b): in the sky for the cascade's cones, one narrow cone
                       toward it for each gather patch, exact in the reference
                       (default off)
  --sun-path P         how the pixels get the sun. sky: through the cascades, as
                       a bright direction of the last one. voxel: each voxel-
                       layer probe casts one narrow cone toward it, and a pixel
                       takes the sun times its cosine times the probes' share
                       that sees it (default voxel)
  --no-lamps           leave the point lamps out
  --sky on|off         the scene's sky light, in the cascade and the reference
                       (default on)
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
  --fine-walk W        how a step at or under --refine-level counts surfaces.
                       sample: trilinear samples a fine voxel apart. cross:
                       each plane once, where the axis crosses it, in the voxel
                       and the box of points that hold the crossing point
                       (default cross)
  --cross-full F       with --fine-walk cross: a crossed voxel this full or more
                       blocks the whole cone; a less full one blocks the share
                       it fills (default 0.5)
  --box-margin M       with --fine-walk cross: metres past the box of a voxel's
                       points for one direction where a crossing still counts
                       (default 0.031, half the finest voxel)
  --cover-mix M        with --fine-walk cross, how crossed planes combine. add:
                       cover adds up to 1. multiply: each plane passes its open
                       share of what reached it, so two half-open layers pass
                       a quarter (default add)
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
  --sun-check          with --eye: build the renderer's field about the eye and,
                       in a scene without triangles, print a map of the sun
                       share the field lets onto the ground; otherwise,
                       over floor points within 16 m of it, compare the sun the
                       exact ray sees (triangles, alpha holes) with the share
                       the field's march lets through (the renderer's sun
                       share); print the faces that block where the ray is open
  --renderer-field     build the renderer's voxel field (crates/render
                       cone_field) for the scene, print its size and build
                       time, and stop; with --eye, the field the renderer
                       builds about that eye (fine levels in clip volumes)
  --field-check        as --renderer-field, then also build the reference
                       field (every point's albedo ray, one thread) and print
                       each level's voxels and area against it
  --gpu-side N         with --gpu-bench: texels along each side of a voxel
                       level's clip volume around the eye (default 96)
  --own-interval       with --cone-start voxel: each cascade's cone marches
                       only its own cascade's interval, as a sparse cascade
                       traces, in place of every interval from the voxel
                       layer (--gpu-bench follows it)
  --near-check C       with --own-interval: a cascade-c cone (c > 0) first
                       marches from its start to its interval; closed, the
                       probe takes no value in that direction. off, filtered
                       (the filtered read only), fine (with the crossing
                       test and the walk), or chain: no march; each probe
                       starts on a finer child's start voxel, the cascades
                       are traced finest first, and the share open is the
                       product of the chain's finer probes' passed shares
                       in the bins holding the direction (default off)
  --transfer-sweeps N  with --mode full: sweeps of E = D + T E a frame, each one
                       bounce more, from E = D (default 32; the room no lamp
                       lights reads the same as at 256)
  --gather-width F     with --mode full: each gather cone's width a metre out,
                       the same at every distance (0: it narrows as the
                       cascade's cones do; default 0)
  --gather-level L     with --mode full: gather patches are voxels of this
                       level, 2^L/64 m (default 4: 25 cm)
  --descend            a covered coarse step is walked front to back from its
                       coarsest half down: a piece with no surface in or next
                       to its cells is passed whole, and only the rest goes
                       down to the refine level's crossing test (--gpu-bench
                       follows it)
  --seat-at-start      with --cone-voxel nearest: the merge's visibility test
                       aims at the voxel-layer point a coarse probe's cones
                       start from, not at the probe's seat
  --refine-cascades N  only cascades below N take the plane-crossing test and
                       walk a covered coarse step again at the refine level;
                       the rest take the filtered read (default all)
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
/// The engine's default reflectance (white paint): the share of arriving light
/// an analytic surface sends back. The analytic mesh's albedo is the colour
/// alone; `--mode full` multiplies it in. A glTF albedo is already the
/// surface's.
const PAINT: f32 = 0.8;
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
    /// Lamps are glowing spheres in the voxels, surfaces keep their albedo and
    /// send out what they gathered the frame before: every light is an emitter
    /// and bounce gains one bounce a frame. The sun is left out (phase 2b).
    Full,
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
    sky: bool,
    sun: bool,
    sun_in_voxels: bool,
    no_lamps: bool,
    gather_cones: u64,
    patch_check: bool,
    see_merge: bool,
    cones_from_voxels: bool,
    random_cone_voxel: bool,
    see_shade: bool,
    lamp_radius: f32,
    cone: bool,
    world_voxels: bool,
    cone_width: f32,
    /// Count a voxel's area only when the cone starts on the side it faces.
    front_order: bool,
    max_level: u8,
    refine_level: u8,
    fine_cross: bool,
    cross_full: f32,
    box_margin: f32,
    multiply_cover: bool,
    voxel_check: bool,
    probe_pixel: Option<(usize, usize)>,
    gpu_bench: bool,
    own_interval: bool,
    seat_at_start: bool,
    near_check: NearCheck,
    descend: bool,
    transfer_sweeps: usize,
    gather_level: u8,
    gather_width: f32,
    gpu_side: u32,
    renderer_field: bool,
    sun_check: bool,
    field_check: bool,
    refine_cascades: usize,
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
        sky: true,
        sun: false,
        sun_in_voxels: true,
        no_lamps: false,
        gather_cones: GATHER_CONES,
        patch_check: false,
        see_merge: true,
        cones_from_voxels: true,
        random_cone_voxel: false,
        see_shade: true,
        lamp_radius: 0.1,
        cone: false,
        world_voxels: true,
        cone_width: 1.0,
        front_order: true,
        max_level: VOXEL_COARSEST,
        refine_level: 3,
        fine_cross: true,
        cross_full: 0.5,
        box_margin: 0.031_25,
        multiply_cover: false,
        voxel_check: false,
        probe_pixel: None,
        gpu_bench: false,
        own_interval: false,
        seat_at_start: false,
        near_check: NearCheck::Off,
        descend: false,
        transfer_sweeps: 32,
        gather_level: GATHER_LEVEL,
        gather_width: 0.0,
        gpu_side: gpu::SIDE,
        renderer_field: false,
        sun_check: false,
        field_check: false,
        refine_cascades: usize::MAX,
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
                    "full" => Mode::Full,
                    other => return Err(format!("--mode is nee, emissive or full, not {other}")),
                }
            }
            "--gather-cones" => o.gather_cones = num::<u64>(&arg, &value()?)?.max(1),
            "--patch-check" => o.patch_check = true,
            "--cone-voxel" => {
                o.random_cone_voxel = match value()?.as_str() {
                    "nearest" => false,
                    "random" => true,
                    other => return Err(format!("bad --cone-voxel: {other}")),
                }
            }
            "--cone-start" => {
                o.cones_from_voxels = match value()?.as_str() {
                    "probe" => false,
                    "voxel" => true,
                    other => return Err(format!("bad --cone-start: {other}")),
                }
            }
            "--combine-visibility" => {
                let v = value()?;
                (o.see_merge, o.see_shade) = match v.as_str() {
                    "none" => (false, false),
                    "merge" => (true, false),
                    "shade" => (false, true),
                    "both" => (true, true),
                    other => return Err(format!("bad --combine-visibility: {other}")),
                };
            }
            "--sun" => {
                o.sun = match value()?.as_str() {
                    "on" => true,
                    "off" => false,
                    other => return Err(format!("bad --sun: {other}")),
                }
            }
            "--no-lamps" => o.no_lamps = true,
            "--sun-path" => {
                o.sun_in_voxels = match value()?.as_str() {
                    "sky" => false,
                    "voxel" => true,
                    other => return Err(format!("bad --sun-path: {other}")),
                }
            }
            "--sky" => {
                o.sky = match value()?.as_str() {
                    "on" => true,
                    "off" => false,
                    other => return Err(format!("bad --sky: {other}")),
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
            "--cover-mix" => {
                o.multiply_cover = match value()?.as_str() {
                    "add" => false,
                    "multiply" => true,
                    other => return Err(format!("bad --cover-mix: {other}")),
                }
            }
            "--box-margin" => o.box_margin = num(&arg, &value()?)?,
            "--cross-full" => o.cross_full = num(&arg, &value()?)?,
            "--fine-walk" => {
                o.fine_cross = match value()?.as_str() {
                    "sample" => false,
                    "cross" => true,
                    other => return Err(format!("bad --fine-walk: {other}")),
                }
            }
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
            "--own-interval" => o.own_interval = true,
            "--seat-at-start" => o.seat_at_start = true,
            "--descend" => o.descend = true,
            "--transfer-sweeps" => o.transfer_sweeps = num(&arg, &value()?)?,
            "--renderer-field" => o.renderer_field = true,
            "--sun-check" => {
                o.renderer_field = true;
                o.sun_check = true;
            }
            "--field-check" => {
                o.renderer_field = true;
                o.field_check = true;
            }
            "--gpu-side" => o.gpu_side = num::<u32>(&arg, &value()?)?.clamp(16, 256),
            "--gather-width" => o.gather_width = num(&arg, &value()?)?,
            "--gather-level" => o.gather_level = num::<u8>(&arg, &value()?)?.clamp(VOXEL_FINEST + 1, VOXEL_COARSEST),
            "--near-check" => {
                o.near_check = match value()?.as_str() {
                    "off" => NearCheck::Off,
                    "filtered" => NearCheck::Filtered,
                    "fine" => NearCheck::Fine,
                    "chain" => NearCheck::Chain,
                    other => return Err(format!("bad --near-check: {other}")),
                }
            }
            "--refine-cascades" => o.refine_cascades = num(&arg, &value()?)?,
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
#[derive(Clone, Copy)]
struct Voxel {
    area: [f32; 6],
    light: [V3; 6],
    /// Area-weighted sum of the positions of the area that faces each way.
    at: [V3; 6],
    /// The box of the points that face each way.
    lo: [V3; 6],
    hi: [V3; 6],
    /// `--mode full`: the gather patch the area facing each way came from
    /// (the first one written; `u32::MAX` none), and that area's albedo sum.
    patch: [u32; 6],
    albedo: [V3; 6],
    lamp_area: f32,
    lamp_light: V3,
    /// Area-weighted sum of the lamp points' positions.
    lamp_at: V3,
}

impl Default for Voxel {
    fn default() -> Self {
        Self {
            area: [0.0; 6],
            light: [[0.0; 3]; 6],
            at: [[0.0; 3]; 6],
            lo: [[f32::INFINITY; 3]; 6],
            hi: [[f32::NEG_INFINITY; 3]; 6],
            patch: [NO_PATCH; 6],
            albedo: [[0.0; 3]; 6],
            lamp_area: 0.0,
            lamp_light: [0.0; 3],
            lamp_at: [0.0; 3],
        }
    }
}

/// The cone tracer's scene: a sparse voxel hierarchy of surface points.
struct Voxels {
    cells: HashMap<(u8, [i32; 3]), Voxel>,
    lookups: AtomicU64,
    /// Shadow rays cast to light the surface points (once, not per frame, while
    /// nothing moves).
    shadow_rays: u64,
    points: usize,
    /// `--descend`: the cells at each level that hold a surface or are next
    /// to one (dilated by one cell), built on first use.
    occupancy: std::sync::OnceLock<HashSet<(u8, [i32; 3])>>,
}

fn voxel_size(level: u8) -> f32 {
    BASE_SPACING * 2f32.powi(i32::from(level))
}

impl Voxels {
    fn new() -> Self {
        Self { cells: HashMap::new(), lookups: AtomicU64::new(0), shadow_rays: 0, points: 0, occupancy: std::sync::OnceLock::new() }
    }

    /// The dilated occupancy: every cell with a surface, and its 26 neighbours.
    fn occupancy(&self) -> &HashSet<(u8, [i32; 3])> {
        self.occupancy.get_or_init(|| {
            let mut set = HashSet::new();
            for ((level, cell), voxel) in &self.cells {
                if voxel.area.iter().all(|a| *a <= 0.0) {
                    continue;
                }
                for dz in -1..=1 {
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            set.insert((*level, [cell[0] + dx, cell[1] + dy, cell[2] + dz]));
                        }
                    }
                }
            }
            set
        })
    }

    /// Whether the piece of the axis from `a` to `b` at `level` can meet a
    /// surface: its start, middle or end is in a dilated occupied cell.
    fn occupied(&self, level: u8, start: V3, dir: V3, a: f32, b: f32) -> bool {
        let size = voxel_size(level);
        let set = self.occupancy();
        [a, f32::midpoint(a, b), b].iter().any(|t| set.contains(&(level, nearest_cell(add(start, scale(dir, *t)), size))))
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
                if facing > 0.0 {
                    #[expect(clippy::needless_range_loop, reason = "the index reads parallel arrays")]
                    for a in 0..3 {
                        voxel.lo[d][a] = voxel.lo[d][a].min(point[a]);
                        voxel.hi[d][a] = voxel.hi[d][a].max(point[a]);
                    }
                }
            }
        }
    }

    /// Record, in the voxels a crossing reads (12.5 cm and finer), the
    /// gather patch each point's area belongs to and its albedo.
    fn assign_patches(&mut self, points: &[Point], patch: &[u32]) {
        for ((at, normal, area, albedo), index) in points.iter().zip(patch) {
            for level in VOXEL_FINEST..=GATHER_LEVEL {
                let size = voxel_size(level);
                let inside = sub(*at, scale(*normal, 0.25 * size.min(0.25)));
                let Some(voxel) = self.cells.get_mut(&(level, nearest_cell(inside, size))) else { continue };
                for (d, axis) in AXES.iter().enumerate() {
                    let facing = dot(*normal, *axis).max(0.0) * area;
                    if facing <= 0.0 {
                        continue;
                    }
                    if voxel.patch[d] == NO_PATCH {
                        voxel.patch[d] = *index;
                    }
                    voxel.albedo[d] = add(voxel.albedo[d], scale(*albedo, facing));
                }
            }
        }
    }

    /// Replace every surface's sent light: `radiance[i]` for `points[i]`. The
    /// areas, centroids, boxes and lamps stay.
    fn relight(&mut self, points: &[Point], radiance: &[V3]) {
        for voxel in self.cells.values_mut() {
            voxel.light = [[0.0; 3]; 6];
        }
        for ((at, normal, area, _), light) in points.iter().zip(radiance) {
            if luma(*light) <= 0.0 {
                continue;
            }
            for level in VOXEL_FINEST..=VOXEL_COARSEST {
                let size = voxel_size(level);
                let inside = sub(*at, scale(*normal, 0.25 * size.min(0.25)));
                let Some(voxel) = self.cells.get_mut(&(level, nearest_cell(inside, size))) else { continue };
                for (d, axis) in AXES.iter().enumerate() {
                    let facing = dot(*normal, *axis).max(0.0) * area;
                    voxel.light[d] = add(voxel.light[d], scale(*light, facing));
                }
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

    /// The surfaces whose plane the cone's axis crosses between `from` and
    /// `to` at `level`, nearest first: for each, the cover (the share of the
    /// voxel that holds the crossing point that the plane fills) and its
    /// radiance. At a fine level a voxel holds about one plane for each way it
    /// faces, so its centroid places that plane.
    fn crossed(&self, level: u8, from: V3, to: V3, dir: V3, (full, margin): (f32, f32), out: &mut Vec<Met>) {
        out.clear();
        let size = voxel_size(level);
        let mut cells: Vec<[i32; 3]> = Vec::with_capacity(16);
        for p in [from, to] {
            let base = p.map(|c| floor_i32(c / size - 0.5));
            for corner in 0..8i32 {
                let cell = [base[0] + (corner & 1), base[1] + ((corner >> 1) & 1), base[2] + ((corner >> 2) & 1)];
                if !cells.contains(&cell) {
                    cells.push(cell);
                }
            }
        }
        self.lookups.fetch_add(u64::try_from(cells.len()).unwrap_or(u64::MAX), Ordering::Relaxed);
        for cell in cells {
            let Some(voxel) = self.cells.get(&(level, cell)) else { continue };
            for (d, axis) in AXES.iter().enumerate() {
                let toward = (-dot(dir, *axis)).max(0.0);
                if toward <= 0.05 || voxel.area[d] <= 0.0 {
                    continue;
                }
                let centre = scale(voxel.at[d], 1.0 / voxel.area[d]);
                let a = dot(sub(from, centre), *axis);
                let b = dot(sub(to, centre), *axis);
                if !(a > 0.0 && b <= 0.0) {
                    continue;
                }
                let k = a / (a - b);
                let hit = add(from, scale(sub(to, from), k));
                // Only the voxel whose extent across the plane holds the
                // crossing point: there the plane is, or is not.
                let skip = d / 2;
                if (0..3).filter(|x| *x != skip).any(|x| floor_i32(hit[x] / size) != cell[x]) {
                    continue;
                }
                // Across the plane, only where the surface's points are: a
                // curved or narrow surface does not fill its voxel's plane.
                if (0..3).filter(|x| *x != skip).any(|x| hit[x] < voxel.lo[d][x] - margin || hit[x] > voxel.hi[d][x] + margin) {
                    continue;
                }
                let depth = dot(sub(hit, from), dir);
                // The surface points' area in a full voxel comes to 0.8 to 1.2
                // of it: a voxel filled to `full` or more is a whole plane,
                // one less filled (an edge, a thin rail) the part it fills.
                let fill = voxel.area[d] / (size * size);
                out.push(Met {
                    depth,
                    cover: if fill >= full { 1.0 } else { fill },
                    radiance: scale(voxel.light[d], 1.0 / voxel.area[d]),
                    patch: voxel.patch[d],
                    albedo: scale(voxel.albedo[d], 1.0 / voxel.area[d]),
                });
            }
        }
        out.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    }

    /// The lamp voxels whose centroid lies at a depth along the cone between
    /// `from` and `to`, each once: its cover is its disc area over the
    /// cross-section, times the tent of its voxel at the point on the axis
    /// level with its centroid. Steps tile the axis, so each lamp voxel falls
    /// in one step at that step's level; read at a step's centre instead, a
    /// lamp at a level change lost the weight between the two levels.
    fn lamps_between(&self, level: u8, start: V3, dir: V3, from: f32, to: f32, out: &mut Vec<Met>) {
        out.clear();
        let size = voxel_size(level);
        let mut cells: Vec<[i32; 3]> = Vec::with_capacity(16);
        for depth in [from, to] {
            let p = add(start, scale(dir, depth));
            let base = p.map(|c| floor_i32(c / size - 0.5));
            for corner in 0..8i32 {
                let cell = [base[0] + (corner & 1), base[1] + ((corner >> 1) & 1), base[2] + ((corner >> 2) & 1)];
                if !cells.contains(&cell) {
                    cells.push(cell);
                }
            }
        }
        for cell in cells {
            let Some(voxel) = self.cells.get(&(level, cell)) else { continue };
            if voxel.lamp_area <= 0.0 {
                continue;
            }
            let centre = scale(voxel.lamp_at, 1.0 / voxel.lamp_area);
            let depth = dot(sub(centre, start), dir);
            if depth <= from || depth > to {
                continue;
            }
            // A tent across the axis only, about the centroid's distance from
            // it: over the plane it adds to one voxel's area, so the lamp's
            // light, summed over the cones that pass it, is its own.
            let off = sub(centre, add(start, scale(dir, depth)));
            let side = normalize(cross(dir, if dir[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] }));
            let up = cross(dir, side);
            let w = (1.0 - dot(off, side).abs() / size).max(0.0) * (1.0 - dot(off, up).abs() / size).max(0.0);
            if w > 0.0 {
                out.push(Met {
                    depth,
                    cover: w * voxel.lamp_area / (size * size),
                    radiance: scale(voxel.lamp_light, 1.0 / voxel.lamp_area),
                    patch: u32::MAX,
                    // A lamp has no albedo: the slot holds its centroid, for
                    // the visibility walk.
                    albedo: centre,
                });
            }
        }
    }

    /// The share of light that passes from `from` to `to`: the planes the
    /// segment crosses at `level`, read as a cone's fine walk reads them.
    fn passing(&self, level: u8, from: V3, to: V3, rules: (f32, f32)) -> f32 {
        let way = sub(to, from);
        let length = dot(way, way).sqrt();
        if length <= 1.0e-6 {
            return 1.0;
        }
        let dir = scale(way, 1.0 / length);
        let step = voxel_size(level);
        let mut found = Vec::new();
        let mut blocked = 0.0f32;
        let mut u = 0.0f32;
        // Stop short of the lamp: its fixture is not a wall between.
        let end = (length - step).max(0.0);
        while u < end && blocked < 1.0 {
            let next = (u + step).min(end);
            self.crossed(level, add(from, scale(dir, u)), add(from, scale(dir, next)), dir, rules, &mut found);
            for m in &found {
                blocked += m.cover;
            }
            u = next;
        }
        (1.0 - blocked).max(0.0)
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

/// A grid index: `v` rounded down.
fn floor_i32(v: f32) -> i32 {
    v.floor() as i32
}

fn nearest_cell(x: V3, spacing: f32) -> [i32; 3] {
    x.map(|c| floor_i32(c / spacing))
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
    /// For each probe, the sum of the points and normals of the pixels it
    /// serves, and their count: where a probe traced with cones sits.
    seat: Vec<(V3, V3, f32)>,
    /// Where each probe sits: the mean of its pixels' points, lifted 0.75 of
    /// the finest voxel along their mean normal. Its cones start here, and the
    /// layers are combined between these points.
    seat_point: Vec<V3>,
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

/// The cone tracer's frame: every probe of every cascade traces one cone for
/// each of its directions, over its own cascade's interval, as wide as that
/// cascade's direction spacing (the paper's Algorithm 1, its probes traced
/// with cones in place of rays). Cascade 0's probes are the voxel layer over
/// the visible surface: each sits on the mean point of the pixels it serves,
/// lifted along their mean normal, and its short interval holds the fine
/// detail. Each coarser cascade has twice the spacing, four times the
/// directions and four times the reach. A direction keeps the light its cone
/// met in the interval and the share that passed; the merge and the shade
/// combine the layers. No cone starts at a pixel.
/// The cone tracer's frame, every cone from the voxel layer: for each probe
/// of each cascade and each of its directions, one cone starts at the seat of
/// one of the cascade-0 voxel probes under it (a different one for each
/// direction and frame) and runs out through every cascade's interval, as wide
/// as each interval's direction spacing. Each interval is deposited into that
/// voxel probe's probe of the same cascade (the paper's ray splitting, section
/// 5, from the voxel layer instead of from pixels). Every interval starts at a
/// surface: a coarse probe's interval, traced from its own seat, began past
/// whatever wall lay within the shorter intervals under it, and the room no
/// lamp lights read through it. Probes times directions, as in
/// [`trace_probes`].
/// A cone from a voxel probe: the probe, its direction, and per interval
/// reached (cascade, light over what entered, share passed).
type VoxelCone = (u32, V3, Vec<(usize, V3, f32)>);

/// `--gpu-bench`: when armed, the bounce gather copies its cones here (start,
/// first reach; direction), every interval of each to be marched.
static GATHER_ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static GATHER_JOBS: std::sync::Mutex<Vec<[f32; 8]>> = std::sync::Mutex::new(Vec::new());

/// `--gpu-bench`: when armed, the voxel-layer tracer copies its cones here
/// (start, first reach; direction) for the GPU to march.
static CONE_JOBS: std::sync::Mutex<Option<Vec<[f32; 8]>>> = std::sync::Mutex::new(None);

fn trace_from_voxels(voxels: &Voxels, o: &Options, cascades: &mut [Cascade], index: &HashMap<Key, u32>, traced: &AtomicU64, scene: &Scene, jitter: (f32, f32)) {
    let n = cascades.len();
    let sky = if o.mode == Mode::Emissive { [0.0; 3] } else { scene.sky.as_ref().map_or([0.0; 3], |s| s.color) };
    // Each voxel probe's probe in every cascade, and each probe's voxels.
    let voxel_keys = cascades[0].keys.clone();
    let mut ancestor: Vec<Vec<u32>> = vec![Vec::with_capacity(voxel_keys.len()); n];
    for key in &voxel_keys {
        let s0 = spacing(key.lod, 0);
        let centre = key.cell.map(|c| (c as f32 + 0.5) * s0);
        for (c, list) in ancestor.iter_mut().enumerate() {
            let up = Key { lod: key.lod, cascade: u8::try_from(c).unwrap_or(u8::MAX), cell: nearest_cell(centre, spacing(key.lod, c)) };
            list.push(index.get(&up).copied().unwrap_or(u32::MAX));
        }
    }
    let mut under: Vec<Vec<Vec<u32>>> = cascades.iter().map(|c| vec![Vec::new(); c.keys.len()]).collect();
    for (c, list) in ancestor.iter().enumerate() {
        for (v, &slot) in list.iter().enumerate() {
            if slot != u32::MAX {
                under[c][slot as usize].push(u32::try_from(v).unwrap_or(u32::MAX));
            }
        }
    }
    // The voxel probes' surface points and normals.
    let surface: Vec<(V3, V3)> = cascades[0].seat.iter().map(|(sum, nsum, count)| (scale(*sum, 1.0 / count.max(1.0)), normalize(*nsum))).collect();
    if o.own_interval && o.near_check == NearCheck::Chain {
        trace_chain(voxels, o, cascades, &ancestor, &under, &surface, &voxel_keys, traced, sky);
        return;
    }
    let frame = u64::from((jitter.0 * 65_536.0) as u32);
    // The cones: (cascade, probe, direction) -> (voxel probe, direction vector).
    let mut jobs: Vec<(u32, V3)> = Vec::new();
    let mut job_cascade: Vec<u8> = Vec::new();
    let mut starts: Vec<(usize, usize, V3)> = Vec::new();
    for (c, cascade) in cascades.iter().enumerate() {
        let theta = cascade.theta;
        for (slot, voxels_under) in under[c].iter().enumerate() {
            if voxels_under.is_empty() {
                continue;
            }
            // The representative voxel probe: the one whose surface point is
            // nearest the probe's own seat, for every direction and frame.
            let seat = cascade.seat_point[slot];
            let nearest = voxels_under
                .iter()
                .copied()
                .min_by(|a, b| {
                    let (pa, pb) = (surface[*a as usize].0, surface[*b as usize].0);
                    dot(sub(pa, seat), sub(pa, seat)).total_cmp(&dot(sub(pb, seat), sub(pb, seat)))
                })
                .unwrap_or(voxels_under[0]);
            starts.push((c, slot, surface[nearest as usize].0));
            for b in 0..cascade.dirs() {
                let v = if o.random_cone_voxel {
                    let pick = hashed(frame ^ ((c as u64) << 56) ^ ((slot as u64) << 20) ^ b as u64);
                    voxels_under[((pick * voxels_under.len() as f32) as usize).min(voxels_under.len() - 1)]
                } else {
                    nearest
                };
                jobs.push((v, bin_dir(b, theta)));
                job_cascade.push(u8::try_from(c).unwrap_or(u8::MAX));
            }
        }
    }
    if let Ok(mut capture) = CONE_JOBS.lock() {
        if let Some(list) = capture.as_mut() {
            for ((v, dir), c) in jobs.iter().zip(&job_cascade) {
                let (point, normal) = surface[*v as usize];
                let start = add(point, scale(normal, 0.75 * voxel_size(VOXEL_FINEST)));
                let t0 = o.t0 * spacing(voxel_keys[*v as usize].lod, 0);
                list.push([start[0], start[1], start[2], t0, dir[0], dir[1], dir[2], f32::from(*c)]);
            }
        }
    }
    // The merge's visibility test aims at the point a probe's cones start
    // from, not at its seat: a coarse probe over both sides of a wall starts
    // on one of them.
    if o.seat_at_start && !o.random_cone_voxel {
        for (c, slot, start) in starts {
            if c > 0 {
                cascades[c].seat_point[slot] = start;
            }
        }
    }
    let tagged: Vec<(u32, V3, u8)> = jobs.iter().zip(&job_cascade).map(|((v, dir), c)| (*v, *dir, *c)).collect();
    let chunk = tagged.len().div_ceil(o.threads).max(1);
    let deposits: Vec<VoxelCone> = std::thread::scope(|scope| {
        let handles: Vec<_> = tagged
            .chunks(chunk)
            .map(|part| {
                let surface = &surface;
                let voxel_keys = &voxel_keys;
                scope.spawn(move || {
                    part.iter()
                        .map(|(v, dir, c)| {
                            let (point, normal) = surface[*v as usize];
                            let t0 = o.t0 * spacing(voxel_keys[*v as usize].lod, 0);
                            let intervals = if o.own_interval {
                                march_own(voxels, o, point, normal, *dir, t0, sky, usize::from(*c))
                            } else {
                                march_cone(voxels, o, point, normal, *dir, t0, sky)
                            };
                            (*v, *dir, intervals)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    traced.fetch_add(u64::try_from(jobs.len()).unwrap_or(u64::MAX), Ordering::Relaxed);
    for (v, dir, intervals) in deposits {
        for (c, radiance, through) in intervals {
            let slot = ancestor[c][v as usize];
            if slot == u32::MAX {
                continue;
            }
            let cascade = &mut cascades[c];
            let at = slot as usize * cascade.dirs() + dir_bin(dir, cascade.theta);
            cascade.count[at] += 1;
            cascade.radiance[at] = add(cascade.radiance[at], radiance);
            cascade.moment[at] = add(cascade.moment[at], scale(dir, luma(radiance)));
            cascade.through[at] += through;
        }
    }
}

/// `--near-check chain`: the voxel-layer cascade with each cone over its own
/// interval, traced finest first. Every probe starts on the start voxel of
/// one of its finer children (the one nearest its seat), so the stretch
/// before a cone's interval has already been marched by the finer probes on
/// that chain: a cone they closed is not traced, and its bin stays empty.
#[expect(clippy::too_many_arguments, reason = "the voxel-layer tracer's tables")]
fn trace_chain(
    voxels: &Voxels,
    o: &Options,
    cascades: &mut [Cascade],
    ancestor: &[Vec<u32>],
    under: &[Vec<Vec<u32>>],
    surface: &[(V3, V3)],
    voxel_keys: &[Key],
    traced: &AtomicU64,
    sky: V3,
) {
    let n = cascades.len();
    // start[c][slot]: the voxel probe a probe's cones start from; child[c][slot]:
    // the probe of cascade c - 1 on its chain.
    let mut start: Vec<Vec<u32>> = vec![under[0].iter().map(|u| u.first().copied().unwrap_or(u32::MAX)).collect()];
    let mut child: Vec<Vec<u32>> = vec![vec![u32::MAX; cascades[0].keys.len()]];
    for c in 1..n {
        let mut best: Vec<(f32, u32)> = vec![(f32::INFINITY, u32::MAX); cascades[c].keys.len()];
        for (t, &v) in start[c - 1].iter().enumerate() {
            let Some(&parent) = ancestor[c].get(v as usize) else { continue };
            if parent == u32::MAX {
                continue;
            }
            let seat = cascades[c].seat_point[parent as usize];
            let d = sub(surface[v as usize].0, seat);
            let b = &mut best[parent as usize];
            if dot(d, d) < b.0 {
                *b = (dot(d, d), u32::try_from(t).unwrap_or(u32::MAX));
            }
        }
        let next: Vec<u32> = best.iter().map(|(_, t)| start[c - 1].get(*t as usize).copied().unwrap_or(u32::MAX)).collect();
        start.push(next);
        child.push(best.iter().map(|(_, t)| *t).collect());
    }
    let mut skipped = 0u64;
    for c in 0..n {
        let theta = cascades[c].theta;
        let mut jobs: Vec<(u32, V3)> = Vec::new();
        for (slot, &v) in start[c].iter().enumerate() {
            if v == u32::MAX {
                continue;
            }
            for b in 0..cascades[c].dirs() {
                let dir = bin_dir(b, theta);
                let mut open = 1.0f32;
                let (mut t, mut k) = (child[c][slot], c);
                while k > 0 && t != u32::MAX && open >= 0.02 {
                    k -= 1;
                    let fine = &cascades[k];
                    let at = t as usize * fine.dirs() + dir_bin(dir, fine.theta);
                    open *= if fine.count[at] > 0 { fine.through[at] / fine.count[at] as f32 } else { 0.0 };
                    t = child[k][t as usize];
                }
                if open < 0.02 {
                    skipped += 1;
                    continue;
                }
                jobs.push((v, dir));
            }
        }
        if let Ok(mut capture) = CONE_JOBS.lock() {
            if let Some(list) = capture.as_mut() {
                for (v, dir) in &jobs {
                    let (point, normal) = surface[*v as usize];
                    let at = add(point, scale(normal, 0.75 * voxel_size(VOXEL_FINEST)));
                    let t0 = o.t0 * spacing(voxel_keys[*v as usize].lod, 0);
                    list.push([at[0], at[1], at[2], t0, dir[0], dir[1], dir[2], f32::from(u8::try_from(c).unwrap_or(u8::MAX))]);
                }
            }
        }
        let chunk = jobs.len().div_ceil(o.threads).max(1);
        let deposits: Vec<VoxelCone> = std::thread::scope(|scope| {
            let handles: Vec<_> = jobs
                .chunks(chunk)
                .map(|part| {
                    scope.spawn(move || {
                        part.iter()
                            .map(|(v, dir)| {
                                let (point, normal) = surface[*v as usize];
                                let t0 = o.t0 * spacing(voxel_keys[*v as usize].lod, 0);
                                (*v, *dir, march_own(voxels, o, point, normal, *dir, t0, sky, c))
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        traced.fetch_add(u64::try_from(jobs.len()).unwrap_or(u64::MAX), Ordering::Relaxed);
        if let Ok(mut capture) = CONE_JOBS.lock() {
            if capture.is_some() {
                if let Ok(mut covers) = CHAIN_COVERS.lock() {
                    covers.extend(deposits.iter().map(|(_, _, i)| if c + 1 < n { i.first().map_or(f32::NAN, |x| 1.0 - x.2) } else { f32::NAN }));
                }
            }
            let _ = capture.as_mut();
        }
        if c + 1 < n {
            if let Ok(mut sum) = CHAIN_COVER.lock() {
                for (_, _, intervals) in &deposits {
                    for (_, _, through) in intervals {
                        sum.0 += f64::from(1.0 - through);
                        sum.1 += 1;
                    }
                }
            }
        }
        for (v, dir, intervals) in deposits {
            for (k, radiance, through) in intervals {
                let slot = ancestor[k][v as usize];
                if slot == u32::MAX {
                    continue;
                }
                let cascade = &mut cascades[k];
                let at = slot as usize * cascade.dirs() + dir_bin(dir, cascade.theta);
                cascade.count[at] += 1;
                cascade.radiance[at] = add(cascade.radiance[at], radiance);
                cascade.moment[at] = add(cascade.moment[at], scale(dir, luma(radiance)));
                cascade.through[at] += through;
            }
        }
    }
    CHAIN_SKIPPED.store(skipped, Ordering::Relaxed);
}

/// Cones `--near-check chain` left out in the last frame, closed before
/// their interval.
static CHAIN_SKIPPED: AtomicU64 = AtomicU64::new(0);

/// `--near-check chain`: the sum of the cones' cover over their own interval,
/// and their count, all cascades but the last (for `--gpu-bench` to compare).
static CHAIN_COVER: std::sync::Mutex<(f64, u64)> = std::sync::Mutex::new((0.0, 0));

/// `--gpu-bench`: each captured cone's CPU cover, in the capture's order (NaN
/// for the last cascade, whose share passed is not kept).
static CHAIN_COVERS: std::sync::Mutex<Vec<f32>> = std::sync::Mutex::new(Vec::new());

fn trace_probes(voxels: &Voxels, o: &Options, cascades: &mut [Cascade], traced: &AtomicU64, scene: &Scene, jitter: (f32, f32)) {
    let n = cascades.len();
    let _ = jitter;
    let finest = voxel_size(VOXEL_FINEST);
    let sky = if o.mode == Mode::Emissive { [0.0; 3] } else { scene.sky.as_ref().map_or([0.0; 3], |s| s.color) };
    let width0 = o.cone_width * (4.0 * PI / (2 * o.theta * o.theta) as f32).sqrt();
    for (c, cascade) in cascades.iter_mut().enumerate() {
        let dirs = cascade.dirs();
        let theta = cascade.theta;
        let directions: Vec<V3> = (0..dirs).map(|b| bin_dir(b, theta)).collect();
        let width = width0 / (1u32 << c) as f32;
        let probes = cascade.keys.len();
        let chunk = probes.div_ceil(o.threads).max(1);
        let keys = &cascade.keys;
        let seats = &cascade.seat_point;
        let directions = &directions;
        let results: Vec<Vec<(V3, f32)>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..probes)
                .step_by(chunk)
                .map(|first| {
                    scope.spawn(move || {
                        let mut out = Vec::new();
                        for slot in first..(first + chunk).min(probes) {
                            let start = seats[slot];
                            let t0 = o.t0 * spacing(keys[slot].lod, 0);
                            let from = if c == 0 { 0.25 * finest } else { t0 * 4f32.powi(i32::try_from(c).unwrap_or(0) - 1) };
                            let end = if c + 1 == n { REACH } else { t0 * 4f32.powi(i32::try_from(c).unwrap_or(0)) };
                            let mut row = Vec::with_capacity(directions.len());
                            for dir in directions {
                                let mut span = Span { done: from, covered: 0.0, met: [0.0; 3], fine: o.fine_cross };
                                march_span(voxels, o, start, *dir, width, end, &mut span, None);
                                let mut met = span.met;
                                let mut through = 1.0 - span.covered;
                                if c + 1 == n {
                                    met = add(met, scale(sky, through));
                                    through = 0.0;
                                }
                                row.push((met, through));
                            }
                            out.push(row);
                        }
                        out
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        traced.fetch_add((probes * dirs) as u64, Ordering::Relaxed);
        for (slot, row) in results.into_iter().enumerate() {
            for (b, (met, through)) in row.into_iter().enumerate() {
                let at = slot * dirs + b;
                cascade.radiance[at] = met;
                cascade.moment[at] = scale(directions[b], luma(met));
                cascade.through[at] = through;
                cascade.count[at] = 1;
            }
        }
    }
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
            seat: Vec::new(),
            seat_point: Vec::new(),
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
                cascade.seat.push(([0.0; 3], [0.0; 3], 0.0));
                u32::try_from(cascade.keys.len() - 1).unwrap_or(u32::MAX)
            });
            home[p * n + c] = slot;
            let seat = &mut cascade.seat[slot as usize];
            seat.0 = add(seat.0, pixel.point);
            seat.1 = add(seat.1, pixel.normal);
            seat.2 += 1.0;
        }
    }
    for cascade in &mut cascades {
        let finest = voxel_size(VOXEL_FINEST);
        cascade.seat_point = cascade
            .seat
            .iter()
            .map(|(sum, nsum, count)| {
                // A probe over floor and wall and ceiling has normals that
                // cancel: it is not lifted then.
                let held = dot(*nsum, *nsum).sqrt() / count.max(1.0);
                let lift = if held > 0.3 { scale(normalize(*nsum), 0.75 * finest) } else { [0.0; 3] };
                add(scale(*sum, 1.0 / count.max(1.0)), lift)
            })
            .collect();
        let len = cascade.keys.len() * cascade.dirs();
        cascade.radiance = vec![[0.0; 3]; len];
        cascade.moment = vec![[0.0; 3]; len];
        cascade.through = vec![0.0; len];
        cascade.count = vec![0; len];
        cascade.cone = vec![[0.0; 3]; len];
        cascade.cone_moment = vec![[0.0; 3]; len];
        cascade.filled = vec![false; len];
    }
    let cones_traced = AtomicU64::new(0);
    if let Some(voxels) = setup.voxels {
        if o.cones_from_voxels {
            trace_from_voxels(voxels, o, &mut cascades, &index, &cones_traced, setup.scene, jitter);
        } else {
            trace_probes(voxels, o, &mut cascades, &cones_traced, setup.scene, jitter);
        }
    }
    // The ray tracer: rays from the pixels, split into the cascades
    // (the paper's section 5). The cone tracer traced its probes above.
    let counted = Counted { inner: setup.surfaces, rays: AtomicU64::new(0) };
    let shadow_rays = AtomicU64::new(0);
    if setup.voxels.is_none() {
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
                    #[expect(clippy::needless_range_loop, reason = "the index reads parallel arrays")]
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
            // The coarser probes this one reads, once for all its
            // directions. Traced with cones, with --combine-visibility, it
            // takes only the probes this probe's seat sees.
            let seen = setup.voxels.filter(|_| o.see_merge).map(|v| (v, o, cascade.seat_point[slot]));
            let weights = above.map_or_else(Vec::new, |above| probe_weights(above, &index, key.lod, c + 1, centre, seen));
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
                        if let Some((v, m)) = read_cone(above, &weights, child) {
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
    // The sun in the voxel layer (phase 2b, option B): each voxel-layer probe
    // casts one narrow cone toward the sun from its seat; a pixel takes the
    // sun times its cosine times the probes' share that sees it.
    let sun_seen: Vec<f32> = match (sun(), setup.voxels, o.sun_in_voxels) {
        (Some(sun), Some(voxels), true) => {
            let finest = voxel_size(VOXEL_FINEST);
            cascades[0]
                .seat_point
                .iter()
                .map(|seat| {
                    let mut span = Span { done: 0.25 * finest, covered: 0.0, met: [0.0; 3], fine: o.fine_cross };
                    march_span(voxels, o, *seat, sun.toward, 0.01, REACH, &mut span, None);
                    (1.0 - span.covered).max(0.0)
                })
                .collect()
        }
        _ => Vec::new(),
    };
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
        // The voxel layer's probes around the pixel; traced with cones, only
        // those the pixel sees.
        let seen = setup.voxels.filter(|_| o.see_shade).map(|v| (v, o, add(pixel.point, scale(pixel.normal, LIFT))));
        let weights = probe_weights(first, &index, pixel.lod, 0, pixel.point, seen);
        for (bin, dir) in bin_dirs.iter().enumerate() {
            let cosine = dot(*dir, pixel.normal);
            if cosine <= 0.0 {
                continue;
            }
            wanted += f64::from(cosine);
            match read_cone(first, &weights, bin) {
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
        if let (Some(sun), false) = (sun(), sun_seen.is_empty()) {
            let cosine = dot(sun.toward, pixel.normal);
            let total: f32 = weights.iter().map(|w| w.1).sum();
            if cosine > 0.0 && total > 0.0 {
                let seen_share = weights.iter().map(|(slot, w)| sun_seen[*slot] * w).sum::<f32>() / total;
                irradiance[p] = add(irradiance[p], scale(sun.irradiance, cosine * seen_share));
            }
        }
    }
    Frame {
        irradiance,
        rays: counted.rays.load(Ordering::Relaxed) + cones_traced.load(Ordering::Relaxed),
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
static LAST_E: std::sync::Mutex<Vec<V3>> = std::sync::Mutex::new(Vec::new());
static LAST_D: std::sync::Mutex<Vec<V3>> = std::sync::Mutex::new(Vec::new());


/// `--sun on` in `--mode full`: the way to the sun and its irradiance on a
/// surface facing it (the engine's lamp unit at the prototype's fixed 7 m).
#[derive(Clone, Copy)]
struct SunLight {
    toward: V3,
    irradiance: V3,
}

static SUN: std::sync::OnceLock<Option<SunLight>> = std::sync::OnceLock::new();

fn sun() -> Option<SunLight> {
    SUN.get().copied().flatten()
}

/// The sun in the sky (phase 2b, option A): a cone that leaves the field in
/// the last cascade's direction that holds the sun takes the sun's irradiance
/// spread over that direction's solid angle, so the directions together hold
/// it once.
fn sun_in_sky(o: &Options, dir: V3) -> V3 {
    let Some(sun) = sun() else { return [0.0; 3] };
    let theta = o.theta << (o.cascades - 1);
    if dir_bin(dir, theta) != dir_bin(sun.toward, theta) {
        return [0.0; 3];
    }
    let solid = 4.0 * PI / (2 * theta * theta) as f32;
    scale(sun.irradiance, 1.0 / solid)
}

/// The exact sun at a point, for the reference: its irradiance times the
/// cosine, if a ray toward it is clear.
fn sun_exact(walls: &dyn Surfaces, point: V3, normal: V3) -> V3 {
    let Some(sun) = sun() else { return [0.0; 3] };
    let cosine = dot(sun.toward, normal);
    if cosine <= 0.0 {
        return [0.0; 3];
    }
    let origin = add(point, scale(normal, LIFT));
    if walls.blocked(origin, add(origin, scale(sun.toward, REACH))) {
        return [0.0; 3];
    }
    scale(sun.irradiance, cosine)
}

/// A surface bucket no gather patch was assigned to.
const NO_PATCH: u32 = u32::MAX - 1;

/// One surface or lamp a fine step meets.
#[derive(Clone, Copy)]
struct Met {
    depth: f32,
    cover: f32,
    radiance: V3,
    /// The gather patch a surface belongs to; `u32::MAX` for a lamp.
    patch: u32,
    albedo: V3,
}

/// What passes to a lamp the cone's read found: the last stretch of the line
/// from the cone's start to the lamp, two of this step's lengths, walked as a
/// fine step reads planes. The march already holds what lies along its axis;
/// this finds a wall beside the axis between the cone and a lamp it read.
fn lamp_passing(voxels: &Voxels, o: &Options, start: V3, lamp: V3, step: f32) -> f32 {
    let to_lamp = sub(lamp, start);
    let distance = dot(to_lamp, to_lamp).sqrt();
    if distance <= 1.0e-6 {
        return 1.0;
    }
    let from = sub(lamp, scale(to_lamp, (2.0 * step).min(distance) / distance));
    voxels.passing(o.refine_level, from, lamp, (o.cross_full, o.box_margin))
}

/// What a gather cone met, split for `--mode full`'s transfer: the light that
/// is not a surface's (lamps and sky), and for each surface patch met, the
/// share of the cone it filled times its albedo over pi.
#[derive(Default)]
struct Sink {
    direct: V3,
    records: Vec<(u32, V3)>,
    /// Share of the cones' cover from surfaces with no patch.
    unassigned: f32,
    /// The sky's part of `direct`.
    sky: V3,
}

fn march_cone(voxels: &Voxels, o: &Options, point: V3, normal: V3, dir: V3, t0: f32, sky: V3) -> Vec<(usize, V3, f32)> {
    march_cone_into(voxels, o, point, normal, dir, t0, sky, None)
}

/// Where a march along one cone has got to: the depth read up to, the share
/// of the cone covered, and the light met so far.
struct Span {
    done: f32,
    covered: f32,
    met: V3,
    /// Fine levels take the plane-crossing test, and a covered coarse step is
    /// walked again at the refine level; off, the filtered read is taken.
    fine: bool,
}

impl Span {
    /// Take what a step met, nearest first, until the cone is covered.
    fn take(&mut self, found: &[Met], multiply: bool, mut sink: Option<&mut Sink>) {
        for m in found {
            let seen = if multiply { m.cover * (1.0 - self.covered) } else { m.cover.min(1.0 - self.covered) };
            if seen <= 0.0 {
                break;
            }
            // A lamp read wider than the voxel (cover over 1) sends all of it;
            // what it blocks stops at the open share.
            let lit = if m.patch == u32::MAX { seen * m.cover.max(1.0) } else { seen };
            self.met = add(self.met, scale(m.radiance, lit));
            if let Some(sink) = sink.as_deref_mut() {
                if m.patch == u32::MAX {
                    sink.direct = add(sink.direct, scale(m.radiance, lit));
                } else if m.patch == NO_PATCH {
                    sink.unassigned += seen;
                } else {
                    sink.records.push((m.patch, scale(m.albedo, seen / PI)));
                }
            }
            self.covered += seen;
        }
    }
}

/// March one cone from `start` along `dir`, `width` radians wide, from
/// `span.done` to `end`. Steps tile the axis: each covers from where the last
/// ended, so a level that changes between steps leaves no stretch unread. At
/// and under the refine level each step counts the planes the axis crosses,
/// each once; a coarser step that meets cover is walked again so. Lamps are
/// read at the step's own level, each voxel once at its centroid's depth.
#[expect(clippy::too_many_arguments, reason = "the march's inputs, plus the transfer sink")]
fn march_span(voxels: &Voxels, o: &Options, start: V3, dir: V3, width: f32, end: f32, span: &mut Span, mut sink: Option<&mut Sink>) {
    let finest = voxel_size(VOXEL_FINEST);
    let rules = (o.cross_full, o.box_margin);
    let mut found = Vec::new();
    let mut lamps = Vec::new();
    while span.done < end && span.covered < 0.98 {
        let wide = (width * span.done).max(finest);
        let level = ((wide / BASE_SPACING).log2().ceil() as i32)
            .clamp(i32::from(VOXEL_FINEST), i32::from(o.max_level)) as u8;
        let step = voxel_size(level);
        let (from, to) = (span.done, span.done + step);
        let front = o.front_order.then_some(start);
        let lamps_here = |lamps: &mut Vec<Met>| {
            voxels.lamps_between(level, start, dir, from, to, lamps);
            for lamp in lamps.iter_mut() {
                // A wide cone's read can take a lamp beside a wall: only what
                // passes from the cone to the lamp counts.
                lamp.cover *= lamp_passing(voxels, o, start, lamp.albedo, step);
            }
            lamps.retain(|l| l.cover > 0.0);
        };
        if span.fine && level <= o.refine_level {
            voxels.crossed(level, add(start, scale(dir, from)), add(start, scale(dir, to)), dir, rules, &mut found);
            lamps_here(&mut lamps);
            found.extend(lamps.iter().map(|l| Met { depth: l.depth - from, ..*l }));
            found.sort_by(|a, b| a.depth.total_cmp(&b.depth));
            span.take(&found, o.multiply_cover, sink.as_deref_mut());
            span.done = to;
            continue;
        }
        let (cover, light) = voxels.sample(level, add(start, scale(dir, from + 0.5 * step)), dir, front);
        if cover > 0.0 && span.fine {
            // Something is here: walk this step again at the refine level,
            // whose read cannot reach the far side of a wall.
            let fine = voxel_size(o.refine_level);
            lamps_here(&mut lamps);
            let mut u = from;
            if o.descend {
                // Front to back, the coarsest piece first: a piece no surface
                // is in or next to is passed whole (its lamps still taken);
                // else one level finer, down to the refine level's crossing.
                while u < to && span.covered < 0.98 {
                    let mut l = level - 1;
                    loop {
                        let next = (u + voxel_size(l)).min(to);
                        let refine = l <= o.refine_level;
                        if refine || !voxels.occupied(l, start, dir, u, next) {
                            if refine {
                                voxels.crossed(o.refine_level, add(start, scale(dir, u)), add(start, scale(dir, next)), dir, rules, &mut found);
                            } else {
                                found.clear();
                            }
                            found.extend(lamps.iter().filter(|m| m.depth > u && m.depth <= next).map(|m| Met { depth: m.depth - u, ..*m }));
                            found.sort_by(|x, y| x.depth.total_cmp(&y.depth));
                            span.take(&found, o.multiply_cover, sink.as_deref_mut());
                            u = next;
                            break;
                        }
                        l -= 1;
                    }
                }
            }
            while u < to && span.covered < 0.98 {
                let next = (u + fine).min(to);
                voxels.crossed(o.refine_level, add(start, scale(dir, u)), add(start, scale(dir, next)), dir, rules, &mut found);
                found.extend(lamps.iter().filter(|l| l.depth > u && l.depth <= next).map(|l| Met { depth: l.depth - u, ..*l }));
                found.sort_by(|a, b| a.depth.total_cmp(&b.depth));
                span.take(&found, o.multiply_cover, sink.as_deref_mut());
                u = next;
            }
        } else if cover > 0.0 {
            let seen = cover.min(1.0 - span.covered);
            span.met = add(span.met, scale(light, seen / cover));
            span.covered += seen;
        }
        span.done = to;
    }
}

/// `--near-check`: how an own-interval cone tests the stretch before its
/// interval.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NearCheck {
    Off,
    /// The filtered read only.
    Filtered,
    /// With the crossing test and the refine walk.
    Fine,
    /// No march: each probe starts on one of its finer children's start
    /// voxels, and the share open is the product of what the finer probes on
    /// that chain passed in the bins that hold the direction.
    Chain,
}

/// `--own-interval`: a cascade-`c` cone from a voxel-layer point over its own
/// cascade's interval only, as a sparse cascade traces; the merge brings the
/// rest. Returns that one interval's light and the share that passed.
#[expect(clippy::too_many_arguments, reason = "the march's inputs and the cascade")]
fn march_own(voxels: &Voxels, o: &Options, point: V3, normal: V3, dir: V3, t0: f32, sky: V3, c: usize) -> Vec<(usize, V3, f32)> {
    let n = o.cascades;
    let finest = voxel_size(VOXEL_FINEST);
    let width0 = o.cone_width * (4.0 * PI / (2 * o.theta * o.theta) as f32).sqrt();
    let start = add(point, scale(normal, 0.75 * finest));
    let power = |k: usize| 4f32.powi(i32::try_from(k).unwrap_or(0));
    let from = if c == 0 { 0.25 * finest } else { t0 * power(c - 1) };
    let end = if c + 1 == n { REACH } else { t0 * power(c) };
    let width = width0 / (1u32 << c) as f32;
    if c > 0 && matches!(o.near_check, NearCheck::Filtered | NearCheck::Fine) {
        // The stretch before this interval, from the same start: closed, the
        // cone gives this probe no value in this direction, as the march over
        // every interval did.
        let mut near = Span { done: 0.25 * finest, covered: 0.0, met: [0.0; 3], fine: o.near_check == NearCheck::Fine };
        march_span(voxels, o, start, dir, width, from, &mut near, None);
        if near.covered >= 0.98 {
            return Vec::new();
        }
    }
    let mut span = Span { done: from, covered: 0.0, met: [0.0; 3], fine: o.fine_cross && c < o.refine_cascades };
    march_span(voxels, o, start, dir, width, end, &mut span, None);
    let mut met = span.met;
    let mut through = 1.0 - span.covered;
    if c + 1 == n {
        met = add(met, scale(sky, through));
        if !o.sun_in_voxels {
            met = add(met, scale(sun_in_sky(o, dir), through));
        }
        through = 0.0;
    }
    vec![(c, met, through)]
}

/// A whole cone from a surface point, through every cascade's interval in
/// turn (the gather's cones): for each interval reached, the light met in it
/// over the light that entered it, and the share that passed.
#[expect(clippy::too_many_arguments, reason = "the march's inputs, plus the transfer sink")]
fn march_cone_into(voxels: &Voxels, o: &Options, point: V3, normal: V3, dir: V3, t0: f32, sky: V3, mut sink: Option<&mut Sink>) -> Vec<(usize, V3, f32)> {
    let n = o.cascades;
    let finest = voxel_size(VOXEL_FINEST);
    let width0 = o.cone_width * (4.0 * PI / (2 * o.theta * o.theta) as f32).sqrt();
    let start = add(point, scale(normal, 0.75 * finest));
    let mut span = Span { done: 0.25 * finest, covered: 0.0, met: [0.0; 3], fine: o.fine_cross };
    let mut reach = t0;
    let mut out = Vec::with_capacity(n);
    for c in 0..n {
        let end = if c + 1 == n { REACH } else { reach };
        let open = 1.0 - span.covered;
        span.met = [0.0; 3];
        // The gather records its transfer only in the crossing test and the
        // walk, so its cones keep them; the cascade's own cones follow
        // --refine-cascades.
        span.fine = o.fine_cross && (sink.is_some() || c < o.refine_cascades);
        // A gather cone stands for its share of the hemisphere at every
        // distance with --gather-width; else it narrows as the cascade's do.
        let width = if sink.is_some() && o.gather_width > 0.0 { o.gather_width } else { width0 / (1u32 << c) as f32 };
        march_span(voxels, o, start, dir, width, end, &mut span, sink.as_deref_mut());
        let mut through = (1.0 - span.covered) / open;
        if c + 1 == n {
            span.met = add(span.met, scale(sky, 1.0 - span.covered));
            if sink.is_none() && !o.sun_in_voxels {
                span.met = add(span.met, scale(sun_in_sky(o, dir), 1.0 - span.covered));
            }
            if let Some(sink) = sink.as_deref_mut() {
                sink.direct = add(sink.direct, scale(sky, 1.0 - span.covered));
                sink.sky = add(sink.sky, scale(sky, 1.0 - span.covered));
            }
            through = 0.0;
        }
        out.push((c, scale(span.met, 1.0 / open), through));
        if span.covered >= 0.98 {
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

/// The probes of `cascade` a read at `at` takes, with their trilinear
/// weights: the probes that exist among the eight around it. With `seen`, each
/// weight is also scaled by the share of light that passes from the reader to
/// the probe's seat through the voxel field: a probe on the far side of a wall
/// does not lend its light. The weights serve every direction of the read.
fn probe_weights(cascade: &Cascade, index: &HashMap<Key, u32>, lod: i8, level: usize, at: V3, seen: Option<(&Voxels, &Options, V3)>) -> Vec<(usize, f32)> {
    let s = spacing(lod, level);
    let g = [at[0] / s - 0.5, at[1] / s - 0.5, at[2] / s - 0.5];
    let base = g.map(f32::floor);
    let f = [g[0] - base[0], g[1] - base[1], g[2] - base[2]];
    let mut out = Vec::with_capacity(8);
    for corner in 0..8i32 {
        let o = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
        let mut w = (0..3).map(|a| if o[a] == 1 { f[a] } else { 1.0 - f[a] }).product::<f32>();
        if w <= 0.0 {
            continue;
        }
        let key = Key { lod, cascade: u8::try_from(level).unwrap_or(u8::MAX), cell: [floor_i32(base[0]) + o[0], floor_i32(base[1]) + o[1], floor_i32(base[2]) + o[2]] };
        let Some(&slot) = index.get(&key) else { continue };
        if let Some((voxels, opts, from)) = seen {
            w *= voxels.passing(opts.refine_level, from, cascade.seat_point[slot as usize], (opts.cross_full, opts.box_margin));
        }
        if w > 0.0 {
            out.push((slot as usize, w));
        }
    }
    out
}

/// One direction's merged cone from the weighted probes: those that have it,
/// renormalised.
fn read_cone(cascade: &Cascade, weights: &[(usize, f32)], bin: usize) -> Option<(V3, V3)> {
    let dirs = cascade.dirs();
    let (mut sum, mut moment, mut weight) = ([0.0; 3], [0.0; 3], 0.0f32);
    for (slot, w) in weights {
        let i = slot * dirs + bin;
        if cascade.filled[i] {
            sum = add(sum, scale(cascade.cone[i], *w));
            moment = add(moment, scale(cascade.cone_moment[i], *w));
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
/// A surface point: position, normal, area, albedo.
type Point = (V3, V3, f32, V3);

/// The level a surface patch gathers its light at in `--mode full`: 25 cm.
const GATHER_LEVEL: u8 = 4;
/// Cones a patch gathers with each frame.
const GATHER_CONES: u64 = 16;

/// The patch a surface point gathers with: its 25 cm cell and the axis it
/// faces most.
fn gather_patch(at: V3, normal: V3, level: u8) -> ([i32; 3], usize) {
    let axis = (0..6).max_by(|a, b| dot(normal, AXES[*a]).total_cmp(&dot(normal, AXES[*b]))).unwrap_or(0);
    (nearest_cell(sub(at, scale(normal, 0.01)), voxel_size(level)), axis)
}

/// The gather patches of `points`: each point's patch index, and each
/// patch's gather point and normal. A floor or ceiling point inside a solid
/// (under a wall or a box) is buried: it belongs to its patch but does not
/// place the gather point, so a patch across the foot of a box gathers from
/// its open part. A patch with only buried points gets `None` and stays dark.
fn gather_patches(points: &[Point], scene: &Scene, level: u8) -> (Vec<u32>, Vec<Option<(V3, V3)>>) {
    let mut index: HashMap<([i32; 3], usize), u32> = HashMap::new();
    let mut sums: Vec<(V3, V3, f32)> = Vec::new();
    let of_point = points
        .iter()
        .map(|(at, normal, _, _)| {
            let next = u32::try_from(sums.len()).unwrap_or(u32::MAX);
            let i = *index.entry(gather_patch(*at, *normal, level)).or_insert(next);
            if i == next {
                sums.push(([0.0; 3], [0.0; 3], 0.0));
            }
            let buried = normal[1].abs() > 0.9 && scene.inside(add(*at, scale(*normal, LIFT)));
            if !buried {
                let s = &mut sums[i as usize];
                s.0 = add(s.0, *at);
                s.1 = add(s.1, *normal);
                s.2 += 1.0;
            }
            i
        })
        .collect();
    let patches = sums.into_iter().map(|(at, n, count)| (count > 0.0).then(|| (scale(at, 1.0 / count), normalize(n)))).collect();
    (of_point, patches)
}


/// Patches whose cones met a surface with no patch (a probe).
static UNASSIGNED: AtomicU64 = AtomicU64::new(0);

/// Cones a patch's transfer row is baked with, once while the geometry
/// stands still.
const TRANSFER_CONES: u64 = 128;

/// For every patch, `cones` cosine cones through the voxels (new directions
/// with `seed`), each split into what it met that is not a surface's (lamps and
/// sky: D) and, for each surface patch it met, the share it filled times that
/// patch's albedo over pi (a row of T). A floor or ceiling patch inside a solid
/// gathers nothing.
fn gather_rows(voxels: &Voxels, patches: &[Option<(V3, V3)>], o: &Options, cones: u64, seed: u64, sky: V3) -> Vec<(V3, Vec<(u32, V3)>)> {
    let chunk = patches.len().div_ceil(o.threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = patches
            .chunks(chunk)
            .enumerate()
            .map(|(c, part)| {
                scope.spawn(move || {
                    let mut out = Vec::with_capacity(part.len());
                    for (n, patch) in part.iter().enumerate() {
                        // Each patch its own rotation of the directions: patches
                        // that face one way do not all err the same way.
                        let id = (c * chunk + n) as u64;
                        let (ju, jv) = (hashed(seed.wrapping_mul(0x2545_f491) ^ id), hashed(seed.wrapping_add(id.wrapping_mul(0x9e37_79b9))));
                        let Some((at, normal)) = *patch else {
                            out.push(([0.0; 3], Vec::new()));
                            continue;
                        };
                        let side = normalize(cross(normal, if normal[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] }));
                        let up = cross(normal, side);
                        let mut sink = Sink::default();
                        let mut captured = Vec::new();
                        for k in 0..cones {
                            let (u, v) = r2(k);
                            let (u, v) = ((u + ju).fract(), (v + jv).fract());
                            let (r, phi) = (u.sqrt(), 2.0 * PI * v);
                            let dir = normalize(add(add(scale(side, r * phi.cos()), scale(up, r * phi.sin())), scale(normal, (1.0 - u).max(0.0).sqrt())));
                            let _ = march_cone_into(voxels, o, at, normal, dir, voxel_size(o.gather_level), sky, Some(&mut sink));
                            if GATHER_ARMED.load(Ordering::Relaxed) {
                                let start = add(at, scale(normal, 0.75 * voxel_size(VOXEL_FINEST)));
                                captured.push([start[0], start[1], start[2], voxel_size(o.gather_level), dir[0], dir[1], dir[2], 0.0]);
                            }
                        }
                        if !captured.is_empty() {
                            if let Ok(mut list) = GATHER_JOBS.lock() {
                                list.extend(captured);
                            }
                        }
                        // The sun (phase 2b, option B): one narrow cone toward
                        // it, its irradiance times what passes.
                        if let Some(sun) = sun() {
                            let cosine = dot(sun.toward, normal);
                            if cosine > 0.0 {
                                let start = add(at, scale(normal, 0.75 * voxel_size(VOXEL_FINEST)));
                                let mut span = Span { done: 0.25 * voxel_size(VOXEL_FINEST), covered: 0.0, met: [0.0; 3], fine: o.fine_cross };
                                march_span(voxels, o, start, sun.toward, 0.01, REACH, &mut span, None);
                                let pass = (1.0 - span.covered).max(0.0);
                                sink.direct = add(sink.direct, scale(sun.irradiance, cosine * pass * cones as f32 / PI));
                            }
                        }
                        let k = PI / cones as f32;
                        let mut row: HashMap<u32, V3> = HashMap::new();
                        for (j, w) in sink.records {
                            let e = row.entry(j).or_insert([0.0; 3]);
                            *e = add(*e, scale(w, k));
                        }
                        if sink.unassigned > 0.0 {
                            UNASSIGNED.fetch_add(1, Ordering::Relaxed);
                        }
                        out.push((scale(sink.direct, k), row.into_iter().collect()));
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    })
}

/// `--mode full`'s light for one frame: D from [`GATHER_CONES`] cones a patch
/// through the voxels as they are (the lamps where they are now, the sky),
/// and the baked transfer `t`; `E = D + T E` is iterated so the frame's surface
/// light holds every bounce. Returns each point's radiance.
#[expect(clippy::too_many_arguments, reason = "the field, its patches, the baked transfer and the frame")]
fn gather(voxels: &Voxels, points: &[Point], patch_of: &[u32], patches: &[Option<(V3, V3)>], t: &[Vec<(u32, V3)>], o: &Options, frame: u32, sky: V3) -> Vec<V3> {
    // Each patch's cone directions stay the same from frame to frame: a still
    // scene's light does not change, and a lamp that moves changes it in the
    // frame it moves.
    let _ = frame;
    let d: Vec<V3> = gather_rows(voxels, patches, o, o.gather_cones, 7, sky).into_iter().map(|r| r.0).collect();
    let mut e = d.clone();
    for _ in 0..o.transfer_sweeps {
        e = t.iter().zip(&d).map(|(row, d)| row.iter().fold(*d, |acc, (j, w)| add(acc, mul(*w, e[*j as usize])))).collect();
    }
    if let Ok(mut last) = LAST_E.lock() {
        last.clone_from(&e);
    }
    if let Ok(mut last) = LAST_D.lock() {
        last.clone_from(&d);
    }
    points
        .iter()
        .zip(patch_of)
        .map(|((_, _, _, albedo), i)| scale(mul(*albedo, e[*i as usize]), 1.0 / PI))
        .collect()
}

/// A small hash-based random number in [0, 1).
fn hashed(seed: u64) -> f32 {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x ^= x >> 31;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 29;
    ((x >> 40) as f32) / (1u64 << 24) as f32
}

/// One sample of the lamps' light at a point: one point on each lamp's disc,
/// seen or not.
fn sphere_light_once(scene: &dyn Surfaces, lamps: &[(V3, V3)], radius: f32, point: V3, normal: V3, seed: u64) -> V3 {
    let origin = add(point, scale(normal, LIFT));
    let mut sum = [0.0; 3];
    for (k, (centre, glow)) in lamps.iter().enumerate() {
        let to = sub(*centre, origin);
        let d2 = dot(to, to);
        let toward = normalize(to);
        let cosine = dot(toward, normal);
        if cosine <= 0.0 || d2 <= radius * radius {
            continue;
        }
        let side = normalize(cross(toward, if toward[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] }));
        let up = cross(toward, side);
        let (a, b) = (hashed(seed ^ (k as u64 * 977)), hashed(seed.wrapping_add(k as u64 * 1_000_003)));
        let (r, phi) = (radius * a.sqrt(), 2.0 * PI * b);
        let target = add(*centre, add(scale(side, r * phi.cos()), scale(up, r * phi.sin())));
        if !scene.blocked(origin, target) {
            sum = add(sum, scale(*glow, PI * radius * radius / d2 * cosine));
        }
    }
    sum
}

/// `--mode full`'s reference: the irradiance at a point from the lamps and
/// every bounce, by path tracing. The lamps' light at the point is the exact
/// visible share of each disc; each path from it adds the lamps' light at
/// every hit (one disc point each) and goes on in a cosine direction, up to 64
/// bounces with Russian roulette after 2.
#[expect(clippy::too_many_arguments, reason = "the scene, its lamps and the point")]
fn full_reference(walls: &dyn Surfaces, paint: f32, lamps: &[(V3, V3)], radius: f32, sky: V3, point: V3, normal: V3, samples: u32, seed: u64) -> V3 {
    let direct = add(sphere_light(walls, lamps, radius, point, normal), sun_exact(walls, point, normal));
    let cosine_dir = |normal: V3, u: f32, v: f32| {
        let side = normalize(cross(normal, if normal[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] }));
        let up = cross(normal, side);
        let (r, phi) = (u.sqrt(), 2.0 * PI * v);
        normalize(add(add(scale(side, r * phi.cos()), scale(up, r * phi.sin())), scale(normal, (1.0 - u).max(0.0).sqrt())))
    };
    let (ju, jv) = r2(seed.wrapping_mul(2_654_435_761) % 100_003);
    let mut sum = [0.0; 3];
    for k in 0..samples.max(1) {
        let (u, v) = r2(u64::from(k));
        let mut dir = cosine_dir(normal, (u + ju).fract(), (v + jv).fract());
        let mut origin = add(point, scale(normal, LIFT));
        let mut weight = [1.0f32; 3];
        let mut radiance = [0.0f32; 3];
        let path_seed = seed.wrapping_mul(1_000_003).wrapping_add(u64::from(k));
        for bounce in 0..64u64 {
            let Some(hit) = walls.first_hit(origin, dir, REACH) else {
                radiance = add(radiance, mul(weight, sky));
                break;
            };
            // Outgoing radiance of a diffuse hit: albedo over pi times its
            // irradiance, the lamps' share of which is sampled here.
            let s = path_seed.wrapping_mul(31).wrapping_add(bounce * 7);
            let lamp = add(sphere_light_once(walls, lamps, radius, hit.point, hit.normal, s), sun_exact(walls, hit.point, hit.normal));
            weight = mul(weight, scale(hit.albedo, paint));
            radiance = add(radiance, mul(weight, scale(lamp, 1.0 / PI)));
            if bounce >= 2 {
                let keep = weight[0].max(weight[1]).max(weight[2]).min(0.95);
                if hashed(s ^ 0x5555) >= keep {
                    break;
                }
                weight = scale(weight, 1.0 / keep);
            }
            origin = add(hit.point, scale(hit.normal, LIFT));
            dir = cosine_dir(hit.normal, hashed(s ^ 0x1111), hashed(s ^ 0x2222));
        }
        sum = add(sum, radiance);
    }
    add(direct, scale(sum, PI / samples.max(1) as f32))
}

fn build_voxels(scene: &Scene, mesh: &SceneMesh, gltf: Option<&GltfWalls>, walls: &dyn Surfaces, pixels: &[Option<Pixel>], glow: &[(V3, V3)], o: &Options) -> (Voxels, Vec<Point>) {
    // Point, normal, area, albedo.
    let mut points: Vec<(V3, V3, f32, V3)> = Vec::new();
    if let (true, Some(model)) = (o.world_voxels, gltf) {
        // A glTF model: points on every triangle, each point's albedo read by
        // a short ray into the surface at that point (the texture, and an
        // alpha hole gives no point).
        let step = 0.5 * voxel_size(VOXEL_FINEST);
        let triangles: Vec<&genos_mesh::MeshTriangle> = model.0.triangles().collect();
        let chunk = triangles.len().div_ceil(o.threads).max(1);
        let parts: Vec<Vec<(V3, V3, f32, V3)>> = std::thread::scope(|scope| {
            let handles: Vec<_> = triangles
                .chunks(chunk)
                .map(|part| {
                    scope.spawn(move || {
                        let mut out = Vec::new();
                        for tri in part {
                            let [a, b, c] = tri.positions;
                            let (e1, e2) = (sub(b, a), sub(c, a));
                            let face = cross(e1, e2);
                            let area = 0.5 * dot(face, face).sqrt();
                            if area <= 0.0 {
                                continue;
                            }
                            // The front is the side the vertex normal points to.
                            // A one-sided solid's back faces show only from
                            // inside it, where no ray goes: they are left out,
                            // or a cone that slips past the front meets them
                            // as dark walls.
                            let mut normal = normalize(face);
                            if dot(normal, tri.normals[0]) < 0.0 {
                                normal = scale(normal, -1.0);
                            }
                            let sides: &[f32] = if model.0.double_sided(tri.material) { &[1.0, -1.0] } else { &[1.0] };
                            let n1 = (dot(e1, e1).sqrt() / step).ceil().max(1.0) as u32;
                            let n2 = (dot(e2, e2).sqrt() / step).ceil().max(1.0) as u32;
                            let mut made = Vec::new();
                            for i in 0..n1 {
                                for j in 0..n2 {
                                    let u = (i as f32 + 0.5) / n1 as f32;
                                    let v = (j as f32 + 0.5) / n2 as f32;
                                    if u + v <= 1.0 {
                                        made.push(add(a, add(scale(e1, u), scale(e2, v))));
                                    }
                                }
                            }
                            let each = area / made.len().max(1) as f32;
                            for at in made {
                                for side in sides.iter().map(|k| scale(normal, *k)) {
                                    let from = add(at, scale(side, 0.01));
                                    if let Some(hit) = model.first_hit(from, scale(side, -1.0), 0.02) {
                                        out.push((at, side, each, hit.albedo));
                                    }
                                }
                            }
                        }
                        out
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
        });
        points = parts.into_iter().flatten().collect();
    } else if o.world_voxels {
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
            let albedo = if o.mode == Mode::Full { scale(albedo, PAINT) } else { albedo };
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
    // The underside of the ground: the analytic floor is a plane both sides
    // of which are surfaces, and the side under the ground faces empty space.
    // No ray from above reaches it, but it gathers the whole sky and would
    // glow. It is left out.
    points.retain(|(at, normal, _, _)| !(normal[1] < -0.9 && at[1].abs() < 0.01));
    let mut voxels = Voxels::new();
    if o.mode != Mode::Nee {
        for (at, normal, area, _) in &points {
            voxels.add_surface(*at, *normal, *area, [0.0; 3]);
        }
        for (centre, glow) in glow {
            voxels.add_lamp(*centre, o.lamp_radius, *glow);
        }
        return (voxels, points);
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
        let (at, normal) = (scale(*at, 1.0 / count), normalize(*normal));
        // A floor or ceiling patch inside a solid (the floor under a wall)
        // shows to no ray, and its shadow rays start inside the solid and
        // escape: it stays as cover, with no light. Side faces are not tested:
        // the inside test misfires on a third of the stress building's
        // x-facing wall points.
        if normal[1].abs() > 0.9 && scene.inside(add(at, scale(normal, LIFT))) {
            lit.insert(*key, [0.0; 3]);
            continue;
        }
        let (light, rays) = direct_light(scene, walls, at, normal);
        voxels.shadow_rays += rays;
        lit.insert(*key, light);
    }
    for (at, normal, area, albedo) in &points {
        let direct = lit.get(&patch_of(*at, *normal)).copied().unwrap_or([0.0; 3]);
        voxels.add_surface(*at, *normal, *area, scale(mul(*albedo, direct), LAMBERT));
    }
    (voxels, points)
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

/// A value from 0 to 1 as a byte, clamped.
fn unit_byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The first of a voxel's four texels in the stacked clip volumes: `local`
/// inside level `k`'s volume of `side` texels a side.
fn volume_texel(local: [i32; 3], k: usize, side: i32) -> usize {
    let level = i32::try_from(k).unwrap_or(0);
    usize::try_from(((local[2] + side * level) * side + local[1]) * side + local[0]).unwrap_or(0) * 4
}

/// The plane volumes' byte for a position across a voxel: -0.5 to 1.5 voxels
/// from its low corner (`UNORM_FLOOR`, `UNORM_SPAN` in cascade.comp).
fn plane_byte(x: f32, low: f32, size: f32) -> u8 {
    unit_byte(((x - low) / size).mul_add(0.5, 0.25))
}

/// `--gpu-bench`: one frame's voxel-layer cascade cones marched on the GPU
/// over the same voxels, and the hardware ray tracer on the same picture.
#[expect(clippy::too_many_arguments, reason = "the bench takes the frame's whole setup")]
fn gpu_bench(
    o: &Options,
    scene: &Scene,
    mesh: &SceneMesh,
    pixels: &[Option<Pixel>],
    voxels: &Voxels,
    points: &[Point],
    [walls, surfaces]: [&dyn Surfaces; 2],
    eye: V3,
    [forward, right, up]: [V3; 3],
) -> Result<(), String> {
    if !o.cone || !o.cones_from_voxels {
        return Err("--gpu-bench times the voxel-layer cascade: give --tracer cone and --cone-start voxel".into());
    }
    // One CPU frame, to list its cones.
    *CONE_JOBS.lock().map_err(|_| "gpu bench: capture lock")? = Some(Vec::new());
    let started = std::time::Instant::now();
    let _ = run_frame(&Setup { scene, walls, surfaces, voxels: Some(voxels), pixels, options: o }, (0.0, 0.0));
    let cpu_seconds = started.elapsed().as_secs_f64();
    let jobs = CONE_JOBS.lock().map_err(|_| "gpu bench: capture lock")?.take().unwrap_or_default();
    let probes: Vec<String> = (0..o.cascades)
        .map(|c| {
            let tag = f32::from(u8::try_from(c).unwrap_or(u8::MAX));
            let n = jobs.iter().filter(|job| (job[7] - tag).abs() < 0.5).count();
            let theta = o.theta << c;
            let dirs = usize::try_from(2 * theta * theta).unwrap_or(usize::MAX);
            format!("c{c} {n} ({} probes x {dirs})", n / dirs.max(1))
        })
        .collect();
    println!("cones by cascade: {}", probes.join(", "));
    if o.near_check == NearCheck::Chain {
        println!("  closed before their interval and not traced: {}", CHAIN_SKIPPED.load(Ordering::Relaxed));
    }
    let cpu_cover = CHAIN_COVER.lock().map_or(0.0, |s| s.0 / (s.1.max(1)) as f64);
    // A clip volume of each voxel level around the eye, the levels stacked on z.
    let side = i32::try_from(o.gpu_side).unwrap_or(i32::MAX);
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
        let at = volume_texel(local, k, side);
        #[expect(clippy::needless_range_loop, reason = "the index reads parallel arrays")]
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
    // The dilated occupancy over all levels, as `Voxels::occupied` reads it.
    let mut occupancy = vec![0u8; texels / 4];
    for (level, cell) in voxels.occupancy() {
        let k = usize::from(level - VOXEL_FINEST);
        let local = [cell[0] - corner[k][0], cell[1] - corner[k][1], cell[2] - corner[k][2]];
        if local.iter().all(|v| *v >= 0 && *v < side) {
            occupancy[volume_texel(local, k, side) / 4] = u8::MAX;
        }
    }
    // The plane volumes, over the levels a crossing reads.
    let refine = o.refine_level.clamp(VOXEL_FINEST, VOXEL_COARSEST);
    let plane_levels = u32::from(refine - VOXEL_FINEST + 1);
    let plane_texels = texels / usize::try_from(gpu::LEVELS).unwrap_or(1) * usize::try_from(plane_levels).unwrap_or(1);
    let mut planes: [Vec<u8>; 12] = std::array::from_fn(|_| vec![0u8; plane_texels]);
    for ((level, cell), voxel) in &voxels.cells {
        if *level > refine {
            continue;
        }
        let k = usize::from(level - VOXEL_FINEST);
        let local = [cell[0] - corner[k][0], cell[1] - corner[k][1], cell[2] - corner[k][2]];
        if local.iter().any(|v| *v < 0 || *v >= side) {
            continue;
        }
        let size = voxel_size(*level);
        let low = cell.map(|c| c as f32 * size);
        let at = volume_texel(local, k, side);
        for d in 0..6 {
            if voxel.area[d] <= 0.0 {
                continue;
            }
            let (ax, u, v) = (d / 2, (d / 2 + 1) % 3, (d / 2 + 2) % 3);
            let centroid = voxel.at[d][ax] / voxel.area[d];
            planes[d][at..at + 4].copy_from_slice(&[
                plane_byte(centroid, low[ax], size),
                plane_byte(voxel.lo[d][u], low[u], size),
                plane_byte(voxel.hi[d][u], low[u], size),
                plane_byte(voxel.lo[d][v], low[v], size),
            ]);
            planes[6 + d][at] = plane_byte(voxel.hi[d][v], low[v], size);
        }
    }
    let sky = if o.mode == Mode::Emissive { [0.0; 3] } else { scene.sky.as_ref().map_or([0.0; 3], |s| s.color) };
    let timing = gpu::time_cascade(
        &gpu::CascadeInput {
            jobs: &jobs,
            faces: &volumes,
            planes: &planes,
            plane_levels,
            side: o.gpu_side,
            occupancy: &occupancy,
            descend: o.descend,
            clip,
            width0: o.cone_width * (4.0 * PI / (2 * o.theta * o.theta) as f32).sqrt(),
            cascades: u32::try_from(o.cascades).unwrap_or(u32::MAX),
            refine_level: u32::from(refine),
            cross_full: o.cross_full,
            box_margin: o.box_margin,
            sky,
            own_interval: o.own_interval,
            fixed_width: 0.0,
            refine_cascades: u32::try_from(o.refine_cascades).unwrap_or(u32::MAX),
        },
        20,
    )?;
    let shown = pixels.iter().filter(|p| p.is_some()).count().max(1);
    let last = f32::from(u8::try_from(o.cascades - 1).unwrap_or(u8::MAX));
    let (gpu_sum, gpu_n) = jobs
        .iter()
        .zip(&timing.covers)
        .filter(|(job, _)| (job[7] - last).abs() >= 0.5)
        .fold((0.0f64, 0u64), |(s, n), (_, c)| (s + f64::from(*c), n + 1));
    println!(
        "  mean cover over a cone's own interval, all cascades but the last: GPU {:.4}, CPU {cpu_cover:.4}",
        gpu_sum / (gpu_n.max(1)) as f64
    );
    if let Ok(cpu) = CHAIN_COVERS.lock() {
        if cpu.len() == jobs.len() {
            for c in 0..o.cascades.saturating_sub(1) {
                let tag = f32::from(u8::try_from(c).unwrap_or(u8::MAX));
                let rows: Vec<(usize, f32, f32)> = (0..jobs.len())
                    .filter(|i| (jobs[*i][7] - tag).abs() < 0.5)
                    .map(|i| (i, cpu[i], timing.covers[i]))
                    .collect();
                let n = rows.len().max(1) as f64;
                let (mut cpu_sum, mut gpu_sum, mut more, mut less) = (0.0f64, 0.0f64, 0usize, 0usize);
                for (_, a, b) in &rows {
                    cpu_sum += f64::from(*a);
                    gpu_sum += f64::from(*b);
                    more += usize::from(*a - *b > 0.5);
                    less += usize::from(*b - *a > 0.5);
                }
                println!("  c{c}: cover CPU {:.3} GPU {:.3}; CPU closed and GPU open (by 0.5) {more}, the other way {less}, of {}", cpu_sum / n, gpu_sum / n, rows.len());
                for (i, a, b) in rows.iter().filter(|r| r.1 - r.2 > 0.5).take(3) {
                    let j = jobs[*i];
                    println!("      start {:.3},{:.3},{:.3} reach {:.3} dir {:.3},{:.3},{:.3}: CPU {a:.3} GPU {b:.3}", j[0], j[1], j[2], j[3], j[4], j[5], j[6]);
                }
            }
        } else {
            println!("  per-cone covers: {} CPU against {} cones", cpu.len(), jobs.len());
        }
    }
    println!("gpu bench on {}  {}x{}  {} cascades, theta {}", timing.device, o.width, o.height, o.cascades, o.theta);
    println!(
        "voxel-layer cascade: {:.3} ms median, {:.3} ms least for {} cones ({:.1} a shown pixel); {:.1} steps and {:.1} texture reads a cone, mean cover {:.3}",
        timing.ms_median, timing.ms_min, timing.cones, timing.cones as f64 / shown as f64, timing.steps_per_cone, timing.reads_per_cone, timing.covered
    );
    println!("  the same cones on the CPU: {cpu_seconds:.2} s for the whole frame (cones, merge and shade) on {} threads", o.threads);
    println!(
        "  textures {:.0} MiB (7 RGBA16F volumes of {side}x{side}x{}, 12 RGBA8 plane volumes over {plane_levels} levels); cone list {:.0} MiB; {inside} of {} voxels inside the clip volumes",
        timing.texture_mib,
        side * gpu::LEVELS as i32,
        (jobs.len() * 32) as f64 / (1024.0 * 1024.0),
        voxels.cells.len(),
    );
    if o.mode == Mode::Full {
        let (_, patches) = gather_patches(points, scene, o.gather_level);
        GATHER_ARMED.store(true, Ordering::Relaxed);
        let _ = gather_rows(voxels, &patches, o, o.gather_cones, 7, sky);
        GATHER_ARMED.store(false, Ordering::Relaxed);
        let gather_jobs = std::mem::take(&mut *GATHER_JOBS.lock().map_err(|_| "gpu bench: gather lock")?);
        let gather = gpu::time_cascade(
            &gpu::CascadeInput {
                jobs: &gather_jobs,
                faces: &volumes,
                planes: &planes,
                plane_levels,
                side: o.gpu_side,
                occupancy: &occupancy,
                descend: o.descend,
                clip,
                width0: o.cone_width * (4.0 * PI / (2 * o.theta * o.theta) as f32).sqrt(),
                cascades: u32::try_from(o.cascades).unwrap_or(u32::MAX),
                refine_level: u32::from(refine),
                cross_full: o.cross_full,
                box_margin: o.box_margin,
                sky,
                own_interval: false,
                refine_cascades: u32::MAX,
                fixed_width: o.gather_width,
            },
            20,
        )?;
        let live = patches.iter().filter(|p| p.is_some()).count();
        println!(
            "bounce gather: {:.3} ms median, {:.3} ms least for {} cones ({live} patches x {}), every interval; {:.1} steps and {:.1} texture reads a cone",
            gather.ms_median, gather.ms_min, gather.cones, o.gather_cones, gather.steps_per_cone, gather.reads_per_cone
        );
    }
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
    c.map(|v| unit_byte((v * exposure).max(0.0).powf(1.0 / 2.2)))
}

fn run(o: &Options) -> Result<(), String> {
    let root = genos_scene_lib::repo_root();
    let entries = genos_scene_lib::load_dir(&genos_scene_lib::scenes_dir())?;
    let name = genos_scene_lib::canonical(&o.scene);
    let entry = entries
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| format!("no scene {name} under {}", root.display()))?;
    let (mut scene, gltf) = load_scene(&root, entry, o)?;
    if !o.sky {
        scene.sky = None;
    }
    if o.renderer_field {
        if let (Some(eye), true) = (o.eye, o.sun_check) {
            return sun_check(&scene, gltf.as_ref().map(|g| &g.0), eye);
        }
        if let Some(eye) = o.eye {
            let started = std::time::Instant::now();
            let coarse = genos_render::cone_field::FieldVoxels::around_eye(&scene, gltf.as_ref().map(|g| &g.0), eye, false).brick();
            println!(
                "renderer field, coarse levels alone (the first build): {} voxels, {:.0} MiB, {:.2} s",
                coarse.voxels,
                coarse.bytes() as f64 / (1024.0 * 1024.0),
                started.elapsed().as_secs_f64()
            );
            // The renderer's build about an eye: coarse levels for the whole
            // scene, the fine levels in clip volumes around the eye.
            let started = std::time::Instant::now();
            let voxels = genos_render::cone_field::FieldVoxels::around_eye(&scene, gltf.as_ref().map(|g| &g.0), eye, true);
            let filed = started.elapsed().as_secs_f64();
            // A world probe layer about the eye: one probe a surface face.
            for (level, half) in [(5u8, 4.0f32), (6, 8.0), (7, 16.0), (5, 8.0), (6, 16.0), (7, 32.0)] {
                println!(
                    "world layer: {} m voxels within {half} m of the eye: {} faces",
                    genos_render::cone_field::voxel_size(level),
                    voxels.faces_within(level, eye, half)
                );
            }
            let field = voxels.brick();
            println!(
                "renderer field about {eye:?}: {} points, {} voxels, {:.0} MiB; {filed:.2} s to sample and file, {:.2} s to brick",
                field.points,
                field.voxels,
                field.bytes() as f64 / (1024.0 * 1024.0),
                started.elapsed().as_secs_f64() - filed
            );
            for (k, level) in field.levels.iter().enumerate() {
                println!("  level {}: clip {:?}, {:?} bricks in the table", k + 2, level.clip, level.dims);
            }
            return Ok(());
        }
        let started = std::time::Instant::now();
        // The renderer's build: shape points on every core, the albedo ray
        // only where a material has holes, voxels on every core. The lamps
        // are not in it; the renderer writes them each frame.
        let points = genos_render::cone_field::surface_shape_points(&scene, gltf.as_ref().map(|g| &g.0));
        let sampled = started.elapsed().as_secs_f64();
        let voxels = genos_render::cone_field::FieldVoxels::from_surfaces(&points);
        let built = started.elapsed().as_secs_f64();
        println!("renderer field build: {:.2} s to sample, {:.2} s to file", sampled, built - sampled);
        if o.field_check {
            // The reference: every point's albedo ray, one thread.
            let reference = std::time::Instant::now();
            let all = genos_render::cone_field::surface_points(&scene, gltf.as_ref().map(|g| &g.0), PAINT);
            let mut serial = genos_render::cone_field::FieldVoxels::default();
            for (at, normal, area, _) in &all {
                serial.add_surface(*at, *normal, *area, [0.0; 3]);
            }
            println!("reference build: {} points in {:.1} s; renderer build {} points", all.len(), reference.elapsed().as_secs_f64(), points.len());
            for (k, (fast, slow)) in voxels.level_totals().iter().zip(serial.level_totals().iter()).enumerate() {
                println!(
                    "  level {}: voxels {} / {} ({:+.3}%), area {:.1} / {:.1} m2 ({:+.3}%)",
                    k + 2,
                    fast.0,
                    slow.0,
                    100.0 * (fast.0 as f64 - slow.0 as f64) / (slow.0 as f64).max(1.0),
                    fast.1,
                    slow.1,
                    100.0 * (fast.1 - slow.1) / slow.1.max(1e-9),
                );
            }
        }
        let field = voxels.brick();
        let mib = |b: usize| b as f64 / (1024.0 * 1024.0);
        println!(
            "renderer field: {} points, {} voxels; fine: {} bricks, {} records ({:.0} MiB); coarse: {} bricks, atlas {}x{}x{} ({:.0} MiB faces, {:.0} MiB occupancy); table {:.1} MiB; {:.0} MiB in all; {built:.1} s to voxelise, {:.1} s to brick",
            field.points,
            field.voxels,
            field.fine_bricks.len() / genos_render::cone_field::FINE_HEADER,
            field.fine_voxels.len() / genos_render::cone_field::FINE_RECORD,
            mib((field.fine_bricks.len() + field.fine_voxels.len()) * 4),
            field.coarse_bricks,
            field.atlas[0],
            field.atlas[1],
            field.atlas[2],
            mib(field.faces.iter().map(|f| f.len() * 2).sum()),
            mib(field.occupancy.len()),
            mib(field.table.len() * 4),
            mib(field.bytes()),
            started.elapsed().as_secs_f64() - built,
        );
        for (k, level) in field.levels.iter().enumerate() {
            println!("  level {}: bricks from {:?}, {:?} in the table", k + 2, level.origin, level.dims);
        }
        return Ok(());
    }
    if gltf.is_some() && o.gpu_bench {
        return Err(format!("{name}: --gpu-bench takes the analytic building only"));
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
        .filter(|l| !o.no_lamps && l.direction.x == 0.0 && l.direction.y == 0.0 && l.direction.z == 0.0)
        .map(|l| {
            let centre = [l.position.x, l.position.y, l.position.z];
            (centre, scale(l.color, LAMP_UNIT / (PI * o.lamp_radius * o.lamp_radius)))
        })
        .collect();
    // The sun as an emitter (phase 2b): the engine's lamp unit at the
    // prototype's fixed 7 m, as `direct_light` takes it.
    let sun_light = (o.sun && o.mode == Mode::Full)
        .then(|| (0..scene.light_len()).map(|i| scene.light(i)).find(|l| l.direction.x != 0.0 || l.direction.y != 0.0 || l.direction.z != 0.0))
        .flatten()
        .map(|l| SunLight {
            toward: scale(normalize([l.direction.x, l.direction.y, l.direction.z]), -1.0),
            irradiance: scale(l.color, LAMP_UNIT / 49.0),
        });
    let _ = SUN.set(sun_light);
    let glowing = Glowing { scene: walls, lamps: glow.clone(), radius: o.lamp_radius };
    let surfaces: &dyn Surfaces = if o.mode == Mode::Nee { walls } else { &glowing };
    let built = (o.cone || o.gpu_bench).then(|| build_voxels(scene, &mesh, gltf.as_ref(), walls, &pixels, &glow, o));
    let (mut voxels, points) = match built {
        Some((v, p)) => (Some(v), p),
        None => (None, Vec::new()),
    };
    if let (true, Some(voxels)) = (o.gpu_bench, &voxels) {
        return gpu_bench(o, scene, &mesh, &pixels, voxels, &points, [walls, surfaces], eye, [forward, right, up]);
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
        if o.mode == Mode::Full {
            let rows = gather_rows(voxels, &[Some((pixel.point, pixel.normal))], o, 128, 7, sky);
            let row: f32 = rows[0].1.iter().map(|(_, w)| luma(*w)).sum();
            println!("gather at this point: D {:.5} (pi times the cones' lamp and sky light), T row sum {row:.4}", luma(rows[0].0));
        }
        rows.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (d, dir, at, ray, cone) in rows.iter().take(8).chain(rows.iter().rev().take(8)) {
            println!("  dir [{:.2},{:.2},{:.2}] ray hit [{:.2},{:.2},{:.2}] ray {ray:.4} cone {cone:.4} ({d:+.4})", dir[0], dir[1], dir[2], at[0], at[1], at[2]);
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
    let (patch_of, patches) = if o.mode == Mode::Full { gather_patches(&points, scene, o.gather_level) } else { (Vec::new(), Vec::new()) };
    if let (Mode::Full, Some(field)) = (o.mode, voxels.as_mut()) {
        field.assign_patches(&points, &patch_of);
    }
    // The transfer between patches is geometry: baked once with many cones
    // while nothing moves (a mover would re-bake the rows it reaches).
    let baked = std::time::Instant::now();
    let transfer: Vec<Vec<(u32, V3)>> = match (o.mode, voxels.as_ref()) {
        (Mode::Full, Some(field)) => {
            let sky = scene.sky.as_ref().map_or([0.0; 3], |s| s.color);
            gather_rows(field, &patches, o, TRANSFER_CONES, 1_000_003, sky).into_iter().map(|r| r.1).collect()
        }
        _ => Vec::new(),
    };
    if o.mode == Mode::Full {
        let entries: usize = transfer.iter().map(Vec::len).sum();
        println!("transfer: {} patches, {entries} entries, baked with {TRANSFER_CONES} cones a patch in {:.1} s; patches that met a surface with no patch: {}", patches.len(), baked.elapsed().as_secs_f32(), UNASSIGNED.load(Ordering::Relaxed));
    }
    // The reference's diffuse share: the engine's paint on the analytic
    // building, the file's own albedo on a glTF model.
    let paint = if gltf.is_some() { 1.0 } else { PAINT };
    let started = std::time::Instant::now();
    let full_sky = scene.sky.as_ref().map_or([0.0; 3], |s| s.color);
    let mut frames: Vec<Frame> = Vec::new();
    let mut gather_seconds = 0.0f32;
    for f in 0..o.frames {
        let (u, v) = r2(u64::from(f) * 7919 + 1);
        let setup = Setup { scene, walls, surfaces, voxels: voxels.as_ref(), pixels: &pixels, options: o };
        frames.push(run_frame(&setup, if f == 0 { (0.0, 0.0) } else { (u, v) }));
        if let (Mode::Full, Some(field)) = (o.mode, voxels.as_mut()) {
            // Feedback: what the surfaces gathered this frame is what they
            // send out the next.
            let gathered = std::time::Instant::now();
            let radiance = gather(field, &points, &patch_of, &patches, &transfer, o, f, full_sky);
            field.relight(&points, &radiance);
            gather_seconds += gathered.elapsed().as_secs_f32();
        }
    }
    let cascade_seconds = started.elapsed().as_secs_f32() - gather_seconds;

    if o.mode == Mode::Full && o.patch_check {
        let e = LAST_E.lock().map(|v| v.clone()).unwrap_or_default();
        let in_view = |p: V3| dot(sub(p, eye), forward) > 1.0 && dot(sub(p, eye), sub(p, eye)) < 400.0;
        let chosen: Vec<usize> = (0..patches.len()).filter(|&i| patches[i].is_some_and(|p| in_view(p.0))).step_by(97).take(40).collect();
        let dd = LAST_D.lock().map(|v| v.clone()).unwrap_or_default();
        let (mut got, mut want, mut dgot, mut dwant) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for &i in &chosen {
            let r = full_reference(walls, paint, &glow, o.lamp_radius, full_sky, patches[i].map_or([0.0; 3], |p| p.0), patches[i].map_or([0.0; 3], |p| p.1), 8192, i as u64);
            let direct = sphere_light(walls, &glow, o.lamp_radius, patches[i].map_or([0.0; 3], |p| p.0), patches[i].map_or([0.0; 3], |p| p.1));
            got += f64::from(luma(e[i]));
            want += f64::from(luma(r));
            dgot += f64::from(luma(dd[i]));
            dwant += f64::from(luma(direct));
        }
        eprintln!("patch check: {} patches, solved light over reference {:.3}; D over exact lamp light {:.3} (D includes sky)", chosen.len(), got / want.max(1.0e-9), dgot / dwant.max(1.0e-9));
        let mut rows: Vec<(f32, f32, usize)> = chosen.iter().map(|&i| (luma(sphere_light(walls, &glow, o.lamp_radius, patches[i].map_or([0.0; 3], |p| p.0), patches[i].map_or([0.0; 3], |p| p.1))), luma(dd[i]), i)).collect();
        rows.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (exact, d, i) in rows.iter().take(8) {
            let near = glow.iter().map(|(c, _)| dot(sub(*c, patches[*i].map_or([0.0; 3], |p| p.0)), sub(*c, patches[*i].map_or([0.0; 3], |p| p.0))).sqrt()).fold(f32::MAX, f32::min);
            eprintln!("   patch at {:?} n {:?}: exact {exact:.3}, D {d:.3}, nearest lamp {near:.2} m", patches[*i].map_or([0.0; 3], |p| p.0), patches[*i].map_or([0.0; 3], |p| p.1));
        }
    }
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
                                let v = match o.mode {
                                    Mode::Emissive => sphere_light(walls, glow, o.lamp_radius, pixel.point, pixel.normal),
                                    Mode::Full => full_reference(walls, paint, glow, o.lamp_radius, full_sky, pixel.point, pixel.normal, each, j as u64),
                                    Mode::Nee => bounce_reference(scene, walls, pixel.point, pixel.normal, each, j as u64),
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
        #[expect(clippy::needless_range_loop, reason = "the index reads parallel arrays")]
        for k in 0..samples.len() {
            let got = frames[..take].iter().map(|f| got_at(f, k)).sum::<f64>() / take as f64;
            let want = f64::from(luma(reference[k]));
            abs += (got - want).abs();
            signed += got - want;
        }
        let n = samples.len().max(1) as f64;
        (abs / n / ref_mean.max(1.0e-9), signed / n / ref_mean.max(1.0e-9))
    };
    // In `--mode full` frame 0 has no bounce yet: noise is over the frames
    // after the first feedback.
    let settled = if o.mode == Mode::Full { &frames[1.min(frames.len() - 1)..] } else { &frames[..] };
    let mut noise: Vec<f64> = (0..samples.len())
        .filter_map(|k| {
            let values: Vec<f64> = settled.iter().map(|f| got_at(f, k)).collect();
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
    if o.mode == Mode::Full {
        // Bounce gains one bounce a frame: the error after each frame alone.
        for (f, frame) in frames.iter().enumerate() {
            let (mut abs, mut signed) = (0.0, 0.0);
            #[expect(clippy::needless_range_loop, reason = "the index reads parallel arrays")]
            for k in 0..samples.len() {
                let got = got_at(frame, k);
                let want = f64::from(luma(reference[k]));
                abs += (got - want).abs();
                signed += got - want;
            }
            let n = samples.len().max(1) as f64 * ref_mean.max(1.0e-9);
            println!("after frame {f:2}: error {:.4} of the reference mean, bias {:+.4}", abs / n, signed / n);
        }
        println!("gather and relight: {gather_seconds:.1} cpu seconds for {} frames", frames.len());
    }
    for take in [1usize, 4, 8, 16] {
        if take <= frames.len() {
            let (abs, signed) = error_of(take);
            println!("mean of {take:2} frame(s): error {abs:.4} of the reference mean, bias {signed:+.4}");
        }
    }
    println!("single-frame noise (std over mean across {} frames): median {:.4}  p90 {:.4}", settled.len(), pick(0.5), pick(0.9));
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
                got_at(frames.last().unwrap_or(first), k),
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

/// A face that blocks the sun: (level, cell, way).
type BlockerKey = (u8, [i32; 3], usize);
/// Per 1 m cell where a leaked ray stops: points, field cover near it, normal, distance.
type Leak = (u32, f32, [f32; 3], f32);

/// `--sun-check`: over floor points within 16 m of the eye, the sun the exact
/// ray sees (triangles, alpha holes) against the share the field's march lets
/// through (`ConeField::sun_blocked`, as the renderer's sun share takes it):
/// the points it blocks wrongly and the faces that block them, and the points
/// it leaks the sun onto and where their exact ray stops.
fn sun_check(scene: &genos_scene::Scene, gltf: Option<&genos_render::GltfSurface>, eye: V3) -> Result<(), String> {
    use genos_render::cone_field::{voxel_size, FieldVoxels};
    use std::collections::HashMap;
    let sun = (0..scene.light_len())
        .map(|i| scene.light(i))
        .find(|l| l.direction.x != 0.0 || l.direction.y != 0.0 || l.direction.z != 0.0)
        .ok_or("the scene has no sun")?;
    let toward = normalize([-sun.direction.x, -sun.direction.y, -sun.direction.z]);
    let field = FieldVoxels::around_eye(scene, gltf, eye, true).brick();
    let Some(model) = gltf else {
        // No triangles to trace: a map of the share the field lets through
        // onto the ground (y = 0) within 16 m, a row a 0.5 m step in z.
        println!("sun toward {toward:?}; the share the field lets through onto y = 0, x across, z down, 0.5 m a character ('#' none, '+' some, '.' all)");
        let lift = 0.75 * voxel_size(2);
        for zi in -32..=32_i16 {
            let z = eye[2] + 0.5 * f32::from(zi);
            let row: String = (-32..=32)
                .map(|xi: i16| {
                    let x = eye[0] + 0.5 * f32::from(xi);
                    let (blocked, _) = field.sun_blocked([x, lift, z], toward, 2, 200.0);
                    if blocked >= 0.98 { '#' } else if blocked > 0.02 { '+' } else { '.' }
                })
                .collect();
            println!("{z:7.1} {row}");
        }
        return Ok(());
    };
    let lift = 0.75 * voxel_size(2);
    let (mut found, mut lit, mut let_through, mut shadowed, mut leaked) = (0, 0, 0, 0, 0);
    let mut blockers: HashMap<BlockerKey, (u32, f32, f32)> = HashMap::new();
    let mut leaks: HashMap<[i32; 3], Leak> = HashMap::new();
    for xi in -64..=64_i16 {
        for zi in -64..=64_i16 {
            let (x, z) = (eye[0] + 0.25 * f32::from(xi), eye[2] + 0.25 * f32::from(zi));
            let Some(hit) = model.trace([x, eye[1], z], [0.0, -1.0, 0.0], 4.0) else { continue };
            if hit.normal[1] < 0.9 {
                continue;
            }
            found += 1;
            let from = [hit.point[0], hit.point[1] + lift, hit.point[2]];
            let exact = !model.occluded(from, [from[0] + toward[0] * 80.0, from[1] + toward[1] * 80.0, from[2] + toward[2] * 80.0]);
            let (blocked, met) = field.sun_blocked(from, toward, 2, 200.0);
            if exact {
                lit += 1;
                if blocked < 0.5 {
                    let_through += 1;
                    continue;
                }
                for (t, level, cell, way, cover) in met {
                    let e = blockers.entry((level, cell, way)).or_insert((0, 0.0, 0.0));
                    e.0 += 1;
                    e.1 += cover;
                    e.2 = t;
                }
                continue;
            }
            shadowed += 1;
            if blocked >= 0.5 {
                continue;
            }
            leaked += 1;
            if let Some(h) = model.trace(from, toward, 80.0) {
                let t = ((h.point[0] - from[0]).powi(2) + (h.point[1] - from[1]).powi(2) + (h.point[2] - from[2]).powi(2)).sqrt();
                let near: f32 = met.iter().filter(|m| (m.0 - t).abs() < 1.0).map(|m| m.4).sum();
                let e = leaks.entry(h.point.map(floor_i32)).or_insert((0, 0.0, [0.0; 3], 0.0));
                e.0 += 1;
                e.1 += near;
                e.2 = h.normal;
                e.3 = t;
            }
        }
    }
    println!("sun toward {toward:?}, {found} floor points");
    println!("  sunlit by the exact ray: {lit}; the field lets {let_through} through and blocks {}", lit - let_through);
    println!("  shadowed by the exact ray: {shadowed}; the field leaks the sun onto {leaked}");
    let mut list: Vec<_> = blockers.into_iter().collect();
    list.sort_by_key(|e| std::cmp::Reverse(e.1 .0));
    println!("faces that block sunlit points:");
    for ((level, cell, way), (n, cover, t)) in list.into_iter().take(12) {
        let size = voxel_size(level);
        let at = cell.map(|c| (f64::from(c) + 0.5) * f64::from(size));
        println!("  level {level} ({size} m) cell {cell:?} at {at:?} way {way}: blocks {n} points, mean cover {:.2}, {t:.1} m along the ray", f64::from(cover) / f64::from(n));
    }
    let mut list: Vec<_> = leaks.into_iter().collect();
    list.sort_by_key(|e| std::cmp::Reverse(e.1 .0));
    println!("where the exact ray stops for leaked points (1 m cells):");
    for (cell, (n, near, normal, t)) in list.into_iter().take(12) {
        println!("  {cell:?}: {n} points, {t:.1} m up the ray, normal {normal:.2?}, field cover within 1 m of it {:.2}", f64::from(near) / f64::from(n));
    }
    Ok(())
}

