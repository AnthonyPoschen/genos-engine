#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a row is missing"))]

//! A box with an action fires once. A miss releases UI focus.

use genos_ui::{frame, Action, Node, Place, Pointer, Sizing, Space, State};

#[test]
fn a_press_on_a_box_with_an_action_fires_once() {
    let mut node = Node::new(7);
    node.width = Sizing::fixed(40.0);
    node.height = Sizing::fixed(20.0);
    node.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], [10.0, 10.0]));
    node.action = Some(Action::Press(7));
    let mut state = State::default();
    let down = Pointer::at(12.0, 12.0, true);
    let first = frame(&node, &mut state, Space::Screen, [200.0, 100.0], None, down, 0.0, None);
    assert_eq!(first.actions, vec![Action::Press(7)]);
    assert!(!first.look_capture);
    let held = frame(&node, &mut state, Space::Screen, [200.0, 100.0], None, down, 0.0, None);
    assert_eq!(held.actions, []);
    assert!(held.damage.is_empty(), "{:?}", held.damage);
}

#[test]
fn a_press_that_misses_sets_look_capture() {
    let node = Node::new(1);
    let mut state = State::default();
    let miss = Pointer::at(180.0, 80.0, true);
    let frame = frame(&node, &mut state, Space::Screen, [200.0, 100.0], None, miss, 0.0, None);
    assert!(frame.look_capture);
    assert!(state.focus.is_none());
}

#[test]
fn a_drag_moves_the_node_and_dragging_is_only_a_read() {
    let mut node = Node::new(4);
    node.width = Sizing::fixed(40.0);
    node.height = Sizing::fixed(20.0);
    node.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], [10.0, 10.0]));
    node.draggable = true;
    let mut state = State::default();
    let press = frame(
        &node,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        Pointer::at(12.0, 12.0, true),
        0.0,
        None,
    );
    let pressed = press.shown.iter().find(|row| row.id == 4).expect("node");
    assert!((pressed.rect.x - 10.0).abs() < 0.01);
    assert!(state.dragging());
    let moved = frame(
        &node,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        Pointer::at(30.0, 22.0, true),
        0.0,
        None,
    );
    let dragged = moved.shown.iter().find(|row| row.id == 4).expect("node");
    assert!((dragged.rect.x - 28.0).abs() < 0.01, "x {}", dragged.rect.x);
    assert!((dragged.rect.y - 20.0).abs() < 0.01, "y {}", dragged.rect.y);
    assert!(state.dragging());
    let _up = frame(
        &node,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        Pointer::at(30.0, 22.0, false),
        0.0,
        None,
    );
    assert!(!state.dragging());
    let stay = frame(
        &node,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        Pointer::at(0.0, 0.0, false),
        0.0,
        None,
    );
    let parked = stay.shown.iter().find(|row| row.id == 4).expect("node");
    assert!((parked.rect.x - 28.0).abs() < 0.01, "x {}", parked.rect.x);
    assert!((parked.rect.y - 20.0).abs() < 0.01, "y {}", parked.rect.y);
}
