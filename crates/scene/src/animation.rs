//! CPU animation: settings, a compiled graph, and one character component.
//!
//! Pose time is `tick / 60`. The frame does not read a clock and does not
//! allocate. Conditions are a compiled predicate, not a Rhai call. Root motion
//! is taken off joint 0 and added to the physics body. A drawn pose past
//! `far_distance_m` keeps the last full sample except the root, which stays
//! on the sim tick. `pelvis_offset` is the visual foot plant; this stage leaves
//! it at zero. Foot IK writes it later and does not move the body.

use genos_json::Value;
use genos_load::{sample_clip, Clip};
use genos_math::{Quat, Vec3};
use genos_physics::Body;

use crate::EngineSettings;

/// Sim steps per second. Pose time is the tick divided by this.
pub const SIM_HZ: u32 = 60;

/// GPU bytes for one skinned vertex: position, normal, tangent.
pub const VERTEX_BYTES: u64 = 40;

/// GPU bytes for one palette joint, a `mat3x4`.
pub const JOINT_BYTES: u64 = 48;

/// Low budget for the skinned vertex buffer plus joint palettes.
pub const SKIN_BUDGET_BYTES: u64 = 64 * 1024 * 1024;

const PARAMS: [&str; 4] = ["speed", "heading", "vx", "vz"];

/// Quality row from ADR 0038 Addition C. The table is the only branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimQuality {
    Low,
    Medium,
    High,
}

/// Knobs any system can read. The debug binary and the command catalogue edit them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimationSettings {
    pub max_skinned: u32,
    pub update_rate_far_hz: u32,
    pub far_distance_m: f32,
    pub dqs: bool,
    pub ik: bool,
}

impl AnimationSettings {
    /// The row for `quality`. Low is the GTX 1060 ceiling.
    #[must_use]
    pub const fn preset(quality: AnimQuality) -> Self {
        match quality {
            AnimQuality::Low => Self {
                max_skinned: 32,
                update_rate_far_hz: 15,
                far_distance_m: 25.0,
                dqs: false,
                ik: false,
            },
            AnimQuality::Medium => Self {
                max_skinned: 96,
                update_rate_far_hz: 60,
                far_distance_m: 25.0,
                dqs: false,
                ik: true,
            },
            AnimQuality::High => Self {
                max_skinned: 256,
                update_rate_far_hz: 60,
                far_distance_m: 25.0,
                dqs: false,
                ik: true,
            },
        }
    }

    /// Vertex buffer plus palettes. Clip bytes are not included.
    #[must_use]
    pub fn skin_bytes(characters: u32, joints: u32, verts: u32) -> u64 {
        u64::from(characters) * u64::from(verts) * VERTEX_BYTES
            + u64::from(characters) * u64::from(joints) * JOINT_BYTES
    }

    /// True when `skin_bytes` fits in [`SKIN_BUDGET_BYTES`].
    #[must_use]
    pub fn within_budget(characters: u32, joints: u32, verts: u32) -> bool {
        Self::skin_bytes(characters, joints, verts) <= SKIN_BUDGET_BYTES
    }

    #[must_use]
    pub fn to_text(&self) -> String {
        format!(
            "max_skinned={}\nupdate_rate_far_hz={}\nfar_distance_m={}\ndqs={}\nik={}",
            self.max_skinned,
            self.update_rate_far_hz,
            self.far_distance_m,
            on_off(self.dqs),
            on_off(self.ik),
        )
    }

    /// # Errors
    ///
    /// A missing key, an unknown key, or a value outside the table.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut settings = Self::preset(AnimQuality::Low);
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("use key=value, got {line}"));
            };
            settings.set_key(key, value)?;
        }
        Ok(settings)
    }

    /// # Errors
    ///
    /// An unknown key or a value the table does not accept.
    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key.trim() {
            "max_skinned" | "animation.max_skinned" => {
                self.max_skinned = parse_count(value, 1, 256, "max_skinned")?;
            }
            "update_rate_far_hz" | "animation.update_rate_far_hz" => {
                self.update_rate_far_hz = parse_count(value, 1, SIM_HZ, "update_rate_far_hz")?;
            }
            "far_distance_m" | "animation.far_distance_m" => {
                self.far_distance_m = parse_distance(value)?;
            }
            "dqs" | "animation.dqs" => self.dqs = parse_switch(value, "dqs")?,
            "ik" | "animation.ik" => self.ik = parse_switch(value, "ik")?,
            other => return Err(format!("unknown animation setting {other}")),
        }
        Ok(())
    }
}

