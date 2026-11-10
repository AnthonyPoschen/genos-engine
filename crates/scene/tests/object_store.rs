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
        .handles_with::<genos_scene::Solid>()
        .into_iter()
        .next()
        .expect("solid")
}

#[test]
fn publish_repeats_the_same_handles() {
    let first = publish(&load_str(ROOM).expect("script")).expect("publish");
    let second = publish(&load_str(ROOM).expect("script")).expect("publish");
    let mut left = first.handles_with::<genos_scene::Floor>();
    left.extend(first.handles_with::<genos_scene::Wall>());
    left.extend(first.handles_with::<genos_scene::Solid>());
    left.extend(first.handles_with::<genos_scene::Light>());
    left.sort_by_key(|handle| handle.index());
    let mut right = second.handles_with::<genos_scene::Floor>();
    right.extend(second.handles_with::<genos_scene::Wall>());
    right.extend(second.handles_with::<genos_scene::Solid>());
    right.extend(second.handles_with::<genos_scene::Light>());
    right.sort_by_key(|handle| handle.index());
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
    assert_eq!(deltas[0].component, "solid");
    assert_eq!(deltas[0].handle, solid);
    assert_eq!(world.changes_since(world.clock()).len(), 0);
}

#[test]
fn an_unchanged_move_emits_nothing_and_the_draw_list_stays() {
    let scene = load_str(ROOM).expect("script");
    let solids = scene.solids_cloned();
    let mut world = publish(&scene).expect("publish");
    let solid = first_solid(&world);
    let row = world.get::<genos_scene::Solid>(solid).expect("row").clone();
    let cursor = world.clock();
    move_placed(
        &mut world,
        solid,
        row.position.x,
        row.position.y,
        row.position.z,
    )
    .expect("same");
    assert_eq!(world.changes_since(cursor).len(), 0);
    move_placed(&mut world, solid, 3.0, 0.0, 1.0).expect("move");
    assert_eq!(
        world
            .get::<genos_scene::Solid>(solid)
            .expect("row")
            .position
            .x
            .to_bits(),
        3.0_f32.to_bits()
    );
    assert_eq!(scene.solids_cloned(), solids);
}

#[test]
fn a_menu_edit_emits_none() {
    let mut published =
        genos_scene::Published::new(load_str(ROOM).expect("script")).expect("publish");
    let cursor = published.world.clock();
    let visible = published.scene.menu.visible;
    published.set_menu_visible(!visible).expect("menu");
    assert_eq!(published.world.changes_since(cursor).len(), 0);
}

#[test]
fn a_stale_move_is_counted_and_writes_nothing() {
    let scene = load_str(ROOM).expect("script");
    let mut world = publish(&scene).expect("publish");
    let handle = world
        .handles_with::<genos_scene::Floor>()
        .first()
        .copied()
        .expect("floor");
    let before = world.stats().stale_rejects;
    world.despawn(handle).expect("despawn");
    let err = move_placed(&mut world, handle, 0.0, 0.0, 0.0);
    assert_eq!(err, Err(StoreError::Stale));
    assert_eq!(world.stats().stale_rejects, before + 1);
    assert!(world.get::<genos_scene::Floor>(handle).is_none());
}
