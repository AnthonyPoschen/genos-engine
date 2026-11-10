//! Store behavior: handles, queries, ticks, and one-component deltas.

#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

use genos_ecs::{push_u32, read_u32, Component, DecodeError, SceneHandle, World};
use genos_load::{sha256, AssetId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Health(u32);

impl Component for Health {
    fn component_name() -> &'static str {
        "health"
    }

    fn encode(&self, out: &mut Vec<u8>) {
        push_u32(out, self.0);
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut input = bytes;
        let value = read_u32(&mut input)?;
        if !input.is_empty() {
            return Err(DecodeError::Trailing);
        }
        Ok(Self(value))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Position(u32);

impl Component for Position {
    fn component_name() -> &'static str {
        "position"
    }

    fn encode(&self, out: &mut Vec<u8>) {
        push_u32(out, self.0);
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut input = bytes;
        let value = read_u32(&mut input)?;
        if !input.is_empty() {
            return Err(DecodeError::Trailing);
        }
        Ok(Self(value))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Outfit {
    mesh: AssetId,
}

impl Component for Outfit {
    fn component_name() -> &'static str {
        "outfit"
    }

    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.mesh);
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() != 32 {
            return Err(DecodeError::Truncated);
        }
        let mut mesh = [0; 32];
        mesh.copy_from_slice(bytes);
        Ok(Self { mesh })
    }
}

fn spawn_health(world: &mut World, value: u32) -> SceneHandle {
    world.spawn_with(Health(value)).expect("spawn")
}

#[test]
fn the_first_handle_reads_back_the_component() {
    let mut world = World::new();
    let handle = spawn_health(&mut world, 4);
    assert_eq!(handle.index(), 0);
    assert_eq!(handle.generation(), 1);
    assert_eq!(world.get::<Health>(handle).expect("health").0, 4);
    world.audit().expect("audit");
}

#[test]
fn despawn_makes_the_handle_stale_and_the_lowest_index_is_reused() {
    let mut world = World::new();
    let first = spawn_health(&mut world, 1);
    let second = spawn_health(&mut world, 2);
    world.despawn(first).expect("despawn");
    let err = world.insert(first, Health(9)).expect_err("stale");
    assert!(matches!(err, genos_ecs::StoreError::Stale));
    assert_eq!(world.stats().stale_rejects, 1);
    assert!(world.get::<Health>(first).is_none());
    world.despawn(second).expect("despawn second");
    let reused = spawn_health(&mut world, 3);
    assert_eq!(reused.index(), 0, "the lowest free index is reused");
    assert_eq!(reused.generation(), 2);
    assert!(world.get::<Health>(first).is_none());
    assert_eq!(world.get::<Health>(reused).expect("reused").0, 3);
    world.audit().expect("audit");
}

#[test]
fn a_free_middle_index_is_taken_before_a_new_one() {
    let mut world = World::new();
    let first = spawn_health(&mut world, 1);
    let second = spawn_health(&mut world, 2);
    world.despawn(second).expect("despawn");
    let reused = spawn_health(&mut world, 3);
    assert_eq!(reused.index(), second.index());
    assert_eq!(reused.generation(), 2);
    assert_eq!(world.get::<Health>(first).expect("first").0, 1);
    world.audit().expect("audit");
}

#[test]
fn a_pair_query_skips_objects_missing_either_component() {
    let mut world = World::new();
    let health_only = spawn_health(&mut world, 1);
    let both = spawn_health(&mut world, 2);
    world.insert(both, Position(8)).expect("position");
    let position_only = world.spawn_with(Position(9)).expect("position only");
    let mut pairs = Vec::new();
    world.for_each_pair::<Health, Position>(|handle, health, position| {
        pairs.push((handle, health.0, position.0));
    });
    assert_eq!(pairs, vec![(both, 2, 8)]);
    assert!(world.get::<Position>(health_only).is_none());
    assert!(world.get::<Health>(position_only).is_none());
    assert_eq!(world.handles_with::<Health>(), vec![health_only, both]);
    world.audit().expect("audit");
}

