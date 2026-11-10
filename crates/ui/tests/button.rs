#![cfg_attr(test, expect(clippy::float_cmp, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]

//! Button press, hover, disabled text, and the focus border.

use genos_ui::{
    apply_looks, frame, looks, theme_dark, Event, Kind, Node, Place, Pointer, Sizing, Space, State,
};

fn button(enabled: bool) -> Node {
    let mut node = Node::new(4);
    node.kind = Kind::Button { enabled };
    node.width = Sizing::fixed(80.0);
    node.height = Sizing::fixed(24.0);
    node.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], [10.0, 10.0]));
    node.text = Some("OK".into());
    apply_looks(&mut node, looks(&theme_dark()));
    node
}

#[test]
fn a_press_fires_once_and_a_disabled_button_stays_quiet() {
    let theme = theme_dark();
    let mut state = State::default();
    state.theme = theme;
    let node = button(true);
    let down = Pointer::at(12.0, 12.0, true);
    let first = frame(&node, &mut state, Space::Screen, [200.0, 80.0], None, down, 0.0, None);
    assert_eq!(first.events, vec![Event::Press { id: 4 }]);
    let held = frame(&node, &mut state, Space::Screen, [200.0, 80.0], None, down, 0.0, None);
    assert_eq!(held.events, []);

    let mut state = State::default();
    state.theme = theme;
    let disabled = button(false);
    let quiet = frame(
        &disabled,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        down,
        0.0,
        None,
    );
    assert_eq!(quiet.events, []);
    assert!(quiet.paints.iter().any(|paint| paint.color == theme.muted));
}

#[test]
fn hover_uses_the_accent_and_focus_uses_the_focus_border() {
    let theme = theme_dark();
    let node = button(true);
    let mut state = State::default();
    state.theme = theme;
    let hover = frame(
        &node,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::at(12.0, 12.0, false),
        0.0,
        None,
    );
    assert!(hover.paints.iter().any(|paint| paint.color == theme.accent));

    let mut state = State::default();
    state.theme = theme;
    let _press = frame(
        &node,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::at(12.0, 12.0, true),
        0.0,
        None,
    );
    let focused = frame(
        &node,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::at(12.0, 12.0, false),
        0.0,
        None,
    );
    assert_eq!(state.focus, Some(4));
    assert!(focused.paints.iter().any(|paint| paint.color == theme.focus));
}
