//! Live frames for one check.
//!
//! The path trace is the reference. These frames are the candidate. Every mode
//! uses the same [`super::schedule::FramePlan`]: warmup, then an approach or a
//! held scene, then settle. A metric event holds the pose before the jump for
//! the settle cap, then jumps. Wall time is logged per frame and does not choose
//! when to stop. `picture_settled` is logged and ignored on the first frame of
//! a pose, because that flag can still describe the pose before it.

use std::path::Path;
use std::time::Instant;

use genos_lighting_ref::{
    blur_relative, mean_luma_ratio, mean_relative, settle_line, MetricParams, Picture,
};
use genos_render::Renderer;
use genos_window::Window;

use super::scenes::{self, Shot};
use super::schedule::{
    self, candidate_settle_index, format_mean, stop_after, FramePlan, FrameRecord, Phase,
    SettleMode, Target, Timeline,
};

pub struct Job {
    pub scene: String,
    pub checkpoint: String,
    pub still: bool,
    pub event: Option<&'static str>,
    pub reference: Option<Picture>,
    /// Noise stored beside the reference. The settle line rises with it.
    pub reference_noise: f64,
}

pub struct LiveShot {
    pub scene: String,
    pub checkpoint: String,
    pub frames: Vec<Picture>,
    pub settled: Option<Picture>,
    pub event_frames: Vec<(u32, Picture)>,
    pub log: Vec<FrameRecord>,
    pub note: String,
    /// Frame index where a fresh renderer crossed the settle line.
    pub cold_frames: Option<f64>,
    /// `cold_frames` is the cap, and the line was not crossed.
    pub cold_settled: bool,
}

pub struct CaptureOutcome {
    pub shots: Vec<LiveShot>,
    pub skipped: Vec<(String, String, String)>,
}

pub struct CaptureOptions {
    pub width: u32,
    pub height: u32,
    pub full_effort: bool,
    /// Renderer bounce count. `None` leaves the scene. Direct-only still
    /// forces bounce 0 and ambient off inside `open_stage`.
    pub bounces: Option<u32>,
    pub lavapipe_small: bool,
    pub plan: FramePlan,
    pub params: MetricParams,
    pub jobs: Vec<Job>,
    /// Keep one renderer, and its lighting caches, across checkpoints.
    /// The default opens a fresh renderer for each checkpoint.
    pub warm: bool,
}

pub fn capture(opts: &CaptureOptions) -> Result<CaptureOutcome, String> {
    if opts.lavapipe_small && std::env::var_os("VK_ICD_FILENAMES").is_none() {
        let icd = Path::new("/usr/share/vulkan/icd.d/lvp_icd.json");
        if icd.exists() {
            // The loader reads this when the device opens. Capture has not
            // spawned another thread yet.
            unsafe { std::env::set_var("VK_ICD_FILENAMES", icd) }
        }
    }
    let mut window = Window::open_proof(opts.width, opts.height)?;
    window.set_title("Genos lighting reference check");
    let mut renderer = open_renderer(&mut window, opts.width, opts.height)?;
    let mut shots = Vec::new();
    let mut skipped = Vec::new();
    for (index, job) in opts.jobs.iter().enumerate() {
        if !opts.warm && index > 0 {
            // Drop the device before the next open. Assignment would keep the
            // old swapchain alive until the new one had already failed.
            drop(renderer);
            renderer = open_renderer(&mut window, opts.width, opts.height)?;
        }
        let Some(shot) = scenes::shot(&job.scene, &job.checkpoint) else {
            skipped.push((
                job.scene.clone(),
                job.checkpoint.clone(),
                "no camera for this checkpoint".into(),
            ));
            continue;
        };
        match capture_one(&mut window, &mut renderer, opts, job, &shot) {
            Ok(live) => shots.push(live),
            Err(err) => skipped.push((job.scene.clone(), job.checkpoint.clone(), err)),
        }
    }
    Ok(CaptureOutcome { shots, skipped })
}

fn open_renderer(window: &mut Window, width: u32, height: u32) -> Result<Renderer, String> {
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    if renderer.width() != width || renderer.height() != height {
        return Err(format!(
            "proof window is {}x{}, the reference is {}x{}",
            renderer.width(),
            renderer.height(),
            width,
            height
        ));
    }
    Ok(renderer)
}

