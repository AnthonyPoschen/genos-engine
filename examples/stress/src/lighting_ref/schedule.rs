//! The frame clock for a reference check.
//!
//! Scene time advances by `fixed_dt` once per frame. Wall time is recorded and
//! does not choose the pose. Owner mode and lavapipe-small use this same plan.
//! Lavapipe-small changes the picture size and the sample count only.

use std::fmt::Write as _;
use std::path::Path;

/// Scene seconds per frame. The same step the stress benchmark uses.
pub const DEFAULT_DT: f32 = 1.0 / 60.0;
/// Blurred relative luminance that ends a metric settle, before the
/// reference-noise floor. Under the pass band of that blurred error.
pub const DEFAULT_SETTLE_THRESHOLD: f64 = 0.05;
/// Settle frames before a metric settle stops anyway.
pub const DEFAULT_SETTLE_CAP: u32 = 180;
/// Seconds in one day at sun speed 1. Matches the stress binary's default.
pub const DAY_SECONDS: f32 = 120.0;
pub const SUN_SPEED: f32 = 1.0;

#[derive(Clone, Debug, PartialEq)]
pub struct FramePlan {
    pub fixed_dt: f32,
    pub warmup_frames: u32,
    pub settle: SettleMode,
    pub capture_offsets: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettleMode {
    /// Draw this many frames on the scored pose. The candidate is that frame.
    Frames(u32),
    /// Draw until blurred relative luminance is at or under the line, or `cap`
    /// frames. The line is `threshold` raised by this checkpoint's reference
    /// noise after the blur.
    Metric { threshold: f64, cap: u32 },
}

impl FramePlan {
    pub fn default_plan() -> Self {
        Self {
            fixed_dt: DEFAULT_DT,
            warmup_frames: 0,
            settle: SettleMode::Metric {
                threshold: DEFAULT_SETTLE_THRESHOLD,
                cap: DEFAULT_SETTLE_CAP,
            },
            capture_offsets: vec![1, 2, 4, 8],
        }
    }
}

/// Arguments win over the environment. The two settle switches cannot both be set.
pub struct PlanText<'a> {
    pub fixed_dt: Option<&'a str>,
    pub warmup_frames: Option<&'a str>,
    pub settle_frames: Option<&'a str>,
    pub settle: Option<&'a str>,
    pub settle_cap: Option<&'a str>,
    pub capture_frames: Option<&'a str>,
}

#[cfg(test)]
impl<'a> PlanText<'a> {
    fn empty() -> Self {
        Self {
            fixed_dt: None,
            warmup_frames: None,
            settle_frames: None,
            settle: None,
            settle_cap: None,
            capture_frames: None,
        }
    }
}

pub fn resolve_plan(args: &PlanText<'_>, env: &PlanText<'_>) -> Result<FramePlan, String> {
    let fixed_dt = match first(args.fixed_dt, env.fixed_dt) {
        Some(text) => parse_positive_f32("fixed dt", text)?,
        None => DEFAULT_DT,
    };
    let warmup_frames = match first(args.warmup_frames, env.warmup_frames) {
        Some(text) => parse_u32("warmup frames", text)?,
        None => 0,
    };
    let cap = match first(args.settle_cap, env.settle_cap) {
        Some(text) => parse_u32("settle cap", text)?,
        None => DEFAULT_SETTLE_CAP,
    };
    if cap == 0 {
        return Err("settle cap counts frames and is at least 1".into());
    }
    let capture_offsets = match first(args.capture_frames, env.capture_frames) {
        Some(text) => parse_capture_frames(text)?,
        None => vec![1, 2, 4, 8],
    };
    if args.settle_frames.is_some() && args.settle.is_some() {
        return Err("pass either --settle-frames or --settle".into());
    }
    let settle = if let Some(text) = args.settle_frames {
        SettleMode::Frames(parse_settle_count(text)?)
    } else if let Some(text) = args.settle {
        SettleMode::Metric {
            threshold: parse_threshold(text)?,
            cap,
        }
    } else if env.settle_frames.is_some() && env.settle.is_some() {
        return Err(
            "GENOS_LIGHTING_REF_SETTLE_FRAMES and GENOS_LIGHTING_REF_SETTLE are both set".into(),
        );
    } else if let Some(text) = env.settle_frames {
        SettleMode::Frames(parse_settle_count(text)?)
    } else if let Some(text) = env.settle {
        SettleMode::Metric {
            threshold: parse_threshold(text)?,
            cap,
        }
    } else {
        SettleMode::Metric {
            threshold: DEFAULT_SETTLE_THRESHOLD,
            cap,
        }
    };
    let mut plan = FramePlan::default_plan();
    plan.fixed_dt = fixed_dt;
    plan.warmup_frames = warmup_frames;
    plan.settle = settle;
    plan.capture_offsets = capture_offsets;
    Ok(plan)
}

