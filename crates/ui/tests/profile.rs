//! Profiler gate, frame graph, and the flushed profile file.

use std::time::Duration;

use genos_render::{Renderer, ScreenRect, World};
use genos_scene::{Camera, Floor, Scene, Vec3};
use genos_ui::{
    profile_overlay, profiler_enabled, FrameSample, ProfileStream, STAGE_COUNT, STAGE_LABELS,
};
use genos_window::Window;

#[test]
fn the_profiler_follows_proof_or_a_frame_limit() {
    assert!(profiler_enabled(true, None));
    assert!(profiler_enabled(false, Some(0)));
    assert!(profiler_enabled(true, Some(3)));
    assert!(!profiler_enabled(false, None));
}

#[test]
fn the_graph_shows_the_latest_rate_and_one_line_per_series() {
    let calm = [Duration::from_millis(1); STAGE_COUNT];
    let mut spiked_cpu = calm;
    spiked_cpu[2] = Duration::from_millis(30);
    let draw_gpu = Duration::from_micros(200);
    let history = vec![
        FrameSample::from_stages(Duration::from_millis(8), calm, draw_gpu),
        FrameSample::from_stages(Duration::from_millis(40), spiked_cpu, draw_gpu),
        FrameSample::from_stages(Duration::from_millis(9), calm, draw_gpu),
    ];
    for sample in &history {
        for (index, stage) in sample.stages.iter().enumerate() {
            assert_eq!(stage.label, STAGE_LABELS[index]);
            if stage.label == "gpu draw/present" {
                assert_eq!(stage.gpu, draw_gpu);
            } else {
                assert_eq!(stage.gpu, Duration::ZERO);
            }
        }
    }
    assert!(history[1].duration > history[0].duration);
    assert!(history[1].frame_rate < history[0].frame_rate);
    assert_eq!(history[1].stages[2].cpu, spiked_cpu[2]);

    let view = profile_overlay(&history, [1280.0, 720.0]);
    let latest = history.last().expect("history");
    assert_eq!(view.frame_rate, latest.frame_rate);
    assert_eq!(view.readout, format!("{:.1}", latest.frame_rate));
    assert_eq!(view.lines.len(), STAGE_COUNT * 2);
    let mut names = Vec::new();
    let mut peak_y = f32::MAX;
    let mut peak_name = String::new();
    for line in &view.lines {
        assert_eq!(line.points.len(), history.len());
        assert!(names.iter().all(|name| name != &line.name));
        names.push(line.name.clone());
        for point in &line.points {
            assert_ne!(point.color, view.background.color);
            if point.y < peak_y {
                peak_y = point.y;
                peak_name = line.name.clone();
            }
        }
    }
    assert_eq!(peak_name, "ui cpu");
    let spike = view
        .lines
        .iter()
        .find(|line| line.name == "ui cpu")
        .expect("ui cpu line");
    assert!(spike.points[1].y < spike.points[0].y);
    assert!(spike.points[1].y < spike.points[2].y);
    assert_eq!(spike.points[1].nanos, spiked_cpu[2].as_nanos());
    for line in &view.lines {
        if line.name == "ui cpu" {
            continue;
        }
        assert!(
            line.points[1].y > spike.points[1].y,
            "{} rose with the ui spike",
            line.name
        );
    }
    assert!(view.paints.len() > view.lines.len() * history.len());
}

#[test]
fn the_stream_flushes_each_frame_and_keeps_a_slow_stage() {
    let path = std::env::temp_dir().join(format!("genos-profile-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut stream = ProfileStream::create(&path).expect("create");
    let calm = [Duration::from_millis(1); STAGE_COUNT];
    let fast = FrameSample::from_stages(Duration::from_millis(10), calm, Duration::from_micros(50));
    stream.append(&fast).expect("append fast");
    let after_first = std::fs::read_to_string(&path).expect("read first");
    let first = parse_frames(&after_first);
    assert_eq!(first.len(), 1);
    assert_frame(&first[0], &fast);

    let mut slow_cpu = calm;
    slow_cpu[4] = Duration::from_millis(80);
    let slow = FrameSample::from_stages(
        Duration::from_millis(90),
        slow_cpu,
        Duration::from_micros(50),
    );
    stream.append(&slow).expect("append slow");
    let after_second = std::fs::read_to_string(&path).expect("read second");
    let both = parse_frames(&after_second);
    assert_eq!(both.len(), 2);
    assert_frame(&both[0], &fast);
    assert_frame(&both[1], &slow);
    assert!(both[1].duration > both[0].duration);
    let slow_stage = both[1]
        .stages
        .iter()
        .find(|stage| stage.label == "simulation step")
        .expect("simulation step");
    assert!(slow_stage.cpu > both[0].stages[4].cpu);
    assert!(both[1]
        .stages
        .iter()
        .all(|stage| stage.label == "simulation step" || stage.cpu < slow_stage.cpu));
    let _ = std::fs::remove_file(&path);
}

struct ParsedStage {
    label: String,
    cpu: u128,
    gpu: u128,
}

struct ParsedFrame {
    rate: f64,
    duration: u128,
    stages: Vec<ParsedStage>,
}

fn parse_frames(text: &str) -> Vec<ParsedFrame> {
    let mut frames = Vec::new();
    let mut rate = None;
    let mut duration = None;
    let mut stages = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            if rate.is_some() {
                frames.push(ParsedFrame {
                    rate: rate.take().unwrap(),
                    duration: duration.take().unwrap(),
                    stages: std::mem::take(&mut stages),
                });
            }
            continue;
        }
        let mut parts = line.split('\t');
        match parts.next().unwrap() {
            "frame_rate" => rate = Some(parts.next().unwrap().parse().unwrap()),
            "duration_ns" => duration = Some(parts.next().unwrap().parse().unwrap()),
            "stage" => stages.push(ParsedStage {
                label: parts.next().unwrap().to_string(),
                cpu: parts.next().unwrap().parse().unwrap(),
                gpu: parts.next().unwrap().parse().unwrap(),
            }),
            other => panic!("unexpected profile line {other}"),
        }
    }
    frames
}

