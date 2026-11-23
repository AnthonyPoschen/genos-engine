#![expect(
    clippy::as_conversions,
    reason = "pixel statistics against the path-traced reference"
)]
#![expect(
    clippy::cast_precision_loss,
    reason = "pixel statistics against the path-traced reference"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "LAMP_PICTURE_* shortens a smoke run; the reported measurement leaves them unset"
)]
#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

//! Report-only frames until a lamp change is accurate.
//!
//! Not a gate. A slow picture still passes. The metric is the oracle's
//! `ref_err_near` (`scripts/lighting/oracle_metrics.py`): on 2×2 blocks of
//! the whole frame, `sum(w |Ylive − Yref|) / sum(w Yref)` in scene-referred
//! Rec.709 luminance, `w = 1 / max(eye distance, 0.5 m)`. Direct light and
//! bounce are both in the picture. The reference is a CPU path trace of the
//! new lamp state at the cache depth (64 bounces), stopped at the cache-energy
//! acceptance noise (0.02), 4096 spp, or 600 s. The frozen bar is the turn
//! gate 0.403. It is not a new limit and this test does not assert it.
//!
//! `frames_90` is the first frame after the change that stays at or under
//! the line where 90 % of the error from the change is gone:
//! `E_tail + 0.1 * max(E0 − E_tail, 0)`, `E0` the frame before the change
//! scored against the new reference, `E_tail` the mean of the last 30 frames.
//! `frames_bar` is the first frame that stays at or under 0.403. `0` means
//! the picture was already inside that line before the change and never left.
//! Pop is the `wrc_lamps_off` band: the picture stays between the pre-change
//! level and where it settles, with slack of 1 % of the brighter of those two
//! (that brighter level is `lit` when a case starts lit). Flicker is a
//! reversal of that mean larger than the same 1 %.

use std::collections::HashMap;
use std::path::PathBuf;

use genos_debug::reference::{render_cached, RefSetup, Reference};
use genos_render::{Renderer, SceneRead};
use genos_scene::{
    Camera, DisplayTransform, ExposureMode, HdrSettings, LightingPreset, LightingSettings,
    OutputMode, Scene, Vec3,
};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 90;
/// 22:00 on the stage clock (0 is 06:00). Same night as `wrc_lamps_off`.
const NIGHT: f32 = 16.0 / 24.0;
const WARM_FRAMES: usize = 240;
const AFTER_FRAMES: usize = 240;
const TAIL: usize = 30;
/// Frozen turn gate (`docs/systems/lighting/adr-0039-remaining.md`). Not moved.
const BAR: f64 = 0.403;
const POP_SHARE: f64 = 0.01;
const BLOCK: usize = 2;
const NEAR_M: f64 = 0.5;
const NOISE_TARGET: f32 = 0.02;
const MAX_SPP: u32 = 4096;
const TRACE_SECONDS: f32 = 600.0;
const MAX_BOUNCES: u32 = 64;

struct Case {
    name: &'static str,
    from: f32,
    to: f32,
}

const CASES: [Case; 6] = [
    Case {
        name: "off_to_on",
        from: 0.0,
        to: 1.0,
    },
    Case {
        name: "ten_to_ninety",
        from: 0.1,
        to: 0.9,
    },
    Case {
        name: "on_to_off",
        from: 1.0,
        to: 0.0,
    },
    Case {
        name: "half",
        from: 1.0,
        to: 0.5,
    },
    Case {
        name: "double",
        from: 1.0,
        to: 2.0,
    },
    Case {
        name: "dim_10",
        from: 1.0,
        to: 0.9,
    },
];

struct Score {
    err_near: f64,
    err_plain: f64,
    near_mean: f64,
}

struct Sample {
    score: Score,
    byte_mean: f64,
}

struct Row {
    name: &'static str,
    frames_90: Option<usize>,
    frames_90_first: Option<usize>,
    frames_bar: Option<usize>,
    pop_byte: Option<usize>,
    pop_near: Option<usize>,
    flicker_byte: bool,
    flicker_near: bool,
    e0: f64,
    e_tail: f64,
    e_min: f64,
}

fn knob_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|text| text.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

fn knob_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|text| text.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

fn knob_f32(name: &str, default: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|text| text.parse().ok())
        .filter(|n: &f32| n.is_finite() && *n > 0.0)
        .unwrap_or(default)
}

fn ref_dir() -> PathBuf {
    std::env::var("LAMP_PICTURE_REF_DIR").map_or_else(
        |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/lamp-picture-refs"),
        PathBuf::from,
    )
}

