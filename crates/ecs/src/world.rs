//! Archetype tables. One object lives in the table of its component set.

use std::any::{Any, TypeId};
use std::collections::BTreeSet;
use std::fmt;
use std::ops::{Deref, DerefMut};

use crate::{Component, DecodeError, SceneHandle};

/// Why a write was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// The generation is not the live one, or the slot is empty.
    Stale,
    /// The live object has no component of that type.
    Missing,
    /// The handle table cannot grow past `u32::MAX` slots.
    Full,
    /// The component name was not registered, or a column was the wrong type.
    Unknown,
    /// [`Component::decode`] refused the bytes.
    Decode,
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("stale scene handle"),
            Self::Missing => formatter.write_str("missing component"),
            Self::Full => formatter.write_str("scene handle table is full"),
            Self::Unknown => formatter.write_str("unknown component"),
            Self::Decode => formatter.write_str("component bytes did not decode"),
        }
    }
}

impl std::error::Error for StoreError {}

/// One changed component, ready for a replication pass.
///
/// A write carries [`Component::encode`] of that component only. A removal
/// carries empty [`Self::bytes`]: an empty [`Self::component`] name means the
/// object was despawned, and a name means that component was removed. That is
/// the empty-bounds removal from ADR 0032. A component's own `encode` must
/// write at least one byte so a payload is not read as a removal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentDelta {
    /// Object that changed.
    pub handle: SceneHandle,
    /// [`Component::component_name`], or `""` when the object was despawned.
    pub component: &'static str,
    /// Tick stored on the cell. A later write is ahead of an earlier one until the clock wraps.
    pub tick: u32,
    /// Canonical component bytes, or empty for a removal.
    pub bytes: Vec<u8>,
}

/// Counters a debug binary and a test can read without walking the tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    /// Objects that have not been despawned.
    pub alive: u32,
    /// Slots waiting to be reused.
    pub free: u32,
    /// Archetype tables, including empty ones left behind by a move.
    pub archetypes: u32,
    /// Next tick a write will store. Starts at 1. Skips 0 after a wrap.
    ///
    /// Store this after applying [`World::changes_since`]. The next call with
    /// the stored value returns only the writes that happened after it.
    pub clock: u32,
    /// Writes refused because the handle was stale.
    pub stale_rejects: u32,
}

/// Exclusive access to one component. Dropping it after a write stores the tick.
///
/// A peek through [`Deref`] does not take a tick. [`DerefMut`] marks the guard
/// dirty, and drop then stores the clock.
pub struct Mut<'a, T> {
    value: &'a mut T,
    tick: &'a mut u32,
    clock: &'a mut u32,
    dirty: bool,
}

impl<T> Deref for Mut<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value
    }
}

impl<T> DerefMut for Mut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.dirty = true;
        self.value
    }
}

impl<T> Drop for Mut<'_, T> {
    fn drop(&mut self) {
        if self.dirty {
            *self.tick = take_tick(self.clock);
        }
    }
}

#[derive(Clone, Copy)]
struct Place {
    archetype: u32,
    row: u32,
}

#[derive(Clone)]
struct Slot {
    generation: u32,
    place: Option<Place>,
}

struct Archetype {
    signature: Vec<TypeId>,
    entities: Vec<SceneHandle>,
    columns: Vec<Box<dyn Column>>,
}

impl Archetype {
    fn empty() -> Self {
        Self {
            signature: Vec::new(),
            entities: Vec::new(),
            columns: Vec::new(),
        }
    }

    fn cloned(&self) -> Self {
        Self {
            signature: self.signature.clone(),
            entities: self.entities.clone(),
            columns: self
                .columns
                .iter()
                .map(|column| column.clone_box())
                .collect(),
        }
    }

    fn index_of(&self, type_id: TypeId) -> Option<usize> {
        self.signature.binary_search(&type_id).ok()
    }
}

struct TypedCol<T> {
    name: &'static str,
    values: Vec<T>,
    ticks: Vec<u32>,
}

trait Column: Send + Sync {
    fn len(&self) -> usize;
    fn swap_remove(&mut self, row: usize) -> bool;
    fn pop_row(&mut self) -> bool;
    fn push_clone_from(&mut self, source: &dyn Column, row: usize) -> bool;
    fn changes(&self, since: u32) -> Vec<(usize, u32, Vec<u8>)>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn clone_box(&self) -> Box<dyn Column>;
    fn name(&self) -> &'static str;
}

impl<T: Component> Column for TypedCol<T> {
    fn len(&self) -> usize {
        self.values.len()
    }

    fn swap_remove(&mut self, row: usize) -> bool {
        swap_remove_vec(&mut self.values, row).is_some()
            && swap_remove_vec(&mut self.ticks, row).is_some()
    }

    fn pop_row(&mut self) -> bool {
        self.values.pop().is_some() && self.ticks.pop().is_some()
    }

    fn push_clone_from(&mut self, source: &dyn Column, row: usize) -> bool {
        let Some(source) = source.as_any().downcast_ref::<TypedCol<T>>() else {
            return false;
        };
        let (Some(value), Some(tick)) = (source.values.get(row), source.ticks.get(row)) else {
            return false;
        };
        self.values.push(value.clone());
        self.ticks.push(*tick);
        true
    }

