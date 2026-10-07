//! Benchmark sweep: lamp count × dynamic share × scale from one fixed viewpoint on a
//! fixed timeline, then tables with deltas and the cost of one static or dynamic lamp.

use std::fmt::Write as _;
use std::time::Instant;

use genos_render::{LightBuildTimes, Renderer, TierStats};
use genos_scene::Camera;
use genos_window::Window;

use crate::building::{Layout, LightMix};
use crate::stage::{Scale, Stage};

/// Scene seconds per frame. Every frame of every run sees the same scene, whatever the
/// frame rate, so two runs (and two machines) draw the same frames.
pub const STEP: f32 = 1.0 / 60.0;

/// Names of the light build timestamp spans, in order.
const PASSES: [&str; 4] = ["copy", "world direct", "world bounce", "tier"];

pub struct Plan {
    pub seconds: f32,
    pub warmup_frames: u32,
    pub lights: Vec<usize>,
    pub dynamic: Vec<u32>,
    pub scales: Vec<Scale>,
    pub day: f32,
    pub layout: Layout,
}

struct Run {
    scale: Scale,
    mix: LightMix,
    lamps: usize,
    occluders: usize,
    frames: usize,
    seconds: f64,
    frame_ms: Vec<f64>,
    cpu_ms: Vec<f64>,
    gpu_ms: Vec<f64>,
    builds: Vec<LightBuildTimes>,
    tier: TierStats,
}

impl Run {
    fn frame_mean(&self) -> f64 {
        mean(&self.frame_ms)
    }

