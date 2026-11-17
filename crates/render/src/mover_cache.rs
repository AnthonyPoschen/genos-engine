//! Object-attached lighting cache for movers.
//!
//! Separate fixed pool of small (~64 B) texels, not full world-hash slots.
//! A mover table maps object id → entry in O(1). Lighting is stored in world
//! directions so a spinning mover stays correct (GIBS §1.5 stores object-local
//! position + transform id; departure: world-direction SH, movers only).
//!
//! Design: `docs/systems/lighting/mover-attach-cache.md`.

#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

/// Bytes per mover texel (object-local pos + id + world-direction SH-L1).
pub const MOVER_TEXEL_BYTES: usize = 64;

/// High-preset pool size (~2 MiB at 64 B).
pub const MOVER_POOL_HIGH: u32 = 32_768;
/// Medium-preset pool size.
pub const MOVER_POOL_MEDIUM: u32 = 16_384;
/// Low-preset pool size.
pub const MOVER_POOL_LOW: u32 = 4_096;

/// Near tier: up to this many texels when the pool has room.
pub const MOVER_TIER_NEAR: u32 = 384;
/// Mid tier.
pub const MOVER_TIER_MID: u32 = 24;
/// Far tier writes nothing; the mover reads world cells.
pub const MOVER_TIER_FAR: u32 = 0;

/// Screen-projected radius above which a mover is Near.
pub const MOVER_NEAR_SCREEN: f32 = 0.08;
/// Below this is Far; between Mid and Near is Mid.
pub const MOVER_MID_SCREEN: f32 = 0.02;
/// Hysteresis band so tiers do not chatter.
pub const MOVER_TIER_HYSTERESIS: f32 = 0.015;

/// One mover texel: object-local position, object id, world-direction SH-L1.
///
/// Layout is fixed at [`MOVER_TEXEL_BYTES`]. Lighting lobes are world-space so
/// a spin does not require rotating stored irradiance.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoverTexel {
    /// Object-local position (GIBS §1.5).
    pub local_pos: [f32; 3],
    /// Transform / object id (1-based; 0 is free). Skinned: top-weight bone id.
    pub object_id: u32,
    /// SH-L1 irradiance in world directions: L0 RGB, then L1x/y/z RGB.
    pub sh: [[f32; 3]; 4],
}

const _: () = assert!(std::mem::size_of::<MoverTexel>() == MOVER_TEXEL_BYTES);

impl MoverTexel {
    pub const EMPTY: Self = Self {
        local_pos: [0.0; 3],
        object_id: 0,
        sh: [[0.0; 3]; 4],
    };

    pub fn is_live(&self) -> bool {
        self.object_id != 0
    }
}

/// Distance / screen-size tier for a mover's texel share.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MoverTier {
    /// Up to [`MOVER_TIER_NEAR`] texels.
    Near,
    /// Up to [`MOVER_TIER_MID`] texels.
    Mid,
    /// No texels; reads world cells, never writes them.
    Far,
}

impl MoverTier {
    pub fn texel_cap(self) -> u32 {
        match self {
            Self::Near => MOVER_TIER_NEAR,
            Self::Mid => MOVER_TIER_MID,
            Self::Far => MOVER_TIER_FAR,
        }
    }

    /// Pick a tier from screen-projected size (radius / distance).
    pub fn from_screen(screen: f32) -> Self {
        if !screen.is_finite() || screen < MOVER_MID_SCREEN {
            Self::Far
        } else if screen >= MOVER_NEAR_SCREEN {
            Self::Near
        } else {
            Self::Mid
        }
    }

    /// Apply hysteresis against the previous tier.
    pub fn with_hysteresis(prev: Self, screen: f32) -> Self {
        if !screen.is_finite() {
            return Self::Far;
        }
        let h = MOVER_TIER_HYSTERESIS;
        match prev {
            Self::Near => {
                if screen >= MOVER_NEAR_SCREEN - h {
                    Self::Near
                } else if screen >= MOVER_MID_SCREEN {
                    Self::Mid
                } else {
                    Self::Far
                }
            }
            Self::Mid => {
                if screen >= MOVER_NEAR_SCREEN + h {
                    Self::Near
                } else if screen >= MOVER_MID_SCREEN - h {
                    Self::Mid
                } else {
                    Self::Far
                }
            }
            Self::Far => {
                if screen >= MOVER_NEAR_SCREEN {
                    Self::Near
                } else if screen >= MOVER_MID_SCREEN + h {
                    Self::Mid
                } else {
                    Self::Far
                }
            }
        }
    }
}

