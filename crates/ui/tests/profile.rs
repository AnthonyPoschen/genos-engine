//! Profiler window, held graph, and the flushed profile file.

use std::time::{Duration, Instant};

use genos_render::{Renderer, ScreenRect, World};
use genos_scene::{Camera, Floor, Scene, Vec3};
use genos_ui::{
    profile_overlay, remember_frame, FrameSample, ProfileGraph, ProfileStream, GRAPH_WINDOW,
    OVERLAY_PERIOD, STAGE_COUNT, STAGE_LABELS,
};
use genos_window::Window;

const VIEW: [f32; 2] = [1280.0, 720.0];

fn calm() -> [Duration; STAGE_COUNT] {
    [Duration::from_millis(1); STAGE_COUNT]
}

fn at(time: Duration, cpu: [Duration; STAGE_COUNT], gpu: Duration) -> FrameSample {
    FrameSample::at(time, Duration::from_millis(8), cpu, gpu)
}

#[test]
fn the_graph_keeps_a_ten_second_window() {
    let frame = Duration::from_millis(8);
    let gpu = Duration::from_micros(100);
    let newest = Duration::from_secs(90);
    let mut history = Vec::new();
    remember_frame(
        &mut history,
        FrameSample::at(Duration::ZERO, frame, calm(), gpu),
    );
    remember_frame(
        &mut history,
        FrameSample::at(
            newest - GRAPH_WINDOW - Duration::from_millis(1),
            frame,
            calm(),
            gpu,
        ),
    );
    remember_frame(
        &mut history,
        FrameSample::at(newest - GRAPH_WINDOW, frame, calm(), gpu),
    );
    remember_frame(&mut history, FrameSample::at(newest, frame, calm(), gpu));
    let times: Vec<_> = history.iter().map(|sample| sample.time).collect();
    assert_eq!(times, vec![newest - GRAPH_WINDOW, newest]);

    history.clear();
    for index in 0..=120 {
        let time = if index == 120 {
            GRAPH_WINDOW
        } else {
            Duration::from_nanos((GRAPH_WINDOW.as_nanos() / 120 * index as u128) as u64)
        };
        remember_frame(&mut history, FrameSample::at(time, frame, calm(), gpu));
    }
    assert_eq!(history.len(), 121);
    assert_eq!(history[0].time, Duration::ZERO);
    assert_eq!(history.last().expect("newest").time, GRAPH_WINDOW);
}

#[test]
fn a_short_history_is_spaced_across_the_plot() {
    let history = vec![
        at(Duration::ZERO, calm(), Duration::from_micros(100)),
        at(Duration::from_secs(1), calm(), Duration::from_micros(100)),
        at(Duration::from_secs(10), calm(), Duration::from_micros(100)),
    ];
    let view = profile_overlay(&history, VIEW);
    let line = view
        .lines
        .iter()
        .find(|line| line.name == "window pump cpu")
        .expect("window series");
    let left = line.points[0].center_x();
    let mid = line.points[1].center_x();
    let right = line.points[2].center_x();
    let fraction = (mid - left) / (right - left);
    assert!(
        (fraction - 0.1).abs() < 0.02,
        "gap fraction {fraction} did not follow the 1s and 10s times"
    );
    assert!((line.points[0].x - view.plot.x).abs() < 1.0);
    let last = line.points[2];
    assert!((last.x + last.w - (view.plot.x + view.plot.w)).abs() < 1.0);
}

#[test]
fn a_full_window_puts_the_time_midpoint_at_the_middle() {
    let mut history = Vec::new();
    for index in 0..16 {
        history.push(at(
            Duration::from_millis(index),
            calm(),
            Duration::from_micros(100),
        ));
    }
    let mut mid_cpu = calm();
    mid_cpu[0] = Duration::from_millis(7);
    history.push(at(GRAPH_WINDOW / 2, mid_cpu, Duration::from_micros(100)));
    history.push(at(
        GRAPH_WINDOW - Duration::from_secs(1),
        calm(),
        Duration::from_micros(100),
    ));
    history.push(at(
        GRAPH_WINDOW - Duration::from_millis(500),
        calm(),
        Duration::from_micros(100),
    ));
    history.push(at(GRAPH_WINDOW, calm(), Duration::from_micros(100)));
    let view = profile_overlay(&history, VIEW);
    let line = view
        .lines
        .iter()
        .find(|line| line.name == "window pump cpu")
        .expect("window series");
    let left = line.points[0].center_x();
    let mid = line
        .points
        .iter()
        .find(|point| point.nanos == Duration::from_millis(7).as_nanos())
        .expect("midpoint mark")
        .center_x();
    let right = line.points.last().expect("newest").center_x();
    let fraction = (mid - left) / (right - left);
    let index_fraction = 16.0 / (history.len() - 1) as f32;
    assert!(line.points.len() < history.len());
    assert!(
        (fraction - 0.5).abs() < 0.02,
        "midpoint fraction {fraction} left the middle of the window"
    );
    assert!(
        (fraction - index_fraction).abs() > 0.2,
        "midpoint followed sample count {index_fraction} instead of time"
    );
    assert!((line.points[0].x - view.plot.x).abs() < 1.0);
}

