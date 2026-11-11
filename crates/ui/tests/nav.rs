//! Arrow and gamepad edges move UI focus. A held key does not repeat.

use genos_input::{InputCode, InputSystem};
use genos_ui::{frame, Event, Kind, Node, Pointer, Sizing, Space, State};

fn button(id: u32) -> Node {
    let mut node = Node::new(id);
    node.kind = Kind::Button { enabled: true };
    node.width = Sizing::fixed(80.0);
    node.height = Sizing::fixed(20.0);
    node
}

#[test]
fn key_down_moves_once_and_enter_presses_the_focused_button() {
    let mut root = Node::new(9);
    root.direction = genos_ui::Direction::TopToBottom;
    root.children = vec![button(1), button(2)];
    let mut state = State::default();
    state.focus = Some(1);
    let mut input = InputSystem::new();
    input.begin_frame();
    input.set_key(InputCode::key_down, true);
    let moved = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::default(),
        0.0,
        Some(&input),
    );
    assert_eq!(state.focus, Some(2));
    assert!(moved.keys_captured);
    input.begin_frame();
    let held = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::default(),
        0.0,
        Some(&input),
    );
    assert_eq!(state.focus, Some(2));
    assert!(!held.keys_captured);
    input.begin_frame();
    input.set_key(InputCode::key_down, false);
    input.set_key(InputCode::key_enter, true);
    let pressed = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::default(),
        0.0,
        Some(&input),
    );
    assert_eq!(pressed.events, vec![Event::Press { id: 2 }]);
    assert!(pressed.keys_captured);
}

#[test]
fn shift_tab_moves_to_the_previous_control() {
    let mut root = Node::new(9);
    root.direction = genos_ui::Direction::TopToBottom;
    root.children = vec![button(1), button(2)];
    let mut state = State::default();
    state.focus = Some(2);
    let mut input = InputSystem::new();
    input.begin_frame();
    input.set_key(InputCode::key_shift_left, true);
    input.set_key(InputCode::key_tab, true);
    let moved = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::default(),
        0.0,
        Some(&input),
    );
    assert_eq!(state.focus, Some(1));
    assert!(moved.keys_captured);
}