/// One live mover in the table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoverEntry {
    pub object_id: u32,
    pub tier: MoverTier,
    /// First texel index in the pool (inclusive).
    pub texel_base: u32,
    /// Allocated texel count (may be below the tier cap when the pool is tight).
    pub texel_count: u32,
    /// Bounding sphere centre (world).
    pub centre: [f32; 3],
    /// Bounding sphere radius (world metres).
    pub radius: f32,
}

impl MoverEntry {
    pub const EMPTY: Self = Self {
        object_id: 0,
        tier: MoverTier::Far,
        texel_base: 0,
        texel_count: 0,
        centre: [0.0; 3],
        radius: 0.0,
    };

    pub fn is_live(&self) -> bool {
        self.object_id != 0
    }
}

#[derive(Clone, Copy, Debug)]
struct FreeSpan {
    base: u32,
    count: u32,
}

/// Empty open-address slot (probe may stop).
const SLOT_EMPTY: u32 = u32::MAX;
/// Tombstone: deleted; probe continues so later entries stay reachable.
const SLOT_TOMB: u32 = u32::MAX - 1;

/// Fixed pool of mover texels plus an open-addressed mover table.
///
/// Lookup by object id is O(1). Dropping a mover frees its texels immediately
/// (no orphaned texels after the keep TTL).
pub struct MoverCache {
    texels: Vec<MoverTexel>,
    /// Contiguous free spans, coalesced.
    free: Vec<FreeSpan>,
    /// Open-addressed slots; empty when `object_id == 0` and slot is [`SLOT_EMPTY`].
    table: Vec<MoverEntry>,
    /// Occupancy: [`SLOT_EMPTY`], [`SLOT_TOMB`], or the slot index when live.
    slots: Vec<u32>,
    capacity: u32,
    live_movers: u32,
    live_texels: u32,
}

