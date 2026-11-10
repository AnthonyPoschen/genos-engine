//! A dialog swallows an outside press and restores focus on back.

use genos_input::{InputCode, InputSystem};
use genos_ui::{frame, Event, Kind, Node, Place, Pointer, Sizing, Space, State};

fn button(id: u32, x: f32) -> Node {
    let mut node = Node::new(id);
    node.kind = Kind::Button { enabled: true };
    node.width = Sizing::fixed(40.0);
    node.height = Sizing::fixed(20.0);
    node.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], [x, 10.0]));
    node
}

fn dialog() -> Node {
    let mut node = Node::new(9);
    node.width = Sizing::fixed(120.0);
    node.height = Sizing::fixed(40.0);
    node.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], [80.0, 0.0]));
    node.children = vec![button(2, 0.0), button(3, 50.0)];
    node
}

#[test]
fn an_outside_press_misses_the_root_and_back_restores_focus() {
    let root = button(1, 10.0);
    let mut state = State::default();
    state.focus = Some(1);
    state.open_dialog(dialog());
    assert_eq!(state.focus, Some(2));
    let blocked = frame(
        &root,
        &mut state,
        Space::Screen,
        [240.0, 80.0],
        None,
        Pointer::at(12.0, 12.0, true),
        0.0,
        None,
    );
    assert_eq!(blocked.events, []);
    assert_eq!(state.focus, Some(2));

    let mut input = InputSystem::new();
    input.begin_frame();
    input.set_key(InputCode::key_escape, true);
    let back = frame(
        &root,
        &mut state,
        Space::Screen,
        [240.0, 80.0],
        None,
        Pointer::at(12.0, 12.0, true),
        0.0,
        Some(&input),
    );
    assert_eq!(back.events, vec![Event::Dismiss { id: 9 }]);
    assert_eq!(state.focus, Some(1));
    assert!(back.keys_captured);
}

#[test]
fn tab_wraps_inside_the_dialog() {
    let root = button(1, 10.0);
    let mut state = State::default();
    state.open_dialog(dialog());
    assert_eq!(state.focus, Some(2));
    let mut input = InputSystem::new();
    input.begin_frame();
    input.set_key(InputCode::key_tab, true);
    let _ = frame(
        &root,
        &mut state,
        Space::Screen,
        [240.0, 80.0],
        None,
        Pointer::default(),
        0.0,
        Some(&input),
    );
    assert_eq!(state.focus, Some(3));
    input.set_key(InputCode::key_tab, false);
    input.begin_frame();
    input.set_key(InputCode::key_tab, true);
    let _ = frame(
        &root,
        &mut state,
        Space::Screen,
        [240.0, 80.0],
        None,
        Pointer::default(),
        0.0,
        Some(&input),
    );
    assert_eq!(state.focus, Some(2));
}