#[test]
fn editing_one_component_emits_only_that_delta() {
    let mut world = World::new();
    let handle = spawn_health(&mut world, 1);
    world.insert(handle, Position(4)).expect("position");
    let position_tick = world.tick_of::<Position>(handle).expect("tick");
    let cursor = world.clock();
    {
        let mut health = world.get_mut::<Health>(handle).expect("health");
        health.0 = 5;
    }
    let deltas = world.changes_since(cursor);
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].component, "health");
    assert_eq!(deltas[0].handle, handle);
    assert_eq!(deltas[0].bytes, Health(5).to_bytes());
    assert_eq!(world.tick_of::<Position>(handle), Some(position_tick));
    assert_eq!(world.changes_since(world.clock()).len(), 0);
}

#[test]
fn a_read_does_not_advance_the_component_tick() {
    let mut world = World::new();
    let handle = spawn_health(&mut world, 1);
    let tick = world.tick_of::<Health>(handle).expect("tick");
    let cursor = world.clock();
    let _ = world.get::<Health>(handle);
    {
        let health = world.get_mut::<Health>(handle).expect("health");
        let _ = health.0;
    }
    assert_eq!(world.tick_of::<Health>(handle), Some(tick));
    assert_eq!(world.clock(), cursor);
    assert_eq!(world.changes_since(cursor).len(), 0);
}

#[test]
fn an_outfit_asset_change_is_a_32_byte_delta() {
    let coat = sha256(b"coat");
    let hat = sha256(b"hat");
    assert_eq!(coat.len(), 32);
    let mut world = World::new();
    let handle = world.spawn_with(Outfit { mesh: coat }).expect("spawn");
    world.insert(handle, Health(1)).expect("health");
    let cursor = world.clock();
    world.insert(handle, Outfit { mesh: hat }).expect("hat");
    let deltas = world.changes_since(cursor);
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].component, "outfit");
    assert_eq!(deltas[0].bytes, hat);
    assert_ne!(deltas[0].bytes, coat);
    let mut replica = World::new();
    replica.register::<Outfit>().expect("register");
    let copied = replica.spawn().expect("replica");
    assert_eq!(copied, handle);
    replica.apply_delta(&deltas[0]).expect("apply");
    assert_eq!(replica.get::<Outfit>(copied).expect("outfit").mesh, hat);
}

#[test]
fn a_stale_delta_and_a_bad_payload_are_refused() {
    let mut world = World::new();
    world.register::<Health>().expect("register");
    let handle = spawn_health(&mut world, 1);
    let delta = world.changes_since(0).into_iter().next().expect("delta");
    world.despawn(handle).expect("despawn");
    let err = world.apply_delta(&delta).expect_err("stale");
    assert!(matches!(err, genos_ecs::StoreError::Stale));
    let live = spawn_health(&mut world, 2);
    let mut bad = delta;
    bad.handle = live;
    bad.bytes = vec![1, 2];
    let err = world.apply_delta(&bad).expect_err("decode");
    assert!(matches!(err, genos_ecs::StoreError::Decode));
    assert_eq!(world.get::<Health>(live).expect("unchanged").0, 2);
}

#[test]
fn removing_a_component_drops_it_from_the_query_and_keeps_the_other() {
    let mut world = World::new();
    let handle = spawn_health(&mut world, 6);
    world.insert(handle, Position(7)).expect("position");
    let removed = world.remove::<Health>(handle).expect("remove");
    assert_eq!(removed, Health(6));
    assert!(world.get::<Health>(handle).is_none());
    assert_eq!(world.get::<Position>(handle).expect("position").0, 7);
    assert!(world.alive(handle));
    world.audit().expect("audit");
}

#[test]
fn a_clone_does_not_share_later_writes() {
    let mut world = World::new();
    let handle = spawn_health(&mut world, 1);
    let mut copy = world.clone();
    copy.insert(handle, Health(9)).expect("copy");
    assert_eq!(world.get::<Health>(handle).expect("original").0, 1);
    assert_eq!(copy.get::<Health>(handle).expect("copy").0, 9);
}