impl MoverCache {
    /// Build a pool of `capacity` texels. `0` yields an empty disabled cache.
    pub fn new(capacity: u32) -> Self {
        if capacity == 0 {
            return Self {
                texels: Vec::new(),
                free: Vec::new(),
                table: Vec::new(),
                slots: Vec::new(),
                capacity: 0,
                live_movers: 0,
                live_texels: 0,
            };
        }
        let n = capacity as usize;
        let table_slots = n.next_power_of_two().max(16);
        Self {
            texels: vec![MoverTexel::EMPTY; n],
            free: vec![FreeSpan {
                base: 0,
                count: capacity,
            }],
            table: vec![MoverEntry::EMPTY; table_slots],
            slots: vec![SLOT_EMPTY; table_slots],
            capacity,
            live_movers: 0,
            live_texels: 0,
        }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn live_movers(&self) -> u32 {
        self.live_movers
    }

    pub fn live_texels(&self) -> u32 {
        self.live_texels
    }

    pub fn free_texels(&self) -> u32 {
        self.free.iter().map(|s| s.count).sum()
    }

    pub fn texels(&self) -> &[MoverTexel] {
        &self.texels
    }

    pub fn texels_mut(&mut self) -> &mut [MoverTexel] {
        &mut self.texels
    }

    fn hash_slot(&self, object_id: u32) -> usize {
        let mask = self.table.len() - 1;
        let h = object_id.wrapping_mul(0x9e37_79b9);
        (h as usize) & mask
    }

    /// O(1) expected lookup by object id.
    pub fn lookup(&self, object_id: u32) -> Option<&MoverEntry> {
        if object_id == 0 || self.table.is_empty() {
            return None;
        }
        let mask = self.table.len() - 1;
        let mut i = self.hash_slot(object_id);
        for _ in 0..self.table.len() {
            let e = &self.table[i];
            if e.object_id == object_id {
                return Some(e);
            }
            if self.slots[i] == SLOT_EMPTY {
                return None;
            }
            i = (i + 1) & mask;
        }
        None
    }

    fn lookup_slot(&self, object_id: u32) -> Option<usize> {
        if object_id == 0 || self.table.is_empty() {
            return None;
        }
        let mask = self.table.len() - 1;
        let mut i = self.hash_slot(object_id);
        for _ in 0..self.table.len() {
            let e = &self.table[i];
            if e.object_id == object_id {
                return Some(i);
            }
            if self.slots[i] == SLOT_EMPTY {
                return None;
            }
            i = (i + 1) & mask;
        }
        None
    }

    fn find_empty_slot(&self, object_id: u32) -> Option<usize> {
        if self.table.is_empty() {
            return None;
        }
        let mask = self.table.len() - 1;
        let mut i = self.hash_slot(object_id);
        let mut tomb = None;
        for _ in 0..self.table.len() {
            match self.slots[i] {
                SLOT_EMPTY => return Some(tomb.unwrap_or(i)),
                SLOT_TOMB => {
                    if tomb.is_none() {
                        tomb = Some(i);
                    }
                }
                _ => {}
            }
            i = (i + 1) & mask;
        }
        tomb
    }

    /// Fair share when the pool is crowded: each mover gets at most
    /// `capacity / live_count`, floored, but never above its tier cap.
    pub fn share_for(&self, tier: MoverTier, live_count: u32) -> u32 {
        let cap = tier.texel_cap();
        if cap == 0 || live_count == 0 || self.capacity == 0 {
            return 0;
        }
        let fair = (self.capacity / live_count.max(1)).max(1);
        cap.min(fair)
    }

    fn alloc_span(&mut self, count: u32, object_id: u32) -> Option<(u32, u32)> {
        if count == 0 {
            return Some((0, 0));
        }
        let pos = self.free.iter().position(|s| s.count >= count)?;
        let span = self.free[pos];
        let base = span.base;
        if span.count == count {
            self.free.remove(pos);
        } else {
            self.free[pos] = FreeSpan {
                base: span.base + count,
                count: span.count - count,
            };
        }
        for i in 0..count {
            self.texels[(base + i) as usize] = MoverTexel {
                local_pos: [0.0; 3],
                object_id,
                sh: [[0.0; 3]; 4],
            };
        }
        self.live_texels += count;
        Some((base, count))
    }

    fn free_span(&mut self, base: u32, count: u32) {
        if count == 0 {
            return;
        }
        for i in 0..count {
            let idx = base + i;
            if (idx as usize) < self.texels.len() {
                self.texels[idx as usize] = MoverTexel::EMPTY;
            }
        }
        self.live_texels = self.live_texels.saturating_sub(count);
        // Insert and coalesce.
        let mut i = 0;
        while i < self.free.len() && self.free[i].base < base {
            i += 1;
        }
        self.free.insert(
            i,
            FreeSpan {
                base,
                count,
            },
        );
        // Coalesce with next.
        if i + 1 < self.free.len() {
            let next = self.free[i + 1];
            let cur = self.free[i];
            if cur.base + cur.count == next.base {
                self.free[i].count += next.count;
                self.free.remove(i + 1);
            }
        }
        // Coalesce with previous.
        if i > 0 {
            let prev = self.free[i - 1];
            let cur = self.free[i];
            if prev.base + prev.count == cur.base {
                self.free[i - 1].count += cur.count;
                self.free.remove(i);
            }
        }
    }

    fn alloc_with_shrink(&mut self, want: u32, object_id: u32) -> (u32, u32) {
        let mut try_n = want;
        loop {
            if let Some(v) = self.alloc_span(try_n, object_id) {
                return v;
            }
            if try_n == 0 {
                return (0, 0);
            }
            try_n /= 2;
        }
    }

    /// Insert or refresh a mover. Allocates texels for its tier share.
    pub fn upsert(
        &mut self,
        object_id: u32,
        screen: f32,
        centre: [f32; 3],
        radius: f32,
    ) -> Option<MoverEntry> {
        if object_id == 0 || self.capacity == 0 {
            return None;
        }
        let prev_tier = self
            .lookup(object_id)
            .map(|e| e.tier)
            .unwrap_or(MoverTier::Far);
        let tier = MoverTier::with_hysteresis(prev_tier, screen);
        let live_after = if self.lookup(object_id).is_some() {
            self.live_movers.max(1)
        } else {
            self.live_movers + 1
        };
        let want = self.share_for(tier, live_after);

        if let Some(slot) = self.lookup_slot(object_id) {
            let old = self.table[slot];
            if old.texel_count != want {
                self.free_span(old.texel_base, old.texel_count);
                let (base, count) = self.alloc_with_shrink(want, object_id);
                self.table[slot].texel_base = base;
                self.table[slot].texel_count = count;
            }
            self.table[slot].tier = tier;
            self.table[slot].centre = centre;
            self.table[slot].radius = radius;
            return Some(self.table[slot]);
        }

        let slot = self.find_empty_slot(object_id)?;
        let (base, count) = self.alloc_with_shrink(want, object_id);
        self.table[slot] = MoverEntry {
            object_id,
            tier,
            texel_base: base,
            texel_count: count,
            centre,
            radius,
        };
        self.slots[slot] = slot as u32;
        self.live_movers += 1;
        Some(self.table[slot])
    }

    /// Drop a mover and free its texels immediately.
    pub fn remove(&mut self, object_id: u32) -> bool {
        let Some(slot) = self.lookup_slot(object_id) else {
            return false;
        };
        let old = self.table[slot];
        self.free_span(old.texel_base, old.texel_count);
        self.table[slot] = MoverEntry::EMPTY;
        self.slots[slot] = SLOT_TOMB;
        self.live_movers = self.live_movers.saturating_sub(1);
        true
    }

    /// Rebalance every live mover to the fair share for `live_movers`.
    /// Call after a burst of inserts so crowded scenes converge to ~equal shares.
    pub fn rebalance(&mut self) {
        if self.live_movers == 0 {
            return;
        }
        let live = self.live_movers;
        let ids: Vec<(usize, u32, MoverTier, [f32; 3], f32)> = self
            .table
            .iter()
            .enumerate()
            .filter(|(_, e)| e.is_live())
            .map(|(slot, e)| (slot, e.object_id, e.tier, e.centre, e.radius))
            .collect();
        // Free every span first so later movers are not starved by early large
        // allocations, then hand out the fair share.
        for &(slot, _, _, _, _) in &ids {
            let old = self.table[slot];
            self.free_span(old.texel_base, old.texel_count);
            self.table[slot].texel_base = 0;
            self.table[slot].texel_count = 0;
        }
        for &(slot, id, tier, centre, radius) in &ids {
            let want = self.share_for(tier, live);
            let (base, count) = self.alloc_with_shrink(want, id);
            self.table[slot].texel_base = base;
            self.table[slot].texel_count = count;
            self.table[slot].centre = centre;
            self.table[slot].radius = radius;
        }
    }

    /// World cells within 1 m of the mover's swept sphere get a priority boost
    /// when the mover is large enough at its distance to matter.
    pub fn dirties_world(entry: &MoverEntry) -> bool {
        entry.is_live() && entry.tier != MoverTier::Far && entry.radius > 0.05
    }

    /// Brick half-extent (metres) around the swept bounds for the world-cell boost.
    pub fn dirty_brick_metres(entry: &MoverEntry) -> f32 {
        entry.radius + 1.0
    }
}

/// Preset → default mover texel pool size.
pub fn mover_pool_for_preset(preset: genos_scene::LightingPreset) -> u32 {
    use genos_scene::LightingPreset::{DayCycle, Gi1Bounce, High, Low, Medium, Potato};
    match preset {
        High => MOVER_POOL_HIGH,
        Medium => MOVER_POOL_MEDIUM,
        Low | Gi1Bounce => MOVER_POOL_LOW,
        Potato | DayCycle => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texel_is_sixty_four_bytes() {
        assert_eq!(std::mem::size_of::<MoverTexel>(), 64);
        assert_eq!(MOVER_TEXEL_BYTES, 64);
    }

    #[test]
    fn lookup_is_o1_by_object_id() {
        let mut cache = MoverCache::new(256);
        assert!(cache.lookup(7).is_none());
        let e = cache
            .upsert(7, 0.2, [1.0, 0.0, 0.0], 0.5)
            .expect("insert");
        assert_eq!(e.object_id, 7);
        assert_eq!(e.tier, MoverTier::Near);
        assert!(e.texel_count > 0);
        assert_eq!(cache.lookup(7).map(|x| x.object_id), Some(7));
        assert!(cache.lookup(8).is_none());
    }

    #[test]
    fn pool_holds_high_capacity() {
        let cache = MoverCache::new(MOVER_POOL_HIGH);
        assert_eq!(cache.capacity(), MOVER_POOL_HIGH);
        assert_eq!(cache.free_texels(), MOVER_POOL_HIGH);
        assert_eq!(
            u64::from(MOVER_POOL_HIGH) * MOVER_TEXEL_BYTES as u64,
            2 * 1024 * 1024
        );
    }

    #[test]
    fn far_tier_gets_no_texels() {
        let mut cache = MoverCache::new(256);
        let e = cache
            .upsert(3, 0.001, [0.0; 3], 0.2)
            .expect("insert");
        assert_eq!(e.tier, MoverTier::Far);
        assert_eq!(e.texel_count, 0);
        assert_eq!(cache.live_texels(), 0);
    }

    #[test]
    fn remove_frees_texels_immediately() {
        let mut cache = MoverCache::new(512);
        let e = cache
            .upsert(11, 0.2, [0.0; 3], 0.4)
            .expect("insert");
        let n = e.texel_count;
        assert!(n > 0);
        assert_eq!(cache.live_texels(), n);
        assert!(cache.remove(11));
        assert!(cache.lookup(11).is_none());
        assert_eq!(cache.live_texels(), 0);
        assert_eq!(cache.free_texels(), 512);
        assert!(!cache.remove(11));
    }

    #[test]
    fn crowded_pool_shrinks_each_share() {
        let mut cache = MoverCache::new(1000);
        for id in 1..=1000 {
            cache
                .upsert(id, 0.2, [0.0; 3], 0.3)
                .expect("insert");
        }
        cache.rebalance();
        assert_eq!(cache.live_movers(), 1000);
        let e = cache.lookup(1).expect("present");
        // Fair share of 1000 texels across 1000 Near movers is 1 each.
        assert!(
            e.texel_count <= 32,
            "crowded share was {}",
            e.texel_count
        );
        assert_eq!(e.texel_count, 1);
        assert_eq!(cache.live_texels(), 1000);
        assert_eq!(cache.free_texels(), 0);
    }

    #[test]
    fn hysteresis_holds_near_across_band() {
        let t = MoverTier::with_hysteresis(MoverTier::Near, MOVER_NEAR_SCREEN - 0.01);
        assert_eq!(t, MoverTier::Near);
        let t = MoverTier::with_hysteresis(MoverTier::Near, MOVER_MID_SCREEN + 0.001);
        assert_eq!(t, MoverTier::Mid);
    }

    #[test]
    fn scales_past_sixteen_movers() {
        let mut cache = MoverCache::new(4096);
        for id in 1..=64 {
            cache
                .upsert(id, 0.05, [0.0; 3], 0.25)
                .expect("insert");
        }
        assert_eq!(cache.live_movers(), 64);
        for id in 1..=64 {
            assert!(cache.lookup(id).is_some());
        }
    }

    #[test]
    fn mid_tier_caps_at_twenty_four() {
        let mut cache = MoverCache::new(4096);
        let e = cache
            .upsert(1, 0.05, [0.0; 3], 0.4)
            .expect("insert");
        assert_eq!(e.tier, MoverTier::Mid);
        assert_eq!(e.texel_count, MOVER_TIER_MID);
    }
}
