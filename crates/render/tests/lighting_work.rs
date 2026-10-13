//! Phase 0 of the moving-scene work cut: the full-effort picture floor, the
//! phase 1 stop rule, and the small-sun compose check.
//!
//! Two cold runs of the corner scene, then a third. The floor is the mean
//! absolute linear-luminance difference of the first pair, for the sum, the
//! direct view, and the bounce view. The third run has to land inside that
//! floor. Pop is the fill-run metric on the same frames. A later sun-only
//! step checks that a small directional change does not dirty the whole
//! compose list.

use genos_render::{
    linear_from_display, DebugView, Renderer, ViewMode, World,
};
use genos_scene::{Camera, Ceiling, Floor, Light, Scene, Shape, Solid, Vec3, Wall};
use genos_window::Window;

const WARM: usize = 4;
const MOVE: usize = 4;
const STOP: usize = 4;
const FRAMES: usize = WARM + MOVE + STOP;
const POP_STEP: f64 = 0.01;

fn open() -> (std::sync::MutexGuard<'static, ()>, Window, Renderer) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    let mut window = Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, window, renderer)
}

fn wall(x: f32, z: f32, half_x: f32, half_z: f32) -> Wall {
    Wall {
        position: Vec3::new(x, 0.0, z),
        half_x,
        half_z,
        height: 2.6,
        base: 0.0,
        color: [1.0, 1.0, 1.0],
        absorption: 0.0,
        reflectance: 0.35,
        color_mix: 1.0,
    }
}

/// The scene loader wants three solids. These sit behind the camera.
fn pad_solid(shape: Shape, x: f32, z: f32, size: f32) -> Solid {
    Solid {
        shape,
        position: Vec3::new(x, 0.0, z),
        size,
        height: 1.2,
        yaw: 0.0,
        color: [0.02, 0.02, 0.02],
        absorption: 0.0,
        reflectance: 0.02,
        color_mix: 0.0,
    }
}

/// Lamp place for one frame. Still, then a short walk toward the corner, then still.
fn lamp_at(frame: usize) -> Vec3 {
    let start = Vec3::new(1.6, 1.55, 1.5);
    let end = Vec3::new(1.1, 1.55, 1.15);
    if frame < WARM {
        start
    } else if frame < WARM + MOVE {
        let t = (frame - WARM + 1) as f32 / MOVE as f32;
        start + (end - start) * t
    } else {
        end
    }
}

fn corner_world(frame: usize) -> World {
    World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(1.5, 0.0, 1.5),
            half_x: 3.0,
            half_z: 3.0,
            color: [1.0, 1.0, 1.0],
        },
        walls: vec![wall(1.5, 0.08, 1.5, 0.08), wall(0.08, 1.5, 0.08, 1.5)],
        solids: vec![
            pad_solid(Shape::Square, 4.2, 3.4, 0.25),
            pad_solid(Shape::Square, 3.4, 4.2, 0.25),
            pad_solid(Shape::Circle, 4.2, 4.2, 0.2),
        ],
        lights: vec![Light {
            position: lamp_at(frame),
            color: [8.0, 7.2, 5.6],
            direction: Vec3::ZERO,
        }],
        ceiling: Some(Ceiling {
            height: 2.8,
            color: [0.12, 0.12, 0.14],
        }),
        sky: None,
    })
}

fn corner_camera() -> Camera {
    let eye = Vec3::new(2.4, 1.55, 2.3);
    let at = Vec3::new(0.25, 0.55, 0.25);
    let d = at - eye;
    let pitch = (d.y / d.length()).asin();
    let yaw = d.x.atan2(-d.z);
    let mut camera = Camera::opening();
    camera.set_pose(eye, yaw, pitch);
    camera
}

fn draw(window: &mut Window, renderer: &mut Renderer, frame: usize, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    let world = corner_world(frame);
    renderer
        .draw(&world, camera, true)
        .expect("draw")
        .expect("readback")
}

fn luma(bgra: &[u8]) -> Vec<f64> {
    bgra.chunks_exact(4)
        .map(|p| {
            let r = linear_from_display(p[2] as f32 / 255.0) as f64;
            let g = linear_from_display(p[1] as f32 / 255.0) as f64;
            let b = linear_from_display(p[0] as f32 / 255.0) as f64;
            0.2126 * r + 0.7152 * g + 0.0722 * b
        })
        .collect()
}

