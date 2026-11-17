//! A label paints glyphs and ignores a press.

use genos_ui::{frame, Kind, Node, Pointer, Sizing, Space, State};

#[test]
fn a_label_paints_glyphs_and_ignores_a_press() {
    let mut node = Node::new(3);
    node.kind = Kind::Label;
    node.text = Some("AB".into());
    node.width = Sizing::fixed(80.0);
    node.height = Sizing::fit();
    let mut state = State::default();
    let down = Pointer::at(4.0, 4.0, true);
    let first = frame(&node, &mut state, Space::Screen, [200.0, 32.0], None, down, 0.0, None);
    assert_eq!(first.actions, []);
    assert_eq!(first.events, []);
    assert!(first.paints.len() > 1, "glyphs: {}", first.paints.len());
    let second = frame(&node, &mut state, Space::Screen, [200.0, 32.0], None, down, 0.0, None);
    assert!(second.damage.is_empty());
}