fn capture_one(
    window: &mut Window,
    renderer: &mut Renderer,
    opts: &CaptureOptions,
    job: &Job,
    shot: &Shot,
) -> Result<LiveShot, String> {
    let mut timeline = Timeline::start(
        opts.plan.clone(),
        Target {
            box_time: shot.box_time,
            lamp_time: shot.lamp_time,
            day: shot.day,
            event: job.event.is_some(),
        },
    );
    let (mut stage, camera) = scenes::open_stage(shot, opts.full_effort, opts.bounces);
    renderer.set_lighting_seed(scenes::pose_seed(shot, opts.full_effort, opts.bounces));
    let max_offset = opts.plan.capture_offsets.iter().copied().max().unwrap_or(0);
    let mut log = Vec::new();
    let mut settle_pics = Vec::new();
    let mut settle_frame_ids = Vec::new();
    let mut crossed: Option<u32> = None;
    let mut prev_phase: Option<Phase> = None;
    let mut scene_frames: u32 = 0;

    loop {
        let step = timeline.next();
        scenes::apply_clocks(&mut stage, step.box_time, step.lamp_time, step.day);
        let started = Instant::now();
        let _ = window.pump();
        let pixels = renderer.draw(&stage.world, &camera, true)?;
        let wall_s = started.elapsed().as_secs_f64();
        let pixels = pixels.ok_or_else(|| "readback returned no picture".to_string())?;
        let picture = Picture::from_bgra(renderer.width(), renderer.height(), &pixels)?;
        let raw_settled = renderer.picture_settled();
        let trusted = prev_phase == Some(step.phase);
        prev_phase = Some(step.phase);
        let mean_rel = job
            .reference
            .as_ref()
            .and_then(|reference| mean_relative(reference, &picture, &opts.params).ok());
        let blur_rel = job
            .reference
            .as_ref()
            .and_then(|reference| blur_relative(reference, &picture, &opts.params).ok());
        let luma_ratio = job
            .reference
            .as_ref()
            .and_then(|reference| mean_luma_ratio(reference, &picture).ok());
        if step.phase == Phase::Settle {
            if crossed.is_none() {
                if let SettleMode::Metric { threshold, .. } = opts.plan.settle {
                    let line = settle_line(threshold, job.reference_noise, picture.height);
                    if blur_rel.is_some_and(|err| err <= line) {
                        crossed = Some(step.settle_index);
                    }
                }
            }
            settle_pics.push(picture);
            settle_frame_ids.push(step.frame);
        } else if step.phase == Phase::Scene {
            // The picture is scored after this pose, on the event. The hold is
            // the settle cap, the same count for every run of this checkpoint.
            scene_frames += 1;
            if let SettleMode::Metric { cap, .. } = opts.plan.settle {
                if scene_frames >= cap {
                    timeline.release_event();
                }
            }
        }
        let settled_text = if trusted && raw_settled {
            "true"
        } else if trusted {
            "false"
        } else {
            "false untrusted"
        };
        eprintln!(
            "lighting-ref: {} {} frame {} sim {:.6} wall {:.3}s settled {settled_text} mean_rel {} blur_rel {} luma_ratio {}",
            job.scene,
            job.checkpoint,
            step.frame,
            step.sim_time,
            wall_s,
            format_mean(mean_rel),
            format_mean(blur_rel),
            format_mean(luma_ratio),
        );
        log.push(FrameRecord {
            scene: job.scene.clone(),
            checkpoint: job.checkpoint.clone(),
            frame: step.frame,
            sim_time: step.sim_time,
            wall_s,
            picture_settled: raw_settled,
            trusted,
            mean_rel,
            blur_rel,
            luma_ratio,
            captured: false,
            phase: step.phase,
            box_time: step.box_time,
            lamp_time: step.lamp_time,
            day: step.day,
        });
        let settle_count = settle_pics.len() as u32;
        if stop_after(opts.plan.settle, settle_count, job.still, max_offset) {
            break;
        }
    }

    if settle_pics.is_empty() {
        return Err("the frame plan finished with no settle frame".into());
    }
    let settle_count = settle_pics.len() as u32;
    let candidate_at = candidate_settle_index(opts.plan.settle, settle_count);
    let candidate_index = (candidate_at - 1) as usize;
    let settled = settle_pics.get(candidate_index).cloned();
    let frames = flicker_pair(&settle_pics, candidate_index);
    let mut keep = Vec::new();
    if let Some(frame) = settle_frame_ids.get(candidate_index) {
        keep.push(*frame);
    }
    if let Some(next) = settle_frame_ids.get(candidate_index + 1) {
        keep.push(*next);
    } else if candidate_index > 0 {
        if let Some(prev) = settle_frame_ids.get(candidate_index - 1) {
            keep.push(*prev);
        }
    }
    let mut event_frames = Vec::new();
    for (index, picture) in settle_pics.iter().enumerate() {
        let settle_index = (index as u32) + 1;
        if opts.plan.capture_offsets.contains(&settle_index) {
            if let Some(frame) = settle_frame_ids.get(index) {
                keep.push(*frame);
            }
            if job.event.is_some() {
                event_frames.push((settle_index, picture.clone()));
            }
        }
    }
    for row in &mut log {
        row.captured = keep.contains(&row.frame);
    }
    let cap_hit = matches!(opts.plan.settle, SettleMode::Metric { cap, .. } if crossed.is_none() && settle_count >= cap);
    let candidate_frame = settle_frame_ids.get(candidate_index).copied().unwrap_or(0);
    let candidate_error = log
        .iter()
        .find(|row| row.frame == candidate_frame)
        .and_then(|row| row.blur_rel);
    let candidate_ratio = log
        .iter()
        .find(|row| row.frame == candidate_frame)
        .and_then(|row| row.luma_ratio);
    let picture_height = settled
        .as_ref()
        .map(|picture| picture.height)
        .unwrap_or(opts.height);
    let cold = cold_start(opts, &log, job.reference_noise, picture_height, cap_hit);
    Ok(LiveShot {
        scene: job.scene.clone(),
        checkpoint: job.checkpoint.clone(),
        frames,
        settled,
        event_frames,
        log,
        cold_frames: cold.frames,
        cold_settled: cold.settled,
        note: shot_note(
            &opts.plan,
            candidate_frame,
            candidate_error,
            candidate_ratio,
            job.reference_noise,
            picture_height,
            cap_hit,
            job.event.is_some(),
            scene_frames,
            job.still && candidate_index + 1 >= settle_pics.len() && settle_pics.len() >= 2,
            opts.warm,
            cold,
        ),
    })
}

