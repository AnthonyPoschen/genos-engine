//! A text field inserts ASCII at the caret, grows a selection, and clamps home.

use genos_input::{InputCode, InputSystem};
use genos_ui::{frame, theme_dark, Caret, Event, Kind, Node, Pointer, Sizing, Space, State};

fn field(text: &str) -> Node {
    let mut node = Node::new(4);
    node.kind = Kind::TextField {
        text: text.into(),
        multiline: false,
    };
    node.width = Sizing::fixed(200.0);
    node.height = Sizing::fixed(24.0);
    node
}

fn near(color: [f32; 3], other: [f32; 3]) -> bool {
    (color[0] - other[0]).abs() < 0.01
        && (color[1] - other[1]).abs() < 0.01
        && (color[2] - other[2]).abs() < 0.01
}

fn tap(code: InputCode, shift: bool) -> InputSystem {
    let mut input = InputSystem::new();
    input.begin_frame();
    if shift {
        input.set_key(InputCode::key_shift_left, true);
    }
    input.set_key(code, true);
    input
}

fn run(node: &Node, state: &mut State, input: &InputSystem) -> genos_ui::Frame {
    frame(
        node,
        state,
        Space::Screen,
        [240.0, 40.0],
        None,
        Pointer::default(),
        0.0,
        Some(input),
    )
}

#[test]
fn key_a_inserts_backspace_deletes_a_selection_and_home_clamps() {
    let mut node = field("");
    let mut state = State::default();
    state.focus = Some(4);
    let typed = run(&node, &mut state, &tap(InputCode::key_a, false));
    assert_eq!(
        typed.events,
        vec![Event::Text {
            id: 4,
            text: "a".into(),
        }]
    );
    assert!(typed.keys_captured);
    assert_eq!(state.caret(4), Caret { anchor: 1, head: 1 });

    node.kind = Kind::TextField {
        text: "ab".into(),
        multiline: false,
    };
    let end = run(&node, &mut state, &tap(InputCode::key_end, false));
    assert_eq!(end.events, []);
    assert_eq!(state.caret(4), Caret { anchor: 2, head: 2 });
    let _ = run(&node, &mut state, &tap(InputCode::key_home, false));
    assert_eq!(state.caret(4), Caret { anchor: 0, head: 0 });
    let left = run(&node, &mut state, &tap(InputCode::key_left, false));
    assert!(left.keys_captured);
    assert_eq!(state.caret(4).head, 0);

    node.kind = Kind::TextField {
        text: "abcd".into(),
        multiline: false,
    };
    let _ = run(&node, &mut state, &tap(InputCode::key_right, true));
    let selected = run(&node, &mut state, &tap(InputCode::key_right, true));
    assert_eq!(state.caret(4), Caret { anchor: 0, head: 2 });
    let accent = theme_dark().accent;
    assert!(
        selected.paints.iter().any(|paint| {
            near(paint.color, accent)
                && (paint.w - 24.0).abs() < 0.01
                && (paint.h - 16.0).abs() < 0.01
        }),
        "selection paint missing"
    );
    assert!(
        selected
            .paints
            .iter()
            .any(|paint| (paint.w - 2.0).abs() < 0.01 && (paint.h - 16.0).abs() < 0.01),
        "caret paint missing"
    );
    let deleted = run(&node, &mut state, &tap(InputCode::key_backspace, false));
    assert_eq!(
        deleted.events,
        vec![Event::Text {
            id: 4,
            text: "cd".into(),
        }]
    );
    assert_eq!(state.caret(4), Caret { anchor: 0, head: 0 });
}

#[test]
fn a_held_letter_types_once_and_stays_captured() {
    let mut node = field("");
    let mut state = State::default();
    state.focus = Some(4);
    let mut input = InputSystem::new();
    input.begin_frame();
    input.set_key(InputCode::key_w, true);
    let typed = run(&node, &mut state, &input);
    assert_eq!(
        typed.events,
        vec![Event::Text {
            id: 4,
            text: "w".into(),
        }]
    );
    assert!(typed.keys_captured);
    assert_eq!(state.caret(4), Caret { anchor: 1, head: 1 });

    node.kind = Kind::TextField {
        text: "w".into(),
        multiline: false,
    };
    input.begin_frame();
    let held = run(&node, &mut state, &input);
    assert_eq!(held.events, []);
    assert!(held.keys_captured);
    assert_eq!(state.caret(4), Caret { anchor: 1, head: 1 });
}

