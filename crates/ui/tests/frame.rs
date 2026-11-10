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