    fn frame_p95(&self) -> f64 {
        let mut sorted = self.frame_ms.clone();
        sorted.sort_by(f64::total_cmp);
        sorted
            .get(((sorted.len() as f64 * 0.95) as usize).min(sorted.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0.0)
    }

    /// Light build GPU time spread over the frames: what the light costs per frame.
    fn light_per_frame(&self) -> f64 {
        self.builds
            .iter()
            .map(LightBuildTimes::total_ms)
            .sum::<f64>()
            / self.frames.max(1) as f64
    }

    fn pass_per_frame(&self, pass: usize) -> f64 {
        self.builds
            .iter()
            .map(|b| b.passes_ms.get(pass).copied().unwrap_or(0.0))
            .sum::<f64>()
            / self.frames.max(1) as f64
    }

    fn builds_per_second(&self) -> f64 {
        self.builds.len() as f64 / self.seconds.max(1.0e-9)
    }
}

type Metric = Box<dyn Fn(&Run) -> f64>;

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

/// Run every configuration in `plan` and return the report text.
pub fn run(
    renderer: &mut Renderer,
    window: &mut Window,
    stage: &mut Stage,
    camera: &Camera,
    plan: &Plan,
) -> Result<String, String> {
    let mut profiled = true;
    let mut runs = Vec::new();
    for &scale in &plan.scales {
        stage.set_scale(scale);
        for &count in &plan.lights {
            for &dynamic_pct in &plan.dynamic {
                stage.mix = LightMix {
                    count,
                    dynamic_pct,
                    layout: plan.layout,
                };
                stage.rewind(plan.day);
                let run = run_one(renderer, window, stage, camera, plan, &mut profiled)?;
                eprintln!(
                    "bench {:>5} lights={:>3} dynamic={:>3}% frames={:>4} frame={:.2} ms p95={:.2} raster gpu={:.2} ms light gpu={:.2} ms/frame builds/s={:.1}",
                    scale.label(),
                    run.lamps,
                    dynamic_pct,
                    run.frames,
                    run.frame_mean(),
                    run.frame_p95(),
                    mean(&run.gpu_ms),
                    run.light_per_frame(),
                    run.builds_per_second(),
                );
                runs.push(run);
            }
        }
    }
    Ok(report(
        plan,
        &runs,
        renderer.width(),
        renderer.height(),
        profiled,
    ))
}

fn run_one(
    renderer: &mut Renderer,
    window: &mut Window,
    stage: &mut Stage,
    camera: &Camera,
    plan: &Plan,
    profiled: &mut bool,
) -> Result<Run, String> {
    // Host time of the draw call, and its GPU time when the fence had already signaled.
    let mut frame = |renderer: &mut Renderer,
                     stage: &mut Stage,
                     profiled: &mut bool|
     -> Result<(f64, Option<f64>), String> {
        let pumped = window.pump();
        if pumped.closing {
            return Err("window closed during the benchmark".into());
        }
        stage.advance(STEP);
        if *profiled {
            match renderer.draw_profiled(&stage.world, camera, &[], false, false) {
                Ok((_, timing)) => {
                    let gpu = timing.gpu.map(|d| d.as_secs_f64() * 1.0e3);
                    return Ok((timing.cpu.as_secs_f64() * 1.0e3, gpu));
                }
                Err(err) if err.contains("timestamp") => {
                    eprintln!("genos-stress: {err}; raster GPU time is not measured");
                    *profiled = false;
                }
                Err(err) => return Err(err),
            }
        }
        let at = Instant::now();
        renderer.draw_with_overlay(&stage.world, camera, &[], false, false)?;
        Ok((at.elapsed().as_secs_f64() * 1.0e3, None))
    };
    for _ in 0..plan.warmup_frames {
        frame(renderer, stage, profiled)?;
    }
    if *profiled {
        renderer.finish_gpu_times()?;
    }
    renderer.take_light_builds();
    let mut frame_ms = Vec::new();
    let mut cpu_ms = Vec::new();
    let mut gpu_ms = Vec::new();
    let mut builds = Vec::new();
    let start = Instant::now();
    let mut last = start;
    while start.elapsed().as_secs_f32() < plan.seconds || frame_ms.is_empty() {
        let (cpu, gpu) = frame(renderer, stage, profiled)?;
        cpu_ms.push(cpu);
        gpu_ms.extend(gpu);
        if *profiled {
            gpu_ms.extend(
                renderer
                    .poll_gpu_times()?
                    .iter()
                    .map(|d| d.as_secs_f64() * 1.0e3),
            );
        }
        builds.extend(renderer.take_light_builds());
        let now = Instant::now();
        frame_ms.push(now.duration_since(last).as_secs_f64() * 1.0e3);
        last = now;
    }
    let seconds = start.elapsed().as_secs_f64();
    if *profiled {
        gpu_ms.extend(
            renderer
                .finish_gpu_times()?
                .iter()
                .map(|d| d.as_secs_f64() * 1.0e3),
        );
    }
    builds.extend(renderer.take_light_builds());
    Ok(Run {
        scale: stage.scale,
        mix: stage.mix,
        lamps: stage.lamp_count(),
        occluders: stage.occluders(),
        frames: frame_ms.len(),
        seconds,
        frame_ms,
        cpu_ms,
        gpu_ms,
        builds,
        tier: renderer.tier_stats(),
    })
}

fn find(runs: &[Run], scale: Scale, count: usize, dynamic: u32) -> Option<&Run> {
    runs.iter()
        .find(|r| r.scale == scale && r.mix.count == count && r.mix.dynamic_pct == dynamic)
}

/// One table: rows are lamp count × dynamic share, columns the scales, then the change
/// from the first scale to each other scale.
fn table(out: &mut String, title: &str, plan: &Plan, runs: &[Run], metric: &dyn Fn(&Run) -> f64) {
    let _ = writeln!(out, "\n### {title}\n");
    let mut head = String::from("| lights | dynamic |");
    let mut rule = String::from("|---:|---:|");
    for scale in &plan.scales {
        let _ = write!(head, " {} |", scale.label());
        rule.push_str("---:|");
    }
    for scale in plan.scales.iter().skip(1) {
        let _ = write!(head, " Δ {} | Δ% |", scale.label());
        rule.push_str("---:|---:|");
    }
    let _ = writeln!(out, "{head}\n{rule}");
    for &count in &plan.lights {
        for &dynamic in &plan.dynamic {
            let mut row = format!("| {count} | {dynamic}% |");
            let values: Vec<Option<f64>> = plan
                .scales
                .iter()
                .map(|s| find(runs, *s, count, dynamic).map(metric))
                .collect();
            for value in &values {
                match value {
                    Some(v) => {
                        let _ = write!(row, " {v:.2} |");
                    }
                    None => row.push_str(" - |"),
                }
            }
            for value in values.iter().skip(1) {
                match (values[0], value) {
                    (Some(a), Some(b)) => {
                        let pct = if a.abs() > 1.0e-9 {
                            (b - a) / a * 100.0
                        } else {
                            f64::NAN
                        };
                        let _ = write!(row, " {:+.2} | {:+.0}% |", b - a, pct);
                    }
                    _ => row.push_str(" - | - |"),
                }
            }
            let _ = writeln!(out, "{row}");
        }
    }
}

fn report(plan: &Plan, runs: &[Run], width: u32, height: u32, profiled: bool) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "## genos-stress benchmark\n");
    let _ = writeln!(
        out,
        "{width}x{height}, {} s per configuration after {} warm-up frames, scene step {:.4} s per frame, sun starting at day {:.3}, lamp layout `{}`.",
        plan.seconds,
        plan.warmup_frames,
        STEP,
        plan.day,
        plan.layout.label()
    );
    for scale in &plan.scales {
        if let Some(run) = runs.iter().find(|r| r.scale == *scale) {
            let _ = writeln!(
                out,
                "- {}: {} sections, {:.0} m × {:.0} m, {} occluders (walls and solids), tier window {} bricks, {} resident, {} pending, {} dropped at the end of the last run.",
                scale.label(),
                scale.sections(),
                scale.cols as f32 * crate::building::SECTION,
                scale.rows as f32 * crate::building::SECTION,
                run.occluders,
                run.tier.window_bricks,
                run.tier.bricks,
                run.tier.pending_bricks,
                run.tier.dropped_bricks,
            );
        }
    }
    table(
        &mut out,
        "Frame time, ms (wall clock per frame)",
        plan,
        runs,
        &Run::frame_mean,
    );
    table(&mut out, "Frame time p95, ms", plan, runs, &Run::frame_p95);
    table(&mut out, "Draw call CPU, ms", plan, runs, &|r| {
        mean(&r.cpu_ms)
    });
    if profiled {
        table(&mut out, "Raster GPU, ms per frame", plan, runs, &|r| {
            mean(&r.gpu_ms)
        });
    }
    table(
        &mut out,
        "Light build GPU, ms per frame",
        plan,
        runs,
        &Run::light_per_frame,
    );
    table(
        &mut out,
        "Light builds per second",
        plan,
        runs,
        &Run::builds_per_second,
    );
    for (pass, name) in PASSES.iter().enumerate() {
        table(
            &mut out,
            &format!("Light pass `{name}`, GPU ms per frame"),
            plan,
            runs,
            &move |r: &Run| r.pass_per_frame(pass),
        );
    }
    per_lamp(&mut out, plan, runs, profiled);
    out
}