fn mean_abs(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .sum::<f64>()
        / a.len().max(1) as f64
}

/// Share of pixel-frames whose linear luminance steps more than 0.01 from the
/// 9-frame average. The fill-run metric, on however many frames this harness drew.
fn pop_share(frames: &[Vec<f64>]) -> f64 {
    let n = frames.len();
    if n < 9 {
        return 0.0;
    }
    let px = frames[0].len();
    let mut over = 0usize;
    let mut count = 0usize;
    for t in 4..n - 4 {
        for i in 0..px {
            let ma = (t - 4..=t + 4).map(|k| frames[k][i]).sum::<f64>() / 9.0;
            over += usize::from((frames[t][i] - ma).abs() > POP_STEP);
            count += 1;
        }
    }
    over as f64 / count.max(1) as f64
}

fn run(
    window: &mut Window,
    renderer: &mut Renderer,
    mode: ViewMode,
) -> (Vec<Vec<u8>>, Vec<Vec<f64>>) {
    renderer.reset_lighting_caches();
    renderer.set_debug_view(DebugView {
        mode,
        ..DebugView::default()
    });
    let camera = corner_camera();
    let mut pictures = Vec::with_capacity(FRAMES);
    let mut planes = Vec::with_capacity(FRAMES);
    for frame in 0..FRAMES {
        let picture = draw(window, renderer, frame, &camera);
        planes.push(luma(&picture));
        pictures.push(picture);
    }
    (pictures, planes)
}

fn pair_floor(a: &[Vec<f64>], b: &[Vec<f64>]) -> f64 {
    assert_eq!(a.len(), b.len());
    let mut sum = 0.0;
    for (left, right) in a.iter().zip(b) {
        sum += mean_abs(left, right);
    }
    sum / a.len().max(1) as f64
}

fn encode_png(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((width * 3 + 1) * height) as usize);
    for y in 0..height {
        raw.push(0);
        let row = (y * width * 3) as usize;
        raw.extend_from_slice(&rgb[row..row + (width * 3) as usize]);
    }
    let mut zlib = Vec::new();
    zlib.extend_from_slice(&[0x78, 0x01]);
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    let mut rest = raw.as_slice();
    while !rest.is_empty() {
        let take = rest.len().min(65535);
        let last = take == rest.len();
        zlib.push(if last { 1 } else { 0 });
        zlib.extend_from_slice(&(take as u16).to_le_bytes());
        zlib.extend_from_slice(&(!take as u16).to_le_bytes());
        zlib.extend_from_slice(&rest[..take]);
        for byte in &rest[..take] {
            s1 = (s1 + u32::from(*byte)) % 65521;
            s2 = (s2 + s1) % 65521;
        }
        rest = &rest[take..];
    }
    zlib.extend_from_slice(&((s2 << 16) | s1).to_be_bytes());
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let crc = crc32(&out[4..]);
        out.extend_from_slice(&crc.to_be_bytes());
        out
    }
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        !crc
    }
    let mut png = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    png.extend(chunk(b"IHDR", &ihdr));
    png.extend(chunk(b"IDAT", &zlib));
    png.extend(chunk(b"IEND", &[]));
    png
}

fn rgb_of(bgra: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(bgra.len() / 4 * 3);
    for p in bgra.chunks_exact(4) {
        rgb.push(p[2]);
        rgb.push(p[1]);
        rgb.push(p[0]);
    }
    rgb
}

