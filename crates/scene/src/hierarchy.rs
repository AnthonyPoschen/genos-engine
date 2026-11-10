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

/// World pose of `handle`'s parent. `None` when `handle` is a root.
pub(crate) fn parent_basis(scene: &Scene, handle: SceneHandle) -> Option<(Vec3, f32)> {
    let link = scene.object_world().get::<Parent>(handle)?;
    world_anchor(scene, link.of)
}

/// Bake each direct child's world translation (and a square's world yaw) into
/// its local values, then drop parent links that name `dead`.
///
/// The child is still parented while the pose is read, so the baked local
/// value is the world value. A wall, the floor, and a light have no yaw field;
/// their orientation is the parent's until the link goes, and only the
/// translation is kept.
pub(crate) fn detach_orphans(scene: &mut Scene, dead: SceneHandle) {
    let mut children = Vec::new();
    scene.objects.for_each::<Parent>(|handle, parent| {
        if parent.of == dead {
            children.push(handle);
        }
    });
    for child in children {
        let Some((origin, yaw)) = world_anchor(scene, child) else {
            continue;
        };
        if scene.objects.remove::<Parent>(child).is_err() {
            continue;
        }
        write_baked(scene, child, origin, yaw);
    }
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

    /// World translation of the floor's position.
    #[must_use]
    pub fn floor_translation(&self) -> Vec3 {
        slot_pose(self, 0).0
    }

    /// World translation of wall `index`'s position.
    #[must_use]
    pub fn wall_translation(&self, index: usize) -> Vec3 {
        if index >= self.wall_len() {
            return Vec3::ZERO;
        }
        slot_pose(self, 1 + index).0
    }

    /// World yaw of wall `index`.
    #[must_use]
    pub fn wall_yaw(&self, index: usize) -> f32 {
        if index >= self.wall_len() {
            return 0.0;
        }
        slot_pose(self, 1 + index).1
    }

    /// World position of `local` on wall `index`.
    #[must_use]
    pub fn wall_point(&self, index: usize, local: Vec3) -> Vec3 {
        let Some(anchor) = self.wall_at(index).map(|wall| wall.position) else {
            return local;
        };
        apply_pose(slot_pose(self, 1 + index), anchor, local)
    }

    /// World translation of solid `index`'s position.
    #[must_use]
    pub fn solid_translation(&self, index: usize) -> Vec3 {
        if index >= self.solid_len() {
            return Vec3::ZERO;
        }
        slot_pose(self, solid_slot(self, index)).0
    }

    /// World yaw of solid `index`. A circle contributes `0`.
    #[must_use]
    pub fn solid_yaw(&self, index: usize) -> f32 {
        if index >= self.solid_len() {
            return 0.0;
        }
        slot_pose(self, solid_slot(self, index)).1
    }

    /// World position of `local` on solid `index`.
    #[must_use]
    pub fn solid_point(&self, index: usize, local: Vec3) -> Vec3 {
        let Some(anchor) = self.solid_at(index).map(|solid| solid.position) else {
            return local;
        };
        apply_pose(slot_pose(self, solid_slot(self, index)), anchor, local)
    }

    /// World translation of light `index`'s position.
    #[must_use]
    pub fn light_translation(&self, index: usize) -> Vec3 {
        if index >= self.light_len() {
            return Vec3::ZERO;
        }
        slot_pose(self, light_slot(self, index)).0
    }

    /// World yaw of light `index`. A light contributes `0` of its own.
    #[must_use]
    pub fn light_yaw(&self, index: usize) -> f32 {
        if index >= self.light_len() {
            return 0.0;
        }
        slot_pose(self, light_slot(self, index)).1
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
    let slot = pose_slot(scene, handle)?;
    Some(slot_pose(scene, slot))
}

fn write_baked(scene: &mut Scene, handle: SceneHandle, origin: Vec3, yaw: f32) {
    if let Some(index) = scene.solid_index(handle) {
        if let Some(solid) = scene.solid_at_mut(index) {
            solid.position = origin;
            if solid.shape == Shape::Square {
                solid.yaw = yaw;
            }
        }
        return;
    }
    if let Some(index) = scene.wall_order.iter().position(|item| *item == handle) {
        if let Some(wall) = scene.wall_at_mut(index) {
            wall.position = origin;
        }
        return;
    }
    if let Some(index) = scene.light_order.iter().position(|item| *item == handle) {
        if let Some(light) = scene.light_at_mut(index) {
            light.position = origin;
        }
        return;
    }
    if handle == scene.floor_handle() {
        scene.floor.position = origin;
        scene.sync_floor();
    }
}

fn apply_pose(pose: (Vec3, f32), anchor: Vec3, local: Vec3) -> Vec3 {
    let (origin, yaw) = pose;
    origin + Quat::from_axis_angle(Vec3::Y, yaw).rotate(local - anchor)
}

fn solid_slot(scene: &Scene, index: usize) -> usize {
    1 + scene.wall_len() + index
}

fn light_slot(scene: &Scene, index: usize) -> usize {
    1 + scene.wall_len() + scene.solid_len() + index
}

fn pose_slot(scene: &Scene, handle: SceneHandle) -> Option<usize> {
    if handle == scene.floor_handle() {
        return Some(0);
    }
    if let Some(index) = scene.wall_order.iter().position(|item| *item == handle) {
        return Some(1 + index);
    }
    let walls = scene.wall_len();
    if let Some(index) = scene.solid_order.iter().position(|item| *item == handle) {
        return Some(1 + walls + index);
    }
    let solids = scene.solid_len();
    if let Some(index) = scene.light_order.iter().position(|item| *item == handle) {
        return Some(1 + walls + solids + index);
    }
    None
}

fn slot_pose(scene: &Scene, slot: usize) -> (Vec3, f32) {
    ensure_poses(scene);
    let scratch = read_poses(scene);
    if slot >= scratch.origin.len() {
        return (Vec3::ZERO, 0.0);
    }
    (scratch.origin[slot], scratch.yaw[slot])
}

fn read_poses(scene: &Scene) -> std::sync::RwLockReadGuard<'_, crate::types::PoseScratch> {
    scene.poses.read()
}

