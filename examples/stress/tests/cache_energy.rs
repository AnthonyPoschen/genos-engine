#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]
#![cfg_attr(
    test,
    expect(
        clippy::disallowed_methods,
        reason = "the renderer reads GENOS_LIGHTING_DUMP; this test is the argument that asks for the dump, and CACHE_ENERGY_* are harness knobs"
    )
)]
#![allow(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "pixel statistics in a test"
)]

//! Surface-cache energy against the path-traced reference (ADR 0037, stage 1
//! of the lighting simplification).
//!
//! A still scene in the default surface cache must settle at the light the
//! CPU path trace finds: the bounce view (indirect light only) over the
//! reference's indirect (64 bounces minus direct only) is between 0.9 and
//! 1.1 on the pixels the reference hits, in a closed box, an open floor and
//! the stress hall at night. The direct view against the direct-only trace
//! checks the harness (the oracle: the same pixels, units and exposure).
//!
//! The cache itself is graded too: the 25 cm cells (and 12.5 cm texels) in
//! a lighting dump against the CPU bounce irradiance at each cell's point and
//! normal (`genos_debug::reference::bounce_irradiance`). Their sum and their
//! median lit cell must hold at least 0.9 of it, and a still scene must settle.
//! A fourth case starts the hall dark and grades it 300 lit frames later (lag).
//! The picture ratio is the energy metric of `crates/cache-ref` (summed cache
//! over summed reference on the pixels the reference hits).
//!
//! Per scene the test also reports blotch (mean relative error of 8x8 block
//! means of the indirect) and the still cost: cache batch entries per frame
//! (16 rays each) over the last 120 frames, the counter proxy for the idle
//! relight cost on lavapipe, where milliseconds mean nothing.
//!
//! Lavapipe and minutes of CPU tracing, so ignored by default:
//! `genos-box-test cargo test --release -p genos-stress --test cache_energy -- --ignored --nocapture --test-threads=1`.
//! The reference is cached in `target/reference-cache`.

use genos_render::{DebugView, Renderer, ViewMode, World};
use genos_scene::{Camera, LightingPreset, LightingSettings, Scene, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 90;
/// 22:00 on the stage clock (0 is 06:00).
const NIGHT: f32 = 16.0 / 24.0;
/// Still frames before grading: three times the old 300-frame freeze.
const FRAMES: usize = 900;
const LOW: f64 = 0.9;
/// The highest picture byte read back: 0.745 on the shoulder, linear 3.9.
const SHOULDER_TOP: u8 = 190;
const HIGH: f64 = 1.1;

fn luma(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn scene_text(name: &str, text: &str) -> Scene {
    let dir = std::env::temp_dir().join(format!("cache-energy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{name}.rhai"));
    std::fs::write(&path, text).expect("scene file");
    genos_scene::load_path(&path).expect("scene")
}

/// 6 x 6 x 3 m closed room, 10 cm walls, three grey blocks (the scene
/// script wants three solids), one warm lamp in a corner, no sun: every
/// bounce stays inside.
fn closed_box() -> (Scene, Camera) {
    let scene = scene_text(
        "box",
        "floor(0.0, 0.0, 6.0, 6.0);\n\
         roof(3.0, 0.9, 0.9, 0.9);\n\
         wall(0.0, -3.0, 6.1, 0.1, 3.0);\n\
         wall(-3.0, 0.0, 0.1, 6.1, 3.0);\n\
         wall(0.0, 3.0, 6.1, 0.1, 3.0);\n\
         wall(3.0, 0.0, 0.1, 6.1, 3.0);\n\
         solid(\"square\", 0.6, 0.4, 0.5, 0.7, 0.7, 0.7);\n\
         solid(\"square\", -0.8, 1.0, 0.4, 0.7, 0.7, 0.7);\n\
         solid(\"circle\", 1.2, -1.0, 0.3, 0.7, 0.7, 0.7);\n\
         light(-1.5, 2.2, -1.5, 0.4, 0.375, 0.325);\n",
    );
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(2.2, 1.6, 2.2), -2.356, -0.15);
    (scene, camera)
}

/// A 24 m floor, open above, with one wall (and a low far one: the script
/// wants two), three grey blocks and a lamp beside the wall: light that leaves the scene does not come back.
fn open_floor() -> (Scene, Camera) {
    let scene = scene_text(
        "floor",
        "floor(0.0, 0.0, 24.0, 24.0);\n\
         wall(0.0, 0.0, 8.0, 0.2, 3.0);\n\
         wall(-9.0, 4.0, 0.2, 3.0, 1.0);\n\
         solid(\"square\", -2.0, 2.5, 0.5, 0.7, 0.7, 0.7);\n\
         solid(\"square\", 2.5, 3.0, 0.4, 0.7, 0.7, 0.7);\n\
         solid(\"circle\", 0.5, 4.0, 0.3, 0.7, 0.7, 0.7);\n\
         light(0.0, 1.5, 1.2, 0.8, 0.75, 0.65);\n",
    );
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(0.0, 1.7, 6.0), 0.0, -0.12);
    (scene, camera)
}