/// Two rows of bounce frames: the two full-effort runs, at the last still
/// frame, the middle of the move, and the first stopped frame.
fn write_capture(a: &[Vec<u8>], b: &[Vec<u8>]) {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let dir = std::path::PathBuf::from(home).join("genos-perf/bounce-corner");
    std::fs::create_dir_all(&dir).expect("capture dir");
    let picks = [WARM - 1, WARM + MOVE / 2, WARM + MOVE];
    let (w, h) = (320usize, 180usize);
    let cols = picks.len();
    let mut rgb = vec![0u8; cols * w * 2 * h * 3];
    for (row, run) in [a, b].into_iter().enumerate() {
        for (col, frame) in picks.into_iter().enumerate() {
            let src = rgb_of(&run[frame]);
            for y in 0..h {
                let dst = ((row * h + y) * cols * w + col * w) * 3;
                let from = y * w * 3;
                rgb[dst..dst + w * 3].copy_from_slice(&src[from..from + w * 3]);
            }
        }
    }
    let png = encode_png((cols * w) as u32, (2 * h) as u32, &rgb);
    let sheet = dir.join("full-effort-bounce-sheet.png");
    std::fs::write(&sheet, png).expect("sheet");
    let watch = dir.join("WATCH.txt");
    std::fs::write(
        &watch,
        "\
Bounce corner, lavapipe, 320x180. This sheet is two full-effort runs of the bounce view.
Columns: last still frame, middle of the lamp move, first frame the lamp has stopped.
Rows: run A and run B. They are the reference pair. A reduced run is not in this sheet yet.

On your PC, windowed, 1920x1080, watch the corner where the two walls meet, not the lamp bulb.
Direct light and the shadow edge must move with the lamp on the same frame.
The bounce in that corner may keep the old tint for a few frames after the lamp stops, then catch up.

cargo run --release -p genos-stress -- --scene examples/stress/scenes/bounce_corner.rhai --script examples/stress/scripts/bounce_corner.rhai --size 1920x1080 --no-panel --report ~/genos-perf/bounce-corner
GENOS_LIGHTING_FULL_EFFORT=1 cargo run --release -p genos-stress -- --scene examples/stress/scenes/bounce_corner.rhai --script examples/stress/scripts/bounce_corner.rhai --size 1920x1080 --no-panel --report ~/genos-perf/bounce-corner-full
",
    )
    .expect("watch");
    eprintln!("capture {}", sheet.display());
}

