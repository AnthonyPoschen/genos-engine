//! Parent link and the world translation it produces.
//!
//! A root's world translation is its local position. A child's world translation
//! is the parent's world translation plus the parent's yaw about +Y applied to
//! the child's local position. Scale stays `(1, 1, 1)`. A cycle is rejected
//! and the previous parent stays.

use genos_ecs::{push_u32, read_u32, Component, DecodeError, SceneHandle, StoreError};
use genos_math::{Quat, Vec3};

use crate::types::{Scene, Shape};

/// The parent of one placed object. The root has no `Parent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parent {
    /// Handle of the parent object.
    pub of: SceneHandle,
}

impl Component for Parent {
    fn component_name() -> &'static str {
        "parent"
    }

    fn encode(&self, out: &mut Vec<u8>) {
        push_u32(out, self.of.index());
        push_u32(out, self.of.generation());
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut input = bytes;
        let index = read_u32(&mut input)?;
        let generation = read_u32(&mut input)?;
        if !input.is_empty() {
            return Err(DecodeError::Trailing);
        }
        let Some(of) = SceneHandle::new(index, generation) else {
            return Err(DecodeError::Truncated);
        };
        Ok(Self { of })
    }
}

/// Why a parent write did not apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HierarchyError {
    /// The child or the parent handle is not live.
    Stale,
    /// The link would make the child an ancestor of itself.
    Cycle,
}

impl std::fmt::Display for HierarchyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stale => formatter.write_str("scene handle is stale"),
            Self::Cycle => formatter.write_str("parent link would cycle"),
        }
    }
}

impl std::error::Error for HierarchyError {}

/// Set `child`'s parent. A cycle increments [`Scene::cycle_rejects`] and
/// leaves the stored parent as it was.
///
/// # Errors
///
/// [`HierarchyError::Stale`] when either handle is not live.
/// [`HierarchyError::Cycle`] when the link would loop.
pub fn set_parent(
    scene: &mut Scene,
    child: SceneHandle,
    parent: SceneHandle,
) -> Result<(), HierarchyError> {
    if !scene.object_world().alive(child) || !scene.object_world().alive(parent) {
        return Err(HierarchyError::Stale);
    }
    if closes_cycle(scene, child, parent) {
        scene.cycle_rejects = scene.cycle_rejects.saturating_add(1);
        return Err(HierarchyError::Cycle);
    }
    scene
        .objects
        .insert(child, Parent { of: parent })
        .map_err(error_of)
}

/// World position of `local`, which is expressed in the object's local space.
///
/// A handle that is not in this scene returns `local`.
#[must_use]
pub fn world_point(scene: &Scene, handle: SceneHandle, local: Vec3) -> Vec3 {
    let Some((anchor, _)) = local_anchor(scene, handle) else {
        return local;
    };
    let Some((origin, yaw)) = world_anchor(scene, handle) else {
        return local;
    };
    origin + Quat::from_axis_angle(Vec3::Y, yaw).rotate(local - anchor)
}

/// World yaw about +Y. A wall, the floor, and a light contribute `0`.
#[must_use]
pub fn world_yaw(scene: &Scene, handle: SceneHandle) -> f32 {
    world_anchor(scene, handle).map_or(0.0, |(_, yaw)| yaw)
}

impl Scene {
    /// World position of a point stored in this object's local space.
    #[must_use]
    pub fn world_point(&self, handle: SceneHandle, local: Vec3) -> Vec3 {
        world_point(self, handle, local)
    }

    /// World yaw about +Y for one object.
    #[must_use]
    pub fn world_yaw(&self, handle: SceneHandle) -> f32 {
        world_yaw(self, handle)
    }
}

fn error_of(error: StoreError) -> HierarchyError {
    match error {
        StoreError::Stale
        | StoreError::Missing
        | StoreError::Full
        | StoreError::Unknown
        | StoreError::Decode => HierarchyError::Stale,
    }
}

fn closes_cycle(scene: &Scene, child: SceneHandle, parent: SceneHandle) -> bool {
    if child == parent {
        return true;
    }
    let mut cursor = parent;
    for _ in 0..=placed(scene) {
        if cursor == child {
            return true;
        }
        match scene.object_world().get::<Parent>(cursor) {
            Some(link) => cursor = link.of,
            None => return false,
        }
    }
    true
}

fn placed(scene: &Scene) -> usize {
    1 + scene.wall_len() + scene.solid_len() + scene.light_len()
}

fn local_anchor(scene: &Scene, handle: SceneHandle) -> Option<(Vec3, f32)> {
    if handle == scene.floor_handle() {
        return Some((scene.floor.position, 0.0));
    }
    if let Some(index) = scene.wall_order.iter().position(|item| *item == handle) {
        let wall = scene.wall_at(index)?;
        return Some((wall.position, 0.0));
    }
    if let Some(index) = scene.solid_order.iter().position(|item| *item == handle) {
        let solid = scene.solid_at(index)?;
        let yaw = match solid.shape {
            Shape::Square => solid.yaw,
            Shape::Circle => 0.0,
        };
        return Some((solid.position, yaw));
    }
    if let Some(index) = scene.light_order.iter().position(|item| *item == handle) {
        let light = scene.light_at(index)?;
        return Some((light.position, 0.0));
    }
    None
}

fn world_anchor(scene: &Scene, handle: SceneHandle) -> Option<(Vec3, f32)> {
    let mut chain = Vec::new();
    let mut cursor = handle;
    for _ in 0..=placed(scene) {
        let (local, yaw) = local_anchor(scene, cursor)?;
        let parent = scene
            .object_world()
            .get::<Parent>(cursor)
            .map(|link| link.of);
        chain.push((local, yaw));
        match parent {
            Some(next) if next != cursor => cursor = next,
            _ => break,
        }
    }
    let mut origin = Vec3::ZERO;
    let mut yaw = 0.0;
    for (local, local_yaw) in chain.iter().rev() {
        origin += Quat::from_axis_angle(Vec3::Y, yaw).rotate(*local);
        yaw += *local_yaw;
    }
    Some((origin, yaw))
}