    fn changes(&self, since: u32) -> Vec<(usize, u32, Vec<u8>)> {
        let mut out = Vec::new();
        for (row, (value, tick)) in self.values.iter().zip(&self.ticks).enumerate() {
            if changed_since(*tick, since) {
                out.push((row, *tick, value.to_bytes()));
            }
        }
        out
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Column> {
        Box::new(Self {
            name: self.name,
            values: self.values.clone(),
            ticks: self.ticks.clone(),
        })
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

#[derive(Clone, Copy)]
struct Kind {
    type_id: TypeId,
    name: &'static str,
    make: fn() -> Box<dyn Column>,
    apply: fn(&mut World, SceneHandle, &[u8]) -> Result<(), StoreError>,
    remove: fn(&mut World, SceneHandle) -> Result<(), StoreError>,
}

#[derive(Clone)]
struct Removal {
    handle: SceneHandle,
    /// Empty when the object itself was despawned.
    component: &'static str,
    tick: u32,
}

/// The scene store.
///
/// Objects with the same component set share one table. Columns of that table
/// are contiguous. A write stores the current clock on that cell, then
/// advances it. [`Self::clock`] is the next tick a write will store.
/// [`Self::changes_since`] returns cells and removals whose tick is that
/// cursor or newer, in handle-index order, then wrapping distance from the
/// cursor, then component name. A removal older than half the tick space is
/// dropped, so a wrapped clock does not re-emit it.
pub struct World {
    slots: Vec<Slot>,
    free: BTreeSet<u32>,
    archetypes: Vec<Archetype>,
    kinds: Vec<Kind>,
    removals: Vec<Removal>,
    clock: u32,
    stale_rejects: u32,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for World {
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
            free: self.free.clone(),
            archetypes: self.archetypes.iter().map(Archetype::cloned).collect(),
            kinds: self.kinds.clone(),
            removals: self.removals.clone(),
            clock: self.clock,
            stale_rejects: self.stale_rejects,
        }
    }
}

impl World {
    /// An empty store. The first spawned handle is index 0, generation 1.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: BTreeSet::new(),
            archetypes: vec![Archetype::empty()],
            kinds: Vec::new(),
            removals: Vec::new(),
            clock: 1,
            stale_rejects: 0,
        }
    }

    /// Record `T` so [`Self::apply_delta`] can rebuild it by name.
    ///
    /// [`Self::insert`] registers the type as well. Call this on a receiver
    /// that has not inserted `T` yet.
    ///
    /// # Errors
    ///
    /// [`StoreError::Unknown`] when the name is empty or another component
    /// already registered it.
    pub fn register<T: Component>(&mut self) -> Result<(), StoreError> {
        self.ensure_kind::<T>()
    }

    /// The tick the next write will store.
    ///
    /// Sample this, then write. [`Self::changes_since`] of the sampled value
    /// includes that write and every later one, and it does not skip an older
    /// write whose tick is still at or after the cursor. After the deltas are
    /// applied, store [`Self::clock`] again: that value is past the ticks just
    /// consumed.
    #[must_use]
    pub const fn clock(&self) -> u32 {
        self.clock
    }

    /// Alive objects, free slots, tables, the clock, and stale rejects.
    #[must_use]
    pub fn stats(&self) -> Stats {
        let alive = self
            .slots
            .iter()
            .filter(|slot| slot.place.is_some())
            .count();
        Stats {
            alive: as_u32(alive),
            free: as_u32(self.free.len()),
            archetypes: as_u32(self.archetypes.len()),
            clock: self.clock,
            stale_rejects: self.stale_rejects,
        }
    }

    /// `Ok(())` when every live handle points at the row that stores it.
    ///
    /// # Errors
    ///
    /// A string naming the first broken row. A healthy store returns `Ok(())`.
    pub fn audit(&self) -> Result<(), String> {
        for (arch_index, arch) in self.archetypes.iter().enumerate() {
            for column in &arch.columns {
                if column.len() != arch.entities.len() {
                    return Err(format!("archetype {arch_index} column length"));
                }
            }
            for (row, handle) in arch.entities.iter().enumerate() {
                self.audit_row(arch_index, row, *handle)?;
            }
        }
        for index in &self.free {
            let Some(slot_index) = usize::try_from(*index).ok() else {
                return Err("free index".into());
            };
            let Some(slot) = self.slots.get(slot_index) else {
                return Err("free slot".into());
            };
            if slot.place.is_some() {
                return Err(format!("free slot {index} is live"));
            }
        }
        Ok(())
    }

    /// Allocate a handle and place the object in the empty table.
    ///
    /// # Errors
    ///
    /// [`StoreError::Full`] when the table cannot grow.
    pub fn spawn(&mut self) -> Result<SceneHandle, StoreError> {
        let handle = self.alloc_handle()?;
        self.attach_new(handle)?;
        Ok(handle)
    }

    /// [`Self::spawn`] then [`Self::insert`].
    ///
    /// # Errors
    ///
    /// [`StoreError::Full`] when the table cannot grow. The object is despawned
    /// again when the insert fails.
    pub fn spawn_with<T: Component>(&mut self, value: T) -> Result<SceneHandle, StoreError> {
        let handle = self.spawn()?;
        if let Err(err) = self.insert(handle, value) {
            let _ = self.despawn(handle);
            return Err(err);
        }
        Ok(handle)
    }

