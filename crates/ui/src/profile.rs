//! Frame profile. The file stores every completed frame. The graph draws one
//! box for each 250 ms average.
//!
//! The camera records CPU stages on every user launch. GPU timestamps and the
//! profile file run only in detailed mode. CPU time is the host time of a stage.
//! GPU time is a Vulkan timestamp span for the draw. A stage that does not
//! submit GPU work stores 0.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::panel::Paint;
use crate::text;

/// Stages the camera frame already runs, in order.
pub const STAGE_LABELS: [&str; 6] = [
    "window pump",
    "input",
    "ui",
    "camera/scene update",
    "simulation step",
    "gpu draw/present",
];

pub const STAGE_COUNT: usize = STAGE_LABELS.len();

const DRAW_INDEX: usize = 5;

/// The only stage that submits GPU work.
pub const DRAW_STAGE: &str = STAGE_LABELS[DRAW_INDEX];

/// On-screen names. The profile file keeps [`STAGE_LABELS`].
const STAGE_SHORT: [&str; STAGE_COUNT] = ["WINDOW", "INPUT", "UI", "SCENE", "SIM", "DRAW"];

/// Samples older than this, measured from the newest sample, leave the graph.
pub const GRAPH_WINDOW: Duration = Duration::from_secs(10);

/// The on-screen picture is built at most 5 times a second.
pub const OVERLAY_PERIOD: Duration = Duration::from_millis(200);

/// The graph draws one box for this interval.
///
/// A CPU stage uses the average of the frames in the interval. The draw CPU
/// box and the draw GPU box keep the highest sample in the interval.
pub const GRAPH_BUCKET: Duration = Duration::from_millis(250);

const GRAPH_W: f32 = 460.0;
const GRAPH_H: f32 = 320.0;
const MARGIN: f32 = 16.0;
const POINT: f32 = 6.0;
const TEXT_PAD: f32 = 6.0;
const BG: [f32; 3] = [0.05, 0.06, 0.08];
const TEXT: [f32; 3] = [0.93, 0.95, 0.92];
const SELECT: [f32; 3] = [0.16, 0.22, 0.30];
const PAUSED: [f32; 3] = [0.95, 0.78, 0.28];

const CPU_COLOR: [[f32; 3]; STAGE_COUNT] = [
    [0.95, 0.28, 0.22],
    [0.96, 0.62, 0.16],
    [0.92, 0.86, 0.22],
    [0.30, 0.82, 0.38],
    [0.22, 0.74, 0.86],
    [0.42, 0.46, 0.96],
];

const GPU_COLOR: [[f32; 3]; STAGE_COUNT] = [
    [0.62, 0.16, 0.18],
    [0.70, 0.36, 0.12],
    [0.62, 0.54, 0.12],
    [0.14, 0.52, 0.28],
    [0.12, 0.44, 0.58],
    [0.24, 0.26, 0.70],
];

/// Store one completed frame and drop samples more than [`GRAPH_WINDOW`] older
/// than the newest sample. A sample at exactly that age stays.
pub fn remember_frame(history: &mut Vec<FrameSample>, sample: FrameSample) {
    history.push(sample);
    let newest = history
        .iter()
        .map(|sample| sample.time)
        .max()
        .unwrap_or(Duration::ZERO);
    history.retain(|sample| newest.saturating_sub(sample.time) <= GRAPH_WINDOW);
}

/// One stage on one completed frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StageSample {
    pub label: &'static str,
    pub cpu: Duration,
    pub gpu: Duration,
}

/// Frame rate and per-stage times. The graph and the file both use this value.
///
/// `time` orders the frame. The graph drops by the gap between these times.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameSample {
    pub time: Duration,
    pub frame_rate: f64,
    pub duration: Duration,
    pub stages: [StageSample; STAGE_COUNT],
}

/// A frame whose GPU draw time is not known yet.
#[derive(Clone, Copy, Debug)]
pub struct OpenFrame {
    pub time: Duration,
    pub duration: Duration,
    pub cpu: [Duration; STAGE_COUNT],
}