struct ColdStart {
    frames: Option<f64>,
    settled: bool,
}

fn cold_start(
    opts: &CaptureOptions,
    log: &[FrameRecord],
    reference_noise: f64,
    height: u32,
    cap_hit: bool,
) -> ColdStart {
    if opts.warm {
        return ColdStart {
            frames: None,
            settled: false,
        };
    }
    let SettleMode::Metric { threshold, cap } = opts.plan.settle else {
        return ColdStart {
            frames: None,
            settled: false,
        };
    };
    let line = settle_line(threshold, reference_noise, height);
    if let Some(row) = log
        .iter()
        .find(|row| row.phase == Phase::Settle && row.blur_rel.is_some_and(|error| error <= line))
    {
        return ColdStart {
            frames: Some(f64::from(row.frame)),
            settled: true,
        };
    }
    if cap_hit {
        return ColdStart {
            frames: Some(f64::from(cap)),
            settled: false,
        };
    }
    ColdStart {
        frames: None,
        settled: false,
    }
}

fn flicker_pair(pics: &[Picture], candidate: usize) -> Vec<Picture> {
    let Some(chosen) = pics.get(candidate) else {
        return Vec::new();
    };
    if let Some(next) = pics.get(candidate + 1) {
        vec![chosen.clone(), next.clone()]
    } else if candidate > 0 {
        vec![pics[candidate - 1].clone(), chosen.clone()]
    } else {
        vec![chosen.clone()]
    }
}

fn shot_note(
    plan: &FramePlan,
    candidate_frame: u32,
    blur_rel: Option<f64>,
    luma_ratio: Option<f64>,
    reference_noise: f64,
    height: u32,
    cap_hit: bool,
    event: bool,
    scene_frames: u32,
    tail_is_previous: bool,
    warm: bool,
    cold: ColdStart,
) -> String {
    let settle = match plan.settle {
        SettleMode::Frames(n) => format!("{n} settle frames"),
        SettleMode::Metric { threshold, cap } => {
            let line = settle_line(threshold, reference_noise, height);
            if cap_hit {
                format!(
                    "scored frame {cap} of the {cap} frame budget; blurred relative luminance stayed above {line:.4} (threshold {threshold}, reference noise {reference_noise:.4})"
                )
            } else {
                format!(
                    "scored frame {cap} of the {cap} frame budget; the cold-start line {line:.4} was crossed earlier (threshold {threshold}, reference noise {reference_noise:.4})"
                )
            }
        }
    };
    let captures = if event {
        let list = plan
            .capture_offsets
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",+");
        format!(
            " The event fired after {scene_frames} scene frames on the pose before the jump. Captures at event+{list}."
        )
    } else {
        String::new()
    };
    let tail = if tail_is_previous {
        " Flicker uses the previous held frame because the run ended on the candidate."
    } else {
        ""
    };
    let cold_note = if warm {
        " Warm chain: lighting caches were kept from the previous checkpoint."
    } else if let Some(frames) = cold.frames {
        if cold.settled {
            return format!(
                "fixed dt {} s, warmup {} frames, {settle}, candidate frame {candidate_frame}, blur_rel {}, luma_ratio {}. Cold start settled at frame {frames:.0}. The 180 frame cold-start budget is separate from this picture settle.{tail}{captures}",
                schedule::format_dt(plan.fixed_dt),
                plan.warmup_frames,
                format_mean(blur_rel),
                format_mean(luma_ratio),
            );
        }
        return format!(
            "fixed dt {} s, warmup {} frames, {settle}, candidate frame {candidate_frame}, blur_rel {}, luma_ratio {}. Renderer issue: cold start did not cross the settle line within {frames:.0} frames. The picture is scored on the cap frame. The 180 frame cold-start budget was not raised.{tail}{captures}",
            schedule::format_dt(plan.fixed_dt),
            plan.warmup_frames,
            format_mean(blur_rel),
            format_mean(luma_ratio),
        );
    } else {
        ""
    };
    format!(
        "fixed dt {} s, warmup {} frames, {settle}, candidate frame {candidate_frame}, blur_rel {}, luma_ratio {}.{cold_note}{tail}{captures}",
        schedule::format_dt(plan.fixed_dt),
        plan.warmup_frames,
        format_mean(blur_rel),
        format_mean(luma_ratio),
    )
}