#[test]
fn the_graph_averages_each_quarter_second_into_one_box() {
    let mut history = Vec::new();
    let mut draw_cpu = calm();
    draw_cpu[5] = Duration::from_millis(25);
    for index in 0..4000 {
        let time = Duration::from_millis(index * 15);
        let cpu = if index == 3800 { draw_cpu } else { calm() };
        let gpu = if index == 3600 {
            Duration::from_millis(40)
        } else {
            Duration::from_micros(100)
        };
        remember_frame(&mut history, at(time, cpu, gpu));
    }
    assert!(history.len() > 120);
    let view = profile_overlay(&history, VIEW);
    let draw = view
        .lines
        .iter()
        .find(|line| line.name == "gpu draw/present cpu")
        .expect("draw series");
    let gpu = view
        .lines
        .iter()
        .find(|line| line.name == "gpu draw/present gpu")
        .expect("gpu series");
    let quarter = Duration::from_millis(250).as_nanos();
    let span = view.plot.span.as_nanos().max(1);
    let mut groups = std::collections::BTreeSet::new();
    for sample in &history {
        let mut offset = sample.time.saturating_sub(view.plot.start).as_nanos();
        if offset >= span {
            offset = span - 1;
        }
        groups.insert(offset / quarter);
    }
    assert_eq!(draw.points.len(), groups.len());
    assert!(
        draw.points.len() < history.len() / 10,
        "drew {} boxes for {} frames",
        draw.points.len(),
        history.len()
    );
    let max_draw = draw.points.iter().map(|point| point.nanos).max().unwrap();
    let max_gpu = gpu.points.iter().map(|point| point.nanos).max().unwrap();
    assert_eq!(max_draw, Duration::from_millis(25).as_nanos());
    assert_eq!(max_gpu, Duration::from_millis(40).as_nanos());
    let cover: f32 = draw.points.iter().map(|point| point.w).sum();
    assert!(
        cover > view.plot.w * 0.9,
        "boxes left a gap, cover {cover} plot {}",
        view.plot.w
    );
    assert!(
        cover < view.plot.w * 1.15,
        "boxes stacked, cover {cover} plot {}",
        view.plot.w
    );
    for pair in draw.points.windows(2) {
        assert!(
            pair[0].x + pair[0].w <= pair[1].x + 0.6,
            "boxes overlap at {} and {}",
            pair[0].x,
            pair[1].x
        );
    }
    let point = &draw.points[0];
    assert!(view.paints.iter().any(|paint| {
        paint.color == point.color
            && (paint.x - point.x).abs() < 0.01
            && (paint.w - point.w).abs() < 0.01
            && (paint.y - point.y).abs() < 0.01
    }));
}

#[test]
fn a_time_bucket_averages_every_stage() {
    let mut hot_window = calm();
    hot_window[0] = Duration::from_millis(11);
    let mut hot_draw = calm();
    hot_draw[5] = Duration::from_millis(25);
    let history = vec![
        at(Duration::ZERO, hot_window, Duration::from_millis(1)),
        at(
            Duration::from_millis(100),
            calm(),
            Duration::from_millis(40),
        ),
        at(Duration::from_secs(1), hot_draw, Duration::from_micros(100)),
    ];
    let view = profile_overlay(&history, VIEW);
    let window = view
        .lines
        .iter()
        .find(|line| line.name == "window pump cpu")
        .expect("window series");
    let gpu = view
        .lines
        .iter()
        .find(|line| line.name == "gpu draw/present gpu")
        .expect("gpu series");
    let draw = view
        .lines
        .iter()
        .find(|line| line.name == "gpu draw/present cpu")
        .expect("draw series");
    assert_eq!(window.points.len(), 2);
    assert_eq!(window.points[0].nanos, Duration::from_millis(6).as_nanos());
    assert_eq!(gpu.points[0].nanos, Duration::from_millis(40).as_nanos());
    assert_eq!(draw.points[1].nanos, Duration::from_millis(25).as_nanos());
    assert!(window.points[0].w > 6.0);
}