/// The stress hall at 22:00 with 32 still lamps, the drain tests' pose.
fn hall() -> (Scene, Camera) {
    let (stage, camera) = hall_stage(1.0);
    (stage.world.scene.clone(), camera)
}

fn hall_stage(power: f32) -> (Stage, Camera) {
    let mix = LightMix {
        count: 32,
        dynamic_pct: 0,
        layout: Layout::Spread,
    };
    let mut stage = Stage::new(Scale::SMALL, 1, mix, 0.0, NIGHT, 120.0);
    stage.sun_frozen = true;
    stage.boxes_still = true;
    stage.lamps_still = true;
    stage.power = power;
    stage.apply();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    (stage, camera)
}

fn reference(scene: &Scene, camera: &Camera, bounces: u32) -> genos_debug::Reference {
    let mut setup = genos_debug::reference::RefSetup::from_camera(scene, camera, WIDTH, HEIGHT, 32);
    setup.max_bounces = bounces;
    setup.noise_target = 0.02;
    setup.max_spp = 4096;
    setup.seconds = 600.0;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/reference-cache");
    genos_debug::reference::render_cached(&setup, &dir)
}

/// Linear luma per pixel of the picture in `mode`: the scene keeps the legacy
/// picture (exposure 1, no gamma, the shoulder above 0.64). A pixel with a
/// channel near the shoulder's ceiling comes back as `None`.
fn read(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera, mode: ViewMode) -> Vec<Option<f64>> {
    renderer.set_debug_view(DebugView { mode, lamp: 0 });
    let mut pixels = Vec::new();
    for _ in 0..2 {
        let _ = window.pump();
        pixels = renderer.draw(world, camera, true).expect("draw").expect("pixels");
    }
    renderer.set_debug_view(DebugView::default());
    assert_eq!(pixels.len(), (WIDTH * HEIGHT * 4) as usize, "picture size");
    // The shoulder is undone. Its top bytes are too coarse to undo (and 0.78,
    // byte 199, is its ceiling), so a pixel with a channel above `SHOULDER_TOP`
    // is left out.
    let untone = |b: u8| {
        let v = f64::from(b) / 255.0;
        if v <= 0.64 {
            v
        } else {
            let x = v - 0.64;
            1.1 * x / (0.14 - x) + 0.64
        }
    };
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|px| px[..3].iter().all(|&b| b <= SHOULDER_TOP).then(|| luma([untone(px[2]), untone(px[1]), untone(px[0])])))
        .collect()
}

struct Grade {
    indirect: f64,
    direct: f64,
    blotch: f64,
}

fn grade(live_bounce: &[Option<f64>], live_direct: &[Option<f64>], b0: &genos_debug::Reference, b64: &genos_debug::Reference) -> Grade {
    let ref_luma = |r: &genos_debug::Reference, i: usize| {
        let c = r.linear[i];
        luma([f64::from(c[0]), f64::from(c[1]), f64::from(c[2])])
    };
    let (mut lb, mut ld, mut ri, mut rd) = (0.0, 0.0, 0.0, 0.0);
    for i in 0..live_bounce.len() {
        if !b64.hit[i] {
            continue;
        }
        let (d, full) = (ref_luma(b0, i), ref_luma(b64, i));
        if let Some(v) = live_bounce[i] {
            lb += v;
            ri += (full - d).max(0.0);
        }
        if let Some(v) = live_direct[i] {
            ld += v;
            rd += d;
        }
    }
    // Blotch: 8x8 blocks, |live - ref| / ref of the indirect, weighted by ref.
    let (mut err, mut wsum) = (0.0, 0.0);
    for by in (0..HEIGHT).step_by(8) {
        for bx in (0..WIDTH).step_by(8) {
            let (mut l, mut r, mut n) = (0.0, 0.0, 0);
            for y in by..(by + 8).min(HEIGHT) {
                for x in bx..(bx + 8).min(WIDTH) {
                    let i = (y * WIDTH + x) as usize;
                    if let (true, Some(v)) = (b64.hit[i], live_bounce[i]) {
                        l += v;
                        r += (ref_luma(b64, i) - ref_luma(b0, i)).max(0.0);
                        n += 1;
                    }
                }
            }
            if n >= 32 && r > 0.0 {
                err += (l - r).abs();
                wsum += r;
            }
        }
    }
    Grade {
        indirect: lb / ri.max(1e-12),
        direct: ld / rd.max(1e-12),
        blotch: err / wsum.max(1e-12),
    }
}