/// `1,2,4,8` and `event+1,+2,+4,+8` are the same list. Offsets start at 1.
pub fn parse_capture_frames(text: &str) -> Result<Vec<u32>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("capture frames are empty".into());
    }
    let mut out = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err("capture frames have an empty entry".into());
        }
        let digits = part
            .strip_prefix("event+")
            .or_else(|| part.strip_prefix('+'))
            .unwrap_or(part);
        let n = parse_u32("capture frame", digits)?;
        if n == 0 {
            return Err(
                "capture offsets start at 1 (event+1 is the first frame of the new pose)".into(),
            );
        }
        out.push(n);
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

fn first<'a>(arg: Option<&'a str>, env: Option<&'a str>) -> Option<&'a str> {
    arg.or(env)
}

fn parse_settle_count(text: &str) -> Result<u32, String> {
    let n = parse_u32("settle frames", text)?;
    if n == 0 {
        return Err("settle frames count at least 1".into());
    }
    Ok(n)
}

fn parse_threshold(text: &str) -> Result<f64, String> {
    let value: f64 = text
        .parse()
        .map_err(|_| format!("expected a threshold, got {text}"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!("settle threshold is zero or greater, got {text}"));
    }
    Ok(value)
}

fn parse_positive_f32(name: &str, text: &str) -> Result<f32, String> {
    let value: f32 = text
        .parse()
        .map_err(|_| format!("expected a number for {name}, got {text}"))?;
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{name} is greater than 0, got {text}"));
    }
    Ok(value)
}

fn parse_u32(name: &str, text: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .map_err(|_| format!("expected a whole number for {name}, got {text}"))
}

/// Where the scored pose sits. An event holds the pose before the jump until
/// the scene is released, then jumps. A fixed settle count jumps after warmup.
/// Motion approaches the target by `fixed_dt`, then holds.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub box_time: f32,
    pub lamp_time: f32,
    pub day: f32,
    pub event: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Warmup,
    Approach,
    /// Pose before an event. The picture is not scored here.
    Scene,
    Settle,
}

impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Self::Warmup => "warmup",
            Self::Approach => "approach",
            Self::Scene => "scene",
            Self::Settle => "settle",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub frame: u32,
    pub sim_time: f32,
    pub box_time: f32,
    pub lamp_time: f32,
    pub day: f32,
    pub phase: Phase,
    /// 1-based index on the scored pose. Zero during warmup and approach.
    pub settle_index: u32,
    pub capture_offset: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Timeline {
    plan: FramePlan,
    kind: Kind,
    frame: u32,
    box_time: f32,
    lamp_time: f32,
    day: f32,
    settle_index: u32,
}

#[derive(Clone, Debug)]
enum Kind {
    Held,
    Event {
        box_after: f32,
        lamp_after: f32,
        /// Metric settles hold the before-pose until [`Timeline::release_event`].
        /// A fixed settle count jumps on the first scored frame.
        hold_scene: bool,
    },
    Approach {
        box_target: f32,
        lamp_target: f32,
        day_target: f32,
    },
}

