//! Lighting correctness on any Vulkan device, lavapipe included.
//!
//! These tests check only correctness, never timing. They run headless under a
//! Wayland compositor (`tools/lighting/headless.sh` starts Weston on its headless
//! backend and runs them on Mesa's lavapipe). The scene is the small stress
//! building with a fixed seed: sun frozen at 12:12, sky on, five still lamps, still
//! boxes. Each frame steps exactly one scripted camera pose, so a run draws the
//! same frames on any machine.
//!
//! The lighting buffers come from the existing debug dump
//! (`GENOS_LIGHTING_DUMP`), which copies the GPU buffers once at a chosen frame.
//! The frame loop has no other readback.
//!
//! `GENOS_GI_TEST_SETTLE` sets the frames held at the final pose (default
//! [`SETTLE`]); `GENOS_GI_TEST_OUT` keeps the dumps and pictures in that directory;
//! `GENOS_GI_TEST_DUMPS` (comma-separated frames) dumps more frames of each run.

use std::path::{Path, PathBuf};

use genos_render::Renderer;
use genos_scene::{Camera, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
/// 12:12 as a fraction of the stress day (0 is 06:00).
const DAY: f32 = (12.2 - 6.0) / 24.0;
/// Frames held at the final pose after the walk.
const SETTLE: u32 = 300;
/// The walk: from the hall's south-west, walking and turning to the bench view.
const WALK_FROM: Pose = Pose {
    position: [6.0, 1.7, 30.0],
    yaw: 2.1,
    pitch: 0.0,
};
const BENCH: Pose = Pose {
    position: [11.59, 1.7, 23.52],
    yaw: -1.0,
    pitch: -0.25,
};
const WALK_FRAMES: u32 = 90;

/// Slots in the light cache (lighting_cache_slots.glsl).
const SLOTS: usize = 1 << 20;
/// Fine texels are 12.5 cm; world radiance cache cells are 25 cm.
const FINE_SIZE: f32 = 0.13;

#[derive(Clone, Copy)]
struct Pose {
    position: [f32; 3],
    yaw: f32,
    pitch: f32,
}

impl Pose {
    fn lerp(a: Pose, b: Pose, t: f32) -> Pose {
        let m = |x: f32, y: f32| x + (y - x) * t;
        Pose {
            position: [
                m(a.position[0], b.position[0]),
                m(a.position[1], b.position[1]),
                m(a.position[2], b.position[2]),
            ],
            yaw: m(a.yaw, b.yaw),
            pitch: m(a.pitch, b.pitch),
        }
    }

    fn camera(self) -> Camera {
        let [x, y, z] = self.position;
        let mut camera = Camera::new(x, z, self.yaw);
        camera.set_pose(Vec3::new(x, y, z), self.yaw, self.pitch);
        camera
    }
}

fn settle_frames() -> u32 {
    std::env::var("GENOS_GI_TEST_SETTLE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(SETTLE)
}

fn out_dir(name: &str) -> PathBuf {
    let base = std::env::var_os("GENOS_GI_TEST_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("genos-gi-headless"));
    let dir = base.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("output directory");
    dir
}

fn stage() -> Stage {
    let mut stage = Stage::new(
        Scale::SMALL,
        1,
        LightMix {
            count: 5,
            dynamic_pct: 0,
            layout: Layout::Spread,
        },
        1.0,
        DAY,
        120.0,
    );
    stage.sun_frozen = true;
    stage.boxes_still = true;
    stage.lamps_still = true;
    stage.apply();
    stage
}

/// One slot of the light cache as the dump holds it (lighting_cache.glsl).
#[derive(Clone, Copy, Debug)]
struct Texel {
    key: u32,
    position: [f32; 3],
    last_used: u32,
    size: f32,
    rgb: [f32; 3],
    sets: f32,
}

impl Texel {
    fn luminance(&self) -> f32 {
        0.2126 * self.rgb[0] + 0.7152 * self.rgb[1] + 0.0722 * self.rgb[2]
    }

    fn fine(&self) -> bool {
        self.size <= FINE_SIZE
    }
}

struct Run {
    /// BGRA8 picture of the last frame.
    picture: Vec<u8>,
    /// Frame the dump was taken at (the lighting's frame counter).
    frame: u32,
    texels: Vec<Texel>,
    /// Every float of the dumped cache, work, G-buffer and output buffers that is
    /// not finite: (buffer, count).
    non_finite: Vec<(String, usize)>,
}

fn read_f32(path: &Path) -> Vec<f32> {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    raw.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn texels(cache: &[f32], keys: &[u32]) -> Vec<Texel> {
    (0..SLOTS)
        .filter(|&i| keys[i] != 0)
        .map(|i| {
            let a = &cache[12 * i..12 * i + 12];
            Texel {
                key: keys[i],
                position: [a[0], a[1], a[2]],
                last_used: a[3].to_bits(),
                size: a[7],
                rgb: [a[8], a[9], a[10]],
                sets: a[11],
            }
        })
        .collect()
}

/// Draw `frames` frames, frame `i` from `pose(i)`, and dump the lighting buffers
/// three frames before the end.
fn run(name: &str, frames: u32, pose: impl Fn(u32) -> Pose) -> Run {
    let dir = out_dir(name);
    let dump_at = frames.saturating_sub(5).max(1);
    let mut dumps = dump_at.to_string();
    if let Ok(extra) = std::env::var("GENOS_GI_TEST_DUMPS") {
        dumps = format!("{extra},{dumps}");
    }
    // The dump is read every frame (lighting_dump_copy); set before the first draw.
    std::env::set_var("GENOS_LIGHTING_DUMP", dumps);
    std::env::set_var("GENOS_LIGHTING_DUMP_DIR", &dir);
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let size = window.pump();
    let mut renderer =
        Renderer::open(window.display, window.surface, size.width, size.height).expect("renderer");
    assert_eq!((size.width, size.height), (WIDTH, HEIGHT), "the window keeps its size");
    let stage = stage();
    let mut picture = Vec::new();
    for i in 0..frames {
        let _ = window.pump();
        let last = i + 1 == frames;
        let got = renderer
            .draw(&stage.world, &pose(i).camera(), last)
            .expect("draw");
        if let Some(p) = got {
            picture = p;
        }
    }
    drop(renderer);
    drop(window);
    std::env::remove_var("GENOS_LIGHTING_DUMP");
    let frame = dump_at;
    let at = |part: &str| dir.join(format!("f{frame}_{part}.bin"));
    let cache = read_f32(&at("cache"));
    let keys: Vec<u32> = read_f32(&at("keys")).iter().map(|f| f.to_bits()).collect();
    let mut non_finite = Vec::new();
    let live: Vec<usize> = (0..SLOTS).filter(|&i| keys[i] != 0).collect();
    let bad = live
        .iter()
        .flat_map(|&i| cache[12 * i + 8..12 * i + 12].iter())
        .filter(|v| !v.is_finite())
        .count();
    non_finite.push(("cache texel light".to_string(), bad));
    for part in ["out"] {
        let bad = read_f32(&at(part)).iter().filter(|v| !v.is_finite()).count();
        non_finite.push((part.to_string(), bad));
    }
    write_png(&dir.join("final.png"), &picture);
    Run {
        picture,
        frame,
        texels: texels(&cache, &keys),
        non_finite,
    }
}

fn write_png(path: &Path, bgra: &[u8]) {
    let _ = std::fs::write(path, genos_debug::png::encode_png(WIDTH, HEIGHT, bgra));
}

fn mean_level(bgra: &[u8]) -> f64 {
    bgra.chunks_exact(4)
        .map(|p| (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0)
        .sum::<f64>()
        / (bgra.len() / 4).max(1) as f64
}

fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return f32::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

/// Fine texels the last frames' pixels read (the ones on screen).
fn on_screen(run: &Run) -> impl Iterator<Item = &Texel> {
    run.texels
        .iter()
        .filter(move |t| t.fine() && run.frame.wrapping_sub(t.last_used) <= 2)
}

#[test]
#[ignore = "GPU: run with tools/lighting/headless.sh (lavapipe) or on a desktop"]
fn gi_still_frames_are_finite_and_filled() {
    let frames = settle_frames().min(120);
    let run = run("still", frames, |_| BENCH);
    for (part, bad) in &run.non_finite {
        assert_eq!(*bad, 0, "{part}: {bad} values are NaN or infinite");
    }
    let shown: Vec<&Texel> = on_screen(&run).collect();
    let empty = shown.iter().filter(|t| t.sets < 0.5).count();
    let black = shown
        .iter()
        .filter(|t| t.sets >= 0.5 && t.luminance() <= 0.0)
        .count();
    eprintln!(
        "frame {}: {} texels live, {} fine on screen, {empty} without a set, {black} black, picture mean {:.1}",
        run.frame,
        run.texels.len(),
        shown.len(),
        mean_level(&run.picture)
    );
    assert!(shown.len() > 1000, "the view reads fine texels ({})", shown.len());
    assert!(
        empty * 100 <= shown.len(),
        "after {} frames at most 1 % of the shown texels have no set ({empty} of {})",
        run.frame,
        shown.len()
    );
    assert_eq!(black, 0, "no lit texel is black");
    let black_pixels = run
        .picture
        .chunks_exact(4)
        .filter(|p| p[0] == 0 && p[1] == 0 && p[2] == 0)
        .count();
    assert_eq!(black_pixels, 0, "no pixel of the noon picture is black");
}

/// A camera that walked to a pose ends with the same light as one that sat there
/// all along: the fine texels both runs hold, and the picture.
#[test]
#[ignore = "GPU: run with tools/lighting/headless.sh (lavapipe) or on a desktop"]
fn gi_moved_camera_matches_never_moved() {
    let settle = settle_frames();
    let frames = WALK_FRAMES + settle;
    let reference = run("never-moved", frames, |_| BENCH);
    let moved = run("moved", frames, |i| {
        if i < WALK_FRAMES {
            Pose::lerp(WALK_FROM, BENCH, i as f32 / WALK_FRAMES as f32)
        } else {
            BENCH
        }
    });
    for run in [&reference, &moved] {
        for (part, bad) in &run.non_finite {
            assert_eq!(*bad, 0, "{part}: {bad} values are NaN or infinite");
        }
    }
    let by_key: std::collections::HashMap<u32, &Texel> =
        reference.texels.iter().map(|t| (t.key, t)).collect();
    let mut ratios = Vec::new();
    let mut bright_far = Vec::new();
    for t in on_screen(&moved) {
        if let Some(r) = by_key.get(&t.key) {
            if r.sets >= 0.5 && t.sets >= 0.5 && r.luminance() > 1.0e-3 {
                let ratio = t.luminance() / r.luminance();
                ratios.push(ratio);
                if ratio > 1.1 {
                    let d: f32 = (0..3)
                        .map(|k| (t.position[k] - BENCH.position[k]).powi(2))
                        .sum();
                    bright_far.push(d.sqrt());
                }
            }
        }
    }
    let n = ratios.len();
    let bright = ratios.iter().filter(|&&r| r > 1.1).count();
    eprintln!(
        "bright texels' distance from the eye: median {:.1} m",
        median(bright_far)
    );
    let dark = ratios.iter().filter(|&&r| r < 1.0 / 1.1).count();
    let med = median(ratios);
    let fine = |r: &Run| r.texels.iter().filter(|t| t.fine()).count();
    let screen = mean_level(&moved.picture) - mean_level(&reference.picture);
    eprintln!(
        "frame {}: fine texels moved {} / never moved {}; {n} shared on screen, median ratio {med:.3}, {bright} over 1.1, {dark} under 1/1.1; screen {screen:+.2} levels",
        moved.frame,
        fine(&moved),
        fine(&reference),
    );
    assert!(n > 1000, "the runs share the texels on screen ({n})");
    assert!(
        (med - 1.0).abs() < 0.03,
        "shared fine texels: median moved / never-moved {med:.3}"
    );
    assert!(
        bright * 10 < n,
        "{bright} of {n} shared fine texels are over 10 % brighter after the walk"
    );
    assert!(
        screen.abs() < 2.0,
        "the moved picture is {screen:+.2} levels off the never-moved one"
    );
}
