#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! A tooltip opens after a hover delay and is not a press target.

use genos_ui::{frame, Node, Place, Pointer, Sizing, Space, State};

fn target() -> Node {
    let mut node = Node::new(1);
    node.width = Sizing::fixed(40.0);
    node.height = Sizing::fixed(20.0);
    node.tooltip = Some("TIP".into());
    node.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], [0.0, 0.0]));
    node
}

fn shows_tip(frame: &genos_ui::Frame) -> bool {
    frame.shown.iter().any(|row| row.text.as_deref() == Some("TIP"))
}

#[test]
fn a_tip_waits_then_opens_under_the_pointer_and_closes_on_leave() {
    let node = target();
    let mut state = State::default();
    let hover = Pointer::at(4.0, 4.0, false);
    let early = frame(
        &node,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        hover,
        0.1,
        None,
    );
    assert!(!shows_tip(&early));
    let open = frame(
        &node,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        hover,
        0.3,
        None,
    );
    assert!(shows_tip(&open));
    let left = frame(
        &node,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        Pointer::at(180.0, 4.0, false),
        0.0,
        None,
    );
    assert!(!shows_tip(&left));
}

#[test]
fn a_tip_near_the_bottom_flips_above_the_pointer() {
    let mut node = target();
    node.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], [0.0, 80.0]));
    let mut state = State::default();
    let hover = Pointer::at(4.0, 96.0, false);
    let frame = frame(
        &node,
        &mut state,
        Space::Screen,
        [100.0, 100.0],
        None,
        hover,
        0.4,
        None,
    );
    let tip = frame
        .shown
        .iter()
        .find(|row| row.text.as_deref() == Some("TIP"))
        .expect("tip");
    assert!(tip.rect.y < hover.y);
    assert!(tip.rect.y + tip.rect.h <= 100.0 + 1.0e-3);
}

#[test]
fn a_dialog_blocks_the_tip() {
    let node = target();
    let mut state = State::default();
    state.open_dialog(Node::new(9));
    let frame = frame(
        &node,
        &mut state,
        Space::Screen,
        [200.0, 100.0],
        None,
        Pointer::at(4.0, 4.0, false),
        1.0,
        None,
    );
    assert!(!shows_tip(&frame));
}