fn write_poses(scene: &Scene) -> std::sync::RwLockWriteGuard<'_, crate::types::PoseScratch> {
    scene.poses.write()
}

fn ensure_poses(scene: &Scene) {
    let clock = scene.objects.clock();
    let floor_x = scene.floor.position.x.to_bits();
    let floor_y = scene.floor.position.y.to_bits();
    let floor_z = scene.floor.position.z.to_bits();
    let count = placed(scene);
    {
        let scratch = read_poses(scene);
        if scratch.clock == clock
            && scratch.floor_x == floor_x
            && scratch.floor_y == floor_y
            && scratch.floor_z == floor_z
            && scratch.origin.len() == count
        {
            return;
        }
    }
    {
        let mut scratch = write_poses(scene);
        scratch.origin.resize(count, Vec3::ZERO);
        scratch.yaw.resize(count, 0.0);
        scratch.ready.resize(count, 0);
        scratch.parent.resize(count, count);
        scratch.ready.fill(0);
        scratch.parent.fill(count);
    }
    for slot in 0..count {
        let Some((handle, local, local_yaw)) = slot_local(scene, slot) else {
            continue;
        };
        let parent_slot = scene
            .object_world()
            .get::<Parent>(handle)
            .and_then(|link| pose_slot(scene, link.of))
            .filter(|parent| *parent != slot);
        let mut scratch = write_poses(scene);
        scratch.origin[slot] = local;
        scratch.yaw[slot] = local_yaw;
        if let Some(parent) = parent_slot {
            scratch.parent[slot] = parent;
        }
    }
    let mut scratch = write_poses(scene);
    resolve_poses(&mut scratch, count);
    scratch.clock = clock;
    scratch.floor_x = floor_x;
    scratch.floor_y = floor_y;
    scratch.floor_z = floor_z;
}

fn resolve_poses(scratch: &mut crate::types::PoseScratch, count: usize) {
    for _ in 0..count {
        let mut moved = false;
        for slot in 0..count {
            if scratch.ready[slot] == 1 {
                continue;
            }
            let parent = scratch.parent[slot];
            if parent == count || scratch.ready[parent] == 1 {
                if parent != count {
                    let (origin, yaw) = compose(
                        scratch.origin[parent],
                        scratch.yaw[parent],
                        scratch.origin[slot],
                        scratch.yaw[slot],
                    );
                    scratch.origin[slot] = origin;
                    scratch.yaw[slot] = yaw;
                }
                scratch.ready[slot] = 1;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
}

fn compose(origin: Vec3, yaw: f32, local: Vec3, local_yaw: f32) -> (Vec3, f32) {
    (
        origin + Quat::from_axis_angle(Vec3::Y, yaw).rotate(local),
        yaw + local_yaw,
    )
}

fn slot_local(scene: &Scene, slot: usize) -> Option<(SceneHandle, Vec3, f32)> {
    let walls = scene.wall_len();
    let solids = scene.solid_len();
    if slot == 0 {
        return Some((scene.floor_handle(), scene.floor.position, 0.0));
    }
    if slot <= walls {
        let index = slot - 1;
        let handle = scene.wall_handle(index)?;
        let position = scene.wall_at(index)?.position;
        return Some((handle, position, 0.0));
    }
    if slot <= walls + solids {
        let index = slot - 1 - walls;
        let handle = scene.solid_handle(index)?;
        let solid = scene.solid_at(index)?;
        let yaw = match solid.shape {
            Shape::Square => solid.yaw,
            Shape::Circle => 0.0,
        };
        return Some((handle, solid.position, yaw));
    }
    let index = slot - 1 - walls - solids;
    let handle = scene.light_handle(index)?;
    let position = scene.light_at(index)?.position;
    Some((handle, position, 0.0))
}
