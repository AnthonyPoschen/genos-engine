#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! CPU sampling, blends, the state machine, root motion, cues, and the far rate.

use genos_load::{Channel, ChannelPath, Clip, Interpolation};
use genos_math::{Quat, Vec3};
use genos_physics::Body;
use genos_scene::{
    pose_time, tick, AnimCharacter, AnimFrame, AnimGraph, AnimQuality, AnimationSettings,
    PoseSource, Predicate, SIM_HZ, SKIN_BUDGET_BYTES,
};

fn clip(values: &[f32], path: ChannelPath, interpolation: Interpolation, joint: u16) -> Clip {
    let stride = match path {
        ChannelPath::Rotation => 4,
        ChannelPath::Translation | ChannelPath::Scale => 3,
    };
    let keys = values.len() / stride;
    let denom = u16::try_from(keys.saturating_sub(1).max(1)).unwrap_or(1);
    let mut times = Vec::new();
    for index in 0..keys {
        let step = u16::try_from(index).unwrap_or(0);
        times.push(f32::from(step) / f32::from(denom));
    }
    if keys == 1 {
        times = vec![0.0];
    }
    Clip {
        name: "clip".into(),
        duration: 1.0,
        skin: 0,
        animation: 0,
        channels: vec![Channel {
            joint,
            path,
            interpolation,
            times,
            values: values.to_vec(),
        }],
    }
}

struct Rest {
    t: Vec<[f32; 3]>,
    r: Vec<[f32; 4]>,
    s: Vec<[f32; 3]>,
}

fn rest(joints: usize) -> Rest {
    Rest {
        t: vec![[0.0, 0.0, 0.0]; joints],
        r: vec![[0.0, 0.0, 0.0, 1.0]; joints],
        s: vec![[1.0, 1.0, 1.0]; joints],
    }
}

fn source<'a>(clips: &'a [Clip], rest: &'a Rest) -> PoseSource<'a> {
    PoseSource {
        clips,
        rest_t: &rest.t,
        rest_r: &rest.r,
        rest_s: &rest.s,
    }
}

fn frame(tick_index: u32, distance_m: f32) -> AnimFrame {
    AnimFrame {
        tick: tick_index,
        distance_m,
    }
}

#[test]
fn low_settings_round_trip_and_the_byte_cap() {
    let settings = AnimationSettings::preset(AnimQuality::Low);
    let text = settings.to_text();
    assert_eq!(AnimationSettings::parse(&text).unwrap(), settings);
    assert_eq!(settings.max_skinned, 32);
    assert_eq!(settings.update_rate_far_hz, 15);
    assert!(!settings.dqs);
    assert!(!settings.ik);
    assert!(settings.far_distance_m > 24.0 && settings.far_distance_m < 26.0);
    let bytes = AnimationSettings::skin_bytes(32, 60, 52_356);
    assert_eq!(bytes, 67_107_840);
    assert!(bytes < SKIN_BUDGET_BYTES);
    assert_eq!(SKIN_BUDGET_BYTES - bytes, 1_024);
    assert!(AnimationSettings::within_budget(32, 60, 52_356));
    assert!(!AnimationSettings::within_budget(32, 60, 52_357));
    assert!(AnimationSettings::preset(AnimQuality::Medium).ik);
    assert_eq!(pose_time(0), 0.0);
    assert_eq!(pose_time(SIM_HZ), 1.0);
    assert_eq!(pose_time(SIM_HZ / 2), 0.5);
}

#[test]
fn a_predicate_compiles_once_and_reads_named_params() {
    assert!(Predicate::compile("speed > 0.5 && heading <= 1").is_ok());
    assert!(Predicate::compile("bogus > 1").is_err());
    assert!(drives([0.6, 1.0, 0.0, 0.0]));
    assert!(!drives([0.5, 0.0, 0.0, 0.0]));
    assert!(!drives([0.6, 2.0, 0.0, 0.0]));
}

fn drives(params: [f32; 4]) -> bool {
    let mut character = AnimCharacter::bind(1, AnimationSettings::preset(AnimQuality::Low));
    character.params = params;
    let rest = rest(1);
    let clips = vec![
        clip(
            &[0.0, 0.0, 0.0],
            ChannelPath::Translation,
            Interpolation::Linear,
            0,
        ),
        clip(
            &[1.0, 0.0, 0.0],
            ChannelPath::Translation,
            Interpolation::Linear,
            0,
        ),
    ];
    let graph = AnimGraph::parse(
        r#"{"states":[{"name":"idle","clip":0},{"name":"run","clip":1}],
            "transitions":[{"from":"idle","to":"run","duration":0,"when":"speed > 0.5 && heading <= 1"}]}"#,
    )
    .unwrap();
    let posed = source(&clips, &rest);
    tick(&graph, &posed, &mut character, frame(0, 0.0), None, &mut []);
    character.state == 1
}

