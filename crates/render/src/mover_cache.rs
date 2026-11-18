//! Object-attached mover lighting cache (GIBS §1.5 style).
//!
//! A fixed pool of small (~64 B) texels sits beside the fat world hash table.
//! Each texel stores an object-local position and the object/transform id, and
//! keeps lighting in world directions so a spinning mover stays correct. An
//! open-address mover table maps object id to a record in O(1). Compose reads
//! these texels for mover pixels; world cells cover everything else.
//!
//! Stages A–C: pool, table, lookup, tiers, and the fill/boost scheduler
//! helpers. Compose wiring lands in Stage D.

#![expect(
    clippy::as_conversions,
    reason = "brick indices and pool lengths use small integer casts"
)]
#![expect(
    clippy::missing_errors_doc,
    reason = "MoverCache Error variants are named; callers match on the enum"
)]
#![expect(
    clippy::unreadable_literal,
    reason = "hash golden multipliers match the GPU path literals"
)]
#![cfg_attr(
    test,
    expect(
        clippy::cast_precision_loss,
        reason = "pool indices and brick coords fit the float mantissa in practice"
    )
)]
#![cfg_attr(
    test,
    expect(
        clippy::too_many_lines,
        reason = "rotating-box face convergence asserts layout, sets, and ratios together"
    )
)]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]
#![cfg_attr(test, expect(clippy::panic, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(
    test,
    expect(clippy::assert_is_empty, reason = "fixtures assert empty dirty/fill lists")
)]

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

/// Distance (m) enter Near / leave Near (hysteresis).
pub const MOVER_NEAR_ENTER_M: f32 = 8.0;
pub const MOVER_NEAR_LEAVE_M: f32 = 10.0;
/// Distance (m) enter Mid / leave Mid (beyond this → Far).
pub const MOVER_MID_ENTER_M: f32 = 40.0;
pub const MOVER_MID_LEAVE_M: f32 = 48.0;
/// Screen radius (px) enter Near / leave Near.
pub const MOVER_NEAR_ENTER_PX: f32 = 40.0;
pub const MOVER_NEAR_LEAVE_PX: f32 = 32.0;
/// Screen radius (px) enter Mid / leave Mid.
pub const MOVER_MID_ENTER_PX: f32 = 8.0;
pub const MOVER_MID_LEAVE_PX: f32 = 6.0;

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