#[test]
fn the_graph_paint_is_a_different_color_from_its_background() {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    let mut window = Window::open_proof(1280, 720).expect("proof window");
    let frame = window.pump();
    let mut renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    let world = World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 8.0,
            half_z: 8.0,
            color: [1.0, 1.0, 1.0],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
    });
    let camera = Camera::opening();
    let calm = [Duration::from_millis(1); STAGE_COUNT];
    let mut spiked = calm;
    spiked[0] = Duration::from_millis(25);
    let history = vec![
        FrameSample::from_stages(Duration::from_millis(8), calm, Duration::from_micros(100)),
        FrameSample::from_stages(
            Duration::from_millis(30),
            spiked,
            Duration::from_micros(100),
        ),
    ];
    let view = profile_overlay(
        &history,
        [renderer.width() as f32, renderer.height() as f32],
    );
    let paints: Vec<ScreenRect> = view
        .paints
        .iter()
        .map(|paint| ScreenRect {
            x: paint.x,
            y: paint.y,
            w: paint.w,
            h: paint.h,
            color: paint.color,
        })
        .collect();
    let _ = window.pump();
    let pixels = renderer
        .draw_with_overlay(&world, &camera, &paints, true)
        .expect("draw")
        .expect("readback");
    let width = renderer.width();
    let background = pixel(
        &pixels,
        width,
        view.background.x + view.background.w - 4.0,
        view.background.y + 4.0,
    );
    let point = view
        .lines
        .iter()
        .flat_map(|line| line.points.iter())
        .min_by(|left, right| {
            left.y
                .partial_cmp(&right.y)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .expect("line point");
    let ink = pixel(&pixels, width, point.x + 3.0, point.y + 3.0);
    assert_ne!(background, ink, "line point matched the graph background");
    assert!(
        closer(ink, point.color, view.background.color),
        "line pixel {ink:?} was not the line color {:?} against {:?}",
        point.color,
        view.background.color
    );
    assert!(
        closer(background, view.background.color, point.color),
        "background pixel {background:?} was not the graph background"
    );
}

fn pixel(pixels: &[u8], width: u32, x: f32, y: f32) -> [u8; 3] {
    let x = x.round() as u32;
    let y = y.round() as u32;
    let index = ((y * width + x) * 4) as usize;
    [pixels[index + 2], pixels[index + 1], pixels[index]]
}

fn closer(pixel: [u8; 3], want: [f32; 3], other: [f32; 3]) -> bool {
    distance(pixel, want) < distance(pixel, other)
}

fn distance(pixel: [u8; 3], color: [f32; 3]) -> f32 {
    let channel = |index: usize| pixel[index] as f32 - color[index] * 255.0;
    channel(0) * channel(0) + channel(1) * channel(1) + channel(2) * channel(2)
}

fn assert_frame(parsed: &ParsedFrame, sample: &FrameSample) {
    assert_eq!(parsed.rate, sample.frame_rate);
    assert_eq!(parsed.duration, sample.duration.as_nanos());
    assert_eq!(parsed.stages.len(), sample.stages.len());
    for (parsed_stage, stage) in parsed.stages.iter().zip(sample.stages.iter()) {
        assert_eq!(parsed_stage.label, stage.label);
        assert_eq!(parsed_stage.cpu, stage.cpu.as_nanos());
        assert_eq!(parsed_stage.gpu, stage.gpu.as_nanos());
    }
}
