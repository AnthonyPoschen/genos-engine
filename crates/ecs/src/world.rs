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
/// The bytes are [`Component::encode`] of that component only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentDelta {
    /// Object that changed.
    pub handle: SceneHandle,
    /// [`Component::component_name`].
    pub component: &'static str,
    /// Tick stored on the cell. Later writes have a greater tick until the clock wraps.
    pub tick: u32,
    /// Canonical component bytes.
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
    pub clock: u32,
    /// Writes refused because the handle was stale.
    pub stale_rejects: u32,
}

/// Exclusive access to one component. Dropping it after a write stores the tick.
pub struct Mut<'a, T> {
    value: &'a mut T,
    tick: &'a mut u32,
    now: u32,
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
            *self.tick = self.now;
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
            if *tick > since {
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
}

/// The scene store.
///
/// Objects with the same component set share one table. Columns of that table
/// are contiguous. A write stores the store clock on that cell and advances
/// the clock. [`Self::changes_since`] returns those cells in handle-index
/// order, then by component name.
pub struct World {
    slots: Vec<Slot>,
    free: BTreeSet<u32>,
    archetypes: Vec<Archetype>,
    kinds: Vec<Kind>,
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
            clock: 1,
            stale_rejects: 0,
        }
    }

    /// Record `T` so [`Self::apply_delta`] can rebuild it by name.
    ///
    /// [`Self::insert`] registers the type as well. Call this on a receiver
    /// that has not inserted `T` yet.
    pub fn register<T: Component>(&mut self) {
        self.ensure_kind::<T>();
    }

    /// The tick the next write will store.
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
        let row = self
            .archetypes
            .first()
            .map_or(0, |arch| arch.entities.len());
        let Some(arch) = self.archetypes.first_mut() else {
            return Err(StoreError::Unknown);
        };
        arch.entities.push(handle);
        self.bind_row(handle, 0, row)?;
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
    /// A read through the guard does not. Forgetting the guard (instead of
    /// dropping it) skips the tick.
    ///
    /// # Errors
    ///
    /// [`StoreError::Stale`] when the handle is not live.
    /// [`StoreError::Missing`] when the object has no `T`.
    pub fn get_mut<T: Component>(&mut self, handle: SceneHandle) -> Result<Mut<'_, T>, StoreError> {
        let (archetype, row) = self.place(handle)?;
        if self.column::<T>(archetype).is_none() {
            return Err(StoreError::Missing);
        }
        let now = self.clock;
        self.clock = next_clock(now);
        let column = self.column_mut::<T>(archetype).ok_or(StoreError::Missing)?;
        let TypedCol { values, ticks, .. } = column;
        let Some(value) = values.get_mut(row) else {
            return Err(StoreError::Missing);
        };
        let Some(tick) = ticks.get_mut(row) else {
            return Err(StoreError::Missing);
        };
        Ok(Mut {
            value,
            tick,
            now,
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
        self.ensure_kind::<T>();
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
        let now = self.clock;
        self.clock = next_clock(now);
        self.move_row(from, row, to, Some((pos, value, now)))
    }

    /// Take `T` off the object. The object stays alive.
    ///
    /// # Errors
    ///
    /// [`StoreError::Stale`] when the handle is not live.
    /// [`StoreError::Missing`] when the object has no `T`.
    pub fn remove<T: Component>(&mut self, handle: SceneHandle) -> Result<T, StoreError> {
        self.ensure_kind::<T>();
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

    /// Components written after `tick`, sorted by handle index then name.
    ///
    /// A tick of `0` returns every write, because stored ticks start at 1.
    #[must_use]
    pub fn changes_since(&self, tick: u32) -> Vec<ComponentDelta> {
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
        out.sort_by(|left, right| {
            (left.handle.index(), left.component, left.tick).cmp(&(
                right.handle.index(),
                right.component,
                right.tick,
            ))
        });
        out
    }

    /// Write one delta onto the live object.
    ///
    /// The receiver must have [`Self::register`]ed that component name.
    ///
    /// # Errors
    ///
    /// [`StoreError::Unknown`] when the name was not registered.
    /// [`StoreError::Decode`] when the bytes are not that component.
    /// [`StoreError::Stale`] when the handle is not live.
    pub fn apply_delta(&mut self, delta: &ComponentDelta) -> Result<(), StoreError> {
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
            return Ok(SceneHandle::new(index, generation));
        }
        let index = u32::try_from(self.slots.len()).map_err(|_err| StoreError::Full)?;
        self.slots.push(Slot {
            generation: 1,
            place: None,
        });
        Ok(SceneHandle::new(index, 1))
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

    fn ensure_kind<T: Component>(&mut self) {
        let type_id = TypeId::of::<T>();
        if self.kinds.iter().any(|kind| kind.type_id == type_id) {
            return;
        }
        self.kinds.push(Kind {
            type_id,
            name: T::component_name(),
            make: make_column::<T>,
            apply: apply_component::<T>,
        });
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
        let now = self.clock;
        self.clock = next_clock(now);
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
        for (index, type_id) in dest_signature.iter().enumerate() {
            if fresh.as_ref().is_some_and(|(pos, _, _)| *pos == index) {
                if *type_id != TypeId::of::<T>() {
                    return Err(StoreError::Unknown);
                }
                continue;
            }
            let src_index = match source_signature.binary_search(type_id) {
                Ok(src_index) => src_index,
                Err(_missing) => return Err(StoreError::Unknown),
            };
            let source_column = source.columns.get(src_index).ok_or(StoreError::Unknown)?;
            let dest_column = dest.columns.get_mut(index).ok_or(StoreError::Unknown)?;
            if !dest_column.push_clone_from(source_column.as_ref(), row) {
                return Err(StoreError::Unknown);
            }
        }
        if let Some((pos, value, tick)) = fresh {
            let dest_column = dest.columns.get_mut(pos).ok_or(StoreError::Unknown)?;
            if !push_typed(dest_column.as_mut(), value, tick) {
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

fn apply_component<T: Component>(
    world: &mut World,
    handle: SceneHandle,
    bytes: &[u8],
) -> Result<(), StoreError> {
    match T::decode(bytes) {
        Ok(value) => world.insert(handle, value),
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
