#![cfg_attr(test, expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::assert_is_empty, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::field_reassign_with_default, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::float_cmp, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::wildcard_enum_match_arm, reason = "TODO(lint-2026-10-14): pre-existing wildcard match. docs/LINT_RULES.md"))]

//! A game panel slider: a click sets the value under the pointer, a held press
//! follows it, and stepped sliders snap.

use genos_scene::Camera;
use genos_ui::{button_panel, Action, Frame, PanelRow, PanelSlider, Pointer, State};

const VIEW: [f32; 2] = [1280.0, 720.0];
const ID: u32 = 77;

fn rows(value: f64, step: f64) -> Vec<PanelRow> {
    vec![PanelRow {
        label: "Time".into(),
        buttons: Vec::new(),
        slider: Some(PanelSlider {
            id: ID,
            value,
            min: 0.0,
            max: 24.0,
            step,
            text: format!("{value:.1} H"),
            steps: Vec::new(),
        }),
        dropdown: None,
        checks: Vec::new(),
    }]
}

fn frame(state: &mut State, value: f64, step: f64, x: f32, y: f32, down: bool) -> Frame {
    button_panel(
        state,
        "Panel",
        &rows(value, step),
        VIEW,
        &Camera::opening(),
        Pointer::at(x, y, down),
    )
}

fn slides(frame: &Frame) -> Vec<f64> {
    frame
        .actions
        .iter()
        .filter_map(|a| match a {
            Action::Slide { id: ID, value } => Some(*value),
            _ => None,
        })
        .collect()
}

#[test]
fn a_click_sets_the_value_and_a_drag_follows() {
    let mut state = State::default();
    let idle = frame(&mut state, 12.0, 0.0, 0.0, 0.0, false);
    let track = idle
        .shown
        .iter()
        .find(|s| s.id == ID)
        .expect("slider track")
        .rect;
    let y = track.y + track.h * 0.5;
    // Pointer at the left end of the usable track: the minimum.
    let left = frame(&mut state, 12.0, 0.0, track.x + 1.0, y, true);
    assert_eq!(slides(&left), vec![0.0]);
    assert!(!left.look_capture, "a slider press captured look");
    // Dragged past the right end, still held: the maximum.
    let right = frame(
        &mut state,
        0.0,
        0.0,
        track.x + track.w + 40.0,
        y + 30.0,
        true,
    );
    assert_eq!(slides(&right), vec![24.0]);
    // Released: nothing more.
    let up = frame(&mut state, 24.0, 0.0, track.x + 10.0, y, false);
    assert!(slides(&up).is_empty());
    // The thumb sits at the value: one paint in the thumb colour near the middle.
    let mid = frame(&mut state, 12.0, 0.0, 0.0, 0.0, false);
    let centre = track.x + track.w * 0.5;
    assert!(mid
        .paints
        .iter()
        .any(|p| p.w == 10.0 && (p.x + p.w * 0.5 - centre).abs() < 1.0));
}

#[test]
fn a_stepped_slider_snaps_and_an_unchanged_value_fires_nothing() {
    let mut state = State::default();
    let idle = frame(&mut state, 0.0, 6.0, 0.0, 0.0, false);
    let track = idle.shown.iter().find(|s| s.id == ID).unwrap().rect;
    let y = track.y + track.h * 0.5;
    let at = track.x + track.w * 0.55;
    let press = frame(&mut state, 0.0, 6.0, at, y, true);
    assert_eq!(slides(&press), vec![12.0]);
    let held = frame(&mut state, 12.0, 6.0, at + 2.0, y, true);
    assert!(slides(&held).is_empty(), "same snapped value fired again");
}

fn grades(value: f64) -> PanelSlider {
    PanelSlider::named(
        ID,
        value,
        ["Off", "Low", "Medium", "High", "Ultra"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| genos_ui::SliderStep {
                name: name.into(),
                value: index as f64,
            })
            .collect(),
    )
}

fn named_rows(bar: PanelSlider) -> Vec<PanelRow> {
    vec![PanelRow {
        label: "Quality".into(),
        buttons: Vec::new(),
        slider: Some(bar),
        dropdown: None,
        checks: Vec::new(),
    }]
}

