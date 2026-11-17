//! Object-attached mover lighting cache (GIBS §1.5 style).
//!
//! A fixed pool of small (~64 B) texels sits beside the fat world hash table.
//! Each texel stores an object-local position and the object/transform id, and
//! keeps lighting in world directions so a spinning mover stays correct. An
//! open-address mover table maps object id to a record in O(1). Compose reads
//! these texels for mover pixels; world cells cover everything else.
//!
//! Stage A ships the pool, the table, and lookup. Allocation tiers, the
//! scheduler, and compose wiring land in later stages.

#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

/// Bytes per mover texel on the GPU path (fp16 SH + packed normal).
/// The CPU reference keeps full floats for tests; this is the GPU budget.
pub const MOVER_TEXEL_BYTES: usize = 64;

/// High-preset pool: 32 768 × 64 B ≈ 2 MiB.
pub const MOVER_TEXELS_HIGH: u32 = 32_768;

/// Medium-preset pool.
pub const MOVER_TEXELS_MEDIUM: u32 = 16_384;

/// Low-preset pool (still attached, just smaller).
pub const MOVER_TEXELS_LOW: u32 = 4_096;

/// Mover table capacity. Open-address slots; 1000+ live movers fit with headroom.
pub const MOVER_TABLE_SLOTS: u32 = 4_096;

/// Empty object id. Zero is never a live mover id (host packs id + 1).
pub const MOVER_ID_NONE: u32 = 0;

/// One ~64 B mover texel. Lighting is stored in world directions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct MoverTexel {
    /// Object-local position (G-buffer spawn; skinned: top-weight bone frame).
    pub local_pos: [f32; 3],
    /// Transform / object id (same convention as the G-buffer id + 1).
    pub object_id: u32,
    /// Irradiance mean in world directions (not object-local).
    pub mean: [f32; 3],
    /// Accepted sample sets.
    pub samples: f32,
    /// World-frame SH band-1 RGB (x, y, z axes), matching the world-cell tilt.
    pub sh1: [[f32; 3]; 3],
    /// Object-local unit normal (layout / self-shadow; shared per mesh later).
    pub local_n: [f32; 3],
    /// Free-list link when the texel is unused; otherwise 0.
    pub next_free: u32,
}

impl MoverTexel {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn is_live(&self) -> bool {
        self.object_id != MOVER_ID_NONE
    }
}

/// Distance / screen-size tier for a mover's texel share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MoverTier {
    /// Near: up to [`MOVER_TIER_NEAR_TEXELS`] texels.
    Near = 0,
    /// Mid: up to [`MOVER_TIER_MID_TEXELS`] texels.
    Mid = 1,
    /// Far: no texels; reads world cells, never writes them.
    Far = 2,
}

/// Near-tier texel budget before pool pressure.
pub const MOVER_TIER_NEAR_TEXELS: u32 = 384;
/// Mid-tier texel budget before pool pressure.
pub const MOVER_TIER_MID_TEXELS: u32 = 24;

impl MoverTier {
    #[must_use]
    pub fn texel_budget(self) -> u32 {
        match self {
            Self::Near => MOVER_TIER_NEAR_TEXELS,
            Self::Mid => MOVER_TIER_MID_TEXELS,
            Self::Far => 0,
        }
    }
}

/// One live mover in the O(1) table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoverRecord {
    pub object_id: u32,
    /// World-space bounding sphere (centre xyz, radius).
    pub sphere: [f32; 4],
    /// World centre (xyz) and unused w.
    pub centre: [f32; 3],
    /// Half extents (box) or radius on xz for rounds.
    pub half_extents: [f32; 3],
    pub yaw_cos: f32,
    pub yaw_sin: f32,
    /// First pool index owned by this mover, or [`u32::MAX`] when none.
    pub texel_base: u32,
    /// Contiguous texels owned (0 for Far or unallocated).
    pub texel_count: u32,
    pub tier: MoverTier,
}

impl MoverRecord {
    #[must_use]
    pub fn vacant() -> Self {
        Self {
            object_id: MOVER_ID_NONE,
            sphere: [0.0; 4],
            centre: [0.0; 3],
            half_extents: [0.0; 3],
            yaw_cos: 1.0,
            yaw_sin: 0.0,
            texel_base: u32::MAX,
            texel_count: 0,
            tier: MoverTier::Far,
        }
    }

    #[must_use]
    pub fn is_live(&self) -> bool {
        self.object_id != MOVER_ID_NONE
    }
}

