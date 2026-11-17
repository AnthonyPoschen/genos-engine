#![cfg_attr(test, expect(clippy::float_cmp, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]

//! Children of a clip node paint only inside the parent's padding box.

use genos_ui::{frame, Node, Place, Pointer, Sizing, Space, State};

#[test]
fn a_child_paints_only_the_overlap_and_a_hidden_press_misses() {
    let mut inside = Node::new(2);
    inside.width = Sizing::fixed(30.0);
    inside.height = Sizing::fixed(10.0);
    inside.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], [40.0, 0.0]));
    inside.idle.background = [1.0, 0.0, 0.0];
    let mut outside = Node::new(3);
    outside.width = Sizing::fixed(20.0);
    outside.height = Sizing::fixed(10.0);
    outside.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], [80.0, 0.0]));
    outside.idle.background = [0.0, 1.0, 0.0];
    let mut root = Node::new(1);
    root.clip = true;
    root.width = Sizing::fixed(50.0);
    root.height = Sizing::fixed(20.0);
    root.children = vec![inside, outside];
    let mut state = State::default();
    let first = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::at(0.0, 0.0, false),
        0.0,
        None,
    );
    let red: Vec<_> = first
        .paints
        .iter()
        .filter(|paint| paint.color == [1.0, 0.0, 0.0])
        .collect();
    assert!(!red.is_empty());
    assert!(red.iter().all(|paint| paint.x + paint.w <= 50.0 + 1.0e-3));
    assert!(red.iter().all(|paint| paint.w < 30.0));
    assert!(first
        .paints
        .iter()
        .all(|paint| paint.color != [0.0, 1.0, 0.0]));
    let second = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::at(0.0, 0.0, false),
        0.0,
        None,
    );
    assert!(second.damage.is_empty());

    let hidden = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::at(60.0, 4.0, true),
        0.0,
        None,
    );
    assert_eq!(hidden.events, []);
    assert_eq!(hidden.actions, []);
    assert!(state.focus.is_none());
}

#[test]
fn a_dragged_child_stays_inside_a_stationary_clip() {
    let mut mark = Node::new(3);
    mark.width = Sizing::fixed(30.0);
    mark.height = Sizing::fixed(10.0);
    mark.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], [10.0, 0.0]));
    mark.idle.background = [1.0, 0.0, 0.0];
    let mut row = Node::new(2);
    row.draggable = true;
    row.width = Sizing::fixed(40.0);
    row.height = Sizing::fixed(10.0);
    row.children = vec![mark];
    let mut root = Node::new(1);
    root.clip = true;
    root.width = Sizing::fixed(40.0);
    root.height = Sizing::fixed(20.0);
    root.children = vec![row];
    let mut state = State::default();
    let _press = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::at(4.0, 4.0, true),
        0.0,
        None,
    );
    let moved = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::at(24.0, 4.0, true),
        0.0,
        None,
    );
    assert!(moved.paints.iter().all(|paint| paint.x + paint.w <= 40.0 + 1.0e-3));
    let _up = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::at(24.0, 4.0, false),
        0.0,
        None,
    );
    let parked = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::at(180.0, 70.0, false),
        0.0,
        None,
    );
    assert_clipped(&parked);
    let row_x = parked.shown.iter().find(|shown| shown.id == 2).map(|shown| shown.rect.x);
    assert!(row_x.is_some_and(|x| (x - 20.0).abs() < 0.01));
    assert!(!state.dragging());
    let outside = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::at(45.0, 4.0, true),
        0.0,
        None,
    );
    assert!(state.focus.is_none());
    assert!(outside.look_capture);
    let _release = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::default(),
        0.0,
        None,
    );
    let _inside = frame(
        &root,
        &mut state,
        Space::Screen,
        [200.0, 80.0],
        None,
        Pointer::at(32.0, 4.0, true),
        0.0,
        None,
    );
    assert_eq!(state.focus, Some(3));
}

fn assert_clipped(frame: &genos_ui::Frame) {
    let red: Vec<_> = frame
        .paints
        .iter()
        .filter(|paint| paint.color == [1.0, 0.0, 0.0])
        .collect();
    assert!(!red.is_empty());
    assert!(red.iter().all(|paint| paint.x + paint.w <= 40.0 + 1.0e-3));
}