fn named_frame(state: &mut State, bar: PanelSlider, pointer: Pointer) -> Frame {
    button_panel(
        state,
        "Panel",
        &named_rows(bar),
        VIEW,
        &Camera::opening(),
        pointer,
    )
}

fn track_of(state: &mut State, bar: PanelSlider) -> genos_ui::Rect {
    let idle = named_frame(state, bar, Pointer::default());
    idle.shown
        .iter()
        .find(|shown| shown.id == ID)
        .expect("slider track")
        .rect
}

#[test]
fn a_click_snaps_to_the_named_step_under_the_pointer() {
    let mut state = State::default();
    let track = track_of(&mut state, grades(2.0));
    let y = track.y + 12.0;
    let left = named_frame(&mut state, grades(2.0), Pointer::at(track.x + 1.0, y, true));
    assert_eq!(slides(&left), vec![0.0]);
    let mid = named_frame(
        &mut state,
        grades(0.0),
        Pointer::at(track.x + track.w * 0.5, y, true),
    );
    assert_eq!(slides(&mid), vec![2.0]);
    let right = named_frame(
        &mut state,
        grades(2.0),
        Pointer::at(track.x + track.w - 1.0, y, true),
    );
    assert_eq!(slides(&right), vec![4.0]);
}

#[test]
fn uneven_named_steps_never_report_a_value_between_them() {
    let bar = PanelSlider::named(
        ID,
        1.0,
        [(1.0, "1"), (3.0, "3"), (5.0, "5")]
            .into_iter()
            .map(|(value, name)| genos_ui::SliderStep {
                name: name.into(),
                value,
            })
            .collect(),
    );
    let mut state = State::default();
    let track = track_of(&mut state, bar.clone());
    let y = track.y + 12.0;
    for step in 0..=8 {
        let x = track.x + track.w * (step as f32 / 8.0);
        let mut state = State::default();
        let _ = named_frame(&mut state, bar.clone(), Pointer::default());
        let press = named_frame(&mut state, bar.clone(), Pointer::at(x, y, true));
        for value in slides(&press) {
            assert!(
                value == 1.0 || value == 3.0 || value == 5.0,
                "click at {x} mapped to {value}"
            );
        }
    }
    assert_eq!(bar.step_by(1), 3.0);
    let on_three = PanelSlider { value: 3.0, ..bar };
    assert_eq!(on_three.step_by(1), 5.0);
}

#[test]
fn the_wheel_and_arrow_keys_step_and_the_tip_names_the_value() {
    let mut state = State::default();
    let track = track_of(&mut state, grades(2.0));
    let y = track.y + 8.0;
    let x = track.x + track.w * 0.5;
    let hover = named_frame(&mut state, grades(2.0), Pointer::at(x, y, false));
    assert_eq!(hover.tooltip.as_deref(), Some("Medium  2"));
    assert!(slides(&hover).is_empty());
    let mut up = Pointer::at(x, y, false);
    up.wheel_y = 1.0;
    let wheeled = named_frame(&mut state, grades(2.0), up);
    assert_eq!(slides(&wheeled), vec![3.0]);
    assert_eq!(wheeled.tooltip.as_deref(), Some("High  3"));
    let mut down = Pointer::at(x, y, false);
    down.wheel_y = -1.0;
    let back = named_frame(&mut state, grades(3.0), down);
    assert_eq!(slides(&back), vec![2.0]);

    let mut state = State::default();
    let _ = named_frame(&mut state, grades(1.0), Pointer::at(x, y, true));
    let mut key = Pointer::default();
    key.key_inc = true;
    let stepped = named_frame(&mut state, grades(1.0), key);
    assert_eq!(
        slides(&stepped),
        vec![2.0],
        "a focused slider takes the arrow key"
    );
    let mut held = Pointer::default();
    held.key_inc = true;
    // The caller passes the press edge. A second frame with the key still set
    // does step again; the game passes `pressed`, so a hold is one frame.
    let again = named_frame(&mut state, grades(2.0), held);
    assert_eq!(slides(&again), vec![3.0]);

    let hot = hover
        .paints
        .iter()
        .any(|paint| paint.color == genos_ui::LABEL_HOT);
    let dim = hover
        .paints
        .iter()
        .any(|paint| paint.color == genos_ui::LABEL_DIM);
    assert!(hot, "the current step name is not highlighted");
    assert!(dim, "the other step names are not dim");
}