/// Fixed mover texel pool plus open-address table keyed by object id.
#[derive(Clone, Debug)]
pub struct MoverCache {
    pool_cap: u32,
    texels: Vec<MoverTexel>,
    /// Free-stack top; `u32::MAX` when empty.
    free_top: u32,
    free_count: u32,
    table: Vec<MoverRecord>,
    live_movers: u32,
}

impl MoverCache {
    /// Builds a pool of `pool_cap` texels and a [`MOVER_TABLE_SLOTS`] table.
    #[must_use]
    pub fn new(pool_cap: u32) -> Self {
        let cap = pool_cap.max(1);
        let mut texels = vec![MoverTexel::empty(); usize_from_u32(cap)];
        // Free stack: each unused texel points at the next; top is 0.
        for i in 0..cap {
            let next = if i + 1 < cap { i + 1 } else { u32::MAX };
            texels[usize_from_u32(i)].next_free = next;
        }
        Self {
            pool_cap: cap,
            texels,
            free_top: 0,
            free_count: cap,
            table: vec![MoverRecord::vacant(); usize_from_u32(MOVER_TABLE_SLOTS)],
            live_movers: 0,
        }
    }

    /// High-preset defaults.
    #[must_use]
    pub fn high() -> Self {
        Self::new(MOVER_TEXELS_HIGH)
    }

    #[must_use]
    pub fn pool_cap(&self) -> u32 {
        self.pool_cap
    }

    #[must_use]
    pub fn free_count(&self) -> u32 {
        self.free_count
    }

    #[must_use]
    pub fn live_movers(&self) -> u32 {
        self.live_movers
    }

    #[must_use]
    pub fn used_texels(&self) -> u32 {
        self.pool_cap.saturating_sub(self.free_count)
    }

    /// O(1) open-address lookup by object id. Returns the table slot.
    #[must_use]
    pub fn find_slot(&self, object_id: u32) -> Option<u32> {
        if object_id == MOVER_ID_NONE {
            return None;
        }
        let start = table_hash(object_id);
        for step in 0..MOVER_TABLE_SLOTS {
            let slot = (start + step) & (MOVER_TABLE_SLOTS - 1);
            let rec = &self.table[usize_from_u32(slot)];
            if rec.object_id == object_id {
                return Some(slot);
            }
            if rec.object_id == MOVER_ID_NONE {
                return None;
            }
        }
        None
    }

    /// Returns the live record for `object_id`, if present.
    #[must_use]
    pub fn get(&self, object_id: u32) -> Option<&MoverRecord> {
        self.find_slot(object_id)
            .map(|slot| &self.table[usize_from_u32(slot)])
    }

    /// Inserts or refreshes a mover. Does not allocate texels (Stage B).
    pub fn upsert(&mut self, mut record: MoverRecord) -> Result<u32, MoverCacheError> {
        if record.object_id == MOVER_ID_NONE {
            return Err(MoverCacheError::InvalidId);
        }
        if let Some(slot) = self.find_slot(record.object_id) {
            let dst = &mut self.table[usize_from_u32(slot)];
            // Keep any existing texel span across refreshes.
            record.texel_base = dst.texel_base;
            record.texel_count = dst.texel_count;
            *dst = record;
            return Ok(slot);
        }
        let start = table_hash(record.object_id);
        for step in 0..MOVER_TABLE_SLOTS {
            let slot = (start + step) & (MOVER_TABLE_SLOTS - 1);
            let dst = &mut self.table[usize_from_u32(slot)];
            if dst.object_id == MOVER_ID_NONE {
                *dst = record;
                self.live_movers = self.live_movers.saturating_add(1);
                return Ok(slot);
            }
        }
        Err(MoverCacheError::TableFull)
    }

    /// Drops a mover and returns its texels to the free stack (no orphans).
    pub fn remove(&mut self, object_id: u32) -> bool {
        let Some(slot) = self.find_slot(object_id) else {
            return false;
        };
        let rec = self.table[usize_from_u32(slot)];
        self.free_span(rec.texel_base, rec.texel_count);
        self.table[usize_from_u32(slot)] = MoverRecord::vacant();
        self.live_movers = self.live_movers.saturating_sub(1);
        // Rehash the probe chain so later keys stay reachable.
        self.rehash_from(slot);
        true
    }

    /// Claims one free texel for `object_id`. Stage A helper for lookup tests.
    pub fn claim_texel(
        &mut self,
        object_id: u32,
        local_pos: [f32; 3],
        local_n: [f32; 3],
    ) -> Result<u32, MoverCacheError> {
        if object_id == MOVER_ID_NONE {
            return Err(MoverCacheError::InvalidId);
        }
        if self.free_top == u32::MAX || self.free_count == 0 {
            return Err(MoverCacheError::PoolFull);
        }
        let idx = self.free_top;
        let next = self.texels[usize_from_u32(idx)].next_free;
        self.free_top = next;
        self.free_count = self.free_count.saturating_sub(1);
        self.texels[usize_from_u32(idx)] = MoverTexel {
            local_pos,
            object_id,
            mean: [0.0; 3],
            samples: 0.0,
            sh1: [[0.0; 3]; 3],
            local_n,
            next_free: 0,
        };
        Ok(idx)
    }