#[test]
fn full_effort_picture_floor_holds_a_third_run() {
    let scene_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/stress/scenes/bounce_corner.rhai");
    let scene = genos_scene::load_path(&scene_path).expect("corner scene loads");
    assert_eq!(scene.lights.len(), 1, "one lamp");
    assert!(scene.sky.is_none(), "no sky");
    assert!(scene.ceiling.is_some(), "a roof");
    assert!(scene.solids.len() >= 3, "the loader wants three solids");
    let (_gpu, mut window, mut renderer) = open();
    renderer.set_lighting_work(1, true);
    let mut floors = [0.0; 3];
    let mut pops = [0.0; 3];
    let mut bounce_a = Vec::new();
    let mut bounce_b = Vec::new();
    for (i, mode) in [ViewMode::Full, ViewMode::Direct, ViewMode::Bounce]
        .into_iter()
        .enumerate()
    {
        // The first cold start after the window opens can diverge from the
        // later ones. It is not one of the three measured runs.
        let _ = run(&mut window, &mut renderer, mode);
        let (pics_a, a) = run(&mut window, &mut renderer, mode);
        let (pics_b, b) = run(&mut window, &mut renderer, mode);
        let (_pics_c, c) = run(&mut window, &mut renderer, mode);
        let pairs = [pair_floor(&a, &b), pair_floor(&a, &c), pair_floor(&b, &c)];
        let floor = pairs.into_iter().fold(0.0_f64, f64::max);
        let least = pairs.into_iter().fold(floor, f64::min);
        let pop = [pop_share(&a), pop_share(&b), pop_share(&c)];
        let pop_hi = pop.into_iter().fold(0.0_f64, f64::max);
        let pop_lo = pop.into_iter().fold(pop_hi, f64::min);
        eprintln!(
            "phase0 {} pairs {:.6} {:.6} {:.6} floor {floor:.6} least {least:.6} pop {:.4} {:.4} {:.4}",
            mode.name(),
            pairs[0],
            pairs[1],
            pairs[2],
            pop[0] * 100.0,
            pop[1] * 100.0,
            pop[2] * 100.0
        );
        assert!(
            floor <= least * 3.0 + 5.0e-4,
            "{} full-effort pairs {:.6} {:.6} {:.6} disagree",
            mode.name(),
            pairs[0],
            pairs[1],
            pairs[2]
        );
        assert!(
            pop_hi - pop_lo <= 0.005,
            "{} pop spread {:.4}% across full-effort runs",
            mode.name(),
            (pop_hi - pop_lo) * 100.0
        );
        floors[i] = floor;
        pops[i] = pop_hi;
        renderer.set_lighting_work(1, false);
        let (_pics_p, phase) = run(&mut window, &mut renderer, mode);
        renderer.set_lighting_work(1, true);
        let phase_pairs = [
            pair_floor(&phase, &a),
            pair_floor(&phase, &b),
            pair_floor(&phase, &c),
        ];
        let band = [
            pairs[0], pairs[1], pairs[2], phase_pairs[0], phase_pairs[1], phase_pairs[2],
        ];
        let band_hi = band.into_iter().fold(0.0_f64, f64::max);
        let band_lo = band.into_iter().fold(band_hi, f64::min);
        let pop_p = pop_share(&phase);
        eprintln!(
            "phase1 {} diffs {:.6} {:.6} {:.6} pop {:.4}",
            mode.name(),
            phase_pairs[0],
            phase_pairs[1],
            phase_pairs[2],
            pop_p * 100.0
        );
        assert!(
            band_hi <= band_lo * 3.0 + 5.0e-4,
            "{} phase 1 leaves the full-effort band ({:.6} vs {:.6})",
            mode.name(),
            band_hi,
            band_lo
        );
        assert!(
            pop_p <= pop_hi + 0.005 && pop_p + 0.005 >= pop_lo,
            "{} phase 1 pop {:.4}% is outside the full-effort pop",
            mode.name(),
            pop_p * 100.0
        );
        if mode == ViewMode::Bounce {
            bounce_a = pics_a;
            bounce_b = pics_b;
        }
    }
    write_capture(&bounce_a, &bounce_b);
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let floors_path = std::path::PathBuf::from(home).join("genos-perf/bounce-corner/phase0-floors.txt");
    std::fs::write(
        &floors_path,
        format!(
            "phase 0 full-effort floors, mean |ΔL|, linear luminance, 320x180 lavapipe\nsum {:.6}\ndirect {:.6}\nbounce {:.6}\npop percent sum {:.3} direct {:.3} bounce {:.3}\n",
            floors[0], floors[1], floors[2], pops[0] * 100.0, pops[1] * 100.0, pops[2] * 100.0
        ),
    )
    .expect("floors");
    eprintln!(
        "phase0 floors sum {:.6} direct {:.6} bounce {:.6} pop% {:.3} {:.3} {:.3}",
        floors[0],
        floors[1],
        floors[2],
        pops[0] * 100.0,
        pops[1] * 100.0,
        pops[2] * 100.0
    );

    // Stop rule: after a still warmup, a moving frame keeps at least half its
    // surface pixels settled. Below that, later phases do not start.
    renderer.set_lighting_work(1, false);
    renderer.reset_lighting_caches();
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Full,
        ..DebugView::default()
    });
    let camera = corner_camera();
    for _ in 0..24 {
        let _ = draw(&mut window, &mut renderer, 0, &camera);
    }
    for step in 0..MOVE - 1 {
        let _ = draw(&mut window, &mut renderer, WARM + step, &camera);
    }
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Regime,
        ..DebugView::default()
    });
    let regime = draw(&mut window, &mut renderer, WARM + MOVE - 1, &camera);
    let stats = renderer.lighting_work_stats();
    let frac = stats.settled_pixels as f64 / stats.pixels.max(1) as f64;
    eprintln!(
        "phase1 moving settled {:.3} pixels {}/{} missing {} fringe {} pages {}/{} probes {}/{}",
        frac,
        stats.settled_pixels,
        stats.pixels,
        stats.missing,
        stats.fringe,
        stats.settled_pages,
        stats.pages,
        stats.settled_probes,
        stats.probes
    );
    assert!(stats.pixels > 1000, "regime counters saw {} pixels", stats.pixels);
    assert!(
        frac >= 0.5,
        "stop rule: {:.1}% of moving-frame pixels are settled, under half",
        frac * 100.0
    );
    let mut green = 0usize;
    let mut yellow = 0usize;
    let mut pixels = 0usize;
    for p in regime.chunks_exact(4) {
        pixels += 1;
        if p[1] > p[0] && p[1] > p[2] && p[1] > 80 {
            green += 1;
        } else if p[2] > p[1] && p[1] > p[0] && p[2] > 160 {
            yellow += 1;
        }
    }
    eprintln!("phase1 regime green {green} yellow {yellow} / {pixels}");
    assert!(green * 2 > pixels, "regime view is not mostly settled");
    assert!(yellow > 0, "regime view shows no fringe on the moving frame");
}

