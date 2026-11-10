//! A popup is hit before the root. A press outside dismisses it.

use genos_ui::{frame, Event, Kind, Node, Place, Pointer, Sizing, Space, State};

fn button(id: u32) -> Node {
    let mut node = Node::new(id);
    node.kind = Kind::Button { enabled: true };
    node.width = Sizing::fixed(40.0);
    node.height = Sizing::fixed(20.0);
    node.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], [10.0, 10.0]));
    node
}

#[test]
fn a_popup_covers_the_root_and_a_miss_dismisses_it() {
    let root = button(1);
    let mut state = State::default();
    state.open_layer(button(2));
    let down = Pointer::at(12.0, 12.0, true);
    let covered = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        down,
        0.0,
        None,
    );
    assert_eq!(covered.events, vec![Event::Press { id: 2 }]);
    let _up = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        Pointer::at(12.0, 12.0, false),
        0.0,
        None,
    );
    let outside = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        Pointer::at(180.0, 80.0, true),
        0.0,
        None,
    );
    assert_eq!(outside.events, vec![Event::Dismiss { id: 2 }]);
    let _release = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        Pointer::at(180.0, 80.0, false),
        0.0,
        None,
    );
    let root_press = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        down,
        0.0,
        None,
    );
    assert_eq!(root_press.events, vec![Event::Press { id: 1 }]);
}

#[test]
fn a_second_open_replaces_the_popup_and_keeps_focus() {
    let root = button(1);
    let mut state = State::default();
    state.focus = Some(1);
    state.open_layer(button(2));
    state.focus = Some(4);
    let mut next = button(2);
    next.text = Some("NEXT".into());
    state.open_layer(next);
    assert_eq!(state.focus, Some(4));
    let shown = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        Pointer::default(),
        0.0,
        None,
    );
    assert!(shown.shown.iter().any(|row| row.text.as_deref() == Some("NEXT")));
    assert_eq!(state.focus, Some(4));
}

#[test]
fn a_popup_takes_activate_so_the_root_button_does_not_fire() {
    let root = button(1);
    let mut state = State::default();
    state.focus = Some(1);
    state.open_layer(button(2));
    state.focus = Some(1);
    let mut input = genos_input::InputSystem::new();
    input.begin_frame();
    input.set_key(genos_input::InputCode::key_enter, true);
    let pressed = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        Pointer::default(),
        0.0,
        Some(&input),
    );
    assert!(pressed.events.is_empty(), "{:?}", pressed.events);
    assert!(pressed.keys_captured);
}