/// Classifies a mover's tier from distance and projected screen radius, with
/// hysteresis against `prev` so a border wobble does not thrash the share.
#[must_use]
pub fn classify_tier(prev: MoverTier, distance_m: f32, screen_radius_px: f32) -> MoverTier {
    let near = distance_m <= MOVER_NEAR_ENTER_M || screen_radius_px >= MOVER_NEAR_ENTER_PX;
    let mid = distance_m <= MOVER_MID_ENTER_M || screen_radius_px >= MOVER_MID_ENTER_PX;
    let leave_near = distance_m > MOVER_NEAR_LEAVE_M && screen_radius_px < MOVER_NEAR_LEAVE_PX;
    let leave_mid = distance_m > MOVER_MID_LEAVE_M && screen_radius_px < MOVER_MID_LEAVE_PX;
    match prev {
        MoverTier::Near => {
            if leave_near {
                if leave_mid {
                    MoverTier::Far
                } else {
                    MoverTier::Mid
                }
            } else {
                MoverTier::Near
            }
        }
        MoverTier::Mid => {
            if near {
                MoverTier::Near
            } else if leave_mid {
                MoverTier::Far
            } else {
                MoverTier::Mid
            }
        }
        MoverTier::Far => {
            if near {
                MoverTier::Near
            } else if mid {
                MoverTier::Mid
            } else {
                MoverTier::Far
            }
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

/// GPU header at the start of the mover SSBO (64 B).
pub const MOVER_GPU_HEADER_BYTES: usize = 64;
/// GPU table record size (matches [`GpuMoverRecord`]).
pub const MOVER_GPU_RECORD_BYTES: usize = 64;

/// Packed GPU mover table record (64 B).
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct GpuMoverRecord {
    pub object_id: u32,
    pub tier: u32,
    pub texel_base: u32,
    pub texel_count: u32,
    pub sphere: [f32; 4],
    pub centre: [f32; 3],
    pub yaw_cos: f32,
    pub half_extents: [f32; 3],
    pub yaw_sin: f32,
}

const _: () = assert!(std::mem::size_of::<GpuMoverRecord>() == MOVER_GPU_RECORD_BYTES);

/// Packed GPU mover texel (64 B). Lighting in world directions.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct GpuMoverTexel {
    pub local_pos: [f32; 3],
    pub object_id: u32,
    pub mean: [f32; 3],
    pub samples: f32,
    pub local_n: [f32; 3],
    pub next_free: u32,
    /// World-frame SH band-1 as four packed rgb9e5 words (x/y/z axes + pad).
    pub sh_packed: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<GpuMoverTexel>() == MOVER_TEXEL_BYTES);

/// Byte offset of the table inside the mover SSBO.
#[must_use]
pub fn mover_gpu_table_offset() -> u64 {
    u64::try_from(MOVER_GPU_HEADER_BYTES).unwrap_or(0)
}

/// Byte offset of the texel pool inside the mover SSBO.
#[must_use]
pub fn mover_gpu_pool_offset() -> u64 {
    mover_gpu_table_offset()
        + u64::from(MOVER_TABLE_SLOTS) * u64::try_from(MOVER_GPU_RECORD_BYTES).unwrap_or(0)
}

/// Max mover-fill texel indices packed after the pool (shared scheduler list).
pub const MOVER_FILL_LIST_CAP: u32 = 512;

/// Compact live-slot list for GPU `lighting_mover_on` / claim-block / on-mover
/// drop. Sized for the 1000-Near design (table holds `MOVER_TABLE_SLOTS`).
pub const MOVER_PRESENCE_CAP: u32 = 1024;

/// Byte offset of the fill-index list inside the mover SSBO.
#[must_use]
pub fn mover_gpu_fill_offset(pool_cap: u32) -> u64 {
    mover_gpu_pool_offset()
        + u64::from(pool_cap.max(1)) * u64::try_from(MOVER_TEXEL_BYTES).unwrap_or(0)
}

/// Byte offset of the presence slot list (after the fill list).
#[must_use]
pub fn mover_gpu_presence_offset(pool_cap: u32) -> u64 {
    mover_gpu_fill_offset(pool_cap) + u64::from(MOVER_FILL_LIST_CAP) * 4
}

/// Total mover SSBO bytes for a pool of `pool_cap` texels (fill + presence).
#[must_use]
pub fn mover_gpu_bytes(pool_cap: u32) -> u64 {
    mover_gpu_presence_offset(pool_cap) + u64::from(MOVER_PRESENCE_CAP) * 4
}

/// One dirty byte range for a partial SSBO upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtyRange {
    pub offset: u64,
    pub size: u64,
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
    /// Full SSBO rewrite on the next upload.
    dirty_full: bool,
    /// Dirty table slots (merged into ranges on pack).
    dirty_table: Vec<u32>,
    /// Dirty texel indices.
    dirty_texels: Vec<u32>,
    /// Ranked fill-candidate texel indices for the GPU pass (≤ [`MOVER_FILL_LIST_CAP`]).
    fill_list: Vec<u32>,
    /// Equivalent rays claimed by the last [`Self::prepare_fill`] (`len × 16`).
    fill_rays: u32,
    /// Rotates equal-priority underfilled texels so the 256-deposit budget does
    /// not lock onto the lowest pool indices every frame (dark crawling patches).
    fill_epoch: u32,
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
            dirty_full: true,
            dirty_table: Vec::new(),
            dirty_texels: Vec::new(),
            fill_list: Vec::new(),
            fill_rays: 0,
            fill_epoch: 0,
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

    /// Live object ids currently in the table (for host sync / drop).
    #[must_use]
    pub fn table_object_ids(&self) -> Vec<u32> {
        self.table
            .iter()
            .filter(|r| r.is_live())
            .map(|r| r.object_id)
            .collect()
    }

    /// SSBO byte length for the current pool.
    #[must_use]
    pub fn gpu_bytes(&self) -> u64 {
        mover_gpu_bytes(self.pool_cap)
    }

    /// Packs the full SSBO (header + table + pool) for upload.
    #[must_use]
    pub fn pack_gpu(&self) -> Vec<u8> {
        let mut out = vec![0u8; usize_from_u32(u32::try_from(self.gpu_bytes()).unwrap_or(u32::MAX))];
        self.write_header(&mut out);
        for (i, rec) in self.table.iter().enumerate() {
            write_bytes(
                &mut out,
                mover_gpu_table_offset() + u64_from_usize(i) * u64_from_usize(MOVER_GPU_RECORD_BYTES),
                &pack_record(rec),
            );
        }
        let pool_off = mover_gpu_pool_offset();
        for (i, tex) in self.texels.iter().enumerate() {
            write_bytes(
                &mut out,
                pool_off + u64_from_usize(i) * u64_from_usize(MOVER_TEXEL_BYTES),
                &pack_texel(tex),
            );
        }
        let fill_off = mover_gpu_fill_offset(self.pool_cap);
        for (i, idx) in self.fill_list.iter().enumerate() {
            write_bytes(&mut out, fill_off + u64_from_usize(i) * 4, idx);
        }
        let presence = self.presence_slots();
        let pres_off = mover_gpu_presence_offset(self.pool_cap);
        for (i, slot) in presence.iter().enumerate() {
            write_bytes(&mut out, pres_off + u64_from_usize(i) * 4, slot);
        }
        out
    }

    /// Live table slots for the GPU presence list (stable order, capped).
    #[must_use]
    pub fn presence_slots(&self) -> Vec<u32> {
        let mut out = Vec::new();
        for (i, rec) in self.table.iter().enumerate() {
            if !rec.is_live() {
                continue;
            }
            out.push(u32_from_usize(i));
            if u32_from_usize(out.len()) >= MOVER_PRESENCE_CAP {
                break;
            }
        }
        out
    }

    /// Dirty ranges for a partial upload. Empty when nothing changed. A full
    /// rewrite returns one range covering the whole SSBO.
    #[must_use]
    pub fn dirty_ranges(&self) -> Vec<DirtyRange> {
        if self.dirty_full {
            return vec![DirtyRange {
                offset: 0,
                size: self.gpu_bytes(),
            }];
        }
        let mut ranges = Vec::new();
        // Header always when anything is dirty (live counts).
        if !self.dirty_table.is_empty() || !self.dirty_texels.is_empty() {
            ranges.push(DirtyRange {
                offset: 0,
                size: u64_from_usize(MOVER_GPU_HEADER_BYTES),
            });
        }
        let mut slots = self.dirty_table.clone();
        slots.sort_unstable();
        slots.dedup();
        for &slot in &slots {
            ranges.push(DirtyRange {
                offset: mover_gpu_table_offset()
                    + u64::from(slot) * u64_from_usize(MOVER_GPU_RECORD_BYTES),
                size: u64_from_usize(MOVER_GPU_RECORD_BYTES),
            });
        }
        let mut tex = self.dirty_texels.clone();
        tex.sort_unstable();
        tex.dedup();
        // Merge contiguous texel indices into runs.
        let mut i = 0usize;
        while i < tex.len() {
            let start = tex[i];
            let mut end = start;
            while i + 1 < tex.len() && tex[i + 1] == end + 1 {
                i += 1;
                end = tex[i];
            }
            let count = end - start + 1;
            ranges.push(DirtyRange {
                offset: mover_gpu_pool_offset()
                    + u64::from(start) * u64_from_usize(MOVER_TEXEL_BYTES),
                size: u64::from(count) * u64_from_usize(MOVER_TEXEL_BYTES),
            });
            i += 1;
        }
        // Fill + presence ride with the header whenever the table/scheduler moved.
        if !self.dirty_table.is_empty() || !self.dirty_texels.is_empty() {
            if !self.fill_list.is_empty() {
                let n = u32_from_usize(self.fill_list.len()).max(1);
                ranges.push(DirtyRange {
                    offset: mover_gpu_fill_offset(self.pool_cap),
                    size: u64::from(n) * 4,
                });
            }
            let presence_n = u32_from_usize(self.presence_slots().len()).max(1);
            ranges.push(DirtyRange {
                offset: mover_gpu_presence_offset(self.pool_cap),
                size: u64::from(presence_n.min(MOVER_PRESENCE_CAP)) * 4,
            });
        }
        ranges
    }

    /// Clears dirty flags after a successful GPU upload.
    pub fn clear_dirty(&mut self) {
        self.dirty_full = false;
        self.dirty_table.clear();
        self.dirty_texels.clear();
    }

    /// Packs only the dirty ranges into `(offset, bytes)` pairs for upload.
    #[must_use]
    pub fn pack_dirty(&self) -> Vec<(u64, Vec<u8>)> {
        let ranges = self.dirty_ranges();
        if ranges.is_empty() {
            return Vec::new();
        }
        if self.dirty_full {
            return vec![(0, self.pack_gpu())];
        }
        let full = self.pack_gpu();
        ranges
            .into_iter()
            .filter_map(|r| {
                let start = usize_from_u64(r.offset);
                let end = start.saturating_add(usize_from_u64(r.size));
                full.get(start..end).map(|slice| (r.offset, slice.to_vec()))
            })
            .collect()
    }

    fn write_header(&self, out: &mut [u8]) {
        let fill_off = u32::try_from(mover_gpu_fill_offset(self.pool_cap)).unwrap_or(0);
        let presence = self.presence_slots();
        let presence_off = u32::try_from(mover_gpu_presence_offset(self.pool_cap)).unwrap_or(0);
        let words = [
            self.pool_cap,
            MOVER_TABLE_SLOTS,
            self.live_movers,
            self.free_count,
            u32::try_from(MOVER_GPU_HEADER_BYTES).unwrap_or(0),
            u32::try_from(mover_gpu_table_offset()).unwrap_or(0),
            u32::try_from(mover_gpu_pool_offset()).unwrap_or(0),
            self.free_top,
            // Stage H fill list: count, byte offset, capacity, rays.
            u32_from_usize(self.fill_list.len()),
            fill_off,
            MOVER_FILL_LIST_CAP,
            self.fill_rays,
            // Presence list for mover_on / change volume (#286 port).
            u32_from_usize(presence.len()),
            presence_off,
            MOVER_PRESENCE_CAP,
            0,
        ];
        for (i, w) in words.iter().enumerate() {
            let at = i * 4;
            if let Some(dst) = out.get_mut(at..at + 4) {
                dst.copy_from_slice(&w.to_le_bytes());
            }
        }
    }

    fn mark_table_dirty(&mut self, slot: u32) {
        self.dirty_table.push(slot);
    }

    fn mark_texel_dirty(&mut self, index: u32) {
        self.dirty_texels.push(index);
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

    /// Inserts or refreshes a mover. Keeps any existing texel span.
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
            self.mark_table_dirty(slot);
            return Ok(slot);
        }
        let start = table_hash(record.object_id);
        for step in 0..MOVER_TABLE_SLOTS {
            let slot = (start + step) & (MOVER_TABLE_SLOTS - 1);
            let dst = &mut self.table[usize_from_u32(slot)];
            if dst.object_id == MOVER_ID_NONE {
                record.texel_base = u32::MAX;
                record.texel_count = 0;
                *dst = record;
                self.live_movers = self.live_movers.saturating_add(1);
                self.mark_table_dirty(slot);
                return Ok(slot);
            }
        }
        Err(MoverCacheError::TableFull)
    }

    /// How many live movers currently need a non-zero texel share (Near or Mid).
    #[must_use]
    pub fn needy_movers(&self) -> u32 {
        let mut n = 0u32;
        for rec in &self.table {
            if rec.is_live() && rec.tier.texel_budget() > 0 {
                n = n.saturating_add(1);
            }
        }
        n
    }

    /// Per-mover share under pool pressure: `min(tier_budget, pool_cap / needy)`.
    /// A crowded High pool of 1000 Near movers yields about 32 each.
    #[must_use]
    pub fn share_for(&self, tier: MoverTier) -> u32 {
        let budget = tier.texel_budget();
        if budget == 0 {
            return 0;
        }
        let needy = self.needy_movers().max(1);
        let fair = self.pool_cap / needy;
        budget.min(fair)
    }

    /// Updates `object_id`'s tier from distance / screen size (hysteresis) and
    /// resizes its texel span to the current pressure share.
    pub fn update_tier_and_alloc(
        &mut self,
        object_id: u32,
        distance_m: f32,
        screen_radius_px: f32,
    ) -> Result<u32, MoverCacheError> {
        let Some(slot) = self.find_slot(object_id) else {
            return Err(MoverCacheError::MissingMover);
        };
        let prev = self.table[usize_from_u32(slot)].tier;
        let tier = classify_tier(prev, distance_m, screen_radius_px);
        self.table[usize_from_u32(slot)].tier = tier;
        let want = self.share_for(tier);
        self.resize_span(slot, want)?;
        Ok(want)
    }

    /// Resizes every live mover's span to the current pressure share. Call after
    /// a burst of upserts so fair shares settle together.
    pub fn rebalance_all(&mut self) {
        let mut slots = Vec::new();
        for (i, rec) in self.table.iter().enumerate() {
            if rec.is_live() {
                slots.push(u32_from_usize(i));
            }
        }
        for slot in slots {
            let tier = self.table[usize_from_u32(slot)].tier;
            let want = self.share_for(tier);
            let _ = self.resize_span(slot, want);
        }
    }

    fn resize_span(&mut self, slot: u32, want: u32) -> Result<(), MoverCacheError> {
        let rec = self.table[usize_from_u32(slot)];
        let object_id = rec.object_id;
        let have = self.count_texels_for(object_id);
        if want == have {
            let dst = &mut self.table[usize_from_u32(slot)];
            dst.texel_count = want;
            if want == 0 {
                dst.texel_base = u32::MAX;
            }
            // Span size unchanged: keep the surface layout and its irradiance.
            // Re-layout every sync wiped samples once we cleared on assign, and
            // the old first-writer path did not need a per-frame reshuffle.
            return Ok(());
        }
        if want < have {
            // Drop extras (highest pool indices first) so a shrink frees the tail.
            let mut owned = self.indices_for(object_id);
            owned.sort_unstable();
            while u32_from_usize(owned.len()) > want {
                if let Some(i) = owned.pop() {
                    self.free_span(i, 1);
                }
            }
            let dst = &mut self.table[usize_from_u32(slot)];
            dst.texel_count = want;
            dst.texel_base = owned.first().copied().unwrap_or(u32::MAX);
            self.mark_table_dirty(slot);
            self.layout_local_positions(slot);
            return Ok(());
        }
        // Grow: claim more empty texels; keep existing bodies.
        let mut owned = self.indices_for(object_id);
        while u32_from_usize(owned.len()) < want {
            match self.claim_texel(object_id, [0.0; 3], [0.0, 1.0, 0.0]) {
                Ok(i) => owned.push(i),
                Err(e) => {
                    // Leave whatever we already hold; record the partial count.
                    owned.sort_unstable();
                    let dst = &mut self.table[usize_from_u32(slot)];
                    dst.texel_count = u32_from_usize(owned.len());
                    dst.texel_base = owned.first().copied().unwrap_or(u32::MAX);
                    self.layout_local_positions(slot);
                    return Err(e);
                }
            }
        }
        owned.sort_unstable();
        let dst = &mut self.table[usize_from_u32(slot)];
        dst.texel_count = want;
        dst.texel_base = owned.first().copied().unwrap_or(u32::MAX);
        self.mark_table_dirty(slot);
        self.layout_local_positions(slot);
        Ok(())
    }

    /// Places owned texels on the AABB surface with outward normals in a
    /// **dense face-grid order** (face 0..6, then u, then v). Compose looks up
    /// the same index from local position (see `lighting_mover_texel_lookup`).
    ///
    /// The old `hash % count` first-writer path mapped many face cells onto one
    /// texel (wrong normal → dark self-shadowed patches that crawled as the
    /// box moved). Dense packing gives each interior face cell its own texel
    /// when the Near share is large enough (6 × steps² for a cube).
    fn layout_local_positions(&mut self, slot: u32) {
        let rec = self.table[usize_from_u32(slot)];
        if rec.texel_count == 0 || rec.texel_base == u32::MAX {
            return;
        }
        let n = rec.texel_count;
        let hx = rec.half_extents[0].abs().max(MOVER_SURFACE_TEXEL_M);
        let hy = rec.half_extents[1].abs().max(MOVER_SURFACE_TEXEL_M);
        let hz = rec.half_extents[2].abs().max(MOVER_SURFACE_TEXEL_M);
        let extents = [hx, hy, hz];
        // Six faces: (outward normal axis, sign, vary-u axis, vary-v axis).
        let faces: [(usize, f32, usize, usize); 6] = [
            (0, 1.0, 1, 2),
            (0, -1.0, 1, 2),
            (1, 1.0, 0, 2),
            (1, -1.0, 0, 2),
            (2, 1.0, 0, 1),
            (2, -1.0, 0, 1),
        ];
        let mut cursor = 0u32;
        for &(axis, sign, u_ax, v_ax) in &faces {
            if cursor >= n {
                break;
            }
            let mut normal = [0.0f32; 3];
            normal[axis] = sign;
            let u_ext = extents[u_ax];
            let v_ext = extents[v_ax];
            let u_steps = surface_steps(u_ext);
            let v_steps = surface_steps(v_ext);
            for ui in 0..u_steps {
                for vi in 0..v_steps {
                    if cursor >= n {
                        break;
                    }
                    let mut local = [0.0f32; 3];
                    local[axis] = sign * extents[axis];
                    local[u_ax] = surface_coord(ui, u_steps, u_ext);
                    local[v_ax] = surface_coord(vi, v_steps, v_ext);
                    let idx = rec.texel_base.saturating_add(cursor);
                    cursor = cursor.saturating_add(1);
                    let Some(t) = self.texels.get_mut(usize_from_u32(idx)) else {
                        continue;
                    };
                    if t.object_id != rec.object_id {
                        continue;
                    }
                    let same = (t.local_pos[0] - local[0]).abs() < 1.0e-4
                        && (t.local_pos[1] - local[1]).abs() < 1.0e-4
                        && (t.local_pos[2] - local[2]).abs() < 1.0e-4
                        && (t.local_n[0] - normal[0]).abs() < 1.0e-4
                        && (t.local_n[1] - normal[1]).abs() < 1.0e-4
                        && (t.local_n[2] - normal[2]).abs() < 1.0e-4;
                    t.local_pos = local;
                    t.local_n = normal;
                    if !same {
                        t.mean = [0.0; 3];
                        t.samples = 0.0;
                        t.sh1 = [[0.0; 3]; 3];
                    }
                    self.mark_texel_dirty(idx);
                }
            }
        }
        // Pad any leftover span slots so the pool has no zero-normal holes.
        while cursor < n {
            let idx = rec.texel_base.saturating_add(cursor);
            cursor = cursor.saturating_add(1);
            let Some(t) = self.texels.get_mut(usize_from_u32(idx)) else {
                continue;
            };
            if t.object_id != rec.object_id {
                continue;
            }
            if t.local_n[0] != 0.0 || t.local_n[1] != 0.0 || t.local_n[2] != 0.0 {
                continue;
            }
            let (local, normal) = box_surface_from_index(idx - rec.texel_base, n, extents);
            t.local_pos = local;
            t.local_n = normal;
            self.mark_texel_dirty(idx);
        }
    }

    /// Deposits one world-direction irradiance sample into a texel (CPU fill).
    pub fn deposit(&mut self, texel_index: u32, irradiance: [f32; 3], weight: f32) -> bool {
        let Some(t) = self.texels.get_mut(usize_from_u32(texel_index)) else {
            return false;
        };
        if t.object_id == MOVER_ID_NONE || weight <= 0.0 {
            return false;
        }
        let w = weight.max(0.0);
        let prev = t.samples.max(0.0);
        let next = prev + w;
        let a = w / next;
        t.mean = [
            t.mean[0] + (irradiance[0] - t.mean[0]) * a,
            t.mean[1] + (irradiance[1] - t.mean[1]) * a,
            t.mean[2] + (irradiance[2] - t.mean[2]) * a,
        ];
        t.samples = next;
        self.mark_texel_dirty(texel_index);
        true
    }

    /// Ranks fill candidates and packs the GPU fill list under `ray_budget`
    /// (16 rays per texel). Does not write means on the host — the cache pass
    /// deposits world-direction lighting from the lattice.
    pub fn prepare_fill(
        &mut self,
        ray_budget: u32,
        measured_change: f32,
        distance_moved_m: f32,
        lamp_change_near: bool,
        distance_m: f32,
    ) -> FillStats {
        // A still frame must not spend the shared cache batch on mover deposits:
        // closeness alone would otherwise list every Near texel and starve the hall.
        if measured_change <= 0.0 && distance_moved_m <= 0.0 && !lamp_change_near {
            self.fill_list.clear();
            self.fill_rays = 0;
            if !self.dirty_table.contains(&0) {
                self.dirty_table.push(0);
            }
            return FillStats {
                candidates: 0,
                deposited: 0,
                rays: 0,
            };
        }
        let cands = self.fill_candidates(
            measured_change,
            distance_moved_m,
            lamp_change_near,
            distance_m,
        );
        let per = 16u32; // matches LIGHTING_CACHE_RAYS
        let max_deposits = if ray_budget == 0 {
            MOVER_FILL_LIST_CAP
        } else {
            (ray_budget / per).min(MOVER_FILL_LIST_CAP)
        };
        self.fill_list.clear();
        for c in cands.iter().take(max_deposits as usize) {
            self.fill_list.push(c.texel_index);
        }
        self.fill_epoch = self.fill_epoch.wrapping_add(1);
        let deposited = u32_from_usize(self.fill_list.len());
        self.fill_rays = deposited.saturating_mul(per);
        // Header + fill list must reach the GPU even when no texel layout changed.
        self.dirty_full = false;
        if !self.dirty_table.contains(&0) {
            // Force header into the next pack_dirty (header is always first range).
            self.dirty_table.push(0);
        }
        FillStats {
            candidates: u32_from_usize(cands.len()),
            deposited,
            rays: self.fill_rays,
        }
    }

    /// Runs one CPU fill round: deposits into the highest-priority candidates
    /// under `ray_budget` (counted as 16 rays per deposit for reporting).
    pub fn fill_round(
        &mut self,
        ray_budget: u32,
        measured_change: f32,
        distance_moved_m: f32,
        lamp_change_near: bool,
        distance_m: f32,
        irradiance: [f32; 3],
    ) -> FillStats {
        let stats = self.prepare_fill(
            ray_budget,
            measured_change,
            distance_moved_m,
            lamp_change_near,
            distance_m,
        );
        let mut deposited = 0u32;
        for &idx in &self.fill_list.clone() {
            if self.deposit(idx, irradiance, 1.0) {
                deposited = deposited.saturating_add(1);
            }
        }
        FillStats {
            candidates: stats.candidates,
            deposited,
            rays: deposited.saturating_mul(16),
        }
    }

    #[must_use]
    pub fn fill_rays(&self) -> u32 {
        self.fill_rays
    }

    #[must_use]
    pub fn fill_list(&self) -> &[u32] {
        &self.fill_list
    }

    fn count_texels_for(&self, object_id: u32) -> u32 {
        let mut n = 0u32;
        for t in &self.texels {
            if t.object_id == object_id {
                n = n.saturating_add(1);
            }
        }
        n
    }

    fn indices_for(&self, object_id: u32) -> Vec<u32> {
        let mut out = Vec::new();
        for (i, t) in self.texels.iter().enumerate() {
            if t.object_id == object_id {
                out.push(u32_from_usize(i));
            }
        }
        out
    }

    /// Drops a mover and returns its texels to the free stack (no orphans).
    pub fn remove(&mut self, object_id: u32) -> bool {
        let Some(slot) = self.find_slot(object_id) else {
            return false;
        };
        self.free_all_for(object_id);
        self.table[usize_from_u32(slot)] = MoverRecord::vacant();
        self.live_movers = self.live_movers.saturating_sub(1);
        self.mark_table_dirty(slot);
        // Rehash the probe chain so later keys stay reachable.
        self.rehash_from(slot);
        true
    }

    /// Returns every live texel owned by `object_id` to the free stack.
    fn free_all_for(&mut self, object_id: u32) {
        if object_id == MOVER_ID_NONE {
            return;
        }
        for i in 0..self.pool_cap {
            let t = &self.texels[usize_from_u32(i)];
            if t.object_id == object_id {
                self.free_span(i, 1);
            }
        }
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
        self.mark_texel_dirty(idx);
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

    /// Live texel for `object_id` at object-local `pos`.
    ///
    /// Matches the GPU dense face-grid index when a span is laid out. Falls
    /// back to nearest-live for a hand-claimed single texel (unit tests).
    #[must_use]
    pub fn lookup_texel(&self, object_id: u32, local_pos: [f32; 3]) -> Option<(u32, &MoverTexel)> {
        if object_id == MOVER_ID_NONE {
            return None;
        }
        if let Some(slot) = self.find_slot(object_id) {
            let rec = self.table[usize_from_u32(slot)];
            if rec.texel_count > 0 && rec.texel_base != u32::MAX {
                let within = dense_face_index(local_pos, rec.half_extents);
                if within < rec.texel_count {
                    let idx = rec.texel_base.saturating_add(within);
                    if let Some(t) = self.texel(idx) {
                        if t.object_id == object_id {
                            return Some((idx, t));
                        }
                    }
                }
            }
        }
        // Hand-claimed single texels (no layout): nearest live by local distance.
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
            self.mark_texel_dirty(i);
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
    MissingMover,
}

/// One fill-round report (Stage H / scaling).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FillStats {
    pub candidates: u32,
    pub deposited: u32,
    /// Equivalent rays (`deposited × 16`).
    pub rays: u32,
}

fn table_hash(object_id: u32) -> u32 {
    // Knuth multiplicative hash into MOVER_TABLE_SLOTS (power of two).
    object_id.wrapping_mul(2654435761) & (MOVER_TABLE_SLOTS - 1)
}

fn pack_record(rec: &MoverRecord) -> GpuMoverRecord {
    GpuMoverRecord {
        object_id: rec.object_id,
        tier: u32::from(rec.tier as u8),
        texel_base: rec.texel_base,
        texel_count: rec.texel_count,
        sphere: rec.sphere,
        centre: rec.centre,
        yaw_cos: rec.yaw_cos,
        half_extents: rec.half_extents,
        yaw_sin: rec.yaw_sin,
    }
}

fn pack_texel(tex: &MoverTexel) -> GpuMoverTexel {
    GpuMoverTexel {
        local_pos: tex.local_pos,
        object_id: tex.object_id,
        mean: tex.mean,
        samples: tex.samples,
        local_n: tex.local_n,
        next_free: tex.next_free,
        sh_packed: [
            crate::mesh_field::pack_rgb9e5(tex.sh1[0]),
            crate::mesh_field::pack_rgb9e5(tex.sh1[1]),
            crate::mesh_field::pack_rgb9e5(tex.sh1[2]),
            0,
        ],
    }
}

fn write_bytes<T: Copy>(out: &mut [u8], offset: u64, value: &T) {
    let len = std::mem::size_of::<T>();
    let start = usize_from_u64(offset);
    let Some(dst) = out.get_mut(start..start.saturating_add(len)) else {
        return;
    };
    // SAFETY: T is repr(C) POD (GpuMoverRecord / GpuMoverTexel); we only read bytes.
    let src = unsafe { std::slice::from_raw_parts(std::ptr::from_ref(value).cast::<u8>(), len) };
    dst.copy_from_slice(src);
}

fn u64_from_usize(v: usize) -> u64 {
    u64::try_from(v).unwrap_or(u64::MAX)
}

fn usize_from_u64(v: u64) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// World-cell brick size for the mover dirty boost (1 m, DDGI-style +1 cell).
pub const MOVER_DIRTY_BRICK_M: f32 = 1.0;

/// Minimum angular size (radius / distance) before a mover dirties world cells.
pub const MOVER_MATTER_ANGULAR: f32 = 0.01;

/// One mover texel waiting in the shared fill budget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoverFillCandidate {
    pub object_id: u32,
    pub texel_index: u32,
    /// Higher runs sooner. Combines measured change, distance moved, lamp
    /// proximity, and camera closeness (`1 / max(d, 0.5)`).
    pub priority: f32,
}