/// Cost of one more lamp from the slope between the fewest and the most lamps, at 0%
/// dynamic (static lamps) and at 100% (dynamic lamps).
fn per_lamp(out: &mut String, plan: &Plan, runs: &[Run], profiled: bool) {
    let (Some(&lo), Some(&hi)) = (plan.lights.iter().min(), plan.lights.iter().max()) else {
        return;
    };
    if hi <= lo {
        return;
    }
    let _ = writeln!(
        out,
        "\n### Cost of one lamp, ms (slope from {lo} to {hi} lamps)\n\n| scale | metric | per static lamp | per dynamic lamp | dynamic premium |\n|---|---|---:|---:|---:|"
    );
    let mut metrics: Vec<(&str, Metric)> = vec![
        ("frame", Box::new(Run::frame_mean)),
        ("light GPU", Box::new(Run::light_per_frame)),
    ];
    if profiled {
        metrics.push(("raster GPU", Box::new(|r: &Run| mean(&r.gpu_ms))));
    }
    for scale in &plan.scales {
        for (name, metric) in &metrics {
            let slope = |dynamic: u32| {
                let a = find(runs, *scale, lo, dynamic)?;
                let b = find(runs, *scale, hi, dynamic)?;
                Some((metric(b) - metric(a)) / (b.lamps as f64 - a.lamps as f64).max(1.0))
            };
            let cell = |v: Option<f64>| v.map(|v| format!("{v:.4}")).unwrap_or_else(|| "-".into());
            let (s, d) = (slope(0), slope(100));
            let premium = s.zip(d).map(|(s, d)| d - s);
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {} |",
                scale.label(),
                name,
                cell(s),
                cell(d),
                cell(premium)
            );
        }
    }
}