    /// Reads a live texel by pool index.
    #[must_use]
    pub fn texel(&self, index: u32) -> Option<&MoverTexel> {
        let t = self.texels.get(usize_from_u32(index))?;
        if t.is_live() {
            Some(t)
        } else {
            None
        }
    }

    /// Nearest live texel for `object_id` at object-local `pos` (linear scan of
    /// that mover's span, or of all live texels with that id when unspanned).
    #[must_use]
    pub fn lookup_texel(&self, object_id: u32, local_pos: [f32; 3]) -> Option<(u32, &MoverTexel)> {
        if object_id == MOVER_ID_NONE {
            return None;
        }
        if let Some(rec) = self.get(object_id) {
            if rec.texel_count > 0 && rec.texel_base != u32::MAX {
                return self.nearest_in_span(rec.texel_base, rec.texel_count, object_id, local_pos);
            }
        }
        let mut best: Option<(u32, f32)> = None;
        for (i, t) in self.texels.iter().enumerate() {
            if t.object_id != object_id {
                continue;
            }
            let d = dist2(t.local_pos, local_pos);
            match best {
                Some((_, bd)) if bd <= d => {}
                _ => best = Some((u32_from_usize(i), d)),
            }
        }
        best.and_then(|(i, _)| self.texel(i).map(|t| (i, t)))
    }

    fn nearest_in_span(
        &self,
        base: u32,
        count: u32,
        object_id: u32,
        local_pos: [f32; 3],
    ) -> Option<(u32, &MoverTexel)> {
        let mut best: Option<(u32, f32)> = None;
        for off in 0..count {
            let i = base.saturating_add(off);
            let Some(t) = self.texels.get(usize_from_u32(i)) else {
                break;
            };
            if t.object_id != object_id {
                continue;
            }
            let d = dist2(t.local_pos, local_pos);
            match best {
                Some((_, bd)) if bd <= d => {}
                _ => best = Some((i, d)),
            }
        }
        best.and_then(|(i, _)| self.texel(i).map(|t| (i, t)))
    }

    fn free_span(&mut self, base: u32, count: u32) {
        if count == 0 || base == u32::MAX {
            return;
        }
        for off in 0..count {
            let i = base.saturating_add(off);
            if usize_from_u32(i) >= self.texels.len() {
                break;
            }
            self.texels[usize_from_u32(i)] = MoverTexel {
                next_free: self.free_top,
                ..MoverTexel::empty()
            };
            self.free_top = i;
            self.free_count = self.free_count.saturating_add(1).min(self.pool_cap);
        }
    }

    fn rehash_from(&mut self, start: u32) {
        let mut slot = (start + 1) & (MOVER_TABLE_SLOTS - 1);
        while slot != start {
            let rec = self.table[usize_from_u32(slot)];
            if rec.object_id == MOVER_ID_NONE {
                return;
            }
            self.table[usize_from_u32(slot)] = MoverRecord::vacant();
            self.live_movers = self.live_movers.saturating_sub(1);
            let _ = self.upsert(rec);
            slot = (slot + 1) & (MOVER_TABLE_SLOTS - 1);
        }
    }
}

/// Errors from the mover cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoverCacheError {
    InvalidId,
    TableFull,
    PoolFull,
}