/// Priority for a mover texel in the single screen-tile / F-stack budget.
#[must_use]
pub fn fill_priority(
    measured_change: f32,
    distance_moved_m: f32,
    lamp_change_near: bool,
    distance_m: f32,
) -> f32 {
    let closeness = 1.0 / distance_m.max(0.5);
    let move_w = distance_moved_m.max(0.0);
    let lamp_w = if lamp_change_near { 4.0 } else { 0.0 };
    measured_change.max(0.0) * 2.0 + move_w * 1.5 + lamp_w + closeness
}

/// Whether a mover is large enough on screen / in angle to dirty world cells.
/// Far-tiny movers read world cells and write nothing back into them.
#[must_use]
pub fn mover_matters(radius_m: f32, distance_m: f32, screen_radius_px: f32) -> bool {
    if screen_radius_px >= MOVER_MID_LEAVE_PX {
        return true;
    }
    let angular = radius_m.max(0.0) / distance_m.max(0.5);
    angular >= MOVER_MATTER_ANGULAR
}

/// 1 m brick indices that cover the swept bounds of a mover, expanded by one
/// brick on every side (DDGI wake box + 1 cell).
#[must_use]
pub fn swept_boost_bricks(prev_sphere: [f32; 4], curr_sphere: [f32; 4]) -> Vec<[i32; 3]> {
    let (lo, hi) = swept_aabb(prev_sphere, curr_sphere);
    bricks_covering(lo, hi, MOVER_DIRTY_BRICK_M)
}