impl OpenFrame {
    pub fn finish(self, draw_gpu: Duration) -> FrameSample {
        FrameSample::at(self.time, self.duration, self.cpu, draw_gpu)
    }
}

impl FrameSample {
    /// `cpu` follows [`STAGE_LABELS`]. `draw_gpu` is stored on the draw stage.
    /// Every other stage gets GPU time 0. The ordering time is zero.
    pub fn from_stages(
        duration: Duration,
        cpu: [Duration; STAGE_COUNT],
        draw_gpu: Duration,
    ) -> Self {
        Self::at(Duration::ZERO, duration, cpu, draw_gpu)
    }

    /// Same record as [`FrameSample::from_stages`] at an explicit ordering time.
    pub fn at(
        time: Duration,
        duration: Duration,
        cpu: [Duration; STAGE_COUNT],
        draw_gpu: Duration,
    ) -> Self {
        let stages = std::array::from_fn(|index| StageSample {
            label: STAGE_LABELS[index],
            cpu: cpu[index],
            gpu: if index == DRAW_INDEX {
                draw_gpu
            } else {
                Duration::ZERO
            },
        });
        Self {
            time,
            frame_rate: rate_hz(duration),
            duration,
            stages,
        }
    }
}

/// Held graph. Live samples keep arriving, and the held copy does not scroll.
#[derive(Clone, Debug, Default)]
pub struct ProfileGraph {
    live: Vec<FrameSample>,
    hold: Option<Hold>,
    /// Changes when the visible boxes must be built again.
    generation: u64,
    /// Pause or reset builds on the next call. Live samples wait for [`OVERLAY_PERIOD`].
    urgent: bool,
    built: u64,
    cache: Option<OverlayCache>,
}

#[derive(Clone, Debug)]
struct OverlayCache {
    generation: u64,
    viewport: [f32; 2],
    show_gpu: bool,
    built_at: Instant,
    view: ProfileView,
}

#[derive(Clone, Debug)]
struct Hold {
    samples: Vec<FrameSample>,
    region: Option<(Duration, Duration)>,
}