fn selected(name: &str) -> bool {
    std::env::var("LAMP_PICTURE_ONLY")
        .ok()
        .filter(|text| !text.is_empty())
        .is_none_or(|text| text.split(',').any(|part| part == name))
}

fn grade_hdr() -> HdrSettings {
    // `legacy_bytes` skips the scene-referred store. Turning it off is what
    // writes the radiance `compare_linear` grades. Exposure stays fixed and
    // bloom stays off, so the store is the light itself.
    let mut hdr = HdrSettings::legacy_bytes();
    hdr.legacy_bytes = false;
    hdr.exposure_mode = ExposureMode::Fixed;
    hdr.ev100 = 0.0;
    hdr.display = DisplayTransform::Legacy;
    hdr.bloom = false;
    hdr.bloom_intensity = 0.0;
    hdr.bloom_levels = 0;
    hdr.output = OutputMode::Sdr;
    hdr
}

fn night(power: f32) -> (Scene, Camera, Stage) {
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
    stage.world.scene.hdr = grade_hdr();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    let scene = stage.world.scene.clone();
    (scene, camera, stage)
}

fn trace(power: f32) -> Reference {
    let (scene, camera, _) = night(power);
    let mut setup = RefSetup::from_camera(&scene, &camera, WIDTH, HEIGHT, 32);
    setup.max_bounces = MAX_BOUNCES;
    setup.noise_target = NOISE_TARGET;
    setup.max_spp = knob_u32("LAMP_PICTURE_MAX_SPP", MAX_SPP);
    setup.seconds = knob_f32("LAMP_PICTURE_SECONDS", TRACE_SECONDS);
    let dir = ref_dir();
    eprintln!(
        "lamp_picture trace power {power} bounces {MAX_BOUNCES} noise {NOISE_TARGET} max_spp {} seconds {} dir {}",
        setup.max_spp,
        setup.seconds,
        dir.display()
    );
    let started = std::time::Instant::now();
    let reference = render_cached(&setup, &dir);
    eprintln!(
        "lamp_picture trace power {power} spp {} noise {:.4} cached_seconds {:.1} wall {:.1}s",
        reference.spp,
        reference.noise,
        reference.seconds,
        started.elapsed().as_secs_f64()
    );
    reference
}

fn byte_mean(frame: &[u8]) -> f64 {
    let mut sum = 0u64;
    let mut n = 0u64;
    for px in frame.as_chunks::<4>().0 {
        sum += u64::from(px[0]) + u64::from(px[1]) + u64::from(px[2]);
        n += 3;
    }
    if n == 0 {
        0.0
    } else {
        sum as f64 / n as f64
    }
}

fn score_frame(scene: &SceneRead, reference: &Reference) -> Score {
    let width = scene.width as usize;
    let height = scene.height as usize;
    assert!(
        width == reference.image.width as usize
            && height == reference.image.height as usize
            && scene.rgb.len() >= width * height * 3
            && scene.distance_m.len() >= width * height
            && reference.linear.len() >= width * height,
        "scene read {}x{} does not match the reference {}x{}",
        scene.width,
        scene.height,
        reference.image.width,
        reference.image.height
    );
    let mut num_near = 0.0;
    let mut den_near = 0.0;
    let mut num_plain = 0.0;
    let mut den_plain = 0.0;
    let mut live_w = 0.0;
    let mut weight_sum = 0.0;
    let mut row = 0;
    while row + BLOCK <= height {
        let mut col = 0;
        while col + BLOCK <= width {
            let mut live = 0.0;
            let mut refer = 0.0;
            let mut dist = 0.0;
            let mut count = 0.0;
            for py in row..row + BLOCK {
                for px in col..col + BLOCK {
                    let index = py * width + px;
                    let at = index * 3;
                    live += genos_render::rec709_y([
                        scene.rgb[at],
                        scene.rgb[at + 1],
                        scene.rgb[at + 2],
                    ]);
                    let rgb = reference.linear[index];
                    refer += genos_render::rec709_y([
                        f64::from(rgb[0]),
                        f64::from(rgb[1]),
                        f64::from(rgb[2]),
                    ]);
                    dist += scene.distance_m[index];
                    count += 1.0;
                }
            }
            let live = live / count;
            let refer = refer / count;
            let weight = 1.0 / (dist / count).max(NEAR_M);
            let delta = (live - refer).abs();
            num_near += weight * delta;
            den_near += weight * refer;
            num_plain += delta;
            den_plain += refer;
            live_w += weight * live;
            weight_sum += weight;
            col += BLOCK;
        }
        row += BLOCK;
    }
    Score {
        err_near: num_near / (den_near + 1.0e-6),
        err_plain: num_plain / (den_plain + 1.0e-6),
        near_mean: if weight_sum > 0.0 {
            live_w / weight_sum
        } else {
            0.0
        },
    }
}