#[test]
fn wide_column_visits_every_row() {
    let mut world = World::new();
    for value in 0..4096u32 {
        spawn_health(&mut world, value);
    }
    let mut count = 0u32;
    let mut sum = 0u64;
    world.for_each::<Health>(|_, health| {
        count += 1;
        sum += u64::from(health.0);
    });
    assert_eq!(count, 4096);
    assert_eq!(sum, 4095 * 4096 / 2);
    world.audit().expect("audit");
}

#[test]
fn a_cursor_from_clock_sees_the_next_write_and_keeps_older_ones() {
    let mut world = World::new();
    let handle = spawn_health(&mut world, 1);
    let cursor = world.clock();
    world.insert(handle, Position(9)).expect("position");
    let edit = world.changes_since(cursor);
    assert_eq!(edit.len(), 1);
    assert_eq!(edit[0].component, "position");
    let both = world.changes_since(0);
    assert_eq!(both.len(), 2, "an older unsent write was skipped");
    assert!(both.iter().any(|delta| delta.component == "health"));
    assert!(both.iter().any(|delta| delta.component == "position"));
    assert_eq!(world.changes_since(world.clock()).len(), 0);
}

#[test]
fn a_peek_does_not_change_the_tick_a_later_write_stores() {
    fn tick_after_edit(peek: bool) -> u32 {
        let mut world = World::new();
        let handle = spawn_health(&mut world, 1);
        if peek {
            let guard = world.get_mut::<Health>(handle).expect("peek");
            let _ = guard.0;
        }
        world.insert(handle, Health(2)).expect("edit");
        world.tick_of::<Health>(handle).expect("tick")
    }
    assert_eq!(tick_after_edit(false), tick_after_edit(true));
}

#[test]
fn a_follower_drops_a_removed_component_and_a_despawned_object() {
    let mut source = World::new();
    let handle = spawn_health(&mut source, 1);
    source.insert(handle, Position(2)).expect("position");
    let cursor = source.clock();
    source.remove::<Health>(handle).expect("remove");
    let removed = source.changes_since(cursor);
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].component, "health");
    assert_eq!(removed[0].bytes.len(), 0);

    let mut follower = World::new();
    follower.register::<Health>().expect("health");
    follower.register::<Position>().expect("position");
    let copied = follower.spawn_with(Health(1)).expect("spawn");
    follower.insert(copied, Position(2)).expect("position");
    assert_eq!(copied, handle);
    follower.apply_delta(&removed[0]).expect("apply removal");
    assert!(follower.get::<Health>(copied).is_none());
    assert_eq!(follower.get::<Position>(copied).expect("position").0, 2);
    assert!(follower.alive(copied));

    let cursor = source.clock();
    source.despawn(handle).expect("despawn");
    let gone = source.changes_since(cursor);
    assert!(gone.iter().any(|delta| {
        delta.handle == handle && delta.component.is_empty() && delta.bytes.is_empty()
    }));
    for delta in &gone {
        follower.apply_delta(delta).expect("apply despawn");
    }
    assert!(!follower.alive(copied));
}

#[test]
fn a_duplicate_component_name_is_rejected() {
    #[derive(Clone, Copy)]
    struct Alias(u32);
    impl Component for Alias {
        fn component_name() -> &'static str {
            "health"
        }
        fn encode(&self, out: &mut Vec<u8>) {
            push_u32(out, self.0);
        }
        fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
            let mut input = bytes;
            let value = read_u32(&mut input)?;
            if !input.is_empty() {
                return Err(DecodeError::Trailing);
            }
            Ok(Self(value))
        }
    }
    let mut world = World::new();
    world.register::<Health>().expect("health");
    let err = world.register::<Alias>().expect_err("duplicate");
    assert!(matches!(err, genos_ecs::StoreError::Unknown));
}

#[test]
fn scene_handle_new_rejects_generation_zero() {
    assert!(SceneHandle::new(0, 0).is_none());
    assert_eq!(SceneHandle::new(3, 1).expect("handle").generation(), 1);
}

#[test]
fn the_store_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<World>();
}
