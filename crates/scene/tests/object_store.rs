//! The object store copied from a scripted room.
//!
//! These checks cover `publish` and `move_placed`. They are not the later
//! scene-migration stages (handles on the draw types, undo, or lighting records).

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

fn first_solid(world: &genos_ecs::World) -> genos_ecs::SceneHandle {
    world
        .handles_with::<genos_scene::Placed>()
        .into_iter()
        .find(|handle| {
            world
                .get::<genos_scene::Placed>(*handle)
                .is_some_and(|row| row.kind == 3)
        })
        .expect("solid")
}

#[test]
fn publish_repeats_the_same_handles() {
    let first = publish(&load_str(ROOM).expect("script")).expect("publish");
    let second = publish(&load_str(ROOM).expect("script")).expect("publish");
    let left = first.handles_with::<genos_scene::Placed>();
    let right = second.handles_with::<genos_scene::Placed>();
    assert_eq!(left, right);
    assert_eq!(left.len(), 7);
}

#[test]
fn moving_one_placed_solid_emits_one_delta() {
    let scene = load_str(ROOM).expect("script");
    let mut world = publish(&scene).expect("publish");
    let solid = first_solid(&world);
    let cursor = world.clock();
    move_placed(&mut world, solid, 3.0, 0.0, 1.0).expect("move");
    let deltas = world.changes_since(cursor);
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].component, "placed");
    assert_eq!(deltas[0].handle, solid);
    assert_eq!(world.changes_since(world.clock()).len(), 0);
}

#[test]
fn an_unchanged_move_emits_nothing_and_the_draw_list_stays() {
    let scene = load_str(ROOM).expect("script");
    let solids = scene.solids.clone();
    let mut world = publish(&scene).expect("publish");
    let solid = first_solid(&world);
    let row = world
        .get::<genos_scene::Placed>(solid)
        .expect("row")
        .clone();
    let cursor = world.clock();
    move_placed(&mut world, solid, row.x, row.y, row.z).expect("same");
    assert_eq!(world.changes_since(cursor).len(), 0);
    move_placed(&mut world, solid, 3.0, 0.0, 1.0).expect("move");
    assert_eq!(
        world
            .get::<genos_scene::Placed>(solid)
            .expect("row")
            .x
            .to_bits(),
        3.0_f32.to_bits()
    );
    assert_eq!(scene.solids, solids);
}

#[test]
fn a_menu_edit_emits_none() {
    let mut scene = load_str(ROOM).expect("script");
    let world = publish(&scene).expect("publish");
    let cursor = world.clock();
    scene.menu.visible = !scene.menu.visible;
    assert_eq!(world.changes_since(cursor).len(), 0);
}

#[test]
fn a_stale_move_is_counted_and_writes_nothing() {
    let scene = load_str(ROOM).expect("script");
    let mut world = publish(&scene).expect("publish");
    let handle = world
        .handles_with::<genos_scene::Placed>()
        .first()
        .copied()
        .expect("floor");
    let before = world.stats().stale_rejects;
    world.despawn(handle).expect("despawn");
    let err = move_placed(&mut world, handle, 0.0, 0.0, 0.0);
    assert_eq!(err, Err(StoreError::Stale));
    assert_eq!(world.stats().stale_rejects, before + 1);
    assert!(world.get::<genos_scene::Placed>(handle).is_none());
}