fn tail_mean(values: &[f64]) -> f64 {
    let n = TAIL.min(values.len()).max(1);
    let start = values.len() - n;
    values[start..].iter().sum::<f64>() / n as f64
}

/// First sample (1-based) of the trailing run that stays at or under `line`.
/// `0` when that run is the whole series and `before_inside` is set.
fn stays_under(values: &[f64], line: f64, before_inside: bool) -> Option<usize> {
    if values.last().is_none_or(|value| *value > line) {
        return None;
    }
    let mut i = values.len();
    while i > 0 && values[i - 1] <= line {
        i -= 1;
    }
    if i == 0 && before_inside {
        Some(0)
    } else {
        Some(i + 1)
    }
}

fn first_under(values: &[f64], line: f64) -> Option<usize> {
    values
        .iter()
        .position(|value| *value <= line)
        .map(|i| i + 1)
}

fn outside_band(before: f64, trail: &[f64]) -> Option<usize> {
    let end = tail_mean(trail);
    let slack = POP_SHARE * before.abs().max(end.abs());
    let lo = before.min(end) - slack;
    let hi = before.max(end) + slack;
    trail
        .iter()
        .position(|value| *value < lo || *value > hi)
        .map(|i| i + 1)
}

/// A reversal whose step exceeds 1 % of the brighter steady level.
fn flicker(before: f64, trail: &[f64]) -> (bool, f64) {
    let end = tail_mean(trail);
    let lit = before.abs().max(end.abs());
    let slack = POP_SHARE * lit;
    let mut max_step: f64 = 0.0;
    let mut prev = 0.0;
    let mut flickered = false;
    for pair in trail.windows(2) {
        let step = pair[1] - pair[0];
        max_step = max_step.max(step.abs());
        if step.abs() <= slack {
            continue;
        }
        if prev * step < 0.0 {
            flickered = true;
        }
        prev = step;
    }
    (flickered, if lit > 0.0 { max_step / lit } else { 0.0 })
}

fn fmt_frame(frame: Option<usize>) -> String {
    frame.map_or_else(|| "never".to_string(), |n| n.to_string())
}

fn yes_no(flag: bool) -> &'static str {
    if flag { "yes" } else { "no" }
}

fn draw_sample(
    window: &mut Window,
    renderer: &mut Renderer,
    stage: &Stage,
    camera: &Camera,
    reference: &Reference,
) -> Sample {
    let _ = window.pump();
    let pixels = renderer
        .draw(&stage.world, camera, true)
        .expect("draw")
        .expect("pixels");
    let scene = renderer.read_scene().expect("scene read");
    assert!(!scene.rgb.is_empty(), "scene image is empty");
    Sample {
        score: score_frame(&scene, reference),
        byte_mean: byte_mean(&pixels),
    }
}

