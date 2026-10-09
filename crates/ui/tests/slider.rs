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
        }),
    }]
}

fn frame(state: &mut State, value: f64, step: f64, x: f32, y: f32, down: bool) -> Frame {
    button_panel(
        state,
        "Panel",
        &rows(value, step),
        VIEW,
        &Camera::opening(),
        Pointer { x, y, down },
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