#[test]
fn pause_includes_frames_finished_before_the_hold() {
    let mut graph = ProfileGraph::default();
    let early = at(Duration::from_secs(1), calm(), Duration::from_micros(100));
    graph.remember(early);
    let mut draw_cpu = calm();
    draw_cpu[5] = Duration::from_millis(15);
    let spike = at(Duration::from_secs(2), draw_cpu, Duration::from_millis(9));
    graph.pause_after([spike.clone()]);
    graph.remember(at(Duration::from_secs(3), calm(), Duration::from_millis(4)));
    assert_eq!(graph.visible().len(), 2);
    assert_eq!(graph.visible()[1], spike);
    assert!(graph
        .visible()
        .iter()
        .all(|sample| sample.time != Duration::from_secs(3)));
}

#[test]
fn the_basic_overlay_omits_the_gpu_series() {
    let mut graph = ProfileGraph::default();
    graph.remember(at(Duration::from_secs(1), calm(), Duration::from_millis(9)));
    let view = graph.overlay(VIEW, false);
    assert!(view.lines.iter().all(|line| !line.name.ends_with(" gpu")));
    assert!(!view.readout.contains("GPU "));
    assert!(view.readout.contains("DRAW 1.0 MS"));
    assert_eq!(view.lines.len(), STAGE_COUNT);
}