#[test]
fn space_is_captured_and_up_does_not_move_the_caret() {
    let node = field("ab");
    let mut state = State::default();
    state.focus = Some(4);
    let spaced = run(&node, &mut state, &tap(InputCode::key_space, false));
    assert_eq!(
        spaced.events,
        vec![Event::Text {
            id: 4,
            text: " ab".into(),
        }]
    );
    assert!(spaced.keys_captured);
    assert_eq!(state.caret(4), Caret { anchor: 1, head: 1 });

    let up = run(&node, &mut state, &tap(InputCode::key_up, false));
    assert_eq!(up.events, []);
    assert!(up.keys_captured);
    assert_eq!(state.caret(4), Caret { anchor: 1, head: 1 });

    let mut chord = InputSystem::new();
    chord.begin_frame();
    chord.set_key(InputCode::key_control_right, true);
    chord.set_key(InputCode::key_a, true);
    let modified = run(&node, &mut state, &chord);
    assert_eq!(modified.events, []);
    assert!(modified.keys_captured);
    assert_eq!(state.caret(4), Caret { anchor: 1, head: 1 });
}

#[test]
fn an_unshifted_arrow_collapses_a_selection_to_that_edge() {
    let node = field("abcd");
    let mut state = State::default();
    state.focus = Some(4);
    let _ = run(&node, &mut state, &tap(InputCode::key_right, true));
    let _ = run(&node, &mut state, &tap(InputCode::key_right, true));
    assert_eq!(state.caret(4), Caret { anchor: 0, head: 2 });
    let right = run(&node, &mut state, &tap(InputCode::key_right, false));
    assert!(right.keys_captured);
    assert_eq!(state.caret(4), Caret { anchor: 2, head: 2 });

    let _ = run(&node, &mut state, &tap(InputCode::key_home, false));
    let _ = run(&node, &mut state, &tap(InputCode::key_right, true));
    let _ = run(&node, &mut state, &tap(InputCode::key_right, true));
    let left = run(&node, &mut state, &tap(InputCode::key_left, false));
    assert!(left.keys_captured);
    assert_eq!(state.caret(4), Caret { anchor: 0, head: 0 });
}

#[test]
fn a_long_line_stays_on_one_row_and_clips_to_the_content_box() {
    let mut node = field("A B");
    node.width = Sizing::fixed(30.0);
    node.height = Sizing::fixed(16.0);
    let mut state = State::default();
    state.focus = Some(4);
    let open = run(&node, &mut state, &tap(InputCode::key_end, false));
    let text = theme_dark().text;
    let glyphs: Vec<_> = open
        .paints
        .iter()
        .filter(|paint| {
            near(paint.color, text) && (paint.w - 2.0).abs() < 0.01 && (paint.h - 2.0).abs() < 0.01
        })
        .collect();
    assert!(!glyphs.is_empty(), "glyph pixels missing");
    assert!(
        glyphs
            .iter()
            .any(|paint| (paint.x - 24.0).abs() < 0.01 && paint.y < 16.0),
        "the third cell wrapped or drifted"
    );
    assert!(glyphs
        .iter()
        .all(|paint| paint.y + paint.h <= 16.01 && paint.x + paint.w <= 30.01));
    assert!(
        open.paints
            .iter()
            .all(|paint| paint.x + paint.w <= 30.01 && paint.y + paint.h <= 16.01),
        "a caret or selection escaped the content box"
    );
    assert!(
        !open.paints.iter().any(|paint| {
            near(paint.color, text) && (paint.h - 16.0).abs() < 0.01 && paint.x + paint.w > 30.01
        }),
        "caret past the box was still painted"
    );
}

#[test]
fn a_non_ascii_field_ignores_a_letter() {
    let node = field("é");
    let mut state = State::default();
    state.focus = Some(4);
    let typed = run(&node, &mut state, &tap(InputCode::key_a, false));
    assert_eq!(typed.events, []);
    assert!(typed.keys_captured);
}