fn swept_aabb(prev: [f32; 4], curr: [f32; 4]) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for sphere in [prev, curr] {
        let r = sphere[3].abs();
        for a in 0..3 {
            lo[a] = lo[a].min(sphere[a] - r);
            hi[a] = hi[a].max(sphere[a] + r);
        }
    }
    // Expand by one brick.
    for a in 0..3 {
        lo[a] -= MOVER_DIRTY_BRICK_M;
        hi[a] += MOVER_DIRTY_BRICK_M;
    }
    (lo, hi)
}

fn bricks_covering(lo: [f32; 3], hi: [f32; 3], brick: f32) -> Vec<[i32; 3]> {
    let brick = brick.max(1.0e-3);
    let min_i = [
        floor_brick(lo[0], brick),
        floor_brick(lo[1], brick),
        floor_brick(lo[2], brick),
    ];
    let max_i = [
        floor_brick(hi[0], brick),
        floor_brick(hi[1], brick),
        floor_brick(hi[2], brick),
    ];
    let mut out = Vec::new();
    let mut x = min_i[0];
    while x <= max_i[0] {
        let mut y = min_i[1];
        while y <= max_i[1] {
            let mut z = min_i[2];
            while z <= max_i[2] {
                out.push([x, y, z]);
                z = z.saturating_add(1);
            }
            y = y.saturating_add(1);
        }
        x = x.saturating_add(1);
    }
    out
}