#[test]
fn the_graph_shows_the_latest_rate_and_one_line_per_series() {
    let mut spiked_cpu = calm();
    spiked_cpu[2] = Duration::from_millis(30);
    let draw_gpu = Duration::from_micros(200);
    let history = vec![
        FrameSample::at(Duration::ZERO, Duration::from_millis(8), calm(), draw_gpu),
        FrameSample::at(
            Duration::from_secs(1),
            Duration::from_millis(40),
            spiked_cpu,
            draw_gpu,
        ),
        FrameSample::at(
            Duration::from_secs(2),
            Duration::from_millis(9),
            calm(),
            draw_gpu,
        ),
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

    let view = profile_overlay(&history, VIEW);
    let latest = history.last().expect("history");
    assert_eq!(view.frame_rate, latest.frame_rate);
    assert!(view
        .readout
        .starts_with(&format!("{:.1} FPS\n", latest.frame_rate)));
    assert!(view.readout.contains("WINDOW 1.0 MS"));
    assert!(view.readout.contains("UI 1.0 MS"));
    assert!(view.readout.contains("GPU 0.2 MS"));
    let hot = profile_overlay(&history[1..2], VIEW);
    assert!(hot.readout.contains("UI 30.0 MS"));
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
fn pause_holds_the_graph_until_reset() {
    let gpu_spike = at(Duration::from_secs(2), calm(), Duration::from_millis(9));
    let mut draw_cpu = calm();
    draw_cpu[5] = Duration::from_millis(15);
    let draw_spike = at(Duration::from_secs(5), draw_cpu, Duration::from_micros(200));
    let mut calm_ui = calm();
    calm_ui[2] = Duration::from_millis(3);
    let newer = at(Duration::from_secs(8), calm_ui, Duration::from_micros(200));

    let mut graph = ProfileGraph::default();
    graph.remember(at(Duration::ZERO, calm(), Duration::from_micros(200)));
    graph.remember(gpu_spike.clone());
    graph.remember(draw_spike.clone());
    graph.remember(newer.clone());
    let clock = Instant::now();
    let before = graph.overlay_at(VIEW, true, clock);
    let held_times: Vec<_> = graph.visible().iter().map(|sample| sample.time).collect();

    graph.pause();
    graph.remember(at(
        Duration::from_secs(100),
        calm(),
        Duration::from_millis(1),
    ));
    let held = graph.overlay_at(VIEW, true, clock);
    assert_eq!(
        graph
            .visible()
            .iter()
            .map(|sample| sample.time)
            .collect::<Vec<_>>(),
        held_times
    );
    assert_eq!(held.readout, before.readout);
    assert!(graph
        .visible()
        .iter()
        .any(|sample| sample.time == gpu_spike.time));

    graph.select(gpu_spike.time, draw_spike.time);
    let hit = graph.inspect().expect("interval");
    assert_eq!(hit.gpu, gpu_spike);
    assert_eq!(hit.draw, draw_spike);
    for (got, want) in hit.gpu.stages.iter().zip(gpu_spike.stages.iter()) {
        assert_eq!(got.cpu, want.cpu);
        assert_eq!(got.gpu, want.gpu);
    }
    for (got, want) in hit.draw.stages.iter().zip(draw_spike.stages.iter()) {
        assert_eq!(got.cpu, want.cpu);
        assert_eq!(got.gpu, want.gpu);
    }
    let selected = graph.overlay_at(VIEW, true, clock + OVERLAY_PERIOD);
    assert!(selected.readout.contains("GPU 9.0 MS"));
    assert!(selected.readout.contains("DRAW 15.0 MS"));
    assert!(
        !selected.readout.contains("UI 3.0 MS"),
        "readout used the calm frame:\n{}",
        selected.readout
    );

    graph.reset();
    let live_at = Duration::from_secs(460);
    let edge = at(live_at - GRAPH_WINDOW, calm(), Duration::from_micros(100));
    let live = at(live_at, calm(), Duration::from_micros(100));
    let too_old = at(
        live_at - GRAPH_WINDOW - Duration::from_nanos(1),
        calm(),
        Duration::from_micros(100),
    );
    graph.remember(edge.clone());
    graph.remember(live.clone());
    graph.remember(too_old);
    assert!(graph
        .visible()
        .iter()
        .any(|sample| sample.time == edge.time));
    assert!(graph
        .visible()
        .iter()
        .any(|sample| sample.time == live.time));
    assert!(graph
        .visible()
        .iter()
        .all(|sample| live.time.saturating_sub(sample.time) <= GRAPH_WINDOW));
    assert!(graph
        .visible()
        .iter()
        .all(|sample| sample.time != newer.time));
    let resumed = graph.overlay(VIEW, true);
    assert_eq!(resumed.frame_rate, live.frame_rate);
    assert!(resumed
        .readout
        .starts_with(&format!("{:.1} FPS\n", live.frame_rate)));
}

#[test]
fn the_stream_flushes_each_frame_and_keeps_a_slow_stage() {
    let path = std::env::temp_dir().join(format!("genos-profile-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut stream = ProfileStream::create(&path).expect("create");
    let fast = FrameSample::at(
        Duration::from_millis(10),
        Duration::from_millis(10),
        calm(),
        Duration::from_micros(50),
    );
    stream.append(&fast).expect("append fast");
    let after_first = std::fs::read_to_string(&path).expect("read first");
    let first = parse_frames(&after_first);
    assert_eq!(first.len(), 1);
    assert_frame(&first[0], &fast);

    let mut slow_cpu = calm();
    slow_cpu[4] = Duration::from_millis(80);
    let slow = FrameSample::at(
        Duration::from_millis(90),
        Duration::from_millis(90),
        slow_cpu,
        Duration::from_millis(5),
    );
    stream.append(&slow).expect("append slow");
    let after_second = std::fs::read_to_string(&path).expect("read second");
    let both = parse_frames(&after_second);
    assert_eq!(both.len(), 2);
    assert_frame(&both[0], &fast);
    assert_frame(&both[1], &slow);
    assert!(both[1].time > both[0].time);
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
    let spiked = both[1]
        .stages
        .iter()
        .find(|stage| stage.label == "gpu draw/present")
        .expect("draw stage");
    assert_eq!(spiked.gpu, slow.stages[5].gpu.as_nanos());
    let _ = std::fs::remove_file(&path);
}

struct ParsedStage {
    label: String,
    cpu: u128,
    gpu: u128,
}

struct ParsedFrame {
    time: u128,
    rate: f64,
    duration: u128,
    stages: Vec<ParsedStage>,
}

fn parse_frames(text: &str) -> Vec<ParsedFrame> {
    let mut frames = Vec::new();
    let mut time = None;
    let mut rate = None;
    let mut duration = None;
    let mut stages = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            if rate.is_some() {
                frames.push(ParsedFrame {
                    time: time.take().unwrap(),
                    rate: rate.take().unwrap(),
                    duration: duration.take().unwrap(),
                    stages: std::mem::take(&mut stages),
                });
            }
            continue;
        }
        let mut parts = line.split('\t');
        match parts.next().unwrap() {
            "time_ns" => time = Some(parts.next().unwrap().parse().unwrap()),
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
        ceiling: None,
    });
    let camera = Camera::opening();
    let mut spiked = calm();
    spiked[0] = Duration::from_millis(25);
    let history = vec![
        FrameSample::at(
            Duration::ZERO,
            Duration::from_millis(8),
            calm(),
            Duration::from_micros(100),
        ),
        FrameSample::at(
            Duration::from_secs(1),
            Duration::from_millis(30),
            spiked,
            Duration::from_micros(100),
        ),
        FrameSample::at(
            Duration::from_secs(2),
            Duration::from_millis(8),
            calm(),
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
        .draw_with_overlay(&world, &camera, &paints, true, false)
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
    let ink = pixel(&pixels, width, point.center_x(), point.y + 3.0);
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
    assert_eq!(parsed.time, sample.time.as_nanos());
    assert_eq!(parsed.rate, sample.frame_rate);
    assert_eq!(parsed.duration, sample.duration.as_nanos());
    assert_eq!(parsed.stages.len(), sample.stages.len());
    for (parsed_stage, stage) in parsed.stages.iter().zip(sample.stages.iter()) {
        assert_eq!(parsed_stage.label, stage.label);
        assert_eq!(parsed_stage.cpu, stage.cpu.as_nanos());
        assert_eq!(parsed_stage.gpu, stage.gpu.as_nanos());
    }
}