    /// Free the object. A later spawn may reuse the index with the next generation.
    ///
    /// # Errors
    ///
    /// [`StoreError::Stale`] when `handle` is not live. That increments
    /// [`Stats::stale_rejects`].
    pub fn despawn(&mut self, handle: SceneHandle) -> Result<(), StoreError> {
        self.drop_object(handle)?;
        self.note_removal(handle, "");
        Ok(())
    }

    /// True when `handle` names a live object.
    #[must_use]
    pub fn alive(&self, handle: SceneHandle) -> bool {
        self.find(handle).is_some()
    }

    /// Borrow `T` when the object is live and has that component.
    #[must_use]
    pub fn get<T: Component>(&self, handle: SceneHandle) -> Option<&T> {
        let (archetype, row) = self.find(handle)?;
        self.column::<T>(archetype)?.values.get(row)
    }

    /// Exclusive access. A write through the guard stores a new tick on drop.
    ///
    /// A read through the guard does not take a tick, and neither does dropping
    /// the guard without [`DerefMut`]. Forgetting the guard (instead of
    /// dropping it) skips the tick.
    ///
    /// # Errors
    ///
    /// [`StoreError::Stale`] when the handle is not live.
    /// [`StoreError::Missing`] when the object has no `T`.
    pub fn get_mut<T: Component>(&mut self, handle: SceneHandle) -> Result<Mut<'_, T>, StoreError> {
        let (archetype, row) = self.place(handle)?;
        let World {
            clock, archetypes, ..
        } = self;
        let arch = archetypes.get_mut(archetype).ok_or(StoreError::Missing)?;
        let index = arch
            .index_of(TypeId::of::<T>())
            .ok_or(StoreError::Missing)?;
        let column = arch.columns.get_mut(index).ok_or(StoreError::Missing)?;
        let typed = column
            .as_any_mut()
            .downcast_mut::<TypedCol<T>>()
            .ok_or(StoreError::Missing)?;
        let value = typed.values.get_mut(row).ok_or(StoreError::Missing)?;
        let tick = typed.ticks.get_mut(row).ok_or(StoreError::Missing)?;
        Ok(Mut {
            value,
            tick,
            clock,
            dirty: false,
        })
    }

    /// The tick stored on `T`, if the object has that component.
    #[must_use]
    pub fn tick_of<T: Component>(&self, handle: SceneHandle) -> Option<u32> {
        let (archetype, row) = self.find(handle)?;
        self.column::<T>(archetype)?.ticks.get(row).copied()
    }

    /// Insert or overwrite `T`. An overwrite advances only this component's tick.
    ///
    /// Moving to a wider table copies the other components and keeps their ticks.
    ///
    /// # Errors
    ///
    /// [`StoreError::Stale`] when the handle is not live.
    /// [`StoreError::Full`] when a row index does not fit in `u32`.
    /// [`StoreError::Unknown`] when a column cannot be built.
    pub fn insert<T: Component>(
        &mut self,
        handle: SceneHandle,
        value: T,
    ) -> Result<(), StoreError> {
        self.ensure_kind::<T>()?;
        let (from, row) = self.place(handle)?;
        let type_id = TypeId::of::<T>();
        if self
            .archetypes
            .get(from)
            .is_some_and(|arch| arch.index_of(type_id).is_some())
        {
            return self.overwrite(from, row, value);
        }
        let mut signature = self
            .archetypes
            .get(from)
            .map(|arch| arch.signature.clone())
            .ok_or(StoreError::Unknown)?;
        let Err(pos) = signature.binary_search(&type_id) else {
            return self.overwrite(from, row, value);
        };
        signature.insert(pos, type_id);
        let to = self.ensure_archetype(&signature)?;
        let now = self.take_tick();
        self.move_row(from, row, to, Some((pos, value, now)))
    }

    /// Take `T` off the object. The object stays alive.
    ///
    /// # Errors
    ///
    /// [`StoreError::Stale`] when the handle is not live.
    /// [`StoreError::Missing`] when the object has no `T`.
    pub fn remove<T: Component>(&mut self, handle: SceneHandle) -> Result<T, StoreError> {
        let value = self.take_component::<T>(handle)?;
        self.note_removal(handle, T::component_name());
        Ok(value)
    }

    /// Call `visit` for every object that has `T`.
    ///
    /// Order is table-creation order, then row order. That order is a function
    /// of the spawns and the moves, not of which worker is asking.
    pub fn for_each<T: Component>(&self, mut visit: impl FnMut(SceneHandle, &T)) {
        let type_id = TypeId::of::<T>();
        for arch in &self.archetypes {
            let Some(index) = arch.index_of(type_id) else {
                continue;
            };
            let Some(column) = arch.columns.get(index) else {
                continue;
            };
            let Some(typed) = column.as_any().downcast_ref::<TypedCol<T>>() else {
                continue;
            };
            for (handle, value) in arch.entities.iter().zip(&typed.values) {
                visit(*handle, value);
            }
        }
    }

    /// Call `visit` for every object that has both `A` and `B`.
    ///
    /// The two types must differ. Order matches [`Self::for_each`].
    pub fn for_each_pair<A: Component, B: Component>(
        &self,
        mut visit: impl FnMut(SceneHandle, &A, &B),
    ) {
        if TypeId::of::<A>() == TypeId::of::<B>() {
            return;
        }
        for arch in &self.archetypes {
            let Some((left, right)) = pair_columns::<A, B>(arch) else {
                continue;
            };
            for ((handle, a), b) in arch.entities.iter().zip(&left.values).zip(&right.values) {
                visit(*handle, a, b);
            }
        }
    }

    /// Handles that have `T`, sorted by index.
    #[must_use]
    pub fn handles_with<T: Component>(&self) -> Vec<SceneHandle> {
        let mut out = Vec::new();
        self.for_each::<T>(|handle, _value| out.push(handle));
        out.sort_by_key(|handle| handle.index());
        out
    }

    /// Components and removals at or after `tick`, sorted by handle index, then
    /// wrapping distance from `tick`, then name.
    ///
    /// Pass the value [`Self::clock`] returned before the writes. A cursor of
    /// `0` returns every stored tick that has not been retired, because stored
    /// ticks start at 1 and `0` means "from the start". Any other cursor uses
    /// a wrapping compare, so a tick just after `u32::MAX` is still ahead of a
    /// cursor of `u32::MAX`. Removals older than half the tick space are
    /// omitted. The same distance orders the batch, so a removal at `u32::MAX`
    /// stays ahead of the re-insert at tick `1`.
    #[must_use]
    pub fn changes_since(&self, tick: u32) -> Vec<ComponentDelta> {
        let newest = newest_issued(self.clock);
        let mut out = Vec::new();
        for arch in &self.archetypes {
            for column in &arch.columns {
                for (row, stored, bytes) in column.changes(tick) {
                    let Some(handle) = arch.entities.get(row).copied() else {
                        continue;
                    };
                    out.push(ComponentDelta {
                        handle,
                        component: column.name(),
                        tick: stored,
                        bytes,
                    });
                }
            }
        }
        for removal in &self.removals {
            if removal_expired(removal.tick, newest) {
                continue;
            }
            if changed_since(removal.tick, tick) {
                out.push(ComponentDelta {
                    handle: removal.handle,
                    component: removal.component,
                    tick: removal.tick,
                    bytes: Vec::new(),
                });
            }
        }
        out.sort_by(|left, right| {
            (
                left.handle.index(),
                left.tick.wrapping_sub(tick),
                left.component,
            )
                .cmp(&(
                    right.handle.index(),
                    right.tick.wrapping_sub(tick),
                    right.component,
                ))
        });
        out
    }

    /// Write one delta onto this world.
    ///
    /// A payload requires [`Self::register`] for that component name. An empty
    /// payload is a removal. If that object or component is already absent,
    /// the removal succeeds and records nothing: the worlds already agree, so
    /// a `?` over the rest of the batch continues. A removal that does drop
    /// a present component or object does not record another local removal.
    ///
    /// # Errors
    ///
    /// [`StoreError::Unknown`] when a payload's name was not registered.
    /// [`StoreError::Decode`] when the bytes are not that component.
    /// [`StoreError::Stale`] when a payload names a handle that is not live.
    pub fn apply_delta(&mut self, delta: &ComponentDelta) -> Result<(), StoreError> {
        if delta.bytes.is_empty() {
            return self.apply_removal(delta.handle, delta.component);
        }
        let apply = self
            .kinds
            .iter()
            .find(|kind| kind.name == delta.component)
            .map(|kind| kind.apply)
            .ok_or(StoreError::Unknown)?;
        apply(self, delta.handle, &delta.bytes)
    }

    fn audit_row(&self, arch_index: usize, row: usize, handle: SceneHandle) -> Result<(), String> {
        let Some(slot_index) = usize::try_from(handle.index()).ok() else {
            return Err("handle index".into());
        };
        let Some(slot) = self.slots.get(slot_index) else {
            return Err(format!("missing slot {}", handle.index()));
        };
        if slot.generation != handle.generation() {
            return Err(format!("generation {}", handle.index()));
        }
        let Some(place) = slot.place else {
            return Err(format!("empty place {}", handle.index()));
        };
        if usize::try_from(place.archetype).ok() != Some(arch_index) {
            return Err(format!("archetype {}", handle.index()));
        }
        if usize::try_from(place.row).ok() != Some(row) {
            return Err(format!("row {}", handle.index()));
        }
        Ok(())
    }

    fn alloc_handle(&mut self) -> Result<SceneHandle, StoreError> {
        if let Some(index) = self.free.pop_first() {
            let slot_index = usize::try_from(index).map_err(|_err| StoreError::Full)?;
            let generation = self
                .slots
                .get(slot_index)
                .map(|slot| slot.generation)
                .ok_or(StoreError::Full)?;
            return SceneHandle::new(index, generation).ok_or(StoreError::Stale);
        }
        let index = u32::try_from(self.slots.len()).map_err(|_err| StoreError::Full)?;
        self.slots.push(Slot {
            generation: 1,
            place: None,
        });
        SceneHandle::new(index, 1).ok_or(StoreError::Stale)
    }

    fn note_stale(&mut self) {
        self.stale_rejects = self.stale_rejects.saturating_add(1);
    }

    fn place(&mut self, handle: SceneHandle) -> Result<(usize, usize), StoreError> {
        if let Some(place) = self.find(handle) {
            Ok(place)
        } else {
            self.note_stale();
            Err(StoreError::Stale)
        }
    }

    fn find(&self, handle: SceneHandle) -> Option<(usize, usize)> {
        let index = usize::try_from(handle.index()).ok()?;
        let slot = self.slots.get(index)?;
        if slot.generation != handle.generation() {
            return None;
        }
        let place = slot.place?;
        let archetype = usize::try_from(place.archetype).ok()?;
        let row = usize::try_from(place.row).ok()?;
        self.archetypes.get(archetype)?;
        Some((archetype, row))
    }

    fn ensure_kind<T: Component>(&mut self) -> Result<(), StoreError> {
        let type_id = TypeId::of::<T>();
        if self.kinds.iter().any(|kind| kind.type_id == type_id) {
            return Ok(());
        }
        let name = T::component_name();
        if name.is_empty() || self.kinds.iter().any(|kind| kind.name == name) {
            return Err(StoreError::Unknown);
        }
        self.kinds.push(Kind {
            type_id,
            name,
            make: make_column::<T>,
            apply: apply_component::<T>,
            remove: remove_component::<T>,
        });
        Ok(())
    }

    fn take_tick(&mut self) -> u32 {
        let tick = take_tick(&mut self.clock);
        self.retire_removals();
        tick
    }

    fn retire_removals(&mut self) {
        let newest = newest_issued(self.clock);
        self.removals
            .retain(|removal| !removal_expired(removal.tick, newest));
    }

    /// Place `handle` when the slot is free and already carries its generation.
    ///
    /// A post-wrap spawn reuses the index. The follower applies the despawn
    /// first, then this payload. The generation matches, so the object is
    /// created instead of counting a stale reject.
    fn adopt_for_apply(&mut self, handle: SceneHandle) -> Result<(), StoreError> {
        if self.alive(handle) {
            return Ok(());
        }
        let Ok(index) = usize::try_from(handle.index()) else {
            self.note_stale();
            return Err(StoreError::Stale);
        };
        let generation_matches = self
            .slots
            .get(index)
            .is_some_and(|slot| slot.generation == handle.generation() && slot.place.is_none());
        if !generation_matches {
            self.note_stale();
            return Err(StoreError::Stale);
        }
        self.free.remove(&handle.index());
        self.attach_new(handle)
    }

    fn attach_new(&mut self, handle: SceneHandle) -> Result<(), StoreError> {
        let row = self
            .archetypes
            .first()
            .map(|arch| arch.entities.len())
            .ok_or(StoreError::Unknown)?;
        let Some(arch) = self.archetypes.first_mut() else {
            return Err(StoreError::Unknown);
        };
        arch.entities.push(handle);
        self.bind_row(handle, 0, row)
    }

    fn drop_object(&mut self, handle: SceneHandle) -> Result<(), StoreError> {
        let (archetype, row) = self.place(handle)?;
        self.remove_row(archetype, row)?;
        let index = usize::try_from(handle.index()).map_err(|_err| StoreError::Stale)?;
        let Some(slot) = self.slots.get_mut(index) else {
            return Err(StoreError::Stale);
        };
        slot.place = None;
        slot.generation = next_generation(slot.generation);
        self.free.insert(handle.index());
        Ok(())
    }

    fn take_component<T: Component>(&mut self, handle: SceneHandle) -> Result<T, StoreError> {
        self.ensure_kind::<T>()?;
        let (from, row) = self.place(handle)?;
        let value = self
            .column::<T>(from)
            .and_then(|column| column.values.get(row).cloned())
            .ok_or(StoreError::Missing)?;
        let type_id = TypeId::of::<T>();
        let mut signature = self
            .archetypes
            .get(from)
            .map(|arch| arch.signature.clone())
            .ok_or(StoreError::Unknown)?;
        let Ok(pos) = signature.binary_search(&type_id) else {
            return Err(StoreError::Missing);
        };
        signature.remove(pos);
        let to = self.ensure_archetype(&signature)?;
        self.move_row::<T>(from, row, to, None)?;
        Ok(value)
    }

    /// Apply a tombstone. An absent object or component is already the end
    /// state, so this returns success and does not record a removal.
    fn apply_removal(&mut self, handle: SceneHandle, component: &str) -> Result<(), StoreError> {
        if !self.alive(handle) {
            return Ok(());
        }
        if component.is_empty() {
            return self.drop_object(handle);
        }
        match self.remove_named(handle, component) {
            Err(StoreError::Missing | StoreError::Unknown) => Ok(()),
            other => other,
        }
    }

    fn note_removal(&mut self, handle: SceneHandle, component: &'static str) {
        let tick = self.take_tick();
        self.removals.push(Removal {
            handle,
            component,
            tick,
        });
    }

    fn remove_named(&mut self, handle: SceneHandle, name: &str) -> Result<(), StoreError> {
        let remove = self
            .kinds
            .iter()
            .find(|kind| kind.name == name)
            .map(|kind| kind.remove)
            .ok_or(StoreError::Unknown)?;
        remove(self, handle)
    }

    fn ensure_archetype(&mut self, signature: &[TypeId]) -> Result<usize, StoreError> {
        if let Some(index) = self
            .archetypes
            .iter()
            .position(|arch| arch.signature == signature)
        {
            return Ok(index);
        }
        let mut makers = Vec::new();
        for type_id in signature {
            let kind = self
                .kinds
                .iter()
                .find(|kind| kind.type_id == *type_id)
                .ok_or(StoreError::Unknown)?;
            makers.push(kind.make);
        }
        let columns = makers.into_iter().map(|make| make()).collect();
        self.archetypes.push(Archetype {
            signature: signature.to_vec(),
            entities: Vec::new(),
            columns,
        });
        self.archetypes.len().checked_sub(1).ok_or(StoreError::Full)
    }

    fn overwrite<T: Component>(
        &mut self,
        archetype: usize,
        row: usize,
        value: T,
    ) -> Result<(), StoreError> {
        if self.column::<T>(archetype).is_none_or(|column| {
            column.values.get(row).is_none() || column.ticks.get(row).is_none()
        }) {
            return Err(StoreError::Missing);
        }
        let now = self.take_tick();
        let column = self.column_mut::<T>(archetype).ok_or(StoreError::Missing)?;
        let Some(slot) = column.values.get_mut(row) else {
            return Err(StoreError::Missing);
        };
        *slot = value;
        let Some(tick) = column.ticks.get_mut(row) else {
            return Err(StoreError::Missing);
        };
        *tick = now;
        Ok(())
    }

    fn move_row<T: Component>(
        &mut self,
        from: usize,
        row: usize,
        to: usize,
        fresh: Option<(usize, T, u32)>,
    ) -> Result<(), StoreError> {
        let handle = self.copy_row(from, row, to, fresh)?;
        self.remove_row(from, row)?;
        let dest_row = self
            .archetypes
            .get(to)
            .map(|arch| arch.entities.len())
            .and_then(|len| len.checked_sub(1))
            .ok_or(StoreError::Unknown)?;
        self.bind_row(handle, to, dest_row)
    }

    fn copy_row<T: Component>(
        &mut self,
        from: usize,
        row: usize,
        to: usize,
        fresh: Option<(usize, T, u32)>,
    ) -> Result<SceneHandle, StoreError> {
        let Some((source, dest)) = two(&mut self.archetypes, from, to) else {
            return Err(StoreError::Unknown);
        };
        let handle = source
            .entities
            .get(row)
            .copied()
            .ok_or(StoreError::Missing)?;
        let dest_signature = dest.signature.clone();
        let source_signature = source.signature.clone();
        let mut pushed = Vec::new();
        for (index, type_id) in dest_signature.iter().enumerate() {
            if fresh.as_ref().is_some_and(|(pos, _, _)| *pos == index) {
                if *type_id != TypeId::of::<T>() {
                    pop_rows(dest, &pushed);
                    return Err(StoreError::Unknown);
                }
                continue;
            }
            let src_index = match source_signature.binary_search(type_id) {
                Ok(src_index) => src_index,
                Err(_missing) => {
                    pop_rows(dest, &pushed);
                    return Err(StoreError::Unknown);
                }
            };
            let Some(source_column) = source.columns.get(src_index) else {
                pop_rows(dest, &pushed);
                return Err(StoreError::Unknown);
            };
            let copied = match dest.columns.get_mut(index) {
                Some(dest_column) => dest_column.push_clone_from(source_column.as_ref(), row),
                None => false,
            };
            if !copied {
                pop_rows(dest, &pushed);
                return Err(StoreError::Unknown);
            }
            pushed.push(index);
        }
        if let Some((pos, value, tick)) = fresh {
            let copied = match dest.columns.get_mut(pos) {
                Some(dest_column) => push_typed(dest_column.as_mut(), value, tick),
                None => false,
            };
            if !copied {
                pop_rows(dest, &pushed);
                return Err(StoreError::Unknown);
            }
        }
        dest.entities.push(handle);
        Ok(handle)
    }

    fn remove_row(&mut self, archetype: usize, row: usize) -> Result<(), StoreError> {
        let swapped = {
            let arch = self
                .archetypes
                .get_mut(archetype)
                .ok_or(StoreError::Unknown)?;
            let _removed = swap_remove_vec(&mut arch.entities, row).ok_or(StoreError::Missing)?;
            for column in &mut arch.columns {
                let _ = column.swap_remove(row);
            }
            arch.entities.get(row).copied()
        };
        if let Some(swapped) = swapped {
            self.bind_row(swapped, archetype, row)?;
        }
        Ok(())
    }

    fn bind_row(
        &mut self,
        handle: SceneHandle,
        archetype: usize,
        row: usize,
    ) -> Result<(), StoreError> {
        let row_u32 = u32::try_from(row).map_err(|_err| StoreError::Full)?;
        let arch_u32 = u32::try_from(archetype).map_err(|_err| StoreError::Full)?;
        let index = usize::try_from(handle.index()).map_err(|_err| StoreError::Stale)?;
        let Some(slot) = self.slots.get_mut(index) else {
            return Err(StoreError::Stale);
        };
        slot.generation = handle.generation();
        slot.place = Some(Place {
            archetype: arch_u32,
            row: row_u32,
        });
        Ok(())
    }

    fn column<T: Component>(&self, archetype: usize) -> Option<&TypedCol<T>> {
        let arch = self.archetypes.get(archetype)?;
        let index = arch.index_of(TypeId::of::<T>())?;
        arch.columns
            .get(index)?
            .as_any()
            .downcast_ref::<TypedCol<T>>()
    }

    fn column_mut<T: Component>(&mut self, archetype: usize) -> Option<&mut TypedCol<T>> {
        let arch = self.archetypes.get_mut(archetype)?;
        let index = arch.index_of(TypeId::of::<T>())?;
        arch.columns
            .get_mut(index)?
            .as_any_mut()
            .downcast_mut::<TypedCol<T>>()
    }
}