fn measure(case: &Case, reference: &Reference, warm: usize, after: usize) -> Row {
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let size = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, size.width, size.height).expect("renderer");
    let mut owner = genos_window::Lifecycle::new(window, renderer, permit);
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    let (_, camera, mut stage) = night(case.from);
    let mut before = Sample {
        score: Score {
            err_near: 0.0,
            err_plain: 0.0,
            near_mean: 0.0,
        },
        byte_mean: 0.0,
    };
    for frame in 1..=warm {
        before = draw_sample(window, renderer, &stage, &camera, reference);
        if frame % 60 == 0 {
            eprintln!(
                "lamp_picture {} warm {frame} ref_err_near {:.4} byte {:.2}",
                case.name, before.score.err_near, before.byte_mean
            );
        }
    }
    stage.power = case.to;
    stage.apply();
    let mut trail = Vec::with_capacity(after);
    for frame in 1..=after {
        let sample = draw_sample(window, renderer, &stage, &camera, reference);
        if frame % 30 == 0 || frame <= 8 {
            eprintln!(
                "lamp_picture {} frame {frame} ref_err_near {:.4} plain {:.4} byte {:.2} near_mean {:.4}",
                case.name,
                sample.score.err_near,
                sample.score.err_plain,
                sample.byte_mean,
                sample.score.near_mean
            );
        }
        trail.push(sample);
    }
    if case.from > 0.0 {
        assert!(
            before.score.near_mean > 1.0e-4,
            "{}: scene-referred luminance {:.6} after warm-up; the grade image was not stored",
            case.name,
            before.score.near_mean
        );
    }
    let errors: Vec<f64> = trail.iter().map(|sample| sample.score.err_near).collect();
    let bytes: Vec<f64> = trail.iter().map(|sample| sample.byte_mean).collect();
    let nears: Vec<f64> = trail.iter().map(|sample| sample.score.near_mean).collect();
    let e0 = before.score.err_near;
    let e_tail = tail_mean(&errors);
    let e_min = errors.iter().copied().fold(f64::MAX, f64::min);
    let gap = (e0 - e_tail).max(0.0);
    let line = e_tail + 0.1 * gap;
    let frames_90 = stays_under(&errors, line, e0 <= line);
    let frames_90_first = first_under(&errors, line);
    let frames_bar = stays_under(&errors, BAR, e0 <= BAR);
    let pop_byte = outside_band(before.byte_mean, &bytes);
    let pop_near = outside_band(before.score.near_mean, &nears);
    let (flicker_byte, step_byte) = flicker(before.byte_mean, &bytes);
    let (flicker_near, step_near) = flicker(before.score.near_mean, &nears);
    let early = errors.len().saturating_sub(60);
    let mid = errors.len().saturating_sub(30);
    let early_mean = if mid > early {
        errors[early..mid].iter().sum::<f64>() / (mid - early) as f64
    } else {
        e_tail
    };
    eprintln!(
        "lamp_picture {name}: frames_90 {f90} (first {first}) frames_bar {fbar} e0 {e0:.4} line {line:.4} e_tail {e_tail:.4} e_min {e_min:.4} pop_byte {pb} pop_near {pn} flicker_byte {fb} flicker_near {fn_} step_byte {step_byte:.4} step_near {step_near:.4} tail_delta {tail_delta:+.4} ref_spp {spp} ref_noise {noise:.4}",
        name = case.name,
        f90 = fmt_frame(frames_90),
        first = fmt_frame(frames_90_first),
        fbar = fmt_frame(frames_bar),
        pb = fmt_frame(pop_byte),
        pn = fmt_frame(pop_near),
        fb = yes_no(flicker_byte),
        fn_ = yes_no(flicker_near),
        tail_delta = e_tail - early_mean,
        spp = reference.spp,
        noise = reference.noise,
    );
    Row {
        name: case.name,
        frames_90,
        frames_90_first,
        frames_bar,
        pop_byte,
        pop_near,
        flicker_byte,
        flicker_near,
        e0,
        e_tail,
        e_min,
    }
}

fn print_table(rows: &[Row]) {
    eprintln!(
        "lamp_picture_table case frames_90 frames_90_first frames_bar pop_byte pop_near flicker_byte flicker_near e0 e_tail e_min"
    );
    for row in rows {
        eprintln!(
            "lamp_picture_table {name} {f90} {first} {fbar} {pb} {pn} {fb} {fn_} {e0:.4} {tail:.4} {emin:.4}",
            name = row.name,
            f90 = fmt_frame(row.frames_90),
            first = fmt_frame(row.frames_90_first),
            fbar = fmt_frame(row.frames_bar),
            pb = fmt_frame(row.pop_byte),
            pn = fmt_frame(row.pop_near),
            fb = yes_no(row.flicker_byte),
            fn_ = yes_no(row.flicker_near),
            e0 = row.e0,
            tail = row.e_tail,
            emin = row.e_min,
        );
    }
}

#[test]
#[ignore = "lavapipe draws plus CPU path traces; report only, not a gate"]
fn lamp_picture_time() {
    let warm = knob_usize("LAMP_PICTURE_WARM", WARM_FRAMES);
    let after = knob_usize("LAMP_PICTURE_AFTER", AFTER_FRAMES);
    let mut refs: HashMap<u32, Reference> = HashMap::new();
    for case in &CASES {
        if !selected(case.name) {
            continue;
        }
        refs.entry(case.to.to_bits()).or_insert_with(|| trace(case.to));
    }
    let mut rows = Vec::new();
    for case in &CASES {
        if !selected(case.name) {
            continue;
        }
        let reference = refs.get(&case.to.to_bits()).expect("traced new state");
        rows.push(measure(case, reference, warm, after));
    }
    print_table(&rows);
    assert!(!rows.is_empty(), "no lamp case selected");
}
