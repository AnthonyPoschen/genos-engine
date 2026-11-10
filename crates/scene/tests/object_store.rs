//! The object store copied from a scripted room.

#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

use genos_ecs::StoreError;
use genos_scene::{load_str, move_placed, publish};

const ROOM: &str = r#"
floor(0, 0, 4, 4);
wall(0, 2, 4, 0.2, 2);
wall(0, -2, 4, 0.2, 2);
solid("square", 1, 1, 0.5, 1, 0, 0);
solid("square", -1, 1, 0.5, 0, 1, 0);
solid("circle", 0, -1, 0.4, 0, 0, 1);
light(0, 2, 0, 1, 1, 1);
"#;

#[test]
fn reload_keeps_scene_handles() {
    let first = publish(&load_str(ROOM).expect("script")).expect("publish");
    let second = publish(&load_str(ROOM).expect("script")).expect("publish");
    let left = first.handles_with::<genos_scene::Placed>();
    let right = second.handles_with::<genos_scene::Placed>();
    assert_eq!(left, right);
    assert_eq!(left.len(), 7);
}

#[test]
fn a_moved_solid_emits_one_record() {
    let scene = load_str(ROOM).expect("script");
    let mut world = publish(&scene).expect("publish");
    let solid = world
        .handles_with::<genos_scene::Placed>()
        .into_iter()
        .find(|handle| {
            world
                .get::<genos_scene::Placed>(*handle)
                .is_some_and(|row| row.kind == 3)
        })
        .expect("solid");
    let cursor = world.clock().saturating_sub(1);
    move_placed(&mut world, solid, 3.0, 0.0, 1.0).expect("move");
    let deltas = world.changes_since(cursor);
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].component, "placed");
    assert_eq!(deltas[0].handle, solid);
}

#[test]
fn a_menu_edit_emits_none() {
    let mut scene = load_str(ROOM).expect("script");
    let before = publish(&scene).expect("publish");
    scene.menu.visible = !scene.menu.visible;
    let after = publish(&scene).expect("publish");
    assert_eq!(
        before.handles_with::<genos_scene::Placed>(),
        after.handles_with::<genos_scene::Placed>()
    );
}

#[test]
fn stale_handle_writes_nothing() {
    let scene = load_str(ROOM).expect("script");
    let mut world = publish(&scene).expect("publish");
    let handle = world
        .handles_with::<genos_scene::Placed>()
        .first()
        .copied()
        .expect("floor");
    world.despawn(handle).expect("despawn");
    let err = move_placed(&mut world, handle, 0.0, 0.0, 0.0);
    assert_eq!(err, Err(StoreError::Stale));
    assert!(world.get::<genos_scene::Placed>(handle).is_none());
}