/// Floor of `v / brick` as `i32`, toward −∞.
fn floor_brick(v: f32, brick: f32) -> i32 {
    crate::cache_probe::floor_i32(v / brick)
}

/// Matches `LIGHTING_SC_TEXEL` — dense face grid step.
const MOVER_SURFACE_TEXEL_M: f32 = 0.125;

fn surface_steps(extent: f32) -> u32 {
    let span = (extent * 2.0) / MOVER_SURFACE_TEXEL_M;
    let n = crate::cache_probe::floor_i32(span);
    u32::try_from(n.max(1)).unwrap_or(1)
}

/// Dense face-grid linear index (face +X,-X,+Y,-Y,+Z,-Z then u then v).
fn dense_face_index(local: [f32; 3], half_extents: [f32; 3]) -> u32 {
    let hx = half_extents[0].abs().max(MOVER_SURFACE_TEXEL_M);
    let hy = half_extents[1].abs().max(MOVER_SURFACE_TEXEL_M);
    let hz = half_extents[2].abs().max(MOVER_SURFACE_TEXEL_M);
    let a = [local[0].abs(), local[1].abs(), local[2].abs()];
    let (face, u_ext, v_ext, u, v) = if a[0] >= a[1] && a[0] >= a[2] {
        (u32::from(local[0] < 0.0), hy, hz, local[1], local[2])
    } else if a[1] >= a[2] {
        (2 + u32::from(local[1] < 0.0), hx, hz, local[0], local[2])
    } else {
        (4 + u32::from(local[2] < 0.0), hx, hy, local[0], local[1])
    };
    let u_steps = surface_steps(u_ext);
    let v_steps = surface_steps(v_ext);
    let mut off = 0u32;
    for f in 0..face {
        let (fu, fv) = match f {
            0 | 1 => (hy, hz),
            2 | 3 => (hx, hz),
            _ => (hx, hy),
        };
        off = off.saturating_add(surface_steps(fu).saturating_mul(surface_steps(fv)));
    }
    let u_span = (2.0 * u_ext).max(1.0e-6);
    let v_span = (2.0 * v_ext).max(1.0e-6);
    let u01 = ((u + u_ext) / u_span).clamp(0.0, 0.9999);
    let v01 = ((v + v_ext) / v_span).clamp(0.0, 0.9999);
    let ui = floor_u32(u01 * f32_from_u32(u_steps)).min(u_steps.saturating_sub(1));
    let vi = floor_u32(v01 * f32_from_u32(v_steps)).min(v_steps.saturating_sub(1));
    off.saturating_add(ui.saturating_mul(v_steps).saturating_add(vi))
}

fn floor_u32(v: f32) -> u32 {
    u32::try_from(crate::cache_probe::floor_i32(v).max(0)).unwrap_or(0)
}

fn surface_coord(i: u32, steps: u32, extent: f32) -> f32 {
    let steps_f = f32_from_u32(steps.max(1));
    let i_f = f32_from_u32(i);
    ((i_f + 0.5) / steps_f) * 2.0 * extent - extent
}

fn f32_from_u32(v: u32) -> f32 {
    // Small counters (face grid) fit the f32 mantissa exactly.
    let mut acc = 0.0f32;
    let mut bit = 1.0f32;
    let mut n = v;
    while n != 0 {
        if n & 1 == 1 {
            acc += bit;
        }
        bit += bit;
        n >>= 1;
    }
    acc
}

/// Deterministic box-surface point for a leftover hash bucket.
fn box_surface_from_index(i: u32, n: u32, extents: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let face = i % 6;
    let along = f32_from_u32(i / 6) / f32_from_u32((n / 6).max(1));
    let t = along.mul_add(2.0, -1.0);
    let (axis, sign) = match face {
        0 => (0usize, 1.0f32),
        1 => (0, -1.0),
        2 => (1, 1.0),
        3 => (1, -1.0),
        4 => (2, 1.0),
        _ => (2, -1.0),
    };
    let (u_ax, v_ax) = match axis {
        0 => (1usize, 2usize),
        1 => (0, 2),
        _ => (0, 1),
    };
    let mut local = [0.0f32; 3];
    let mut normal = [0.0f32; 3];
    local[axis] = sign * extents[axis];
    local[u_ax] = t * extents[u_ax];
    local[v_ax] = (1.0 - t.abs()) * extents[v_ax];
    normal[axis] = sign;
    (local, normal)
}