fn run(name: &str, scene: Scene, camera: &Camera) {
    run_from(name, None, scene, camera, FRAMES);
}

/// `dark`: a world drawn first for that many frames (the lamps off), so the
/// graded frames measure the cache filling from black (the lag gate).
#[expect(clippy::too_many_lines, reason = "one linear harness run: draw, grade, report, assert")]
fn run_from(name: &str, dark: Option<(&World, usize)>, scene: Scene, camera: &Camera, default_frames: usize) {
    let b0 = reference(&scene, camera, 0);
    let b64 = reference(&scene, camera, 64);
    eprintln!(
        "cache_energy {name}: reference b0 spp {} noise {:.4}, b64 spp {} noise {:.4}",
        b0.spp, b0.noise, b64.spp, b64.noise
    );
    let frames_env = std::env::var("CACHE_ENERGY_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(default_frames);
    let dark_frames = dark.as_ref().map_or(0, |d| d.1);
    let dump_at = (dark_frames + frames_env) as u64 - 8;
    let dump_dir = std::env::temp_dir().join(format!("cache-energy-dump-{name}-{}", std::process::id()));
    std::env::set_var("GENOS_LIGHTING_DUMP", dump_at.to_string());
    std::env::set_var("GENOS_LIGHTING_DUMP_DIR", &dump_dir);
    let cell_scene = scene.clone();
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let size = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, size.width, size.height).expect("renderer");
    let mut owner = genos_window::Lifecycle::new(window, renderer, permit);
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    if let Some((dark, n)) = dark {
        for _ in 0..n {
            let _ = window.pump();
            renderer.draw(dark, camera, true).expect("draw");
        }
    }
    let world = World::from_scene(scene);
    let mut entries = Vec::new();
    let mut draining = 0u32;
    let mut settled_at = None;
    let mut trail = Vec::new();
    // A debug view is part of the frame's scene hash, so a trail read restarts the
    // still count: off by default, CACHE_ENERGY_TRAIL turns it on.
    let trail_on = std::env::var_os("CACHE_ENERGY_TRAIL").is_some();
    let frames = frames_env;
    for f in 1..=frames {
        let _ = window.pump();
        renderer.draw(&world, camera, true).expect("draw");
        let s = renderer.settle_stats();
        if s.settled && settled_at.is_none() {
            settled_at = Some(f);
        }
        if f + 120 > frames {
            entries.push(f64::from(s.round_entries));
            draining += u32::from(s.draining);
        }
        if trail_on && f % 150 == 0 {
            let bounce = read(window, renderer, &world, camera, ViewMode::Bounce);
            let direct = read(window, renderer, &world, camera, ViewMode::Direct);
            let g = grade(&bounce, &direct, &b0, &b64);
            trail.push(format!("{f}:{:.3}", g.indirect));
        }
    }
    let bounce = read(window, renderer, &world, camera, ViewMode::Bounce);
    let direct = read(window, renderer, &world, camera, ViewMode::Direct);
    let g = grade(&bounce, &direct, &b0, &b64);
    if let Ok(dir) = std::env::var("CACHE_ENERGY_PGM") {
        let ref_l = |r: &genos_debug::Reference| -> Vec<Option<f64>> {
            r.linear.iter().map(|c| Some(luma([f64::from(c[0]), f64::from(c[1]), f64::from(c[2])]))).collect()
        };
        for (tag, img) in [("live_direct", direct.clone()), ("live_bounce", bounce.clone()), ("ref_b0", ref_l(&b0)), ("ref_b64", ref_l(&b64))] {
            let mut out = format!("P2\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
            for v in img {
                out.extend(format!("{} ", v.map_or(255, |v| (v.clamp(0.0, 1.0).sqrt() * 254.0) as u32)).bytes());
            }
            std::fs::write(format!("{dir}/{name}_{tag}.pgm"), out).expect("pgm");
        }
    }
    let cost = entries.iter().sum::<f64>() / entries.len().max(1) as f64;
    let cells = cell_grade(&cell_scene, &dump_dir, dump_at, name);
    eprintln!(
        "cache_energy {name}: cells/ref {:.3} (n {}, median {:.3}, lit median {:.3}, under 0.9: {:.2}) texels/ref {:.3} (n {}, median {:.3}) | picture indirect/ref {:.3} direct/ref {:.3} blotch {:.3} settled at {settled_at:?} still entries/frame {cost:.0} ({:.0} rays) draining {draining}/120 trail [{}]",
        cells.coarse.ratio,
        cells.coarse.n,
        cells.coarse.median,
        cells.coarse.median_lit,
        cells.coarse.under,
        cells.fine.ratio,
        cells.fine.n,
        cells.fine.median,
        g.indirect,
        g.direct,
        g.blotch,
        cost * 16.0,
        trail.join(" ")
    );
    assert!(
        (0.95..=1.05).contains(&g.direct),
        "{name}: direct/ref {:.3}: the harness does not match the reference (oracle)",
        g.direct
    );
    assert!(
        cells.coarse.n >= 32 && cells.coarse.ratio >= LOW,
        "{name}: 25 cm cells hold {:.3} of the reference bounce light (n {}), under {LOW}",
        cells.coarse.ratio,
        cells.coarse.n
    );
    // The sum can pass while single cells are a few rays' noise (dark ones and
    // bright ones cancel): the median lit cell holds the reference too. Dim
    // cells (under LIT of the mean) are reported, not graded: their median is
    // ray noise, the blotch gate's job.
    assert!(
        cells.coarse.median_lit >= LOW,
        "{name}: the median lit 25 cm cell holds {:.3} of its reference bounce, under {LOW} (all cells {:.3}, {:.2} under)",
        cells.coarse.median_lit,
        cells.coarse.median,
        cells.coarse.under
    );
    if dark.is_none() {
        assert!(
            settled_at.is_some(),
            "{name}: a still scene never settled in {frames} frames ({cost:.0} entries a frame, draining {draining}/120)"
        );
    }
    assert!(
        (LOW..=HIGH).contains(&g.indirect),
        "{name}: indirect/ref {:.3} outside [{LOW}, {HIGH}]",
        g.indirect
    );
}

/// One size class of cache slots against the CPU bounce irradiance at each
/// slot's point and normal.
#[derive(Default)]
struct SlotGrade {
    /// Sum of slot light over sum of reference light (luma).
    ratio: f64,
    n: usize,
    /// Median of the per-slot ratios, over slots with reference light.
    median: f64,
    /// Share of those slots under `LOW`.
    under: f64,
    /// Median ratio over the lit slots: reference at least `LIT` of the mean
    /// (about 95 % of the reference light in the open floor).
    median_lit: f64,
}

struct CellGrade {
    coarse: SlotGrade,
    fine: SlotGrade,
}

/// Slots graded per size class (deterministic stride over the live ones).
const CELL_SAMPLES: usize = 384;
/// A dumped slot: point, normal, stored irradiance, sets.
type Slot = ([f32; 3], [f32; 3], [f32; 3], f32);

/// A slot whose reference is under this share of the mean is dim: its few
/// bounce paths make its value noise (a far floor cell sees the lit wall in one
/// ray of a thousand), which is blotch, not missing energy.
const LIT: f64 = 0.5;

/// Cosine rays per slot for the CPU irradiance.
const CELL_RAYS: u32 = 1024;

/// The dumped cache (`GENOS_LIGHTING_DUMP`, frame `at`; layout as
/// tools/lighting/dump.py): every live 25 cm cell and 12.5 cm texel's stored
/// irradiance against [`genos_debug::reference::bounce_irradiance`] at its
/// point and normal, 64 bounces.
fn cell_grade(scene: &Scene, dir: &std::path::Path, at: u64, tag: &str) -> CellGrade {
    let layout = std::fs::read_to_string(dir.join(format!("f{at}_layout.json"))).expect("lighting dump layout");
    let slots: usize = layout
        .split("\"cache_slots\": ")
        .nth(1)
        .and_then(|t| t.trim_end_matches(|c: char| !c.is_ascii_digit()).split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|t| t.parse().ok())
        .expect("cache_slots");
    let words = |name: &str| -> Vec<u32> {
        std::fs::read(dir.join(format!("f{at}_{name}.bin")))
            .expect("lighting dump")
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b))
            .collect()
    };
    let keys = words("keys");
    let cache = words("cache");
    let f = |i: usize| f32::from_bits(cache[i]);
    let vec3 = |row: usize| [f(4 * row), f(4 * row + 1), f(4 * row + 2)];
    let mut coarse = Vec::new();
    let mut fine = Vec::new();
    for (i, _) in keys.iter().take(slots).enumerate().filter(|(_, k)| **k != 0) {
        let size = f(4 * (3 * i + 1) + 3);
        let sets = f(4 * (3 * i + 2) + 3);
        if sets.is_nan() || sets < 1.0 {
            continue;
        }
        let slot = (vec3(3 * i), vec3(3 * i + 1), vec3(3 * i + 2), sets);
        if size > 0.13 {
            coarse.push(slot);
        } else {
            fine.push(slot);
        }
    }
    let _ = std::fs::remove_dir_all(dir);
    let grade = |all: Vec<Slot>, dir_tag: String| -> SlotGrade {
        if all.is_empty() {
            return SlotGrade::default();
        }
        let step = all.len().div_ceil(CELL_SAMPLES);
        let picked: Vec<_> = all.into_iter().step_by(step).collect();
        let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
        let refs: Vec<f64> = std::thread::scope(|s| {
            let chunk = picked.len().div_ceil(threads);
            let handles: Vec<_> = picked
                .chunks(chunk)
                .enumerate()
                .map(|(c, part)| {
                    s.spawn(move || {
                        part.iter()
                            .enumerate()
                            .map(|(k, (p, n, _, _))| {
                                let e = genos_debug::reference::bounce_irradiance(scene, *p, *n, CELL_RAYS, 64, (c * chunk + k) as u64);
                                luma([f64::from(e[0]), f64::from(e[1]), f64::from(e[2])])
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().expect("trace")).collect()
        });
        let live: Vec<f64> = picked.iter().map(|(_, _, v, _)| luma([f64::from(v[0]), f64::from(v[1]), f64::from(v[2])])).collect();
        if let Ok(dir) = std::env::var("CACHE_ENERGY_CSV") {
            use std::fmt::Write as _;
            let mut out = String::from("x,y,z,nx,ny,nz,sets,live,ref\n");
            for ((p, n, _, sets), (l, r)) in picked.iter().zip(live.iter().zip(&refs)) {
                let _ = writeln!(out, "{},{},{},{},{},{},{sets},{l},{r}", p[0], p[1], p[2], n[0], n[1], n[2]);
            }
            let path = format!("{dir}/{}_{}.csv", dir_tag, picked.len());
            std::fs::write(path, out).expect("csv");
        }
        let total_ref: f64 = refs.iter().sum();
        let total_live: f64 = live.iter().sum();
        let floor = total_ref / refs.len() as f64 * 0.05;
        let mut per: Vec<f64> = live.iter().zip(&refs).filter(|(_, r)| **r > floor).map(|(l, r)| l / r).collect();
        per.sort_by(f64::total_cmp);
        let lit = total_ref / refs.len() as f64 * LIT;
        let mut per_lit: Vec<f64> = live.iter().zip(&refs).filter(|(_, r)| **r > lit).map(|(l, r)| l / r).collect();
        per_lit.sort_by(f64::total_cmp);
        SlotGrade {
            median_lit: per_lit.get(per_lit.len() / 2).copied().unwrap_or(0.0),
            ratio: total_live / total_ref.max(1e-12),
            n: picked.len(),
            median: per.get(per.len() / 2).copied().unwrap_or(0.0),
            under: per.iter().filter(|r| **r < LOW).count() as f64 / per.len().max(1) as f64,
        }
    };
    CellGrade {
        coarse: grade(coarse, format!("{tag}_coarse")),
        fine: grade(fine, format!("{tag}_fine")),
    }
}

#[test]
#[ignore = "lavapipe and a CPU path trace, minutes"]
fn a_closed_box_reaches_the_reference_bounce() {
    let (scene, camera) = closed_box();
    run("box", scene, &camera);
}

#[test]
#[ignore = "lavapipe and a CPU path trace, minutes"]
fn an_open_floor_reaches_the_reference_bounce() {
    let (scene, camera) = open_floor();
    run("floor", scene, &camera);
}

#[test]
#[ignore = "lavapipe and a CPU path trace, minutes"]
fn the_night_hall_reaches_the_reference_bounce() {
    let (scene, camera) = hall();
    run("hall", scene, &camera);
}

/// The lag gate: the hall dark for 300 frames, then lit; at lit frame 300 the
/// cells and the picture hold at least 0.9 of the reference bounce.
#[test]
#[ignore = "lavapipe and a CPU path trace, minutes"]
fn the_night_hall_fills_from_dark_in_300_frames() {
    let (dark, _) = hall_stage(0.0);
    let (scene, camera) = hall();
    run_from("hall_lag", Some((&dark.world, 300)), scene, &camera, 300);
}
