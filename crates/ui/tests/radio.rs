//! A radio press selects that radio's place in its group.

use genos_ui::{frame, Event, Kind, Node, Pointer, Sizing, Space, State};

fn radio(id: u32, group: u32, selected: bool) -> Node {
    let mut node = Node::new(id);
    node.kind = Kind::Radio { group, selected };
    node.width = Sizing::fixed(80.0);
    node.height = Sizing::fixed(16.0);
    node.text = Some(id.to_string());
    node
}

#[test]
fn pressing_the_second_radio_selects_index_one() {
    let mut root = Node::new(9);
    root.width = Sizing::fixed(80.0);
    root.direction = genos_ui::Direction::TopToBottom;
    root.children = vec![
        radio(1, 1, true),
        radio(2, 1, false),
        radio(3, 1, false),
        radio(4, 9, false),
    ];
    let mut state = State::default();
    let pressed = frame(
        &root,
        &mut state,
        Space::Screen,
        [100.0, 80.0],
        None,
        Pointer::at(8.0, 20.0, true),
        0.0,
        None,
    );
    assert_eq!(pressed.events, vec![Event::Select { id: 2, index: 1 }]);
    assert!(matches!(
        root.children[3].kind,
        Kind::Radio {
            group: 9,
            selected: false
        }
    ));

    let mut state = State::default();
    let repeat = frame(
        &root,
        &mut state,
        Space::Screen,
        [100.0, 80.0],
        None,
        Pointer::at(8.0, 4.0, true),
        0.0,
        None,
    );
    assert_eq!(repeat.events, vec![Event::Select { id: 1, index: 0 }]);
    assert!(matches!(
        root.children[0].kind,
        Kind::Radio {
            group: 1,
            selected: true
        }
    ));
}