impl MoverCache {
    /// Builds fill candidates for every live mover texel. Far movers contribute
    /// nothing. Priority comes from [`fill_priority`], boosted for underfilled
    /// texels, with a rotating tie-break so a 256-deposit budget covers the
    /// whole dense face grid instead of the same low pool indices forever.
    #[must_use]
    pub fn fill_candidates(
        &self,
        measured_change: f32,
        distance_moved_m: f32,
        lamp_change_near: bool,
        distance_m: f32,
    ) -> Vec<MoverFillCandidate> {
        let p = fill_priority(measured_change, distance_moved_m, lamp_change_near, distance_m);
        // One pool pass (not O(movers × pool)): skip Far / vacant owners.
        let mut fillable = std::collections::BTreeSet::new();
        for rec in &self.table {
            if rec.is_live() && rec.tier != MoverTier::Far && rec.texel_count > 0 {
                fillable.insert(rec.object_id);
            }
        }
        let epoch = self.fill_epoch;
        let mut out = Vec::new();
        for (i, t) in self.texels.iter().enumerate() {
            if t.object_id == MOVER_ID_NONE || !fillable.contains(&t.object_id) {
                continue;
            }
            // Prefer texels that still need sample sets (warm-up / empty faces).
            let underfill = 1.0 / (1.0 + t.samples.max(0.0));
            let idx = u32_from_usize(i);
            // Rotate among equal underfill so mover B/C and later faces get rays.
            let spin = idx.wrapping_add(epoch).wrapping_mul(2654435761) >> 16;
            let spin_f = f32_from_u32(spin & 0xffff) / 65536.0;
            out.push(MoverFillCandidate {
                object_id: t.object_id,
                texel_index: idx,
                priority: p * (1.0 + 10.0 * underfill) + spin_f * 0.01,
            });
        }
        out.sort_by(|a, b| {
            b.priority
                .partial_cmp(&a.priority)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    /// World-cell bricks to boost for one mover's swept motion. Empty when the
    /// mover is too small to matter at its distance.
    #[must_use]
    pub fn world_boost_bricks(
        &self,
        object_id: u32,
        prev_sphere: [f32; 4],
        distance_m: f32,
        screen_radius_px: f32,
    ) -> Vec<[i32; 3]> {
        let Some(rec) = self.get(object_id) else {
            return Vec::new();
        };
        let radius = rec.sphere[3].abs();
        if !mover_matters(radius, distance_m, screen_radius_px) {
            return Vec::new();
        }
        swept_boost_bricks(prev_sphere, rec.sphere)
    }
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

    #[test]
    fn tier_hysteresis_holds_near_across_the_enter_band() {
        assert_eq!(
            classify_tier(MoverTier::Far, 7.0, 0.0),
            MoverTier::Near
        );
        // Still inside leave band: stay Near.
        assert_eq!(
            classify_tier(MoverTier::Near, 9.0, 0.0),
            MoverTier::Near
        );
        // Past leave distance and small on screen: drop to Mid.
        assert_eq!(
            classify_tier(MoverTier::Near, 11.0, 0.0),
            MoverTier::Mid
        );
        // Far past mid leave: Far.
        assert_eq!(
            classify_tier(MoverTier::Mid, 50.0, 0.0),
            MoverTier::Far
        );
        // Screen size alone can promote Far → Mid.
        assert_eq!(
            classify_tier(MoverTier::Far, 100.0, 10.0),
            MoverTier::Mid
        );
    }

    #[test]
    fn update_tier_allocates_near_and_frees_on_far() {
        let mut cache = MoverCache::new(512);
        cache.upsert(record(1)).expect("insert");
        let n = cache
            .update_tier_and_alloc(1, 2.0, 80.0)
            .expect("near alloc");
        assert_eq!(n, MOVER_TIER_NEAR_TEXELS);
        assert_eq!(cache.get(1).map(|r| r.texel_count), Some(MOVER_TIER_NEAR_TEXELS));
        assert_eq!(cache.used_texels(), MOVER_TIER_NEAR_TEXELS);
        let z = cache
            .update_tier_and_alloc(1, 60.0, 0.0)
            .expect("far frees");
        assert_eq!(z, 0);
        assert_eq!(cache.get(1).map(|r| r.texel_count), Some(0));
        assert_eq!(cache.used_texels(), 0);
        assert!(cache.lookup_texel(1, [0.0; 3]).is_none());
    }

    #[test]
    fn pool_pressure_shrinks_each_share() {
        // 1000 Near movers on a High pool → about 32 each.
        let mut cache = MoverCache::high();
        for id in 1..=1000 {
            let mut r = record(id);
            r.tier = MoverTier::Near;
            cache.upsert(r).expect("insert");
        }
        cache.rebalance_all();
        let share = cache.share_for(MoverTier::Near);
        assert_eq!(share, MOVER_TEXELS_HIGH / 1000);
        assert_eq!(share, 32);
        let got = cache.get(1).expect("m1");
        assert_eq!(got.texel_count, 32);
        assert_eq!(cache.used_texels(), 32 * 1000);
        // Dropping one returns its texels; no orphans for that id.
        assert!(cache.remove(1));
        assert!(cache.lookup_texel(1, [0.0; 3]).is_none());
        assert_eq!(cache.used_texels(), 32 * 999);
    }

    #[test]
    fn mid_tier_caps_at_twenty_four() {
        let mut cache = MoverCache::new(256);
        cache.upsert(record(5)).expect("insert");
        let n = cache
            .update_tier_and_alloc(5, 20.0, 0.0)
            .expect("mid");
        assert_eq!(n, MOVER_TIER_MID_TEXELS);
        assert_eq!(cache.get(5).map(|r| r.tier), Some(MoverTier::Mid));
    }

    #[test]
    fn tiny_far_mover_dirties_no_world_cells() {
        assert!(!mover_matters(0.05, 50.0, 1.0));
        let mut cache = MoverCache::new(64);
        let mut r = record(9);
        r.sphere = [0.0, 1.0, 0.0, 0.05];
        cache.upsert(r).expect("insert");
        let bricks = cache.world_boost_bricks(9, [0.0, 1.0, 0.0, 0.05], 50.0, 1.0);
        assert!(bricks.is_empty());
    }

    #[test]
    fn swept_boost_covers_box_plus_one_metre_brick() {
        // Stationary 0.5 m radius at origin → AABB [-0.5,0.5], expand ±1 →
        // bricks from floor(-1.5)=-2 through floor(1.5)=1 on each axis.
        let bricks = swept_boost_bricks([0.0, 0.0, 0.0, 0.5], [0.0, 0.0, 0.0, 0.5]);
        assert!(bricks.contains(&[-2, -2, -2]));
        assert!(bricks.contains(&[1, 1, 1]));
        assert!(!bricks.contains(&[-3, 0, 0]));
        // Moving along +X extends the sweep.
        let moved = swept_boost_bricks([0.0, 0.0, 0.0, 0.5], [3.0, 0.0, 0.0, 0.5]);
        assert!(moved.len() > bricks.len());
        assert!(moved.contains(&[4, 0, 0])); // 3+0.5+1 = 4.5 → floor 4
    }

    #[test]
    fn fill_candidates_skip_far_and_rank_by_priority() {
        let mut cache = MoverCache::new(128);
        cache.upsert(record(1)).expect("near");
        cache
            .update_tier_and_alloc(1, 2.0, 80.0)
            .expect("alloc near");
        let mut far = record(2);
        far.tier = MoverTier::Far;
        cache.upsert(far).expect("far");
        let cands = cache.fill_candidates(0.5, 0.1, false, 2.0);
        assert!(!cands.is_empty());
        assert!(cands.iter().all(|c| c.object_id == 1));
        let hot = fill_priority(1.0, 2.0, true, 1.0);
        let cold = fill_priority(0.0, 0.0, false, 20.0);
        assert!(hot > cold);
    }

    #[test]
    fn presence_list_lists_live_table_slots() {
        let mut cache = MoverCache::new(64);
        cache.upsert(record(1)).expect("a");
        cache.upsert(record(2)).expect("b");
        let slots = cache.presence_slots();
        assert_eq!(slots.len(), 2);
        assert!(slots.iter().all(|&s| cache.table[usize_from_u32(s)].is_live()));
        let bytes = cache.pack_gpu();
        assert_eq!(u64_from_usize(bytes.len()), mover_gpu_bytes(64));
        // Header words 12..14: count, offset, cap.
        let count = u32::from_le_bytes(bytes[12 * 4..12 * 4 + 4].try_into().unwrap());
        assert_eq!(count, 2);
    }

    #[test]
    fn prepare_fill_ranks_under_budget_and_skips_far() {
        let mut cache = MoverCache::new(256);
        cache.upsert(record(1)).expect("near");
        cache
            .update_tier_and_alloc(1, 2.0, 80.0)
            .expect("alloc");
        let mut far = record(2);
        far.tier = MoverTier::Far;
        cache.upsert(far).expect("far");
        let stats = cache.prepare_fill(64, 0.5, 0.1, false, 2.0);
        assert_eq!(stats.deposited, 4); // 64 / 16
        assert_eq!(stats.rays, 64);
        assert_eq!(cache.fill_list().len(), 4);
        assert!(cache.fill_list().iter().all(|&t| {
            cache.texel(t).is_some_and(|x| x.object_id == 1)
        }));
    }

    #[test]
    fn deposit_blends_world_direction_mean() {
        let mut cache = MoverCache::new(16);
        cache.upsert(record(1)).expect("ins");
        let i = cache
            .claim_texel(1, [0.0; 3], [0.0, 1.0, 0.0])
            .expect("claim");
        assert!(cache.deposit(i, [1.0, 0.0, 0.0], 1.0));
        assert!(cache.deposit(i, [0.0, 1.0, 0.0], 1.0));
        let t = cache.texel(i).expect("live");
        assert!((t.mean[0] - 0.5).abs() < 1.0e-5);
        assert!((t.mean[1] - 0.5).abs() < 1.0e-5);
        assert!((t.samples - 2.0).abs() < 1.0e-5);
    }

    /// Stage E/H scaling report: 10 / 100 / 1000 Near movers on a High pool.
    #[test]
    fn gpu_pack_is_header_plus_table_plus_pool() {
        let cache = MoverCache::new(32);
        let bytes = cache.pack_gpu();
        assert_eq!(u64_from_usize(bytes.len()), mover_gpu_bytes(32));
        assert_eq!(std::mem::size_of::<GpuMoverTexel>(), MOVER_TEXEL_BYTES);
        assert_eq!(std::mem::size_of::<GpuMoverRecord>(), MOVER_GPU_RECORD_BYTES);
        // High pool stays near the 2 MiB texel budget (+ table/header/fill list).
        let high = mover_gpu_bytes(MOVER_TEXELS_HIGH);
        assert!(high > 2 * 1024 * 1024);
        assert!(high < 3 * 1024 * 1024);
    }

    #[test]
    fn dirty_ranges_cover_only_changed_texels_after_clear() {
        let mut cache = MoverCache::new(64);
        cache.clear_dirty();
        assert!(cache.dirty_ranges().is_empty());
        cache.upsert(record(1)).expect("insert");
        cache
            .claim_texel(1, [0.0; 3], [0.0, 1.0, 0.0])
            .expect("claim");
        let ranges = cache.dirty_ranges();
        assert!(!ranges.is_empty());
        assert!(ranges.iter().any(|r| r.offset == 0)); // header
        let packed = cache.pack_dirty();
        assert!(!packed.is_empty());
        cache.clear_dirty();
        assert!(cache.dirty_ranges().is_empty());
    }

    #[test]
    fn presence_list_covers_a_thousand_live_movers() {
        let mut cache = MoverCache::high();
        for id in 1..=1000 {
            cache.upsert(record(id)).expect("insert");
        }
        let slots = cache.presence_slots();
        assert_eq!(slots.len(), 1000);
        assert!(u32_from_usize(slots.len()) <= MOVER_PRESENCE_CAP);
        let packed = cache.pack_gpu();
        assert_eq!(u64_from_usize(packed.len()), mover_gpu_bytes(cache.pool_cap()));
    }

    #[test]
    fn prepare_fill_skips_still_frames() {
        let mut cache = MoverCache::new(256);
        cache.upsert(record(1)).expect("near");
        cache
            .update_tier_and_alloc(1, 2.0, 80.0)
            .expect("alloc");
        let stats = cache.prepare_fill(4096, 0.0, 0.0, false, 3.0);
        assert_eq!(stats.deposited, 0);
        assert_eq!(stats.rays, 0);
        assert!(cache.fill_list().is_empty());
    }

    #[test]
    fn scaling_10_100_1000_reports_pool_use_boost_and_rays() {
        fn run(n: u32) -> (u32, u32, usize, usize, u32) {
            let mut cache = MoverCache::high();
            for id in 1..=n {
                let mut r = record(id);
                r.tier = MoverTier::Near;
                r.sphere = [id as f32, 1.0, 0.0, 0.5];
                cache.upsert(r).expect("insert");
            }
            cache.rebalance_all();
            let share = cache.share_for(MoverTier::Near);
            let used = cache.used_texels();
            let cands = cache.fill_candidates(0.2, 0.05, false, 3.0);
            let bricks = cache.world_boost_bricks(
                1,
                [0.0, 1.0, 0.0, 0.5],
                3.0,
                80.0,
            );
            let stats = cache.prepare_fill(4096, 0.2, 0.05, false, 3.0);
            eprintln!(
                "movers={n} share={share} used_texels={used} fill_candidates={} dirtied_bricks={} rays_per_frame={}",
                cands.len(),
                bricks.len(),
                stats.rays
            );
            (share, used, cands.len(), bricks.len(), stats.rays)
        }
        let (s10, u10, c10, b10, r10) = run(10);
        assert_eq!(s10, MOVER_TIER_NEAR_TEXELS);
        assert_eq!(u10, 10 * MOVER_TIER_NEAR_TEXELS);
        assert_eq!(c10, usize_from_u32(10 * MOVER_TIER_NEAR_TEXELS));
        assert!(b10 > 0);
        assert_eq!(r10, 4096); // ray_budget / 16 deposits, ×16
        let (s100, u100, c100, _, r100) = run(100);
        assert_eq!(s100, MOVER_TEXELS_HIGH / 100);
        assert_eq!(u100, 100 * (MOVER_TEXELS_HIGH / 100));
        assert_eq!(c100, usize_from_u32(u100));
        assert_eq!(r100, 4096);
        let (s1000, u1000, c1000, _, r1000) = run(1000);
        assert_eq!(s1000, 32);
        assert_eq!(u1000, 32_000);
        assert_eq!(c1000, 32_000);
        assert_eq!(r1000, 4096);
    }

    fn face_index(n: [f32; 3]) -> Option<usize> {
        if n[0] > 0.5 {
            Some(0)
        } else if n[0] < -0.5 {
            Some(1)
        } else if n[1] > 0.5 {
            Some(2)
        } else if n[1] < -0.5 {
            Some(3)
        } else if n[2] > 0.5 {
            Some(4)
        } else if n[2] < -0.5 {
            Some(5)
        } else {
            None
        }
    }

    /// Every AABB face-grid cell must resolve to a texel whose stored cell
    /// matches — the hash%count first-writer path mapped many cells onto the
    /// wrong face and painted dark crawling patches on the 4070.
    #[test]
    fn box_face_cells_dense_index_to_matching_surface_texels() {
        let mut cache = MoverCache::new(512);
        let mut rec = record(1);
        rec.half_extents = [0.5, 0.5, 0.5];
        cache.upsert(rec).expect("box");
        cache
            .update_tier_and_alloc(1, 2.0, 80.0)
            .expect("near share");
        let hx = 0.5f32;
        let faces: [(usize, f32, usize, usize); 6] = [
            (0, 1.0, 1, 2),
            (0, -1.0, 1, 2),
            (1, 1.0, 0, 2),
            (1, -1.0, 0, 2),
            (2, 1.0, 0, 1),
            (2, -1.0, 0, 1),
        ];
        let mut matched = 0u32;
        let mut missed = 0u32;
        for &(axis, sign, u_ax, v_ax) in &faces {
            let u_steps = surface_steps(hx);
            let v_steps = surface_steps(hx);
            for ui in 0..u_steps {
                for vi in 0..v_steps {
                    let mut local = [0.0f32; 3];
                    local[axis] = sign * hx;
                    local[u_ax] = surface_coord(ui, u_steps, hx);
                    local[v_ax] = surface_coord(vi, v_steps, hx);
                    // Edges/corners are shared by two or three faces; the first
                    // face that claimed the cell keeps its normal.
                    let on_edge = local[u_ax].abs() > hx - MOVER_SURFACE_TEXEL_M * 0.6
                        || local[v_ax].abs() > hx - MOVER_SURFACE_TEXEL_M * 0.6;
                    if on_edge {
                        continue;
                    }
                    let want = [
                        crate::cache_probe::floor_i32(local[0] / MOVER_SURFACE_TEXEL_M),
                        crate::cache_probe::floor_i32(local[1] / MOVER_SURFACE_TEXEL_M),
                        crate::cache_probe::floor_i32(local[2] / MOVER_SURFACE_TEXEL_M),
                    ];
                    match cache.lookup_texel(1, local) {
                        Some((_, t)) => {
                            let got = [
                                crate::cache_probe::floor_i32(t.local_pos[0] / MOVER_SURFACE_TEXEL_M),
                                crate::cache_probe::floor_i32(t.local_pos[1] / MOVER_SURFACE_TEXEL_M),
                                crate::cache_probe::floor_i32(t.local_pos[2] / MOVER_SURFACE_TEXEL_M),
                            ];
                            assert_eq!(got, want, "cell {want:?} hit wrong texel cell {got:?}");
                            let face_n = t.local_n[axis] * sign;
                            assert!(
                                face_n > 0.5,
                                "cell {want:?} hit normal {:?} (wanted axis {axis} sign {sign})",
                                t.local_n
                            );
                            matched = matched.saturating_add(1);
                        }
                        None => missed = missed.saturating_add(1),
                    }
                }
            }
        }
        assert_eq!(missed, 0, "{missed} interior face cells missed after dense face-grid layout");
        assert!(matched >= 24, "expected a surface share of lookups, got {matched}");
    }

    #[test]
    fn fill_budget_rotates_across_underfilled_texels() {
        // Three Near movers × 384 need a pool bigger than 512.
        let mut cache = MoverCache::new(2048);
        for id in 1..=3u32 {
            let mut rec = record(id);
            rec.half_extents = [0.5, 0.5, 0.5];
            cache.upsert(rec).expect("box");
            cache
                .update_tier_and_alloc(id, 2.0, 80.0)
                .expect("near");
        }
        let mut seen = std::collections::BTreeSet::new();
        // 256 deposits/frame × 12 frames covers 3×384 with overlap once ranking
        // prefers underfill and rotates ties.
        for _ in 0..12 {
            let stats = cache.prepare_fill(4096, 1.0, 1.0, false, 2.0);
            assert!(stats.deposited > 0);
            let listed: Vec<u32> = cache.fill_list().to_vec();
            for idx in listed {
                seen.insert(idx);
                assert!(cache.deposit(idx, [1.0, 1.0, 1.0], 1.0));
            }
        }
        let mut total = 0u32;
        for id in 1..=3u32 {
            let (base, count) = {
                let r = cache.get(id).expect("live");
                (r.texel_base, r.texel_count)
            };
            total = total.saturating_add(count);
            let mut hit = 0u32;
            for i in 0..count {
                if seen.contains(&base.saturating_add(i)) {
                    hit = hit.saturating_add(1);
                }
            }
            assert!(
                hit * 2 >= count,
                "mover {id}: only {hit}/{count} texels entered the fill list in 12 rounds"
            );
        }
        assert!(
            u32_from_usize(seen.len()) * 2 >= total,
            "fill list covered {}/{} texels",
            seen.len(),
            total
        );
    }

    #[test]
    fn dense_face_index_roundtrips_layout_positions() {
        let extents = [0.5f32, 0.5, 0.5];
        let mut cursor = 0u32;
        let faces: [(usize, f32, usize, usize); 6] = [
            (0, 1.0, 1, 2),
            (0, -1.0, 1, 2),
            (1, 1.0, 0, 2),
            (1, -1.0, 0, 2),
            (2, 1.0, 0, 1),
            (2, -1.0, 0, 1),
        ];
        for &(axis, sign, u_ax, v_ax) in &faces {
            let u_steps = surface_steps(extents[u_ax]);
            let v_steps = surface_steps(extents[v_ax]);
            for ui in 0..u_steps {
                for vi in 0..v_steps {
                    let mut local = [0.0f32; 3];
                    local[axis] = sign * extents[axis];
                    local[u_ax] = surface_coord(ui, u_steps, extents[u_ax]);
                    local[v_ax] = surface_coord(vi, v_steps, extents[v_ax]);
                    let got = dense_face_index(local, extents);
                    assert_eq!(got, cursor, "layout cursor {cursor} at {local:?} → {got}");
                    cursor = cursor.saturating_add(1);
                }
            }
        }
    }

    /// Rotating box under a uniform sky + overhead sun: every face has surface
    /// texels, none stay below N sets after M deposits, and top/side brightness
    /// matches the light (not the old all-+Y volume layout).
    #[test]
    fn rotating_box_face_texels_converge_under_uniform_sky() {
        const MIN_SETS: f32 = 8.0;
        const FRAMES: u32 = 12;
        let mut cache = MoverCache::new(512);
        let mut rec = record(1);
        rec.half_extents = [0.5, 0.5, 0.5];
        cache.upsert(rec).expect("box");
        cache
            .update_tier_and_alloc(1, 2.0, 80.0)
            .expect("near share");
        let (base, count) = {
            let r = cache.get(1).expect("live");
            (r.texel_base, r.texel_count)
        };
        assert!(count >= 24, "expected a surface share, got {count}");
        let mut face_hits = [0u32; 6];
        for i in 0..count {
            let t = cache.texel(base.saturating_add(i)).expect("span texel");
            let n = t.local_n;
            let face = face_index(n).unwrap_or_else(|| {
                panic!("texel {i} has no outward face normal {n:?}")
            });
            face_hits[face] = face_hits[face].saturating_add(1);
            let on_face = (t.local_pos[0].abs() - 0.5).abs() < 1.0e-3
                || (t.local_pos[1].abs() - 0.5).abs() < 1.0e-3
                || (t.local_pos[2].abs() - 0.5).abs() < 1.0e-3;
            assert!(
                on_face,
                "texel {i} local_pos {:?} is not on the AABB surface",
                t.local_pos
            );
        }
        assert!(
            face_hits.iter().all(|&h| h > 0),
            "every face needs texels, hits={face_hits:?}"
        );

        let mut yaw = 0.0f32;
        for _ in 0..FRAMES {
            let yc = yaw.cos();
            let ys = yaw.sin();
            {
                let slot = cache.find_slot(1).expect("slot");
                let dst = &mut cache.table[usize_from_u32(slot)];
                dst.yaw_cos = yc;
                dst.yaw_sin = ys;
            }
            for i in 0..count {
                let idx = base.saturating_add(i);
                let n = cache.texel(idx).expect("texel").local_n;
                // World normal under yaw; overhead sun + dim ambient.
                let wn = [yc * n[0] + ys * n[2], n[1], -ys * n[0] + yc * n[2]];
                let l = 0.2 + 0.8 * wn[1].max(0.0);
                assert!(
                    cache.deposit(idx, [l, l, l], 1.0),
                    "deposit into texel {idx}"
                );
            }
            yaw += 0.35;
        }

        let mut top_sum = 0.0f32;
        let mut top_n = 0u32;
        let mut side_sum = 0.0f32;
        let mut side_n = 0u32;
        let mut stuck = 0u32;
        for i in 0..count {
            let t = cache.texel(base.saturating_add(i)).expect("texel");
            if t.samples < MIN_SETS {
                stuck = stuck.saturating_add(1);
            }
            let luma = (t.mean[0] + t.mean[1] + t.mean[2]) / 3.0;
            if t.local_n[1] > 0.5 {
                top_sum += luma;
                top_n = top_n.saturating_add(1);
            } else if t.local_n[1].abs() <= 0.5 {
                side_sum += luma;
                side_n = side_n.saturating_add(1);
            }
        }
        assert_eq!(stuck, 0, "{stuck} texels stuck below {MIN_SETS} sets after {FRAMES} frames");
        assert!(top_n > 0 && side_n > 0, "top={top_n} side={side_n}");
        let top_mean = top_sum / f32_from_u32(top_n);
        let side_mean = side_sum / f32_from_u32(side_n);
        // Top faces the sun (≈1.0); sides get ambient only (≈0.2). Ratio ≈ 5.
        let ratio = top_mean / side_mean.max(1.0e-3);
        assert!(
            ratio > 2.5 && ratio < 8.0,
            "top/side brightness ratio {ratio} (top={top_mean} side={side_mean}) not sun-consistent"
        );

        // After a yaw, a world point on the +X face maps through inverse yaw to
        // a texel whose local normal is still +X (object-local addressing).
        let yc = yaw.cos();
        let ys = yaw.sin();
        let local_x = [0.5f32, 0.0, 0.0];
        let world_off = [
            yc * local_x[0] + ys * local_x[2],
            local_x[1],
            -ys * local_x[0] + yc * local_x[2],
        ];
        let back = [
            yc * world_off[0] - ys * world_off[2],
            world_off[1],
            ys * world_off[0] + yc * world_off[2],
        ];
        let (_idx, hit) = cache
            .lookup_texel(1, back)
            .expect("lookup after yaw");
        assert!(
            hit.local_n[0] > 0.5,
            "yaw-local lookup hit normal {:?}, wanted +X face",
            hit.local_n
        );
    }
}
