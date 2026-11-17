#![cfg_attr(test, expect(clippy::panic, reason = "a test aborts when a fixture fails"))]

//! Horizontal and vertical slider elements. Lamp notches stay on the panel.

use genos_ui::{frame, Event, Kind, Node, Pointer, Sizing, Space, State};

fn slider(vertical: bool, step: f64) -> Node {
    let mut node = Node::new(5);
    node.kind = Kind::Slider {
        value: 0.0,
        min: 0.0,
        max: 10.0,
        step,
        vertical,
    };
    node.width = Sizing::fixed(if vertical { 24.0 } else { 100.0 });
    node.height = Sizing::fixed(if vertical { 100.0 } else { 24.0 });
    node
}

fn change(frame: &genos_ui::Frame) -> f64 {
    match frame.events.first() {
        Some(Event::Change { value, .. }) => *value,
        other => panic!("expected change, got {other:?}"),
    }
}

#[test]
fn a_horizontal_press_snaps_and_a_zero_step_stays_continuous() {
    let mut state = State::default();
    let mid = frame(
        &slider(false, 1.0),
        &mut state,
        Space::Screen,
        [120.0, 40.0],
        None,
        Pointer::at(50.0, 12.0, true),
        0.0,
        None,
    );
    assert!((change(&mid) - 5.0).abs() < 1.0e-6);

    let mut state = State::default();
    let left = frame(
        &slider(false, 1.0),
        &mut state,
        Space::Screen,
        [120.0, 40.0],
        None,
        Pointer::at(0.0, 12.0, true),
        0.0,
        None,
    );
    assert!((change(&left) - 0.0).abs() < 1.0e-6);

    let mut state = State::default();
    let smooth = frame(
        &slider(false, 0.0),
        &mut state,
        Space::Screen,
        [120.0, 40.0],
        None,
        Pointer::at(50.0, 12.0, true),
        0.0,
        None,
    );
    assert!((change(&smooth) - 5.0).abs() < 1.0e-3);
}

#[test]
fn a_vertical_press_at_the_top_is_the_maximum() {
    let mut state = State::default();
    let top = frame(
        &slider(true, 1.0),
        &mut state,
        Space::Screen,
        [40.0, 120.0],
        None,
        Pointer::at(12.0, 0.0, true),
        0.0,
        None,
    );
    assert!((change(&top) - 10.0).abs() < 1.0e-6);
}