fn table_hash(object_id: u32) -> u32 {
    // Knuth multiplicative hash into MOVER_TABLE_SLOTS (power of two).
    object_id.wrapping_mul(2654435761) & (MOVER_TABLE_SLOTS - 1)
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

fn usize_from_u32(v: u32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

fn u32_from_usize(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: u32) -> MoverRecord {
        MoverRecord {
            object_id: id,
            sphere: [0.0, 1.0, 0.0, 0.5],
            centre: [0.0, 1.0, 0.0],
            half_extents: [0.5, 0.5, 0.5],
            yaw_cos: 1.0,
            yaw_sin: 0.0,
            texel_base: u32::MAX,
            texel_count: 0,
            tier: MoverTier::Near,
        }
    }

    #[test]
    fn high_pool_is_about_two_mib() {
        let cache = MoverCache::high();
        assert_eq!(cache.pool_cap(), MOVER_TEXELS_HIGH);
        assert_eq!(cache.free_count(), MOVER_TEXELS_HIGH);
        let bytes = u64::from(MOVER_TEXELS_HIGH) * u64::try_from(MOVER_TEXEL_BYTES).expect("64 fits");
        assert_eq!(bytes, 2 * 1024 * 1024);
    }

    #[test]
    fn upsert_and_lookup_are_o1_by_object_id() {
        let mut cache = MoverCache::new(64);
        for id in 1..=100 {
            cache.upsert(record(id)).expect("table holds 100");
        }
        assert_eq!(cache.live_movers(), 100);
        for id in 1..=100 {
            let got = cache.get(id).expect("present");
            assert_eq!(got.object_id, id);
            assert_eq!(got.tier, MoverTier::Near);
        }
        assert!(cache.get(999).is_none());
        assert!(cache.find_slot(MOVER_ID_NONE).is_none());
    }

    #[test]
    fn claim_and_lookup_texel_by_local_position() {
        let mut cache = MoverCache::new(32);
        cache.upsert(record(7)).expect("insert");
        let a = cache
            .claim_texel(7, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0])
            .expect("claim a");
        let b = cache
            .claim_texel(7, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
            .expect("claim b");
        assert_ne!(a, b);
        assert_eq!(cache.used_texels(), 2);
        let (idx, t) = cache
            .lookup_texel(7, [0.9, 0.0, 0.0])
            .expect("nearest to 0.9");
        assert_eq!(idx, b);
        assert_eq!(t.object_id, 7);
        assert!((t.local_pos[0] - 1.0).abs() < 1.0e-5);
    }

    #[test]
    fn remove_frees_texels_and_drops_the_record() {
        let mut cache = MoverCache::new(16);
        cache.upsert(record(3)).expect("insert");
        let i0 = cache
            .claim_texel(3, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0])
            .expect("t0");
        let i1 = cache
            .claim_texel(3, [0.2, 0.0, 0.0], [0.0, 1.0, 0.0])
            .expect("t1");
        // Attach a contiguous span so remove frees them.
        let slot = cache.find_slot(3).expect("slot");
        {
            let rec = &mut cache.table[usize_from_u32(slot)];
            rec.texel_base = i0.min(i1);
            rec.texel_count = 2;
            // Ensure both indices sit in that span for this small pool claim order.
            assert_eq!(rec.texel_base, 0);
        }
        assert!(cache.remove(3));
        assert!(cache.get(3).is_none());
        assert_eq!(cache.live_movers(), 0);
        assert_eq!(cache.free_count(), 16);
        assert!(cache.texel(i0).is_none());
        assert!(cache.texel(i1).is_none());
    }

    #[test]
    fn remove_keeps_other_ids_reachable_after_rehash() {
        let mut cache = MoverCache::new(8);
        // Force a collision chain: pick ids that share a start slot.
        let start = table_hash(1);
        let mut ids = vec![1u32];
        let mut next = 2u32;
        while ids.len() < 4 {
            if table_hash(next) == start {
                ids.push(next);
            }
            next = next.saturating_add(1);
            if next > 10_000 {
                break;
            }
        }
        assert!(ids.len() >= 3, "need a collision chain for the rehash test");
        for &id in &ids {
            cache.upsert(record(id)).expect("insert");
        }
        let victim = ids[1];
        assert!(cache.remove(victim));
        for &id in &ids {
            if id == victim {
                assert!(cache.get(id).is_none());
            } else {
                assert_eq!(cache.get(id).map(|r| r.object_id), Some(id));
            }
        }
    }

    #[test]
    fn thousand_movers_fit_the_table() {
        let mut cache = MoverCache::high();
        for id in 1..=1000 {
            cache.upsert(record(id)).expect("1000 movers");
        }
        assert_eq!(cache.live_movers(), 1000);
        assert_eq!(cache.get(1000).map(|r| r.object_id), Some(1000));
        assert_eq!(cache.get(500).map(|r| r.tier.texel_budget()), Some(384));
        assert_eq!(MoverTier::Far.texel_budget(), 0);
        assert_eq!(MoverTier::Mid.texel_budget(), 24);
    }

    #[test]
    fn pool_full_rejects_another_claim() {
        let mut cache = MoverCache::new(2);
        cache.claim_texel(1, [0.0; 3], [0.0, 1.0, 0.0]).expect("1");
        cache.claim_texel(1, [1.0; 3], [0.0, 1.0, 0.0]).expect("2");
        assert_eq!(
            cache.claim_texel(1, [2.0; 3], [0.0, 1.0, 0.0]),
            Err(MoverCacheError::PoolFull)
        );
    }
}
