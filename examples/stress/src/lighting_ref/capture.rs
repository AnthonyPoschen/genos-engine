//! Live frames for one check.
//!
//! The path trace is the reference. These frames are the candidate. Every mode
//! uses the same [`super::schedule::FramePlan`]: warmup, then an approach or an
//! event jump, then settle. Wall time is logged per frame and does not choose
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
    /// The settle cap was reached before the threshold. A passing verdict becomes review.
    pub review: bool,
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
    let frame = window.pump();
    let mut renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    if renderer.width() != opts.width || renderer.height() != opts.height {
        return Err(format!(
            "proof window is {}x{}, the reference is {}x{}",
            renderer.width(),
            renderer.height(),
            opts.width,
            opts.height
        ));
    }
    let mut shots = Vec::new();
    let mut skipped = Vec::new();
    for job in &opts.jobs {
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
    let max_offset = opts.plan.capture_offsets.iter().copied().max().unwrap_or(0);
    let mut log = Vec::new();
    let mut settle_pics = Vec::new();
    let mut settle_frame_ids = Vec::new();
    let mut crossed: Option<u32> = None;
    let mut prev_phase: Option<Phase> = None;

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
        let mut params = opts.params;
        params.reference_noise = job.reference_noise;
        let mean_rel = job
            .reference
            .as_ref()
            .and_then(|reference| mean_relative(reference, &picture, &params).ok());
        let blur_rel = job
            .reference
            .as_ref()
            .and_then(|reference| blur_relative(reference, &picture, &params).ok());
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
        if stop_after(
            opts.plan.settle,
            settle_count,
            crossed,
            job.still,
            max_offset,
        ) {
            break;
        }
    }

    if settle_pics.is_empty() {
        return Err("the frame plan finished with no settle frame".into());
    }
    let settle_count = settle_pics.len() as u32;
    let candidate_at = candidate_settle_index(opts.plan.settle, settle_count, crossed);
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
    Ok(LiveShot {
        scene: job.scene.clone(),
        checkpoint: job.checkpoint.clone(),
        frames,
        settled,
        event_frames,
        log,
        review: cap_hit,
        note: shot_note(
            &opts.plan,
            candidate_frame,
            candidate_error,
            candidate_ratio,
            job.reference_noise,
            picture_height,
            cap_hit,
            job.event.is_some(),
            job.still && candidate_index + 1 >= settle_pics.len() && settle_pics.len() >= 2,
        ),
    })
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
    tail_is_previous: bool,
) -> String {
    let settle = match plan.settle {
        SettleMode::Frames(n) => format!("{n} settle frames"),
        SettleMode::Metric { threshold, cap } => {
            let line = settle_line(threshold, reference_noise, height);
            if cap_hit {
                format!(
                    "settle cap {cap} reached before blurred relative luminance was under {line:.4} (threshold {threshold}, reference noise {reference_noise:.4})"
                )
            } else {
                format!(
                    "settled when blurred relative luminance was under {line:.4} (threshold {threshold}, reference noise {reference_noise:.4}, cap {cap})"
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
        format!(" Captures at event+{list}.")
    } else {
        String::new()
    };
    let review = if cap_hit {
        " The settle cap was reached before the threshold. This capture is marked for review."
    } else {
        ""
    };
    let tail = if tail_is_previous {
        " Flicker uses the previous held frame because the run ended on the candidate."
    } else {
        ""
    };
    format!(
        "fixed dt {} s, warmup {} frames, {settle}, candidate frame {candidate_frame}, blur_rel {}, luma_ratio {}.{review}{tail}{captures}",
        schedule::format_dt(plan.fixed_dt),
        plan.warmup_frames,
        format_mean(blur_rel),
        format_mean(luma_ratio),
    )
}
