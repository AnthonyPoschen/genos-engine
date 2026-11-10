#![cfg_attr(test, expect(clippy::assert_is_empty, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::float_cmp, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]

//! Linear fill and a radial arc. A press does not emit.

use genos_ui::{frame, theme_dark, Kind, Node, Pointer, Sizing, Space, State};

fn progress(value: f32, radial: bool) -> Node {
    let mut node = Node::new(6);
    node.kind = Kind::Progress { value, radial };
    if radial {
        node.width = Sizing::fixed(48.0);
        node.height = Sizing::fixed(48.0);
    } else {
        node.width = Sizing::fixed(100.0);
        node.height = Sizing::fixed(12.0);
    }
    node
}

#[test]
fn linear_fill_matches_the_value_and_a_press_is_ignored() {
    let theme = theme_dark();
    let mut state = State::default();
    state.theme = theme;
    let frame = frame(
        &progress(0.5, false),
        &mut state,
        Space::Screen,
        [120.0, 40.0],
        None,
        Pointer::at(10.0, 6.0, true),
        0.0,
        None,
    );
    assert_eq!(frame.events, []);
    let fill = frame
        .paints
        .iter()
        .find(|paint| paint.color == theme.accent)
        .expect("fill");
    assert!((fill.w - 50.0).abs() < 0.5, "fill width {}", fill.w);
}

#[test]
fn a_radial_value_draws_an_arc() {
    let mut state = State::default();
    let frame = frame(
        &progress(0.5, true),
        &mut state,
        Space::Screen,
        [64.0, 64.0],
        None,
        Pointer::at(1.0, 1.0, false),
        0.0,
        None,
    );
    assert_eq!(frame.events, []);
    assert!(!frame.segments.is_empty());
}