impl EngineSettings for AnimationSettings {
    fn to_text(&self) -> String {
        AnimationSettings::to_text(self)
    }

    fn parse(text: &str) -> Result<Self, String> {
        AnimationSettings::parse(text)
    }

    fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        AnimationSettings::set_key(self, key, value)
    }
}

/// Seconds since tick 0. `tick / 60`, split so the conversion stays exact for
/// a tick that fits in `u16` seconds.
#[must_use]
pub fn pose_time(tick: u32) -> f32 {
    let seconds = tick / SIM_HZ;
    let frames = tick % SIM_HZ;
    let seconds = u16::try_from(seconds).map_or(f32::from(u16::MAX), f32::from);
    let frames = u16::try_from(frames).map_or(0.0, f32::from);
    seconds + frames / f32::from(u16::try_from(SIM_HZ).unwrap_or(60))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmp {
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Instr {
    param: u8,
    cmp: Cmp,
    value: f32,
}

/// Conjunction compiled once. An empty list is always true.
#[derive(Clone, Debug, PartialEq)]
pub struct Predicate {
    instrs: Vec<Instr>,
}

impl Predicate {
    /// # Errors
    ///
    /// A token that is not `name op number`, or a name outside speed, heading, vx, vz.
    pub fn compile(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(Self { instrs: Vec::new() });
        }
        let mut instrs = Vec::new();
        for part in text.split("&&") {
            instrs.push(compile_instr(part.trim())?);
        }
        Ok(Self { instrs })
    }

    fn holds(&self, params: &[f32; 4]) -> bool {
        self.instrs.iter().all(|instr| {
            let value = params.get(usize::from(instr.param)).copied().unwrap_or(0.0);
            match instr.cmp {
                Cmp::Gt => value > instr.value,
                Cmp::Ge => value >= instr.value,
                Cmp::Lt => value < instr.value,
                Cmp::Le => value <= instr.value,
            }
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Clip(u16),
    Blend1D {
        param: u8,
        keys: Vec<(f32, u16)>,
    },
    Blend2D {
        x: u8,
        y: u8,
        min: [f32; 2],
        max: [f32; 2],
        clips: [u16; 4],
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimState {
    pub name: String,
    node: Node,
}

#[derive(Clone, Debug, PartialEq)]
struct Transition {
    from: u16,
    to: u16,
    duration: f32,
    when: Predicate,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimCue {
    pub clip: u16,
    pub time: f32,
    pub name: String,
}

/// States, transitions, and cues. Built once from JSON. The tick only reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimGraph {
    pub states: Vec<AnimState>,
    transitions: Vec<Transition>,
    pub cues: Vec<AnimCue>,
}

impl AnimGraph {
    /// # Errors
    ///
    /// JSON that is not the graph object, or a predicate that does not compile.
    pub fn parse(text: &str) -> Result<Self, String> {
        graph_from(&genos_json::parse(text)?)
    }
}

#[derive(Clone, Copy, Debug)]
struct Blend {
    to: u16,
    start_tick: u32,
    from_entered: u32,
    duration: f32,
}

/// One character. `bind` allocates the pose buffers. `tick` does not.
#[derive(Clone, Debug)]
pub struct AnimCharacter {
    pub settings: AnimationSettings,
    pub state: u16,
    pub params: [f32; 4],
    /// Visual pelvis lift. Foot IK writes it after the graph. It does not move the body.
    pub pelvis_offset: f32,
    entered_tick: u32,
    blend: Option<Blend>,
    prev_time: Option<f32>,
    prev_to_time: Option<f32>,
    prev_root: Option<[f32; 10]>,
    pub pose: Vec<[f32; 10]>,
    hold: Vec<[f32; 10]>,
    scratch: Vec<[f32; 10]>,
}

impl AnimCharacter {
    #[must_use]
    pub fn bind(joints: usize, settings: AnimationSettings) -> Self {
        Self {
            settings,
            state: 0,
            params: [0.0; 4],
            pelvis_offset: 0.0,
            entered_tick: 0,
            blend: None,
            prev_time: None,
            prev_to_time: None,
            prev_root: None,
            pose: vec![[0.0; 10]; joints],
            hold: vec![[0.0; 10]; joints],
            scratch: vec![[0.0; 10]; joints.saturating_mul(5)],
        }
    }
}

/// Clips and the rest pose they sample against.
pub struct PoseSource<'a> {
    pub clips: &'a [Clip],
    pub rest_t: &'a [[f32; 3]],
    pub rest_r: &'a [[f32; 4]],
    pub rest_s: &'a [[f32; 3]],
}

/// The sim tick and the distance from the camera, in metres.
#[derive(Clone, Copy, Debug)]
pub struct AnimFrame {
    pub tick: u32,
    pub distance_m: f32,
}

/// A cue whose time the sample crossed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FiredCue {
    pub index: u16,
    pub time: f32,
}

/// What one tick changed. `sample_ms` is the caller's clock around [`tick`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickReport {
    pub bones: u32,
    pub events: u32,
    pub dropped_events: u32,
    pub sampled_full: bool,
}

struct Played {
    clips: [u16; 8],
    times: [f32; 8],
    lanes: [u8; 8],
    count: usize,
}

impl Played {
    fn empty() -> Self {
        Self {
            clips: [0; 8],
            times: [0.0; 8],
            lanes: [0; 8],
            count: 0,
        }
    }

    fn push(&mut self, clip: u16, time: f32, lane: u8) {
        if let Some(slot) = self.clips.get_mut(self.count) {
            *slot = clip;
            self.times[self.count] = time;
            self.lanes[self.count] = lane;
            self.count += 1;
        }
    }
}

/// Advance one sim tick. Writes `character.pose` and `events`. Does not allocate.
pub fn tick(
    graph: &AnimGraph,
    source: &PoseSource<'_>,
    character: &mut AnimCharacter,
    frame: AnimFrame,
    body: Option<&mut Body>,
    events: &mut [Option<FiredCue>],
) -> TickReport {
    if !character.settings.ik {
        character.pelvis_offset = 0.0;
    }
    let period = far_period(&character.settings);
    let far = frame.distance_m > character.settings.far_distance_m;
    let sampled_full = !far || frame.tick.is_multiple_of(period);
    consider_transition(graph, character, frame.tick);
    let played = sample_pose(graph, source, character, frame.tick, sampled_full);
    apply_root(source, character, body);
    let (events_fired, dropped_events) = fire_cues(graph, character, &played, events);
    remember_times(character, &played);
    TickReport {
        bones: u32::try_from(character.pose.len()).unwrap_or(u32::MAX),
        events: events_fired,
        dropped_events,
        sampled_full,
    }
}

fn far_period(settings: &AnimationSettings) -> u32 {
    let rate = settings.update_rate_far_hz.max(1);
    (SIM_HZ / rate).max(1)
}

fn consider_transition(graph: &AnimGraph, character: &mut AnimCharacter, tick_index: u32) {
    if character.blend.is_some() {
        return;
    }
    let Some(next) = graph.transitions.iter().find(|transition| {
        transition.from == character.state && transition.when.holds(&character.params)
    }) else {
        return;
    };
    character.blend = Some(Blend {
        to: next.to,
        start_tick: tick_index,
        from_entered: character.entered_tick,
        duration: next.duration,
    });
}

fn sample_pose(
    graph: &AnimGraph,
    source: &PoseSource<'_>,
    character: &mut AnimCharacter,
    tick_index: u32,
    sampled_full: bool,
) -> Played {
    let joints = character.pose.len();
    if !sampled_full {
        let time = state_time(
            tick_index,
            character.entered_tick,
            duration_of(graph, source, character.state),
        );
        sample_root(graph, source, character, time);
        return played_root(graph, character.state, time);
    }
    if let Some(blend) = character.blend {
        return sample_blend(graph, source, character, tick_index, blend, joints);
    }
    let time = state_time(
        tick_index,
        character.entered_tick,
        duration_of(graph, source, character.state),
    );
    let mut played = Played::empty();
    eval_state(
        graph,
        source,
        character.state,
        time,
        character,
        &mut Eval {
            dest: 0,
            lane: 0,
            played: &mut played,
        },
    );
    character.pose.copy_from_slice(&character.scratch[..joints]);
    played
}

fn sample_blend(
    graph: &AnimGraph,
    source: &PoseSource<'_>,
    character: &mut AnimCharacter,
    tick_index: u32,
    blend: Blend,
    joints: usize,
) -> Played {
    let from_time = state_time(
        tick_index,
        blend.from_entered,
        duration_of(graph, source, character.state),
    );
    let to_time = state_time(
        tick_index,
        blend.start_tick,
        duration_of(graph, source, blend.to),
    );
    let weight = if blend.duration <= 0.0 {
        1.0
    } else {
        (pose_time(tick_index.saturating_sub(blend.start_tick)) / blend.duration).min(1.0)
    };
    let mut from_played = Played::empty();
    let mut to_played = Played::empty();
    eval_state(
        graph,
        source,
        character.state,
        from_time,
        character,
        &mut Eval {
            dest: 0,
            lane: 0,
            played: &mut from_played,
        },
    );
    character.hold.copy_from_slice(&character.scratch[..joints]);
    eval_state(
        graph,
        source,
        blend.to,
        to_time,
        character,
        &mut Eval {
            dest: 0,
            lane: 1,
            played: &mut to_played,
        },
    );
    blend_poses(
        &character.hold,
        &character.scratch[..joints],
        weight,
        &mut character.pose,
    );
    if weight >= 1.0 {
        character.state = blend.to;
        character.entered_tick = blend.start_tick;
        character.blend = None;
    }
    let mut played = from_played;
    for index in 0..to_played.count {
        played.push(to_played.clips[index], to_played.times[index], 1);
    }
    played
}

fn duration_of(graph: &AnimGraph, source: &PoseSource<'_>, state: u16) -> f32 {
    let Some(state) = graph.states.get(usize::from(state)) else {
        return 0.0;
    };
    let mut duration = 0.0f32;
    match &state.node {
        Node::Clip(clip) => duration = clip_duration(source, *clip),
        Node::Blend1D { keys, .. } => {
            for (_, clip) in keys {
                duration = duration.max(clip_duration(source, *clip));
            }
        }
        Node::Blend2D { clips, .. } => {
            for clip in clips {
                duration = duration.max(clip_duration(source, *clip));
            }
        }
    }
    duration
}

fn clip_duration(source: &PoseSource<'_>, clip: u16) -> f32 {
    source
        .clips
        .get(usize::from(clip))
        .map_or(0.0, |clip| clip.duration)
}

struct Eval<'a> {
    dest: usize,
    lane: u8,
    played: &'a mut Played,
}

fn state_time(tick_index: u32, entered: u32, duration: f32) -> f32 {
    let mut time = pose_time(tick_index.saturating_sub(entered));
    if duration > 0.0 {
        let mut guard = 0u32;
        while time > duration && guard < 10_000 {
            time -= duration;
            guard += 1;
        }
    }
    time
}

fn eval_state(
    graph: &AnimGraph,
    source: &PoseSource<'_>,
    state: u16,
    time: f32,
    character: &mut AnimCharacter,
    out: &mut Eval<'_>,
) {
    let Some(plan) = plan_state(graph, state, &character.params) else {
        return;
    };
    match plan {
        Plan::Clip(clip) => {
            write_at(source, clip, time, character, out.dest);
            out.played.push(clip, time, out.lane);
        }
        Plan::Blend1D {
            left,
            right,
            weight,
        } => {
            blend_pair(source, left, right, weight, time, character, out);
        }
        Plan::Blend2D { tx, ty, clips } => {
            blend_2d(source, character, tx, ty, clips, time, out);
        }
    }
}

#[derive(Clone, Copy)]
enum Plan {
    Clip(u16),
    Blend1D { left: u16, right: u16, weight: f32 },
    Blend2D { tx: f32, ty: f32, clips: [u16; 4] },
}

fn plan_state(graph: &AnimGraph, state: u16, params: &[f32; 4]) -> Option<Plan> {
    let node = &graph.states.get(usize::from(state))?.node;
    Some(match node {
        Node::Clip(clip) => Plan::Clip(*clip),
        Node::Blend1D { param, keys } => {
            let (left, right, weight) = blend_span(
                keys,
                params.get(usize::from(*param)).copied().unwrap_or(0.0),
            )?;
            Plan::Blend1D {
                left,
                right,
                weight,
            }
        }
        Node::Blend2D {
            x,
            y,
            min,
            max,
            clips,
        } => Plan::Blend2D {
            tx: axis_weight(
                params.get(usize::from(*x)).copied().unwrap_or(0.0),
                min[0],
                max[0],
            ),
            ty: axis_weight(
                params.get(usize::from(*y)).copied().unwrap_or(0.0),
                min[1],
                max[1],
            ),
            clips: *clips,
        },
    })
}

fn write_at(
    source: &PoseSource<'_>,
    clip: u16,
    time: f32,
    character: &mut AnimCharacter,
    dest: usize,
) {
    let joints = character.pose.len();
    let start = dest.saturating_mul(joints);
    let end = start.saturating_add(joints).min(character.scratch.len());
    if start >= end {
        return;
    }
    write_clip(source, clip, time, &mut character.scratch[start..end]);
}

fn write_clip(source: &PoseSource<'_>, clip: u16, time: f32, dest: &mut [[f32; 10]]) {
    let Some(clip) = source.clips.get(usize::from(clip)) else {
        return;
    };
    let count = dest
        .len()
        .min(source.rest_t.len())
        .min(source.rest_r.len())
        .min(source.rest_s.len());
    sample_clip(
        clip,
        &source.rest_t[..count],
        &source.rest_r[..count],
        &source.rest_s[..count],
        time,
        &mut dest[..count],
    );
}

fn blend_pair(
    source: &PoseSource<'_>,
    left: u16,
    right: u16,
    weight: f32,
    time: f32,
    character: &mut AnimCharacter,
    out: &mut Eval<'_>,
) {
    let joints = character.pose.len();
    let dest = out.dest;
    write_at(source, left, time, character, dest + 1);
    write_at(source, right, time, character, dest + 2);
    blend_indexed(
        character,
        (dest + 1) * joints,
        (dest + 2) * joints,
        dest * joints,
        joints,
        weight,
    );
    out.played.push(left, time, out.lane);
    if right != left {
        out.played.push(right, time, out.lane);
    }
}

fn blend_span(keys: &[(f32, u16)], value: f32) -> Option<(u16, u16, f32)> {
    let first = keys.first()?;
    if keys.len() == 1 || value <= first.0 {
        return Some((first.1, first.1, 0.0));
    }
    let last = keys.last()?;
    if value >= last.0 {
        return Some((last.1, last.1, 0.0));
    }
    let mut index = 0usize;
    while index + 1 < keys.len() && keys[index + 1].0 < value {
        index += 1;
    }
    let left = keys.get(index)?;
    let right = keys.get(index + 1)?;
    let span = right.0 - left.0;
    let weight = if span <= 0.0 {
        0.0
    } else {
        (value - left.0) / span
    };
    Some((left.1, right.1, weight))
}

fn blend_2d(
    source: &PoseSource<'_>,
    character: &mut AnimCharacter,
    tx: f32,
    ty: f32,
    clips: [u16; 4],
    time: f32,
    out: &mut Eval<'_>,
) {
    let joints = character.pose.len();
    let dest = out.dest;
    write_at(source, clips[0], time, character, dest + 1);
    write_at(source, clips[1], time, character, dest + 2);
    blend_indexed(
        character,
        (dest + 1) * joints,
        (dest + 2) * joints,
        (dest + 3) * joints,
        joints,
        tx,
    );
    write_at(source, clips[2], time, character, dest + 1);
    write_at(source, clips[3], time, character, dest + 2);
    blend_indexed(
        character,
        (dest + 1) * joints,
        (dest + 2) * joints,
        (dest + 4) * joints,
        joints,
        tx,
    );
    blend_indexed(
        character,
        (dest + 3) * joints,
        (dest + 4) * joints,
        dest * joints,
        joints,
        ty,
    );
    for clip in clips {
        out.played.push(clip, time, out.lane);
    }
}

fn blend_indexed(
    character: &mut AnimCharacter,
    left: usize,
    right: usize,
    dest: usize,
    joints: usize,
    weight: f32,
) {
    for joint in 0..joints {
        let Some(a) = character.scratch.get(left + joint).copied() else {
            return;
        };
        let Some(b) = character.scratch.get(right + joint).copied() else {
            return;
        };
        if let Some(slot) = character.scratch.get_mut(dest + joint) {
            *slot = mix_trs(a, b, weight);
        }
    }
}

fn blend_poses(from: &[[f32; 10]], to: &[[f32; 10]], weight: f32, dest: &mut [[f32; 10]]) {
    for (slot, (left, right)) in dest.iter_mut().zip(from.iter().zip(to.iter())) {
        *slot = mix_trs(*left, *right, weight);
    }
}

fn mix_trs(left: [f32; 10], right: [f32; 10], weight: f32) -> [f32; 10] {
    let mut out = [0.0; 10];
    for index in 0..3 {
        out[index] = left[index] + (right[index] - left[index]) * weight;
        out[index + 7] = left[index + 7] + (right[index + 7] - left[index + 7]) * weight;
    }
    let a = Quat::new(left[3], left[4], left[5], left[6]);
    let b = Quat::new(right[3], right[4], right[5], right[6]);
    let blended = a.slerp(b, weight);
    out[3] = blended.x;
    out[4] = blended.y;
    out[5] = blended.z;
    out[6] = blended.w;
    out
}

fn axis_weight(value: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if span <= 0.0 {
        0.0
    } else {
        ((value - min) / span).clamp(0.0, 1.0)
    }
}

fn sample_root(
    graph: &AnimGraph,
    source: &PoseSource<'_>,
    character: &mut AnimCharacter,
    time: f32,
) {
    let joints = character.pose.len();
    character.hold.copy_from_slice(&character.pose);
    let mut played = Played::empty();
    eval_state(
        graph,
        source,
        character.state,
        time,
        character,
        &mut Eval {
            dest: 0,
            lane: 0,
            played: &mut played,
        },
    );
    if joints > 0 {
        let root = character.scratch[0];
        character.pose.copy_from_slice(&character.hold);
        character.pose[0] = root;
    }
}

fn played_root(graph: &AnimGraph, state: u16, time: f32) -> Played {
    let mut played = Played::empty();
    let Some(state) = graph.states.get(usize::from(state)) else {
        return played;
    };
    match &state.node {
        Node::Clip(clip) => played.push(*clip, time, 0),
        Node::Blend1D { keys, .. } => {
            for (_, clip) in keys {
                played.push(*clip, time, 0);
            }
        }
        Node::Blend2D { clips, .. } => {
            for clip in clips {
                played.push(*clip, time, 0);
            }
        }
    }
    played
}

fn apply_root(source: &PoseSource<'_>, character: &mut AnimCharacter, body: Option<&mut Body>) {
    let Some(sampled) = character.pose.first().copied() else {
        return;
    };
    if let (Some(prev), Some(body)) = (character.prev_root, body) {
        let delta = Vec3::new(
            sampled[0] - prev[0],
            sampled[1] - prev[1],
            sampled[2] - prev[2],
        );
        body.position += body.orientation.rotate(delta);
        let previous = Quat::new(prev[3], prev[4], prev[5], prev[6]);
        let next = Quat::new(sampled[3], sampled[4], sampled[5], sampled[6]);
        let delta_rot = hamilton(conjugate(previous), next);
        body.orientation = hamilton(body.orientation, delta_rot);
        if let Some(rest) = source.rest_t.first() {
            character.pose[0][0] = rest[0];
            character.pose[0][1] = rest[1];
            character.pose[0][2] = rest[2];
        }
        if let Some(rest) = source.rest_r.first() {
            character.pose[0][3] = rest[0];
            character.pose[0][4] = rest[1];
            character.pose[0][5] = rest[2];
            character.pose[0][6] = rest[3];
        }
    }
    character.prev_root = Some(sampled);
}

fn fire_cues(
    graph: &AnimGraph,
    character: &AnimCharacter,
    played: &Played,
    events: &mut [Option<FiredCue>],
) -> (u32, u32) {
    let mut fired = 0u32;
    let mut dropped = 0u32;
    let mut slot = 0usize;
    for index in 0..played.count {
        let time = played.times[index];
        let previous = if played.lanes[index] == 0 {
            character.prev_time
        } else {
            character.prev_to_time
        };
        for (cue_index, cue) in graph.cues.iter().enumerate() {
            if cue.clip != played.clips[index] || !crossed(previous, time, cue.time) {
                continue;
            }
            let event = FiredCue {
                index: u16::try_from(cue_index).unwrap_or(u16::MAX),
                time: cue.time,
            };
            if let Some(dest) = events.get_mut(slot) {
                *dest = Some(event);
                slot += 1;
                fired += 1;
            } else {
                dropped += 1;
            }
        }
    }
    (fired, dropped)
}

fn crossed(previous: Option<f32>, time: f32, cue: f32) -> bool {
    match previous {
        None => cue <= time,
        Some(previous) if time >= previous => cue > previous && cue <= time,
        Some(previous) => cue > previous || cue <= time,
    }
}

fn remember_times(character: &mut AnimCharacter, played: &Played) {
    let mut from_time = None;
    let mut to_time = None;
    for index in 0..played.count {
        if played.lanes[index] == 0 {
            from_time = Some(played.times[index]);
        } else {
            to_time = Some(played.times[index]);
        }
    }
    if character.blend.is_none() {
        character.prev_time = to_time.or(from_time);
        character.prev_to_time = None;
    } else {
        character.prev_time = from_time;
        character.prev_to_time = to_time;
    }
}

fn conjugate(quat: Quat) -> Quat {
    Quat::new(-quat.x, -quat.y, -quat.z, quat.w)
}

fn hamilton(left: Quat, right: Quat) -> Quat {
    Quat::new(
        left.w * right.x + left.x * right.w + left.y * right.z - left.z * right.y,
        left.w * right.y - left.x * right.z + left.y * right.w + left.z * right.x,
        left.w * right.z + left.x * right.y - left.y * right.x + left.z * right.w,
        left.w * right.w - left.x * right.x - left.y * right.y - left.z * right.z,
    )
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

fn parse_count(value: &str, min: u32, max: u32, name: &str) -> Result<u32, String> {
    let parsed: u32 = value
        .trim()
        .parse()
        .map_err(|_| format!("{name} needs a number, got {value}"))?;
    if !(min..=max).contains(&parsed) {
        return Err(format!("{name} is {min} to {max}, got {value}"));
    }
    Ok(parsed)
}

fn parse_distance(value: &str) -> Result<f32, String> {
    let distance: f32 = value
        .trim()
        .parse()
        .map_err(|_| format!("far_distance_m needs a number, got {value}"))?;
    if !distance.is_finite() || distance < 0.0 {
        return Err(format!("far_distance_m is zero or more, got {value}"));
    }
    Ok(distance)
}

fn parse_switch(value: &str, name: &str) -> Result<bool, String> {
    match value.trim() {
        "on" | "true" | "1" => Ok(true),
        "off" | "false" | "0" => Ok(false),
        _ => Err(format!("{name} is on or off, got {value}")),
    }
}

fn compile_instr(text: &str) -> Result<Instr, String> {
    split_cmp(text)
}

fn split_cmp(text: &str) -> Result<Instr, String> {
    let (op, cmp) = if text.contains(">=") {
        (">=", Cmp::Ge)
    } else if text.contains("<=") {
        ("<=", Cmp::Le)
    } else if text.contains('>') {
        (">", Cmp::Gt)
    } else if text.contains('<') {
        ("<", Cmp::Lt)
    } else {
        return Err(format!("predicate needs a comparison, got {text}"));
    };
    let Some((name, raw)) = text.split_once(op) else {
        return Err(format!("predicate needs a comparison, got {text}"));
    };
    let param = param_index(name.trim())?;
    let value: f32 = raw
        .trim()
        .parse()
        .map_err(|_| format!("predicate needs a number, got {text}"))?;
    if !value.is_finite() {
        return Err(format!("predicate needs a number, got {text}"));
    }
    Ok(Instr { param, cmp, value })
}

fn f32_of(value: f64) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    format!("{value}").parse().unwrap_or(0.0)
}

fn param_index(name: &str) -> Result<u8, String> {
    PARAMS
        .iter()
        .position(|param| *param == name)
        .and_then(|index| u8::try_from(index).ok())
        .ok_or_else(|| format!("unknown animation param {name}"))
}

fn graph_from(value: &Value) -> Result<AnimGraph, String> {
    let states = value
        .get("states")
        .and_then(Value::as_array)
        .ok_or("graph needs states")?;
    let mut parsed = Vec::new();
    for state in states {
        parsed.push(state_from(state)?);
    }
    let mut transitions = Vec::new();
    if let Some(items) = value.get("transitions").and_then(Value::as_array) {
        for item in items {
            transitions.push(transition_from(item, &parsed)?);
        }
    }
    let mut cues = Vec::new();
    if let Some(items) = value.get("cues").and_then(Value::as_array) {
        for item in items {
            cues.push(cue_from(item)?);
        }
    }
    Ok(AnimGraph {
        states: parsed,
        transitions,
        cues,
    })
}

fn state_from(value: &Value) -> Result<AnimState, String> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("state")
        .to_string();
    let node = if let Some(clip) = value.get("clip").and_then(Value::as_u64) {
        Node::Clip(u16::try_from(clip).map_err(|_| "clip index".to_string())?)
    } else if let Some(blend) = value.get("blend1d") {
        blend_1d_from(blend)?
    } else if let Some(blend) = value.get("blend2d") {
        blend_2d_from(blend)?
    } else {
        return Err("state needs clip, blend1d, or blend2d".into());
    };
    Ok(AnimState { name, node })
}

fn blend_1d_from(value: &Value) -> Result<Node, String> {
    let param = param_index(
        value
            .get("param")
            .and_then(Value::as_str)
            .unwrap_or("speed"),
    )?;
    let mut keys = Vec::new();
    let rows = value
        .get("keys")
        .and_then(Value::as_array)
        .ok_or("blend1d needs keys")?;
    for row in rows {
        let row = row.as_array().ok_or("blend1d key")?;
        let at = f32_of(row.first().and_then(Value::as_f64).unwrap_or(0.0));
        let clip = u16::try_from(row.get(1).and_then(Value::as_u64).unwrap_or(0))
            .map_err(|_| "clip index".to_string())?;
        keys.push((at, clip));
    }
    keys.sort_by(|left, right| left.0.total_cmp(&right.0));
    Ok(Node::Blend1D { param, keys })
}

fn blend_2d_from(value: &Value) -> Result<Node, String> {
    let x = param_index(value.get("x").and_then(Value::as_str).unwrap_or("vx"))?;
    let y = param_index(value.get("y").and_then(Value::as_str).unwrap_or("vz"))?;
    let min = pair(value.get("min"))?;
    let max = pair(value.get("max"))?;
    let clips_value = value
        .get("clips")
        .and_then(Value::as_array)
        .ok_or("blend2d needs clips")?;
    if clips_value.len() != 4 {
        return Err("blend2d needs four clips".into());
    }
    let mut clips = [0u16; 4];
    for (index, clip) in clips_value.iter().enumerate() {
        clips[index] =
            u16::try_from(clip.as_u64().unwrap_or(0)).map_err(|_| "clip index".to_string())?;
    }
    Ok(Node::Blend2D {
        x,
        y,
        min,
        max,
        clips,
    })
}

fn pair(value: Option<&Value>) -> Result<[f32; 2], String> {
    let row = value
        .and_then(Value::as_array)
        .ok_or("blend2d needs min and max")?;
    let x = f32_of(row.first().and_then(Value::as_f64).unwrap_or(0.0));
    let y = f32_of(row.get(1).and_then(Value::as_f64).unwrap_or(0.0));
    Ok([x, y])
}

fn transition_from(value: &Value, states: &[AnimState]) -> Result<Transition, String> {
    let from = state_index(
        states,
        value.get("from").and_then(Value::as_str).unwrap_or(""),
    )?;
    let to = state_index(
        states,
        value.get("to").and_then(Value::as_str).unwrap_or(""),
    )?;
    let duration = f32_of(value.get("duration").and_then(Value::as_f64).unwrap_or(0.0));
    let when = Predicate::compile(value.get("when").and_then(Value::as_str).unwrap_or(""))?;
    Ok(Transition {
        from,
        to,
        duration,
        when,
    })
}

fn state_index(states: &[AnimState], name: &str) -> Result<u16, String> {
    states
        .iter()
        .position(|state| state.name == name)
        .and_then(|index| u16::try_from(index).ok())
        .ok_or_else(|| format!("unknown animation state {name}"))
}

fn cue_from(value: &Value) -> Result<AnimCue, String> {
    let clip = u16::try_from(value.get("clip").and_then(Value::as_u64).unwrap_or(0))
        .map_err(|_| "clip index".to_string())?;
    let time = f32_of(value.get("time").and_then(Value::as_f64).unwrap_or(0.0));
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("cue")
        .to_string();
    Ok(AnimCue { clip, time, name })
}