/// A 0.2° sun step stays on the compose list. A 40° jump does not.
/// Direct light on the step matches a full-effort replay.
#[test]
fn small_sun_step_stays_on_the_compose_list() {
    let blocks = 40 * 23;
    let base = 0.4_f32;
    let step = 0.2_f32.to_radians();
    let (_gpu, mut window, mut renderer) = open();
    let camera = corner_camera();
    renderer.set_lighting_work(1, false);
    renderer.reset_lighting_caches();
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        ..DebugView::default()
    });
    // Eleven still frames. Frame 12 is slice 4: one probe row of the twelve.
    for _ in 0..10 {
        let _ = draw_sun(&mut window, &mut renderer, base, &camera);
    }
    let _ = draw_sun(&mut window, &mut renderer, base, &camera);
    let still_stats = renderer.lighting_work_stats();
    let still_atrous = renderer.lighting_atrous_stats();
    eprintln!(
        "sun still blocks {} sun {} atrous {:.4} ran {} filtered {}",
        still_stats.delta_skip,
        still_stats.stochastic,
        still_atrous.dirty_fraction,
        still_atrous.ran,
        still_atrous.filtered
    );
    let reduced = draw_sun(&mut window, &mut renderer, base + step, &camera);
    let sun_stats = renderer.lighting_work_stats();
    let atrous = renderer.lighting_atrous_stats();
    eprintln!(
        "sun step blocks {} sun {} atrous {:.4} ran {} filtered {}",
        sun_stats.delta_skip,
        sun_stats.stochastic,
        atrous.dirty_fraction,
        atrous.ran,
        atrous.filtered
    );
    assert_eq!(sun_stats.stochastic, 1, "the small sun path was off");
    assert_ne!(
        sun_stats.delta_skip, u32::MAX,
        "a 0.2° sun step took a full compose"
    );
    assert!(
        sun_stats.delta_skip < blocks / 2,
        "sun step marked {} of {blocks} compose blocks",
        sun_stats.delta_skip
    );
    assert!(
        !atrous.ran || atrous.dirty_fraction <= 0.25,
        "sun step a-trous dirty fraction {:.4}",
        atrous.dirty_fraction
    );
    let _jump = draw_sun(&mut window, &mut renderer, base + 40.0_f32.to_radians(), &camera);
    let jump_stats = renderer.lighting_work_stats();
    eprintln!(
        "sun jump blocks {} sun {}",
        jump_stats.delta_skip, jump_stats.stochastic
    );
    assert_eq!(
        jump_stats.delta_skip, u32::MAX,
        "a 40° sun jump stayed on the compose list ({})",
        jump_stats.delta_skip
    );
    assert_eq!(jump_stats.stochastic, 0, "a 40° sun jump took the small path");

    renderer.set_lighting_work(1, true);
    renderer.reset_lighting_caches();
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        ..DebugView::default()
    });
    for _ in 0..11 {
        let _ = draw_sun(&mut window, &mut renderer, base, &camera);
    }
    let full = draw_sun(&mut window, &mut renderer, base + step, &camera);
    let direct = mean_abs(&luma(&reduced), &luma(&full));
    eprintln!("sun step direct |ΔL| {direct:.6}");
    assert!(
        direct <= 5.0e-4,
        "small sun direct |ΔL| {direct:.6} is outside the phase 0 floor"
    );
}

/// Same room as the corner, one directional sun, no point lamp.
fn sun_world(yaw: f32) -> World {
    let dir = Vec3::new(yaw.sin(), -0.75, yaw.cos()).normalize();
    World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(1.5, 0.0, 1.5),
            half_x: 3.0,
            half_z: 3.0,
            color: [1.0, 1.0, 1.0],
        },
        walls: vec![wall(1.5, 0.08, 1.5, 0.08), wall(0.08, 1.5, 0.08, 1.5)],
        solids: vec![
            pad_solid(Shape::Square, 4.2, 3.4, 0.25),
            pad_solid(Shape::Square, 3.4, 4.2, 0.25),
            pad_solid(Shape::Circle, 4.2, 4.2, 0.2),
        ],
        lights: vec![Light {
            position: Vec3::ZERO,
            color: [6.0, 5.4, 4.2],
            direction: dir,
        }],
        ceiling: Some(Ceiling {
            height: 2.8,
            color: [0.12, 0.12, 0.14],
        }),
        sky: None,
    })
}

fn draw_sun(window: &mut Window, renderer: &mut Renderer, yaw: f32, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(&sun_world(yaw), camera, true)
        .expect("draw")
        .expect("readback")
}