impl Timeline {
    pub fn start(plan: FramePlan, target: Target) -> Self {
        let (kind, box_time, lamp_time, day) = if target.event {
            (
                Kind::Event {
                    box_after: target.box_time,
                    lamp_after: target.lamp_time,
                    hold_scene: matches!(plan.settle, SettleMode::Metric { .. }),
                },
                0.0,
                0.0,
                target.day,
            )
        } else if target.box_time > 0.0 || target.lamp_time > 0.0 {
            let approach_t = target.box_time.max(target.lamp_time);
            let day0 = target.day - approach_t * SUN_SPEED / DAY_SECONDS;
            (
                Kind::Approach {
                    box_target: target.box_time,
                    lamp_target: target.lamp_time,
                    day_target: target.day,
                },
                0.0,
                0.0,
                day0,
            )
        } else {
            (Kind::Held, 0.0, 0.0, target.day)
        };
        Self {
            plan,
            kind,
            frame: 0,
            box_time,
            lamp_time,
            day,
            settle_index: 0,
        }
    }

    /// The before-pose has settled. The next frame is the event.
    pub fn release_event(&mut self) {
        if let Kind::Event { hold_scene, .. } = &mut self.kind {
            *hold_scene = false;
        }
    }

    pub fn next(&mut self) -> Step {
        self.frame += 1;
        let phase = if self.frame <= self.plan.warmup_frames {
            Phase::Warmup
        } else {
            self.advance_pose()
        };
        let settle_index = if phase == Phase::Settle {
            self.settle_index += 1;
            self.settle_index
        } else {
            0
        };
        let capture_offset = (phase == Phase::Settle
            && self.plan.capture_offsets.contains(&settle_index))
        .then_some(settle_index);
        Step {
            frame: self.frame,
            sim_time: self.frame as f32 * self.plan.fixed_dt,
            box_time: self.box_time,
            lamp_time: self.lamp_time,
            day: self.day,
            phase,
            settle_index,
            capture_offset,
        }
    }

    fn advance_pose(&mut self) -> Phase {
        match self.kind.clone() {
            Kind::Held => Phase::Settle,
            Kind::Event {
                box_after,
                lamp_after,
                hold_scene,
            } => {
                if hold_scene {
                    Phase::Scene
                } else {
                    self.box_time = box_after;
                    self.lamp_time = lamp_after;
                    Phase::Settle
                }
            }
            Kind::Approach {
                box_target,
                lamp_target,
                day_target,
            } => {
                let box_rem = (box_target - self.box_time).max(0.0);
                let lamp_rem = (lamp_target - self.lamp_time).max(0.0);
                let motion = box_rem.max(lamp_rem);
                if motion > 1.0e-6 {
                    let step = motion.min(self.plan.fixed_dt);
                    self.box_time = (self.box_time + step).min(box_target);
                    self.lamp_time = (self.lamp_time + step).min(lamp_target);
                    let delta = step * SUN_SPEED / DAY_SECONDS;
                    self.day = (self.day + delta).min(day_target);
                }
                let arrived = (box_target - self.box_time) <= 1.0e-5
                    && (lamp_target - self.lamp_time) <= 1.0e-5;
                if arrived {
                    self.box_time = box_target;
                    self.lamp_time = lamp_target;
                    self.day = day_target;
                    Phase::Settle
                } else {
                    Phase::Approach
                }
            }
        }
    }
}

/// Stop drawing. `crossed_at` is the 1-based settle frame that met the metric.
/// A still camera keeps one more held frame after the candidate so flicker has a pair.
/// Capture offsets are drawn when they fit under the cap.
pub fn stop_after(
    settle: SettleMode,
    settle_count: u32,
    crossed_at: Option<u32>,
    still: bool,
    max_offset: u32,
) -> bool {
    if settle_count == 0 {
        return false;
    }
    match settle {
        SettleMode::Frames(n) => {
            let candidate = n.max(1);
            let through = candidate.max(max_offset);
            let need = if still {
                through.max(candidate + 1)
            } else {
                through
            };
            settle_count >= need
        }
        SettleMode::Metric { cap, .. } => {
            if settle_count >= cap {
                return true;
            }
            let Some(at) = crossed_at else {
                return false;
            };
            let captures_done = settle_count >= max_offset;
            let tail_done = !still || settle_count > at;
            captures_done && tail_done
        }
    }
}