fn make_column<T: Component>() -> Box<dyn Column> {
    Box::new(TypedCol::<T> {
        name: T::component_name(),
        values: Vec::new(),
        ticks: Vec::new(),
    })
}

fn remove_component<T: Component>(
    world: &mut World,
    handle: SceneHandle,
) -> Result<(), StoreError> {
    world.take_component::<T>(handle).map(|_value| ())
}

fn apply_component<T: Component>(
    world: &mut World,
    handle: SceneHandle,
    bytes: &[u8],
) -> Result<(), StoreError> {
    match T::decode(bytes) {
        Ok(value) => {
            world.adopt_for_apply(handle)?;
            world.insert(handle, value)
        }
        Err(DecodeError::Truncated | DecodeError::Trailing) => Err(StoreError::Decode),
    }
}

fn push_typed<T: Component>(column: &mut dyn Column, value: T, tick: u32) -> bool {
    let Some(typed) = column.as_any_mut().downcast_mut::<TypedCol<T>>() else {
        return false;
    };
    typed.values.push(value);
    typed.ticks.push(tick);
    true
}

fn pair_columns<A: Component, B: Component>(
    arch: &Archetype,
) -> Option<(&TypedCol<A>, &TypedCol<B>)> {
    let left_at = arch.index_of(TypeId::of::<A>())?;
    let right_at = arch.index_of(TypeId::of::<B>())?;
    if left_at == right_at {
        return None;
    }
    if left_at < right_at {
        let (head, tail) = arch.columns.split_at(right_at);
        let left = head.get(left_at)?.as_any().downcast_ref()?;
        let right = tail.first()?.as_any().downcast_ref()?;
        Some((left, right))
    } else {
        let (head, tail) = arch.columns.split_at(left_at);
        let right = head.get(right_at)?.as_any().downcast_ref()?;
        let left = tail.first()?.as_any().downcast_ref()?;
        Some((left, right))
    }
}