impl ProfileGraph {
    /// Append one live frame. While paused, the visible graph stays put.
    /// The sample still joins the live window for the reset that follows.
    pub fn remember(&mut self, sample: FrameSample) {
        remember_frame(&mut self.live, sample);
        if self.hold.is_none() {
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Freeze the visible graph on the samples kept so far.
    pub fn pause(&mut self) {
        self.pause_after([]);
    }

    /// Remember frames that already finished, then freeze the graph.
    ///
    /// A later [`ProfileGraph::remember`] updates the live window only.
    /// It does not enter the hold. Call this only while the graph is live.
    pub fn pause_after(&mut self, finished: impl IntoIterator<Item = FrameSample>) {
        if self.hold.is_some() {
            return;
        }
        for sample in finished {
            self.remember(sample);
        }
        self.hold = Some(Hold {
            samples: self.live.clone(),
            region: None,
        });
        self.generation = self.generation.wrapping_add(1);
        self.urgent = true;
    }

    pub fn is_paused(&self) -> bool {
        self.hold.is_some()
    }

    /// Leave the held graph. The view follows the live 10-second window again.
    pub fn reset(&mut self) {
        self.hold = None;
        self.generation = self.generation.wrapping_add(1);
        self.urgent = true;
    }

    /// Choose a time interval on the held graph. A live graph ignores this.
    pub fn select(&mut self, start: Duration, end: Duration) {
        if self.hold.is_none() {
            return;
        }
        self.hold.as_mut().expect("hold").region = Some(ordered(start, end));
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn visible(&self) -> &[FrameSample] {
        match self.hold.as_ref() {
            Some(hold) => hold.samples.as_slice(),
            None => self.live.as_slice(),
        }
    }

    /// Peak draw-GPU frame and peak draw-CPU frame inside the held interval.
    pub fn inspect(&self) -> Option<RegionHit> {
        let hold = self.hold.as_ref()?;
        let (start, end) = hold.region?;
        inspect_region(&hold.samples, start, end)
    }

    /// `show_gpu` draws the GPU series and the GPU millisecond row.
    /// Basic mode passes false. Detailed mode passes true.
    ///
    /// Live samples reuse the last picture until [`OVERLAY_PERIOD`] has passed.
    pub fn overlay(&mut self, viewport: [f32; 2], show_gpu: bool) -> ProfileView {
        self.overlay_at(viewport, show_gpu, Instant::now())
    }

    /// Same as [`ProfileGraph::overlay`] at an explicit time.
    pub fn overlay_at(&mut self, viewport: [f32; 2], show_gpu: bool, now: Instant) -> ProfileView {
        if !self.overlay_due(viewport, show_gpu, now) {
            return self
                .cache
                .as_ref()
                .expect("due is false only when a picture exists")
                .view
                .clone();
        }
        let region = self.hold.as_ref().and_then(|hold| hold.region);
        let paused = self.is_paused();
        let view = compose(self.visible(), region, paused, show_gpu, viewport);
        self.built = self.built.wrapping_add(1);
        self.urgent = false;
        self.cache = Some(OverlayCache {
            generation: self.generation,
            viewport,
            show_gpu,
            built_at: now,
            view: view.clone(),
        });
        view
    }

    /// True when the next picture must be built.
    pub fn overlay_due(&self, viewport: [f32; 2], show_gpu: bool, now: Instant) -> bool {
        let Some(cache) = &self.cache else {
            return true;
        };
        if cache.show_gpu != show_gpu || cache.viewport != viewport || self.urgent {
            return true;
        }
        if cache.generation == self.generation {
            return false;
        }
        now.saturating_duration_since(cache.built_at) >= OVERLAY_PERIOD
    }

    /// Map a pointer through the picture already built.
    pub fn cached_time_at(&self, x: f32, y: f32) -> Option<Duration> {
        self.cache.as_ref()?.view.time_at(x, y)
    }

    /// Map a pointer x through the picture already built.
    pub fn cached_time_at_x(&self, x: f32) -> Option<Duration> {
        self.cache.as_ref()?.view.time_at_x(x)
    }
}

/// Frames in `[start, end]` with the highest draw GPU time and the highest
/// `gpu draw/present` CPU time. A tie keeps the later frame.
pub fn inspect_region(
    samples: &[FrameSample],
    start: Duration,
    end: Duration,
) -> Option<RegionHit> {
    let (lo, hi) = ordered(start, end);
    let mut gpu_index: Option<usize> = None;
    let mut draw_index: Option<usize> = None;
    for (index, sample) in samples.iter().enumerate() {
        if sample.time < lo || sample.time > hi {
            continue;
        }
        match gpu_index {
            Some(best) if samples[best].stages[DRAW_INDEX].gpu > sample.stages[DRAW_INDEX].gpu => {}
            _ => gpu_index = Some(index),
        }
        match draw_index {
            Some(best) if samples[best].stages[DRAW_INDEX].cpu > sample.stages[DRAW_INDEX].cpu => {}
            _ => draw_index = Some(index),
        }
    }
    Some(RegionHit {
        gpu: samples[gpu_index?].clone(),
        draw: samples[draw_index?].clone(),
    })
}

/// The two peak frames for one held interval. The samples are the stored frames.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionHit {
    pub gpu: FrameSample,
    pub draw: FrameSample,
}

fn ordered(start: Duration, end: Duration) -> (Duration, Duration) {
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

fn millis(duration: Duration) -> String {
    let ms = duration.as_nanos() as f64 / 1_000_000.0;
    format!("{ms:.1}")
}

fn starts_with_word(line: &str, word: &str) -> bool {
    let Some(rest) = line.strip_prefix(word) else {
        return false;
    };
    rest.starts_with(' ')
}

fn swatch_color(line: &str) -> Option<[f32; 3]> {
    if let Some(index) = STAGE_SHORT
        .iter()
        .position(|name| starts_with_word(line, name))
    {
        return Some(CPU_COLOR[index]);
    }
    if starts_with_word(line, "GPU") {
        return Some(GPU_COLOR[DRAW_INDEX]);
    }
    None
}

fn rate_hz(duration: Duration) -> f64 {
    let nanos = duration.as_nanos();
    if nanos == 0 {
        0.0
    } else {
        1.0e9 / nanos as f64
    }
}

fn sample_rows(sample: &FrameSample, show_gpu: bool) -> Vec<String> {
    let mut rows = Vec::with_capacity(1 + STAGE_COUNT + usize::from(show_gpu));
    rows.push(format!("{:.1} FPS", sample.frame_rate));
    for (index, stage) in sample.stages.iter().enumerate() {
        rows.push(format!("{} {} MS", STAGE_SHORT[index], millis(stage.cpu)));
    }
    if show_gpu {
        rows.push(format!("GPU {} MS", millis(sample.stages[DRAW_INDEX].gpu)));
    }
    rows
}

/// One averaged mark on one series. `x` is the left of the box. `w` is its width.
#[derive(Clone, Copy, Debug)]
pub struct ProfilePoint {
    pub frame: usize,
    pub x: f32,
    pub w: f32,
    pub y: f32,
    pub nanos: u128,
    pub color: [f32; 3],
}

impl ProfilePoint {
    /// Horizontal center of this mark.
    pub fn center_x(self) -> f32 {
        self.x + self.w * 0.5
    }
}

/// One CPU series or one GPU series across the history.
#[derive(Clone, Debug)]
pub struct ProfileLine {
    pub name: String,
    pub points: Vec<ProfilePoint>,
}

/// Time axis of the plot. `start` is the left edge. `span` reaches the right edge.
#[derive(Clone, Copy, Debug)]
pub struct PlotScale {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub start: Duration,
    pub span: Duration,
}

/// Paints for one verification overlay. `frame_rate` is the readout frame.
#[derive(Clone, Debug)]
pub struct ProfileView {
    pub frame_rate: f64,
    pub readout: String,
    pub background: Paint,
    pub plot: PlotScale,
    pub lines: Vec<ProfileLine>,
    pub paints: Vec<Paint>,
}

impl ProfileView {
    /// Map a pointer inside the plot to a sample time.
    pub fn time_at(&self, x: f32, y: f32) -> Option<Duration> {
        if self.plot.w <= 0.0
            || self.plot.h <= 0.0
            || x < self.plot.x
            || y < self.plot.y
            || x > self.plot.x + self.plot.w
            || y > self.plot.y + self.plot.h
        {
            return None;
        }
        self.time_at_x(x)
    }

    /// Map a pointer x to a sample time. The value clamps to the plot.
    pub fn time_at_x(&self, x: f32) -> Option<Duration> {
        if self.plot.w <= 0.0 || self.lines.is_empty() {
            return None;
        }
        if self.plot.span.is_zero() {
            return Some(self.plot.start);
        }
        let t = ((x - self.plot.x) / self.plot.w).clamp(0.0, 1.0) as f64;
        Some(self.plot.start + self.plot.span.mul_f64(t))
    }
}

/// Build the live readout and one line per CPU series and GPU series.
///
/// Each box is the average of one [`GRAPH_BUCKET`] interval.
/// The origin is the top-left. A larger time sits higher on its line.
/// The viewport keeps the graph on screen.
pub fn profile_overlay(history: &[FrameSample], viewport: [f32; 2]) -> ProfileView {
    compose(history, None, false, true, viewport)
}

fn compose(
    history: &[FrameSample],
    region: Option<(Duration, Duration)>,
    paused: bool,
    show_gpu: bool,
    viewport: [f32; 2],
) -> ProfileView {
    let empty_plot = PlotScale {
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
        start: Duration::ZERO,
        span: Duration::ZERO,
    };
    if history.is_empty() {
        return ProfileView {
            frame_rate: 0.0,
            readout: String::new(),
            background: Paint {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
                color: BG,
            },
            plot: empty_plot,
            lines: Vec::new(),
            paints: Vec::new(),
        };
    }
    let latest = &history[history.len() - 1];
    let hit = region.and_then(|(start, end)| inspect_region(history, start, end));
    let (frame_rate, rows) = readout_rows(latest, hit.as_ref(), show_gpu);
    let readout = rows.join("\n");
    let text_block = TEXT_PAD + rows.len() as f32 * text::CELL_H + 8.0 + POINT + 8.0;
    let width = GRAPH_W.min((viewport[0] - MARGIN * 2.0).max(POINT));
    let height = GRAPH_H.min((viewport[1] - MARGIN * 2.0).max(text_block));
    let origin_x = MARGIN;
    let origin_y = (viewport[1] - height - MARGIN).max(0.0);
    let background = Paint {
        x: origin_x,
        y: origin_y,
        w: width,
        h: height,
        color: BG,
    };
    let mut paints = vec![background];
    let text_x = origin_x + 8.0;
    let text_top = origin_y + TEXT_PAD;
    for (row, line) in rows.iter().enumerate() {
        let y = text_top + row as f32 * text::CELL_H;
        let label_x = if let Some(color) = swatch_color(line) {
            paints.push(Paint {
                x: text_x,
                y: y + 3.0,
                w: 10.0,
                h: 10.0,
                color,
            });
            text_x + 16.0
        } else {
            text_x
        };
        for blot in text::blots(
            line,
            label_x,
            y,
            (width - (label_x - origin_x) - 8.0).max(POINT),
        ) {
            paints.push(Paint {
                x: blot.x,
                y: blot.y,
                w: blot.w,
                h: blot.h,
                color: TEXT,
            });
        }
    }
    if paused {
        let label = "PAUSED";
        let label_w = label.chars().count() as f32 * text::CELL_W;
        let x = (origin_x + width - 8.0 - label_w).max(text_x);
        for blot in text::blots(label, x, text_top, label_w) {
            paints.push(Paint {
                x: blot.x,
                y: blot.y,
                w: blot.w,
                h: blot.h,
                color: PAUSED,
            });
        }
    }
    let plot_x = origin_x + 8.0;
    let plot_y = text_top + rows.len() as f32 * text::CELL_H + 8.0;
    let plot_w = (width - 16.0).max(POINT);
    let plot_h = (origin_y + height - 8.0 - plot_y).max(POINT);
    let plot_bottom = plot_y + plot_h;
    let (origin, end) = time_domain(history);
    let span = end.saturating_sub(origin);
    let plot = PlotScale {
        x: plot_x,
        y: plot_y,
        w: plot_w,
        h: plot_h,
        start: origin,
        span,
    };
    if let Some((start, end)) = region {
        let left = time_x(start, origin, span, plot_x, plot_w);
        let right = time_x(end, origin, span, plot_x, plot_w);
        let x = left.min(right);
        paints.push(Paint {
            x,
            y: plot_y,
            w: (left - right).abs().max(2.0),
            h: plot_h,
            color: SELECT,
        });
    }
    let mut prepared = Vec::new();
    let mut max_nanos = 1u128;
    for stage_index in 0..STAGE_COUNT {
        let cpu = averaged_marks(history, stage_index, false, origin, span);
        for mark in &cpu {
            max_nanos = max_nanos.max(mark.nanos);
        }
        prepared.push((stage_index, false, CPU_COLOR[stage_index], cpu));
        if show_gpu {
            let gpu = averaged_marks(history, stage_index, true, origin, span);
            for mark in &gpu {
                max_nanos = max_nanos.max(mark.nanos);
            }
            prepared.push((stage_index, true, GPU_COLOR[stage_index], gpu));
        }
    }
    let mut lines = Vec::new();
    for (stage_index, gpu_series, color, marks) in prepared {
        push_series(
            marks,
            stage_index,
            gpu_series,
            color,
            origin,
            span,
            plot_x,
            plot_w,
            plot_bottom,
            plot_h,
            max_nanos,
            &mut lines,
            &mut paints,
        );
    }
    ProfileView {
        frame_rate,
        readout,
        background,
        plot,
        lines,
        paints,
    }
}

fn readout_rows(
    latest: &FrameSample,
    hit: Option<&RegionHit>,
    show_gpu: bool,
) -> (f64, Vec<String>) {
    let Some(hit) = hit else {
        return (latest.frame_rate, sample_rows(latest, show_gpu));
    };
    if !show_gpu {
        return (hit.draw.frame_rate, sample_rows(&hit.draw, false));
    }
    if hit.gpu == hit.draw {
        return (hit.gpu.frame_rate, sample_rows(&hit.gpu, true));
    }
    let mut rows = Vec::new();
    rows.push("GPU".to_string());
    rows.extend(sample_rows(&hit.gpu, true));
    rows.push("DRAW".to_string());
    rows.extend(sample_rows(&hit.draw, true));
    (hit.gpu.frame_rate, rows)
}

fn time_domain(history: &[FrameSample]) -> (Duration, Duration) {
    let newest = history
        .iter()
        .map(|sample| sample.time)
        .max()
        .unwrap_or(Duration::ZERO);
    let oldest = history
        .iter()
        .map(|sample| sample.time)
        .min()
        .unwrap_or(Duration::ZERO);
    if newest.saturating_sub(oldest) >= GRAPH_WINDOW {
        (newest.saturating_sub(GRAPH_WINDOW), newest)
    } else {
        (oldest, newest)
    }
}

fn series_nanos(sample: &FrameSample, stage_index: usize, gpu_series: bool) -> u128 {
    if gpu_series {
        sample.stages[stage_index].gpu.as_nanos()
    } else {
        sample.stages[stage_index].cpu.as_nanos()
    }
}

fn time_x(time: Duration, origin: Duration, span: Duration, plot_x: f32, plot_w: f32) -> f32 {
    let t = if span.is_zero() {
        0.5
    } else {
        let fraction = time.saturating_sub(origin).as_secs_f64() / span.as_secs_f64();
        fraction.clamp(0.0, 1.0)
    };
    plot_x + (t as f32) * plot_w
}

fn mark_y(nanos: u128, max_nanos: u128, plot_bottom: f32, plot_h: f32) -> f32 {
    let height = (nanos as f64 / max_nanos as f64) as f32 * plot_h;
    plot_bottom - height - POINT
}

fn push_series(
    marks: Vec<SeriesMark>,
    stage_index: usize,
    gpu_series: bool,
    color: [f32; 3],
    origin: Duration,
    span: Duration,
    plot_x: f32,
    plot_w: f32,
    plot_bottom: f32,
    plot_h: f32,
    max_nanos: u128,
    lines: &mut Vec<ProfileLine>,
    paints: &mut Vec<Paint>,
) {
    let name = if gpu_series {
        format!("{} gpu", STAGE_LABELS[stage_index])
    } else {
        format!("{} cpu", STAGE_LABELS[stage_index])
    };
    let points = marks
        .into_iter()
        .map(|mark| {
            let (x, w) = mark_box(mark.index, origin, span, plot_x, plot_w);
            ProfilePoint {
                frame: mark.frame,
                x,
                w,
                y: mark_y(mark.nanos, max_nanos, plot_bottom, plot_h),
                nanos: mark.nanos,
                color,
            }
        })
        .collect::<Vec<_>>();
    for point in &points {
        paints.push(Paint {
            x: point.x,
            y: point.y,
            w: point.w,
            h: POINT,
            color,
        });
    }
    lines.push(ProfileLine { name, points });
}

struct SeriesMark {
    frame: usize,
    nanos: u128,
    index: usize,
}

struct Bucket {
    sum: u128,
    max: u128,
    count: u32,
    frame: usize,
    peak_frame: usize,
    used: bool,
}

/// One mark per [`GRAPH_BUCKET`].
///
/// Draw CPU and draw GPU keep the highest sample. Every other series uses the
/// integer mean of the interval.
fn averaged_marks(
    history: &[FrameSample],
    stage_index: usize,
    gpu_series: bool,
    origin: Duration,
    span: Duration,
) -> Vec<SeriesMark> {
    let step = GRAPH_BUCKET.as_nanos().max(1);
    let slots = if span.is_zero() {
        1
    } else {
        ((span.as_nanos() + step - 1) / step) as usize
    }
    .max(1);
    let mut buckets = Vec::with_capacity(slots);
    for _ in 0..slots {
        buckets.push(Bucket {
            sum: 0,
            max: 0,
            count: 0,
            frame: 0,
            peak_frame: 0,
            used: false,
        });
    }
    let peak = stage_index == DRAW_INDEX;
    for (frame, sample) in history.iter().enumerate() {
        let nanos = series_nanos(sample, stage_index, gpu_series);
        let mut index = (sample.time.saturating_sub(origin).as_nanos() / step) as usize;
        if index >= buckets.len() {
            index = buckets.len() - 1;
        }
        let bucket = &mut buckets[index];
        bucket.sum = bucket.sum.saturating_add(nanos);
        bucket.count = bucket.count.saturating_add(1);
        if !bucket.used || nanos >= bucket.max {
            bucket.max = nanos;
            bucket.peak_frame = frame;
        }
        bucket.frame = frame;
        bucket.used = true;
    }
    buckets
        .into_iter()
        .enumerate()
        .filter(|(_, bucket)| bucket.used)
        .map(|(index, bucket)| SeriesMark {
            frame: if peak {
                bucket.peak_frame
            } else {
                bucket.frame
            },
            nanos: if peak {
                bucket.max
            } else {
                bucket.sum / u128::from(bucket.count.max(1))
            },
            index,
        })
        .collect()
}

fn bucket_bound(origin: Duration, index: usize) -> Duration {
    let step = GRAPH_BUCKET.as_nanos().saturating_mul(index as u128);
    origin.saturating_add(Duration::from_nanos(step.min(u128::from(u64::MAX)) as u64))
}

/// The painted interval is the part of the bucket that lies inside the plot.
fn mark_box(
    index: usize,
    origin: Duration,
    span: Duration,
    plot_x: f32,
    plot_w: f32,
) -> (f32, f32) {
    if span.is_zero() || plot_w <= 0.0 {
        let w = POINT.min(plot_w.max(1.0));
        return (plot_x + (plot_w - w).max(0.0) * 0.5, w);
    }
    let end = origin.saturating_add(span);
    let start = bucket_bound(origin, index).max(origin);
    let stop = bucket_bound(origin, index + 1).min(end);
    let x0 = time_x(start, origin, span, plot_x, plot_w);
    let x1 = time_x(stop, origin, span, plot_x, plot_w);
    let right = plot_x + plot_w;
    let w = x1 - x0;
    if w < 1.0 {
        let x = x0.clamp(plot_x, (right - 1.0).max(plot_x));
        return (x, 1.0_f32.min(plot_w.max(1.0)));
    }
    (x0, w.min(right - x0).max(1.0))
}

/// Append-only profile file. Each [`ProfileStream::append`] flushes that frame.
pub struct ProfileStream {
    file: File,
    path: PathBuf,
}

impl ProfileStream {
    pub fn create(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
            }
        }
        let file = File::create(&path).map_err(|err| err.to_string())?;
        let path = path.canonicalize().unwrap_or(path);
        Ok(Self { file, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write one frame so a later read can see that frame.
    ///
    /// The write stays in the page cache. This does not sync the file to disk.
    pub fn append(&mut self, sample: &FrameSample) -> Result<(), String> {
        writeln!(self.file, "time_ns\t{}", sample.time.as_nanos())
            .map_err(|err| err.to_string())?;
        writeln!(self.file, "frame_rate\t{:?}", sample.frame_rate)
            .map_err(|err| err.to_string())?;
        writeln!(self.file, "duration_ns\t{}", sample.duration.as_nanos())
            .map_err(|err| err.to_string())?;
        for stage in &sample.stages {
            writeln!(
                self.file,
                "stage\t{}\t{}\t{}",
                stage.label,
                stage.cpu.as_nanos(),
                stage.gpu.as_nanos()
            )
            .map_err(|err| err.to_string())?;
        }
        writeln!(self.file).map_err(|err| err.to_string())?;
        self.file.flush().map_err(|err| err.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod reuse {
    use std::time::Instant;

    use super::*;

    fn one(time: Duration) -> FrameSample {
        FrameSample::at(
            time,
            Duration::from_millis(8),
            [Duration::from_millis(1); STAGE_COUNT],
            Duration::from_micros(100),
        )
    }

    #[test]
    fn a_held_graph_reuses_its_boxes() {
        let mut graph = ProfileGraph::default();
        graph.remember(one(Duration::from_secs(1)));
        graph.pause();
        let first = graph.overlay([1280.0, 720.0], true);
        assert_eq!(graph.built, 1);
        let second = graph.overlay([1280.0, 720.0], true);
        assert_eq!(graph.built, 1);
        assert_eq!(second.paints.len(), first.paints.len());
        assert_eq!(second.readout, first.readout);
        graph.remember(one(Duration::from_secs(2)));
        let third = graph.overlay([1280.0, 720.0], true);
        assert_eq!(graph.built, 1);
        assert_eq!(third.readout, first.readout);
        let clock = Instant::now();
        graph.overlay_at([1280.0, 720.0], true, clock);
        let built_before_drag = graph.built;
        graph.select(Duration::from_secs(1), Duration::from_secs(1));
        graph.overlay_at([1280.0, 720.0], true, clock + Duration::from_millis(50));
        assert_eq!(graph.built, built_before_drag);
        graph.overlay_at([1280.0, 720.0], true, clock + OVERLAY_PERIOD);
        assert_eq!(graph.built, built_before_drag + 1);
    }

    #[test]
    fn live_samples_rebuild_the_picture_five_times_a_second() {
        let mut graph = ProfileGraph::default();
        let clock = Instant::now();
        graph.remember(one(Duration::from_millis(1)));
        graph.overlay_at([1280.0, 720.0], false, clock);
        assert_eq!(graph.built, 1);
        for step in 1..5 {
            graph.remember(one(Duration::from_millis(step * 10)));
            graph.overlay_at(
                [1280.0, 720.0],
                false,
                clock + Duration::from_millis(40 * step),
            );
        }
        assert_eq!(graph.built, 1);
        graph.remember(one(Duration::from_millis(80)));
        graph.overlay_at([1280.0, 720.0], false, clock + OVERLAY_PERIOD);
        assert_eq!(graph.built, 2);
        let shown = graph.overlay_at(
            [1280.0, 720.0],
            false,
            clock + OVERLAY_PERIOD + Duration::from_millis(1),
        );
        assert!(shown.readout.contains("WINDOW"));
    }

    #[test]
    fn a_full_window_overlay_stays_a_small_part_of_a_frame() {
        let mut history = Vec::new();
        let step = Duration::from_millis(4);
        let mut time = Duration::ZERO;
        while time <= GRAPH_WINDOW {
            history.push(one(time));
            time += step;
        }
        let start = std::time::Instant::now();
        let view = profile_overlay(&history, [1280.0, 720.0]);
        let built = start.elapsed();
        let boxes: usize = view.lines.iter().map(|line| line.points.len()).sum();
        assert!(boxes < history.len());
        assert!(
            built < Duration::from_millis(20),
            "overlay took {built:?} for {} frames and {boxes} boxes",
            history.len()
        );
    }
}
