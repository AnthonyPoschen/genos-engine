//! A toggle reports the next on state and does not change the caller's kind.

use genos_ui::{frame, Event, Kind, Node, Pointer, Sizing, Space, State};

fn toggle(on: bool) -> Node {
    let mut node = Node::new(8);
    node.kind = Kind::Toggle { on };
    node.width = Sizing::fixed(48.0);
    node.height = Sizing::fixed(24.0);
    node
}

fn thumb_x(frame: &genos_ui::Frame) -> f32 {
    frame
        .paints
        .iter()
        .find(|paint| paint.w > 4.0 && paint.w < 30.0 && paint.h > 8.0)
        .map_or(-1.0, |paint| paint.x)
}

#[test]
fn the_thumb_follows_on_and_the_kind_stays_put() {
    let off = toggle(false);
    let mut state = State::default();
    let pressed = frame(
        &off,
        &mut state,
        Space::Screen,
        [120.0, 40.0],
        None,
        Pointer::at(10.0, 10.0, true),
        0.0,
        None,
    );
    assert_eq!(pressed.events, vec![Event::Toggle { id: 8, on: true }]);
    assert!(matches!(off.kind, Kind::Toggle { on: false }));
    let midpoint = 24.0;
    assert!(thumb_x(&pressed) < midpoint, "off thumb {}", thumb_x(&pressed));

    let on = toggle(true);
    let mut state = State::default();
    let shown = frame(
        &on,
        &mut state,
        Space::Screen,
        [120.0, 40.0],
        None,
        Pointer::at(0.0, 0.0, false),
        0.0,
        None,
    );
    assert!(thumb_x(&shown) >= midpoint, "on thumb {}", thumb_x(&shown));
}