fn two(
    archetypes: &mut [Archetype],
    from: usize,
    to: usize,
) -> Option<(&mut Archetype, &mut Archetype)> {
    if from == to || from >= archetypes.len() || to >= archetypes.len() {
        return None;
    }
    if from < to {
        let (head, tail) = archetypes.split_at_mut(to);
        Some((head.get_mut(from)?, tail.first_mut()?))
    } else {
        let (head, tail) = archetypes.split_at_mut(from);
        Some((tail.first_mut()?, head.get_mut(to)?))
    }
}

fn swap_remove_vec<T>(values: &mut Vec<T>, row: usize) -> Option<T> {
    let last = values.len().checked_sub(1)?;
    if row > last {
        return None;
    }
    if row != last {
        values.swap(row, last);
    }
    values.pop()
}

fn pop_rows(dest: &mut Archetype, pushed: &[usize]) {
    for index in pushed.iter().rev() {
        if let Some(column) = dest.columns.get_mut(*index) {
            let _ = column.pop_row();
        }
    }
}

const TICK_HALF: u32 = 1 << 31;

/// `true` when `tick` is the cursor or newer.
///
/// A cursor of `0` means every stored tick. Stored ticks start at 1, so this
/// is the "from the start" cursor and it does not use the half-range cut.
/// Any other cursor uses a wrapping compare: `tick - since` in the lower half
/// of `u32` is still ahead, which is how a tick of `1` follows `u32::MAX`.
fn changed_since(tick: u32, since: u32) -> bool {
    if since == 0 {
        return tick != 0;
    }
    tick.wrapping_sub(since) < TICK_HALF
}

