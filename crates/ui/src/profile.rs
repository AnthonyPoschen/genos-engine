//! Verification frame profile. The graph and the file share one sample.
//!
//! The camera turns this on for `--proof` or `--frames`. CPU time is the host
//! time of a stage. GPU time is a Vulkan timestamp span for the draw, and 0
//! for a stage that does not submit GPU work.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// The only stage that submits GPU work.
pub const DRAW_STAGE: &str = STAGE_LABELS[5];

const GRAPH_W: f32 = 460.0;
const GRAPH_H: f32 = 168.0;
const MARGIN: f32 = 16.0;
const POINT: f32 = 6.0;
const BG: [f32; 3] = [0.05, 0.06, 0.08];
const TEXT: [f32; 3] = [0.93, 0.95, 0.92];

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

/// True when the launch is a proof window or a fixed frame count.
pub fn profiler_enabled(proof: bool, frame_limit: Option<u32>) -> bool {
    proof || frame_limit.is_some()
}

/// One stage on one completed frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StageSample {
    pub label: &'static str,
    pub cpu: Duration,
    pub gpu: Duration,
}

/// Frame rate and per-stage times. The graph and the file both use this value.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameSample {
    pub frame_rate: f64,
    pub duration: Duration,
    pub stages: [StageSample; STAGE_COUNT],
}

/// A frame whose GPU draw time is not known yet.
#[derive(Clone, Copy, Debug)]
pub struct OpenFrame {
    pub duration: Duration,
    pub cpu: [Duration; STAGE_COUNT],
}

impl OpenFrame {
    pub fn finish(self, draw_gpu: Duration) -> FrameSample {
        FrameSample::from_stages(self.duration, self.cpu, draw_gpu)
    }
}

impl FrameSample {
    /// `cpu` follows [`STAGE_LABELS`]. `draw_gpu` is stored on the draw stage.
    /// Every other stage gets GPU time 0.
    pub fn from_stages(
        duration: Duration,
        cpu: [Duration; STAGE_COUNT],
        draw_gpu: Duration,
    ) -> Self {
        let stages = std::array::from_fn(|index| StageSample {
            label: STAGE_LABELS[index],
            cpu: cpu[index],
            gpu: if STAGE_LABELS[index] == DRAW_STAGE {
                draw_gpu
            } else {
                Duration::ZERO
            },
        });
        Self {
            frame_rate: rate_hz(duration),
            duration,
            stages,
        }
    }
}

fn rate_hz(duration: Duration) -> f64 {
    let nanos = duration.as_nanos();
    if nanos == 0 {
        0.0
    } else {
        1.0e9 / nanos as f64
    }
}

/// One sample on one series.
#[derive(Clone, Copy, Debug)]
pub struct ProfilePoint {
    pub frame: usize,
    pub x: f32,
    pub y: f32,
    pub nanos: u128,
    pub color: [f32; 3],
}

/// One CPU series or one GPU series across the history.
#[derive(Clone, Debug)]
pub struct ProfileLine {
    pub name: String,
    pub points: Vec<ProfilePoint>,
}

/// Paints for one verification overlay. `frame_rate` is the latest sample.
#[derive(Clone, Debug)]
pub struct ProfileView {
    pub frame_rate: f64,
    pub readout: String,
    pub background: Paint,
    pub lines: Vec<ProfileLine>,
    pub paints: Vec<Paint>,
}

/// Build the frame-rate readout and one line per CPU series and GPU series.
///
/// The origin is the top-left. A larger time sits higher on its line.
/// Every supplied frame is a point. The viewport keeps the graph on screen.
pub fn profile_overlay(history: &[FrameSample], viewport: [f32; 2]) -> ProfileView {
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
            lines: Vec::new(),
            paints: Vec::new(),
        };
    }
    let frame_rate = history[history.len() - 1].frame_rate;
    let readout = format!("{frame_rate:.1}");
    let width = GRAPH_W.min((viewport[0] - MARGIN * 2.0).max(POINT));
    let height = GRAPH_H.min((viewport[1] - MARGIN * 2.0).max(POINT));
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
    for blot in text::blots(&readout, origin_x + 8.0, origin_y + 6.0, width - 16.0) {
        paints.push(Paint {
            x: blot.x,
            y: blot.y,
            w: blot.w,
            h: blot.h,
            color: TEXT,
        });
    }
    let plot_x = origin_x + 8.0;
    let plot_y = origin_y + 28.0;
    let plot_w = (width - 16.0).max(POINT);
    let plot_h = (height - 36.0).max(POINT);
    let plot_bottom = plot_y + plot_h;
    let max_nanos = history
        .iter()
        .flat_map(|sample| sample.stages.iter())
        .flat_map(|stage| [stage.cpu.as_nanos(), stage.gpu.as_nanos()])
        .max()
        .unwrap_or(1)
        .max(1);
    let frames = history.len();
    let mut lines = Vec::with_capacity(STAGE_COUNT * 2);
    for series in 0..(STAGE_COUNT * 2) {
        let stage_index = series / 2;
        let gpu_series = series % 2 == 1;
        let color = if gpu_series {
            GPU_COLOR[stage_index]
        } else {
            CPU_COLOR[stage_index]
        };
        let name = if gpu_series {
            format!("{} gpu", STAGE_LABELS[stage_index])
        } else {
            format!("{} cpu", STAGE_LABELS[stage_index])
        };
        let mut points = Vec::with_capacity(frames);
        for (frame, sample) in history.iter().enumerate() {
            let nanos = if gpu_series {
                sample.stages[stage_index].gpu.as_nanos()
            } else {
                sample.stages[stage_index].cpu.as_nanos()
            };
            let x = plot_x + (frame as f32 + 0.5) * plot_w / frames as f32 - POINT * 0.5;
            let y = plot_bottom - (nanos as f64 / max_nanos as f64) as f32 * plot_h - POINT;
            let point = ProfilePoint {
                frame,
                x,
                y,
                nanos,
                color,
            };
            paints.push(Paint {
                x: point.x,
                y: point.y,
                w: POINT,
                h: POINT,
                color,
            });
            points.push(point);
        }
        lines.push(ProfileLine { name, points });
    }
    ProfileView {
        frame_rate,
        readout,
        background,
        lines,
        paints,
    }
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

    /// Write one frame and flush it so a later read can see that frame.
    pub fn append(&mut self, sample: &FrameSample) -> Result<(), String> {
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
        self.file.sync_data().map_err(|err| err.to_string())?;
        Ok(())
    }
}