#[test]
fn linear_midpoint_step_hold_and_blends() {
    let rest = rest(1);
    let clips = vec![
        clip(
            &[0.0, 0.0, 0.0, 2.0, 0.0, 0.0],
            ChannelPath::Translation,
            Interpolation::Linear,
            0,
        ),
        clip(
            &[0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0],
            ChannelPath::Rotation,
            Interpolation::Step,
            0,
        ),
        clip(
            &[0.0, 0.0, 0.0],
            ChannelPath::Translation,
            Interpolation::Linear,
            0,
        ),
        clip(
            &[4.0, 0.0, 0.0],
            ChannelPath::Translation,
            Interpolation::Linear,
            0,
        ),
        clip(
            &[0.0, 0.0, 0.0],
            ChannelPath::Translation,
            Interpolation::Linear,
            0,
        ),
        clip(
            &[2.0, 0.0, 0.0],
            ChannelPath::Translation,
            Interpolation::Linear,
            0,
        ),
        clip(
            &[4.0, 0.0, 0.0],
            ChannelPath::Translation,
            Interpolation::Linear,
            0,
        ),
        clip(
            &[6.0, 0.0, 0.0],
            ChannelPath::Translation,
            Interpolation::Linear,
            0,
        ),
    ];
    let posed = source(&clips, &rest);
    let mut linear = AnimCharacter::bind(1, AnimationSettings::preset(AnimQuality::Low));
    let graph = AnimGraph::parse(r#"{"states":[{"name":"move","clip":0}]}"#).unwrap();
    tick(
        &graph,
        &posed,
        &mut linear,
        frame(SIM_HZ / 2, 0.0),
        None,
        &mut [],
    );
    assert_eq!(linear.pose[0][0], 1.0);

    let mut hold = AnimCharacter::bind(1, AnimationSettings::preset(AnimQuality::Low));
    let graph = AnimGraph::parse(r#"{"states":[{"name":"hold","clip":1}]}"#).unwrap();
    tick(
        &graph,
        &posed,
        &mut hold,
        frame(SIM_HZ - 1, 0.0),
        None,
        &mut [],
    );
    assert_eq!(hold.pose[0][6], 1.0);
    tick(&graph, &posed, &mut hold, frame(SIM_HZ, 0.0), None, &mut []);
    assert!(hold.pose[0][4] > 0.9);

    let mut blend = AnimCharacter::bind(1, AnimationSettings::preset(AnimQuality::Low));
    blend.params[0] = 1.0;
    let graph = AnimGraph::parse(
        r#"{"states":[{"name":"locomotion","blend1d":{"param":"speed","keys":[[0,2],[2,3]]}}]}"#,
    )
    .unwrap();
    tick(&graph, &posed, &mut blend, frame(0, 0.0), None, &mut []);
    assert!((blend.pose[0][0] - 2.0).abs() < 1.0e-4);

    let mut grid = AnimCharacter::bind(1, AnimationSettings::preset(AnimQuality::Low));
    grid.params[2] = 0.5;
    grid.params[3] = 0.5;
    let graph = AnimGraph::parse(
        r#"{"states":[{"name":"grid","blend2d":{"x":"vx","y":"vz","min":[0,0],"max":[1,1],"clips":[4,5,6,7]}}]}"#,
    )
    .unwrap();
    tick(&graph, &posed, &mut grid, frame(0, 0.0), None, &mut []);
    assert!((grid.pose[0][0] - 3.0).abs() < 1.0e-4);
}

#[test]
fn a_transition_starts_on_the_source_pose_and_then_blends() {
    let rest = rest(1);
    let clips = vec![
        clip(
            &[0.0, 0.0, 0.0],
            ChannelPath::Translation,
            Interpolation::Linear,
            0,
        ),
        clip(
            &[4.0, 0.0, 0.0],
            ChannelPath::Translation,
            Interpolation::Linear,
            0,
        ),
    ];
    let posed = source(&clips, &rest);
    let graph = AnimGraph::parse(
        r#"{"states":[{"name":"idle","clip":0},{"name":"run","clip":1}],
            "transitions":[{"from":"idle","to":"run","duration":1,"when":"speed > 0.5"}]}"#,
    )
    .unwrap();
    let mut character = AnimCharacter::bind(1, AnimationSettings::preset(AnimQuality::Low));
    tick(&graph, &posed, &mut character, frame(0, 0.0), None, &mut []);
    assert_eq!(character.pose[0][0], 0.0);
    character.params[0] = 1.0;
    tick(&graph, &posed, &mut character, frame(1, 0.0), None, &mut []);
    assert_eq!(character.pose[0][0], 0.0);
    tick(
        &graph,
        &posed,
        &mut character,
        frame(1 + SIM_HZ / 2, 0.0),
        None,
        &mut [],
    );
    assert!((character.pose[0][0] - 2.0).abs() < 1.0e-3);
}

#[test]
fn root_motion_moves_the_body_and_leaves_the_pose_at_rest() {
    let rest = rest(1);
    let clips = vec![clip(
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        ChannelPath::Translation,
        Interpolation::Linear,
        0,
    )];
    let posed = source(&clips, &rest);
    let graph = AnimGraph::parse(r#"{"states":[{"name":"move","clip":0}]}"#).unwrap();
    let mut character = AnimCharacter::bind(1, AnimationSettings::preset(AnimQuality::Low));
    let mut body = Body::sphere(Vec3::ZERO, 1.0, 0.5);
    tick(
        &graph,
        &posed,
        &mut character,
        frame(0, 0.0),
        Some(&mut body),
        &mut [],
    );
    tick(
        &graph,
        &posed,
        &mut character,
        frame(SIM_HZ, 0.0),
        Some(&mut body),
        &mut [],
    );
    assert!((body.position.x - 1.0).abs() < 1.0e-4);
    assert_eq!(character.pose[0][0], 0.0);
    assert_eq!(body.orientation.w, Quat::IDENTITY.w);
}

#[test]
fn a_cue_fires_once_when_the_sample_crosses_it() {
    let rest = rest(1);
    let clips = vec![clip(
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        ChannelPath::Translation,
        Interpolation::Linear,
        0,
    )];
    let posed = source(&clips, &rest);
    let graph = AnimGraph::parse(
        r#"{"states":[{"name":"move","clip":0}],"cues":[{"clip":0,"time":0.5,"name":"step"}]}"#,
    )
    .unwrap();
    let mut character = AnimCharacter::bind(1, AnimationSettings::preset(AnimQuality::Low));
    let mut events = [None; 2];
    let first = tick(
        &graph,
        &posed,
        &mut character,
        frame(0, 0.0),
        None,
        &mut events,
    );
    assert_eq!(first.events, 0);
    events = [None; 2];
    let crossed = tick(
        &graph,
        &posed,
        &mut character,
        frame(SIM_HZ / 2, 0.0),
        None,
        &mut events,
    );
    assert_eq!(crossed.events, 1);
    assert_eq!(events[0].unwrap().index, 0);
    assert_eq!(graph.cues[0].name, "step");
    events = [None; 2];
    let again = tick(
        &graph,
        &posed,
        &mut character,
        frame(SIM_HZ / 2 + 1, 0.0),
        None,
        &mut events,
    );
    assert_eq!(again.events, 0);
    let dropped = tick(
        &graph,
        &posed,
        &mut AnimCharacter::bind(1, AnimationSettings::preset(AnimQuality::Low)),
        frame(SIM_HZ / 2, 0.0),
        None,
        &mut [],
    );
    assert_eq!(dropped.dropped_events, 1);
}

#[test]
fn past_25m_the_drawn_pose_holds_and_the_root_still_moves() {
    let rest = rest(2);
    let clips = vec![Clip {
        name: "move".into(),
        duration: 1.0,
        skin: 0,
        animation: 0,
        channels: vec![
            Channel {
                joint: 0,
                path: ChannelPath::Translation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 1.0],
                values: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            },
            Channel {
                joint: 1,
                path: ChannelPath::Translation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 1.0],
                values: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            },
        ],
    }];
    let posed = source(&clips, &rest);
    let graph = AnimGraph::parse(r#"{"states":[{"name":"move","clip":0}]}"#).unwrap();
    let mut character = AnimCharacter::bind(2, AnimationSettings::preset(AnimQuality::Low));
    tick(
        &graph,
        &posed,
        &mut character,
        frame(0, 30.0),
        None,
        &mut [],
    );
    let held = character.pose[1][0];
    tick(
        &graph,
        &posed,
        &mut character,
        frame(1, 30.0),
        None,
        &mut [],
    );
    assert_eq!(character.pose[1][0], held);
    assert!(character.pose[0][0] > 0.0);
    tick(
        &graph,
        &posed,
        &mut character,
        frame(4, 30.0),
        None,
        &mut [],
    );
    assert!(character.pose[1][0] > held);
}

#[test]
fn a_tick_does_not_grow_the_pose_buffers() {
    let rest = rest(2);
    let clips = vec![clip(
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        ChannelPath::Translation,
        Interpolation::Linear,
        0,
    )];
    let posed = source(&clips, &rest);
    let graph = AnimGraph::parse(r#"{"states":[{"name":"move","clip":0}]}"#).unwrap();
    let mut character = AnimCharacter::bind(2, AnimationSettings::preset(AnimQuality::Low));
    let pose_cap = character.pose.capacity();
    let before = character.pose.len();
    for tick_index in 0..32 {
        tick(
            &graph,
            &posed,
            &mut character,
            frame(tick_index, 0.0),
            None,
            &mut [],
        );
    }
    assert_eq!(character.pose.capacity(), pose_cap);
    assert_eq!(character.pose.len(), before);
    let mut again = AnimCharacter::bind(2, AnimationSettings::preset(AnimQuality::Low));
    for tick_index in 0..32 {
        tick(
            &graph,
            &posed,
            &mut again,
            frame(tick_index, 0.0),
            None,
            &mut [],
        );
    }
    assert_eq!(character.pose, again.pose);
    character.pelvis_offset = 3.0;
    tick(
        &graph,
        &posed,
        &mut character,
        frame(33, 0.0),
        None,
        &mut [],
    );
    assert_eq!(character.pelvis_offset, 0.0);
}