/// 1-based settle index of the picture that is scored for mean error and FLIP.
pub fn candidate_settle_index(
    settle: SettleMode,
    settle_count: u32,
    crossed_at: Option<u32>,
) -> u32 {
    match settle {
        SettleMode::Frames(n) => n.min(settle_count).max(1),
        SettleMode::Metric { .. } => crossed_at.unwrap_or(settle_count).max(1),
    }
}

#[derive(Clone, Debug)]
pub struct FrameRecord {
    pub scene: String,
    pub checkpoint: String,
    pub frame: u32,
    pub sim_time: f32,
    pub wall_s: f64,
    pub picture_settled: bool,
    pub trusted: bool,
    pub mean_rel: Option<f64>,
    pub blur_rel: Option<f64>,
    pub luma_ratio: Option<f64>,
    pub captured: bool,
    pub phase: Phase,
    pub box_time: f32,
    pub lamp_time: f32,
    pub day: f32,
}

pub fn write_frames_tsv(path: &Path, rows: &[FrameRecord]) -> Result<(), String> {
    let mut text = String::from(
        "scene\tcheckpoint\tframe\tsim_time\twall_s\tpicture_settled\ttrusted\tmean_rel\tblur_rel\tluma_ratio\tcaptured\tphase\tbox_time\tlamp_time\tday\n",
    );
    for row in rows {
        let mean = row
            .mean_rel
            .map(|value| format!("{value:.6}"))
            .unwrap_or_default();
        let blur = row
            .blur_rel
            .map(|value| format!("{value:.6}"))
            .unwrap_or_default();
        let ratio = row
            .luma_ratio
            .map(|value| format!("{value:.6}"))
            .unwrap_or_default();
        let _ = writeln!(
            text,
            "{scene}\t{checkpoint}\t{frame}\t{sim:.6}\t{wall:.3}\t{settled}\t{trusted}\t{mean}\t{blur}\t{ratio}\t{captured}\t{phase}\t{box:.6}\t{lamp:.6}\t{day:.6}",
            scene = row.scene,
            checkpoint = row.checkpoint,
            frame = row.frame,
            sim = row.sim_time,
            wall = row.wall_s,
            settled = row.picture_settled,
            trusted = row.trusted,
            captured = u8::from(row.captured),
            phase = row.phase.name(),
            box = row.box_time,
            lamp = row.lamp_time,
            day = row.day,
        );
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(path, text).map_err(|err| err.to_string())
}

pub fn format_dt(dt: f32) -> String {
    if (dt - DEFAULT_DT).abs() < 1.0e-6 {
        "1/60".to_string()
    } else {
        format!("{dt}")
    }
}

pub fn format_mean(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{value:.4}"),
        None => "—".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(cap: u32) -> SettleMode {
        SettleMode::Metric {
            threshold: 0.05,
            cap,
        }
    }

    fn plan(settle: SettleMode, warmup: u32, dt: f32) -> FramePlan {
        FramePlan {
            fixed_dt: dt,
            warmup_frames: warmup,
            settle,
            capture_offsets: vec![1, 2, 4, 8],
        }
    }

    #[test]
    fn capture_lists_with_and_without_the_event_word_match() {
        assert_eq!(
            parse_capture_frames("event+1,+2,+4,+8").unwrap(),
            parse_capture_frames("1,2,4,8").unwrap()
        );
        assert_eq!(
            parse_capture_frames("event+8,event+1,4,2").unwrap(),
            vec![1, 2, 4, 8]
        );
    }

    #[test]
    fn a_zero_capture_offset_is_rejected() {
        let err = parse_capture_frames("event+0").unwrap_err();
        assert!(err.contains("event+1"), "{err}");
    }

    #[test]
    fn args_win_and_the_two_settle_switches_conflict() {
        let args = PlanText {
            settle_frames: Some("12"),
            ..PlanText::empty()
        };
        let env = PlanText {
            settle: Some("0.01"),
            fixed_dt: Some("0.1"),
            ..PlanText::empty()
        };
        let plan = resolve_plan(&args, &env).unwrap();
        assert_eq!(plan.settle, SettleMode::Frames(12));
        assert!((plan.fixed_dt - 0.1).abs() < 1.0e-6);

        let both = PlanText {
            settle_frames: Some("4"),
            settle: Some("0.05"),
            ..PlanText::empty()
        };
        assert!(resolve_plan(&both, &PlanText::empty()).is_err());

        let env_both = PlanText {
            settle_frames: Some("4"),
            settle: Some("0.05"),
            ..PlanText::empty()
        };
        assert!(resolve_plan(&PlanText::empty(), &env_both).is_err());
    }

    #[test]
    fn the_default_plan_is_a_sixtieth_and_the_same_for_every_mode() {
        let owner = FramePlan::default_plan();
        let lavapipe = FramePlan::default_plan();
        assert_eq!(owner, lavapipe);
        assert!((owner.fixed_dt - DEFAULT_DT).abs() < 1.0e-6);
        assert_eq!(owner.capture_offsets, vec![1, 2, 4, 8]);
    }

    #[test]
    fn a_held_pose_warms_up_then_settles_in_place() {
        let mut timeline = Timeline::start(
            plan(metric(30), 2, DEFAULT_DT),
            Target {
                box_time: 0.0,
                lamp_time: 0.0,
                day: 0.25,
                event: false,
            },
        );
        let first = timeline.next();
        let second = timeline.next();
        let third = timeline.next();
        assert_eq!(first.phase, Phase::Warmup);
        assert_eq!(second.phase, Phase::Warmup);
        assert_eq!(third.phase, Phase::Settle);
        assert_eq!(third.settle_index, 1);
        assert!((third.day - 0.25).abs() < 1.0e-5);
        assert!((third.sim_time - 3.0 / 60.0).abs() < 1.0e-5);
        assert_eq!(third.box_time, 0.0);
    }

    #[test]
    fn an_event_offset_is_counted_on_the_pose_after_the_jump() {
        let mut timeline = Timeline::start(
            plan(SettleMode::Frames(4), 1, DEFAULT_DT),
            Target {
                box_time: 0.45,
                lamp_time: 0.0,
                day: 0.25,
                event: true,
            },
        );
        let warm = timeline.next();
        assert_eq!(warm.phase, Phase::Warmup);
        assert_eq!(warm.box_time, 0.0);
        let jump = timeline.next();
        assert_eq!(jump.phase, Phase::Settle);
        assert_eq!(jump.settle_index, 1);
        assert_eq!(jump.capture_offset, Some(1));
        assert!((jump.box_time - 0.45).abs() < 1.0e-6);
        assert!((jump.day - 0.25).abs() < 1.0e-6);
        let mut last = jump;
        for _ in 0..3 {
            last = timeline.next();
        }
        assert_eq!(last.settle_index, 4);
        assert_eq!(last.capture_offset, Some(4));
        assert!((last.box_time - 0.45).abs() < 1.0e-6);
    }

    #[test]
    fn a_metric_event_holds_the_before_pose_until_the_scene_is_released() {
        let mut timeline = Timeline::start(
            plan(metric(30), 0, DEFAULT_DT),
            Target {
                box_time: 0.45,
                lamp_time: 0.2,
                day: 0.25,
                event: true,
            },
        );
        let held = timeline.next();
        assert_eq!(held.phase, Phase::Scene);
        assert_eq!(held.settle_index, 0);
        assert_eq!(held.capture_offset, None);
        assert_eq!(held.box_time, 0.0);
        assert_eq!(held.lamp_time, 0.0);
        assert!((held.day - 0.25).abs() < 1.0e-6);
        let still = timeline.next();
        assert_eq!(still.phase, Phase::Scene);
        assert_eq!(still.box_time, 0.0);
        timeline.release_event();
        let jump = timeline.next();
        assert_eq!(jump.phase, Phase::Settle);
        assert_eq!(jump.settle_index, 1);
        assert_eq!(jump.capture_offset, Some(1));
        assert!((jump.box_time - 0.45).abs() < 1.0e-6);
        assert!((jump.lamp_time - 0.2).abs() < 1.0e-6);
        assert!((jump.day - 0.25).abs() < 1.0e-6);
    }

    #[test]
    fn motion_lands_on_the_reference_time_and_holds() {
        let dt = 0.1;
        let mut timeline = Timeline::start(
            plan(SettleMode::Frames(1), 0, dt),
            Target {
                box_time: 0.25,
                lamp_time: 0.25,
                day: 0.25,
                event: false,
            },
        );
        let mut landed = None;
        for _ in 0..8 {
            let step = timeline.next();
            if step.phase == Phase::Settle {
                landed = Some(step);
                break;
            }
            assert!(step.box_time < 0.25);
        }
        let landed = landed.expect("approach reaches 0.25");
        assert!((landed.box_time - 0.25).abs() < 1.0e-5);
        assert!((landed.lamp_time - 0.25).abs() < 1.0e-5);
        assert!((landed.day - 0.25).abs() < 1.0e-5);
        assert_eq!(landed.settle_index, 1);
        let held = timeline.next();
        assert_eq!(held.phase, Phase::Settle);
        assert!((held.box_time - 0.25).abs() < 1.0e-5);
        assert!((held.day - 0.25).abs() < 1.0e-5);
    }

    #[test]
    fn a_one_second_approach_at_a_sixtieth_is_sixty_frames() {
        let mut timeline = Timeline::start(
            plan(SettleMode::Frames(1), 0, DEFAULT_DT),
            Target {
                box_time: 1.0,
                lamp_time: 1.0,
                day: 0.25,
                event: false,
            },
        );
        let mut frame = 0;
        for _ in 0..80 {
            let step = timeline.next();
            frame = step.frame;
            if step.phase == Phase::Settle {
                assert!((step.box_time - 1.0).abs() < 1.0e-4);
                break;
            }
        }
        assert!((56..=64).contains(&frame), "landed on frame {frame}");
    }

    #[test]
    fn the_cap_stops_and_a_still_tail_waits_for_one_more_frame() {
        let mode = metric(180);
        assert!(!stop_after(mode, 5, Some(5), true, 8));
        assert!(stop_after(mode, 8, Some(5), true, 8));
        assert!(!stop_after(mode, 8, Some(8), true, 8));
        assert!(stop_after(mode, 9, Some(8), true, 8));
        assert!(stop_after(mode, 8, Some(8), false, 8));
        assert!(stop_after(metric(2), 2, None, true, 8));
        assert_eq!(candidate_settle_index(mode, 8, Some(5)), 5);
        assert_eq!(candidate_settle_index(mode, 8, None), 8);
        assert!(!stop_after(SettleMode::Frames(10), 10, None, true, 8));
        assert!(stop_after(SettleMode::Frames(10), 11, None, true, 8));
        assert!(stop_after(SettleMode::Frames(4), 8, None, true, 8));
        assert_eq!(candidate_settle_index(SettleMode::Frames(4), 8, None), 4);
    }

    #[test]
    fn the_frame_log_records_one_row_per_frame() {
        let path = std::env::temp_dir().join(format!("lref-frames-{}.tsv", std::process::id()));
        write_frames_tsv(
            &path,
            &[FrameRecord {
                scene: "stress-still".into(),
                checkpoint: "hall".into(),
                frame: 3,
                sim_time: 0.05,
                wall_s: 1.25,
                picture_settled: false,
                trusted: true,
                mean_rel: Some(0.041),
                blur_rel: Some(0.02),
                luma_ratio: Some(1.0),
                captured: true,
                phase: Phase::Settle,
                box_time: 0.0,
                lamp_time: 0.0,
                day: 0.25,
            }],
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("scene\tcheckpoint\tframe\t"));
        assert!(text.contains("stress-still\thall\t3\t"));
        assert!(text.contains("0.041000"));
        assert!(text.contains("\tsettle\t"));
        let _ = std::fs::remove_file(&path);
    }
}
