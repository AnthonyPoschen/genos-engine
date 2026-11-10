//! Placed objects as archetype rows.
//!
//! The draw lists on [`Scene`] stay. [`publish`] copies the floor, each wall,
//! each solid, and each light into a [`World`] in that order. A second publish
//! of the same lists returns the same handles. A menu or lighting setting is
//! not a row, so editing one emits no component delta.
//!
//! `FixedPart::Wall(usize)` still indexes the draw list. ADR 0032 keeps that
//! index until a later change. This module is the store those indices do not
//! own yet.

use genos_ecs::{
    push_f32, push_u32, read_f32, read_u32, Component, DecodeError, SceneHandle, StoreError, World,
};

use crate::types::{Light, Scene, Solid, Wall};

const KIND_FLOOR: u32 = 1;
const KIND_WALL: u32 = 2;
const KIND_SOLID: u32 = 3;
const KIND_LIGHT: u32 = 4;

/// One placed object in the store: a kind, its place in that kind's draw list,
/// and the position the draw list had at publish.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// [`KIND_FLOOR`], [`KIND_WALL`], [`KIND_SOLID`], or [`KIND_LIGHT`].
    pub kind: u32,
    /// Index in that kind's draw list. The floor is 0.
    pub ordinal: u32,
    /// World X, meters.
    pub x: f32,
    /// World Y, meters.
    pub y: f32,
    /// World Z, meters.
    pub z: f32,
}

impl Component for Placed {
    fn component_name() -> &'static str {
        "placed"
    }

    fn encode(&self, out: &mut Vec<u8>) {
        push_u32(out, self.kind);
        push_u32(out, self.ordinal);
        push_f32(out, self.x);
        push_f32(out, self.y);
        push_f32(out, self.z);
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut input = bytes;
        let kind = read_u32(&mut input)?;
        let ordinal = read_u32(&mut input)?;
        let x = read_f32(&mut input)?;
        let y = read_f32(&mut input)?;
        let z = read_f32(&mut input)?;
        if !input.is_empty() {
            return Err(DecodeError::Trailing);
        }
        Ok(Self {
            kind,
            ordinal,
            x,
            y,
            z,
        })
    }
}

/// Copy the draw lists into a new store. Handles follow spawn order: floor,
/// walls, solids, lights. The same lists produce the same handles.
///
/// # Errors
///
/// The store cannot take another object, or a list is longer than `u32::MAX`.
pub fn publish(scene: &Scene) -> Result<World, StoreError> {
    let mut world = World::new();
    world.spawn_with(Placed {
        kind: KIND_FLOOR,
        ordinal: 0,
        x: scene.floor.position.x,
        y: scene.floor.position.y,
        z: scene.floor.position.z,
    })?;
    for (ordinal, wall) in scene.walls.iter().enumerate() {
        world.spawn_with(wall_row(ordinal, wall)?)?;
    }
    for (ordinal, solid) in scene.solids.iter().enumerate() {
        world.spawn_with(solid_row(ordinal, solid)?)?;
    }
    for (ordinal, light) in scene.lights.iter().enumerate() {
        world.spawn_with(light_row(ordinal, light)?)?;
    }
    Ok(world)
}

/// A scene and the store published from its draw lists.
pub struct Published {
    /// The scene the store was copied from.
    pub scene: Scene,
    /// Rows for the floor, walls, solids, and lights on [`Self::scene`].
    pub world: World,
}

impl Published {
    /// Publish `scene` and keep that store beside it.
    ///
    /// # Errors
    ///
    /// [`publish`] failed.
    pub fn new(scene: Scene) -> Result<Self, StoreError> {
        let world = publish(&scene)?;
        Ok(Self { scene, world })
    }

    /// Set menu visibility.
    ///
    /// The menu is a setting, not a placed row, so this does not write
    /// [`Self::world`].
    ///
    /// # Errors
    ///
    /// The menu key rejected the value.
    pub fn set_menu_visible(&mut self, visible: bool) -> Result<(), String> {
        self.scene
            .menu
            .set_key("visible", if visible { "shown" } else { "hidden" })
    }
}

/// Write a new position on the store row.
///
/// This is the store copy only. `scene.walls`, `scene.solids`, and
/// `scene.lights` stay as they were at [`publish`]. An unchanged position
/// does not write, so it emits no delta. A stale handle increments
/// [`genos_ecs::Stats::stale_rejects`] and writes nothing.
///
/// # Errors
///
/// [`StoreError::Stale`] when `handle` is not live. [`StoreError::Missing`]
/// when the row has no [`Placed`] component.
pub fn move_placed(
    world: &mut World,
    handle: SceneHandle,
    x: f32,
    y: f32,
    z: f32,
) -> Result<(), StoreError> {
    let current = match world.get::<Placed>(handle) {
        Some(row) => row.clone(),
        None => {
            return match world.get_mut::<Placed>(handle) {
                Err(err) => Err(err),
                Ok(_row) => Err(StoreError::Missing),
            };
        }
    };
    if same_bits(current.x, x) && same_bits(current.y, y) && same_bits(current.z, z) {
        return Ok(());
    }
    let mut next = current;
    next.x = x;
    next.y = y;
    next.z = z;
    world.insert(handle, next)
}

fn same_bits(left: f32, right: f32) -> bool {
    left.to_bits() == right.to_bits()
}

fn wall_row(ordinal: usize, wall: &Wall) -> Result<Placed, StoreError> {
    Ok(Placed {
        kind: KIND_WALL,
        ordinal: fit(ordinal)?,
        x: wall.position.x,
        y: wall.position.y,
        z: wall.position.z,
    })
}

fn solid_row(ordinal: usize, solid: &Solid) -> Result<Placed, StoreError> {
    Ok(Placed {
        kind: KIND_SOLID,
        ordinal: fit(ordinal)?,
        x: solid.position.x,
        y: solid.position.y,
        z: solid.position.z,
    })
}

fn light_row(ordinal: usize, light: &Light) -> Result<Placed, StoreError> {
    Ok(Placed {
        kind: KIND_LIGHT,
        ordinal: fit(ordinal)?,
        x: light.position.x,
        y: light.position.y,
        z: light.position.z,
    })
}

fn fit(ordinal: usize) -> Result<u32, StoreError> {
    u32::try_from(ordinal).map_err(|_| StoreError::Full)
}