/// The last tick a write stored. Clock `1` follows `u32::MAX`, because tick
/// `0` is never issued.
fn newest_issued(clock: u32) -> u32 {
    if clock == 1 {
        u32::MAX
    } else {
        clock.wrapping_sub(1)
    }
}

fn removal_expired(tick: u32, newest: u32) -> bool {
    newest.wrapping_sub(tick) >= TICK_HALF
}

fn take_tick(clock: &mut u32) -> u32 {
    if *clock == 0 {
        *clock = 1;
    }
    let tick = *clock;
    *clock = next_clock(tick);
    tick
}

fn next_generation(generation: u32) -> u32 {
    match generation.wrapping_add(1) {
        0 => 1,
        next => next,
    }
}

fn next_clock(clock: u32) -> u32 {
    match clock.wrapping_add(1) {
        0 => 1,
        next => next,
    }
}

fn as_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
#[expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]
mod tests {
    use super::*;
    use crate::{push_u32, read_u32, DecodeError};

    #[derive(Clone)]
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

    #[derive(Clone)]
    struct Tag(u32);

    impl Component for Tag {
        fn component_name() -> &'static str {
            "tag"
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

    #[test]
    fn a_tick_after_the_wrap_is_still_ahead_of_max() {
        let mut world = World::new();
        world.clock = u32::MAX;
        let cursor = world.clock();
        let handle = world.spawn_with(Health(1)).expect("spawn");
        let first = world.changes_since(cursor);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].tick, u32::MAX);
        let after = world.clock();
        assert_eq!(after, 1);
        world.insert(handle, Tag(4)).expect("edit");
        let next = world.changes_since(after);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].tick, 1);
        let across = world.changes_since(cursor);
        assert_eq!(across.len(), 2, "the write before the wrap was skipped");
        assert_eq!(across[0].tick, u32::MAX);
        assert_eq!(across[0].component, "health");
        assert_eq!(across[1].tick, 1);
        assert_eq!(across[1].component, "tag");
        let mut follower = World::new();
        follower.register::<Health>().expect("health");
        follower.register::<Tag>().expect("tag");
        let copied = follower.spawn().expect("spawn");
        assert_eq!(copied, handle);
        for delta in &across {
            follower.apply_delta(delta).expect("apply");
        }
        assert_eq!(follower.get::<Health>(copied).expect("health").0, 1);
        assert_eq!(follower.get::<Tag>(copied).expect("tag").0, 4);
    }

    #[test]
    fn a_follower_matches_after_a_remove_and_readd_across_the_wrap() {
        let mut source = World::new();
        let handle = source.spawn_with(Health(1)).expect("spawn");
        source.insert(handle, Tag(3)).expect("tag");
        source.clock = u32::MAX;
        let cursor = source.clock();
        source.remove::<Tag>(handle).expect("remove");
        source.insert(handle, Tag(9)).expect("readd");
        let deltas = source.changes_since(cursor);
        let tags: Vec<_> = deltas
            .iter()
            .filter(|delta| delta.component == "tag")
            .collect();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].tick, u32::MAX);
        assert_eq!(tags[0].bytes.len(), 0);
        assert_eq!(tags[1].tick, 1);
        assert_ne!(tags[1].bytes.len(), 0);

        let mut follower = World::new();
        follower.register::<Health>().expect("health");
        follower.register::<Tag>().expect("tag");
        let copied = follower.spawn_with(Health(1)).expect("spawn");
        follower.insert(copied, Tag(3)).expect("tag");
        assert_eq!(copied, handle);
        for delta in &deltas {
            follower.apply_delta(delta).expect("apply");
        }
        assert_eq!(
            follower.get::<Tag>(copied).expect("tag").0,
            source.get::<Tag>(handle).expect("tag").0
        );
        assert_eq!(
            follower.get::<Health>(copied).expect("health").0,
            source.get::<Health>(handle).expect("health").0
        );
    }

    #[test]
    fn a_pre_wrap_despawn_stays_ahead_of_the_post_wrap_record() {
        let mut source = World::new();
        let old = source.spawn_with(Health(1)).expect("spawn");
        source.clock = u32::MAX;
        let cursor = source.clock();
        source.despawn(old).expect("despawn");
        let renewed = source.spawn_with(Health(7)).expect("reuse");
        let deltas = source.changes_since(cursor);
        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[0].handle, old);
        assert_eq!(deltas[0].tick, u32::MAX);
        assert_eq!(deltas[0].bytes.len(), 0);
        assert_eq!(deltas[1].handle, renewed);
        assert_eq!(deltas[1].tick, 1);
        assert_ne!(deltas[1].bytes.len(), 0);

        let mut follower = World::new();
        follower.register::<Health>().expect("health");
        let seeded = follower.spawn_with(Health(1)).expect("seed");
        assert_eq!(seeded, old);
        let rejects = follower.stats().stale_rejects;
        for delta in &deltas {
            follower.apply_delta(delta).expect("apply");
        }
        assert_eq!(follower.stats().stale_rejects, rejects);
        assert!(!follower.alive(old));
        assert_eq!(follower.get::<Health>(renewed).expect("health").0, 7);
    }

    #[test]
    fn an_old_removal_is_not_reemitted_after_the_half_range() {
        let mut world = World::new();
        let handle = world.spawn_with(Health(1)).expect("spawn");
        world.despawn(handle).expect("despawn");
        let tick = world.changes_since(0)[0].tick;
        world.clock = tick.wrapping_add(TICK_HALF).wrapping_add(1);
        assert_eq!(world.changes_since(0).len(), 0);
        let fresh = world.spawn_with(Health(3)).expect("later");
        let deltas = world.changes_since(0);
        assert!(deltas.iter().all(|delta| delta.handle == fresh));
        assert!(deltas.iter().all(|delta| !delta.bytes.is_empty()));
    }

    #[test]
    fn copy_row_rolls_back_a_failed_push() {
        let mut world = World::new();
        let handle = world.spawn_with(Health(1)).expect("spawn");
        world.insert(handle, Tag(2)).expect("tag");
        let (from, row) = world.find(handle).expect("place");
        world.archetypes.push(Archetype {
            signature: world.archetypes[from].signature.clone(),
            entities: Vec::new(),
            columns: vec![
                Box::new(TypedCol::<Health> {
                    name: "health",
                    values: Vec::new(),
                    ticks: Vec::new(),
                }),
                Box::new(TypedCol::<Health> {
                    name: "tag",
                    values: Vec::new(),
                    ticks: Vec::new(),
                }),
            ],
        });
        let to = world.archetypes.len() - 1;
        let err = world.copy_row::<Tag>(from, row, to, None);
        assert!(err.is_err());
        assert_eq!(world.archetypes[to].columns[0].len(), 0);
        assert_eq!(world.archetypes[to].entities.len(), 0);
        assert_eq!(world.get::<Health>(handle).expect("health").0, 1);
        assert_eq!(world.get::<Tag>(handle).expect("tag").0, 2);
    }
}
