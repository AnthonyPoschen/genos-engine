//! Local GI change tracking.
//!
//! A mover or a changed lamp emits a change record. The record dirties the
//! world-cache cells its volume overlaps: the bounds swept between frames for
//! a mover, the lamp's reach for a lamp. Every other cell keeps the finished
//! value already stored there. Nothing in this module marks the whole grid.

use std::collections::BTreeSet;

/// One world-cache region: a cell of the world-cache grid (`world_cell` metres).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorldCacheRegion {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// Axis-aligned bounds, in metres.
#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    fn union(self, other: Self) -> Self {
        Self {
            min: [
                self.min[0].min(other.min[0]),
                self.min[1].min(other.min[1]),
                self.min[2].min(other.min[2]),
            ],
            max: [
                self.max[0].max(other.max[0]),
                self.max[1].max(other.max[1]),
                self.max[2].max(other.max[2]),
            ],
        }
    }

    fn overlaps(self, other: Self) -> bool {
        self.min[0] < other.max[0]
            && self.max[0] > other.min[0]
            && self.min[1] < other.max[1]
            && self.max[1] > other.min[1]
            && self.min[2] < other.max[2]
            && self.max[2] > other.min[2]
    }

    fn overlaps_sphere(self, center: [f32; 3], radius: f32) -> bool {
        if radius <= 0.0 {
            return false;
        }
        let mut distance_sq = 0.0_f32;
        for (axis, value) in center.into_iter().enumerate() {
            let clamped = value.clamp(self.min[axis], self.max[axis]);
            let delta = value - clamped;
            distance_sq += delta * delta;
        }
        distance_sq <= radius * radius
    }

    /// Sphere that contains the box. The GPU change list stores spheres.
    fn bounding_sphere(self) -> [f32; 4] {
        let center = [
            f32::midpoint(self.min[0], self.max[0]),
            f32::midpoint(self.min[1], self.max[1]),
            f32::midpoint(self.min[2], self.max[2]),
        ];
        let half = [
            0.5 * (self.max[0] - self.min[0]),
            0.5 * (self.max[1] - self.min[1]),
            0.5 * (self.max[2] - self.min[2]),
        ];
        let radius = (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt();
        [center[0], center[1], center[2], radius]
    }
}

/// One frame's description of what moved. A sun step is not a record.
#[derive(Clone, Copy, Debug)]
pub enum ChangeRecord {
    /// Bounds before and after this frame. The affected volume is their union.
    Mover { before: Aabb, after: Aabb },
    /// A point lamp that moved or changed intensity or reach.
    Lamp {
        before_pos: [f32; 3],
        after_pos: [f32; 3],
        before_intensity: f32,
        after_intensity: f32,
        before_reach: f32,
        after_reach: f32,
    },
}

#[derive(Clone, Debug)]
enum Volume {
    Bounds(Aabb),
    Sphere { center: [f32; 3], radius: f32 },
}

impl Volume {
    fn overlaps_cell(&self, cell: Aabb) -> bool {
        match *self {
            Self::Bounds(bounds) => bounds.overlaps(cell),
            Self::Sphere { center, radius } => cell.overlaps_sphere(center, radius),
        }
    }
}

/// Finished light kept for cells this frame did not dirty.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FinishedLight {
    pub radiance: [f32; 3],
}

/// What the debug status command reports for this frame's records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GiRegionStats {
    /// Movers and changed lamps this frame.
    pub change_records: u32,
    /// Frames since the last change record.
    pub still_frames: u32,
    /// Change spheres waiting because the GPU list was full.
    pub deferred_spheres: u32,
}

/// Cells of the world cache, and which of them a frame's records dirty.
#[derive(Clone, Debug)]
pub struct WorldCacheRegions {
    cell_metres: f32,
    finished: std::collections::BTreeMap<WorldCacheRegion, FinishedLight>,
    volumes: Vec<Volume>,
    change_records: u32,
    still_frames: u32,
}

impl WorldCacheRegions {
    #[must_use]
    pub fn new(cell_metres: f32) -> Self {
        Self {
            cell_metres: sanitize_cell(cell_metres),
            finished: std::collections::BTreeMap::new(),
            volumes: Vec::new(),
            change_records: 0,
            still_frames: 0,
        }
    }

    pub fn set_cell(&mut self, cell_metres: f32) {
        let cell_metres = sanitize_cell(cell_metres);
        if (cell_metres - self.cell_metres).abs() > 1.0e-6 {
            self.finished.clear();
            self.volumes.clear();
            self.cell_metres = cell_metres;
        }
    }

    pub fn write_finished(&mut self, region: WorldCacheRegion, light: FinishedLight) {
        self.finished.insert(region, light);
    }

    #[must_use]
    pub fn finished(&self, region: WorldCacheRegion) -> Option<FinishedLight> {
        self.finished.get(&region).copied()
    }

    /// Dirty the cells these records overlap. An empty frame is one more still
    /// frame. Finished light is left as it was, including on dirty cells.
    pub fn apply(&mut self, records: &[ChangeRecord]) {
        self.volumes.clear();
        self.change_records = u32::try_from(records.len()).unwrap_or(u32::MAX);
        if records.is_empty() {
            self.still_frames = self.still_frames.saturating_add(1);
            return;
        }
        self.still_frames = 0;
        for record in records {
            self.volumes.extend(volumes_of(record));
        }
    }

    #[must_use]
    pub fn is_dirty(&self, region: WorldCacheRegion) -> bool {
        let cell = self.cell_bounds(region);
        self.volumes.iter().any(|volume| volume.overlaps_cell(cell))
    }

    /// Cells the current records overlap. A volume wider than [`ENUM_LIMIT`]
    /// cells on one axis is omitted here; [`Self::is_dirty`] still answers it.
    #[must_use]
    pub fn dirty_regions(&self) -> BTreeSet<WorldCacheRegion> {
        let mut out = BTreeSet::new();
        for volume in &self.volumes {
            match *volume {
                Volume::Bounds(bounds) => {
                    collect_box(&mut out, bounds, self.cell_metres);
                }
                Volume::Sphere { center, radius } => {
                    collect_sphere(&mut out, center, radius, self.cell_metres);
                }
            }
        }
        out
    }

    #[must_use]
    pub fn still_frames(&self) -> u32 {
        self.still_frames
    }

    #[must_use]
    pub fn change_records(&self) -> u32 {
        self.change_records
    }

    /// No records for `need` frames, and nothing left dirty.
    #[must_use]
    pub fn settled(&self, need: u32) -> bool {
        self.still_frames >= need && self.volumes.is_empty()
    }

    fn cell_bounds(&self, region: WorldCacheRegion) -> Aabb {
        cell_bounds(region, self.cell_metres)
    }
}

fn sanitize_cell(cell_metres: f32) -> f32 {
    if cell_metres.is_finite() && cell_metres >= 0.05 {
        cell_metres
    } else {
        0.05
    }
}

fn volumes_of(record: &ChangeRecord) -> Vec<Volume> {
    match *record {
        ChangeRecord::Mover { before, after } => vec![Volume::Bounds(before.union(after))],
        ChangeRecord::Lamp {
            before_pos,
            after_pos,
            before_reach,
            after_reach,
            ..
        } => {
            let mut volumes = Vec::new();
            if before_reach > 0.0 {
                volumes.push(Volume::Sphere {
                    center: before_pos,
                    radius: before_reach,
                });
            }
            let moved =
                !same_point(before_pos, after_pos) || (before_reach - after_reach).abs() > 1.0e-4;
            if moved && after_reach > 0.0 {
                volumes.push(Volume::Sphere {
                    center: after_pos,
                    radius: after_reach,
                });
            }
            volumes
        }
    }
}

fn same_point(a: [f32; 3], b: [f32; 3]) -> bool {
    a[0].to_bits() == b[0].to_bits()
        && a[1].to_bits() == b[1].to_bits()
        && a[2].to_bits() == b[2].to_bits()
}

/// Widest volume, in cells, that [`WorldCacheRegions::dirty_regions`] lists.
const ENUM_LIMIT: i32 = 48;

fn cell_bounds(region: WorldCacheRegion, cell: f32) -> Aabb {
    let origin = |index: i32| i32_to_f32(index) * cell;
    Aabb {
        min: [origin(region.x), origin(region.y), origin(region.z)],
        max: [
            origin(region.x) + cell,
            origin(region.y) + cell,
            origin(region.z) + cell,
        ],
    }
}

fn collect_box(out: &mut BTreeSet<WorldCacheRegion>, bounds: Aabb, cell: f32) {
    let (x0, x1) = axis_range(bounds.min[0], bounds.max[0], cell);
    let (y0, y1) = axis_range(bounds.min[1], bounds.max[1], cell);
    let (z0, z1) = axis_range(bounds.min[2], bounds.max[2], cell);
    if x1 - x0 > ENUM_LIMIT || y1 - y0 > ENUM_LIMIT || z1 - z0 > ENUM_LIMIT {
        return;
    }
    let mut x = x0;
    while x <= x1 {
        let mut y = y0;
        while y <= y1 {
            let mut z = z0;
            while z <= z1 {
                out.insert(WorldCacheRegion { x, y, z });
                z += 1;
            }
            y += 1;
        }
        x += 1;
    }
}

fn collect_sphere(out: &mut BTreeSet<WorldCacheRegion>, center: [f32; 3], radius: f32, cell: f32) {
    let bounds = Aabb {
        min: [center[0] - radius, center[1] - radius, center[2] - radius],
        max: [center[0] + radius, center[1] + radius, center[2] + radius],
    };
    let (x0, x1) = axis_range(bounds.min[0], bounds.max[0], cell);
    let (y0, y1) = axis_range(bounds.min[1], bounds.max[1], cell);
    let (z0, z1) = axis_range(bounds.min[2], bounds.max[2], cell);
    if x1 - x0 > ENUM_LIMIT || y1 - y0 > ENUM_LIMIT || z1 - z0 > ENUM_LIMIT {
        return;
    }
    let mut x = x0;
    while x <= x1 {
        let mut y = y0;
        while y <= y1 {
            let mut z = z0;
            while z <= z1 {
                let region = WorldCacheRegion { x, y, z };
                if cell_bounds(region, cell).overlaps_sphere(center, radius) {
                    out.insert(region);
                }
                z += 1;
            }
            y += 1;
        }
        x += 1;
    }
}

fn axis_range(min: f32, max: f32, cell: f32) -> (i32, i32) {
    let first = floor_i32(min / cell);
    let last = last_index(max, cell);
    (first, last)
}

fn last_index(max: f32, cell: f32) -> i32 {
    let quotient = max / cell;
    let floored = floor_i32(quotient);
    let reconstructed = i32_to_f32(floored);
    let scale = reconstructed.abs().max(1.0);
    let exact = (quotient - reconstructed).abs() <= 1.0e-4 * scale;
    if exact {
        floored - 1
    } else {
        floored
    }
}

/// Spheres for the GPU change list. A mover contributes the bounding sphere of
/// its swept bounds. A lamp contributes each place it occupied, at that place's
/// reach. The list is not grown into one scene-sized sphere.
#[must_use]
pub fn change_spheres(records: &[ChangeRecord]) -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    for record in records {
        match *record {
            ChangeRecord::Mover { before, after } => {
                out.push(before.union(after).bounding_sphere());
            }
            ChangeRecord::Lamp {
                before_pos,
                after_pos,
                before_reach,
                after_reach,
                ..
            } => {
                if before_reach > 0.0 {
                    out.push([before_pos[0], before_pos[1], before_pos[2], before_reach]);
                }
                let moved = !same_point(before_pos, after_pos)
                    || (before_reach - after_reach).abs() > 1.0e-4;
                if moved && after_reach > 0.0 {
                    out.push([after_pos[0], after_pos[1], after_pos[2], after_reach]);
                }
            }
        }
    }
    out
}

/// Keep `cap` spheres and return the rest. The rest are retried later. They
/// are not replaced by a sphere around the whole scene.
#[must_use]
pub fn split_spheres(mut spheres: Vec<[f32; 4]>, cap: usize) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
    if spheres.len() <= cap {
        (spheres, Vec::new())
    } else {
        let rest = spheres.split_off(cap);
        (spheres, rest)
    }
}

/// Change records between two resident scene blocks. The camera is ignored.
/// A directional lamp (the sun) is ignored. Occluders and point lamps are
/// matched by index.
#[must_use]
pub fn scene_change_records(old: &[u8], new: &[u8]) -> Vec<ChangeRecord> {
    let mut records = Vec::new();
    records.extend(occ_records(old, new));
    records.extend(lamp_records(old, new));
    records
}

/// Records for bounds that appeared, disappeared, or moved. The first sample
/// (`previous` empty) is not a move.
#[must_use]
pub fn bounds_change_records(previous: &mut Vec<Aabb>, now: &[Aabb]) -> Vec<ChangeRecord> {
    let records = if previous.is_empty() {
        Vec::new()
    } else if previous.len() == now.len() {
        previous
            .iter()
            .zip(now)
            .filter(|(before, after)| aabb_differs(**before, **after))
            .map(|(before, after)| ChangeRecord::Mover {
                before: *before,
                after: *after,
            })
            .collect()
    } else {
        let mut records = Vec::new();
        for bounds in previous.iter().chain(now) {
            records.push(ChangeRecord::Mover {
                before: *bounds,
                after: *bounds,
            });
        }
        records
    };
    previous.clear();
    previous.extend_from_slice(now);
    records
}

fn aabb_differs(a: Aabb, b: Aabb) -> bool {
    !same_point(a.min, b.min) || !same_point(a.max, b.max)
}

fn occ_records(old: &[u8], new: &[u8]) -> Vec<ChangeRecord> {
    let before = occ_slices(old);
    let after = occ_slices(new);
    if before.len() != after.len() {
        let mut records = Vec::new();
        for rec in before.into_iter().chain(after) {
            let bounds = occ_aabb(rec);
            records.push(ChangeRecord::Mover {
                before: bounds,
                after: bounds,
            });
        }
        return records;
    }
    before
        .into_iter()
        .zip(after)
        .filter(|(a, b)| occ_pose_differs(a, b))
        .map(|(a, b)| ChangeRecord::Mover {
            before: occ_aabb(a),
            after: occ_aabb(b),
        })
        .collect()
}

fn lamp_records(old: &[u8], new: &[u8]) -> Vec<ChangeRecord> {
    let before = point_lamps(old);
    let after = point_lamps(new);
    if before.len() != after.len() {
        let mut records = Vec::new();
        for lamp in before.into_iter().chain(after) {
            records.push(lamp_record(lamp, lamp));
        }
        return records;
    }
    before
        .into_iter()
        .zip(after)
        .filter(|(a, b)| lamp_differs(*a, *b))
        .map(|(a, b)| lamp_record(a, b))
        .collect()
}

#[derive(Clone, Copy)]
struct LampSnap {
    pos: [f32; 3],
    intensity: f32,
    reach: f32,
    bits: [u32; 8],
}

fn lamp_record(before: LampSnap, after: LampSnap) -> ChangeRecord {
    ChangeRecord::Lamp {
        before_pos: before.pos,
        after_pos: after.pos,
        before_intensity: before.intensity,
        after_intensity: after.intensity,
        before_reach: before.reach,
        after_reach: after.reach,
    }
}

fn lamp_differs(a: LampSnap, b: LampSnap) -> bool {
    a.bits != b.bits
}

fn point_lamps(bytes: &[u8]) -> Vec<LampSnap> {
    let count = word_u32(bytes, 0);
    let mut out = Vec::new();
    for index in 0..count {
        let Ok(index) = usize::try_from(index) else {
            break;
        };
        let at = crate::pack::SCENE_TAIL + index * 32;
        let Some(rec) = bytes.get(at..at + 32) else {
            break;
        };
        if f32_word(rec, 3) > 0.5 {
            continue;
        }
        let color = [f32_word(rec, 4), f32_word(rec, 5), f32_word(rec, 6)];
        let mut bits = [0_u32; 8];
        for (word, slot) in bits.iter_mut().enumerate() {
            *slot = word_u32(rec, word);
        }
        out.push(LampSnap {
            pos: [f32_word(rec, 0), f32_word(rec, 1), f32_word(rec, 2)],
            intensity: color[0].max(color[1]).max(color[2]),
            reach: f32_word(rec, 7).abs(),
            bits,
        });
    }
    out
}

fn occ_slices(bytes: &[u8]) -> Vec<&[u8]> {
    let lamps = word_u32(bytes, 0);
    let occs = word_u32(bytes, 1);
    let Ok(lamps) = usize::try_from(lamps) else {
        return Vec::new();
    };
    let start = crate::pack::SCENE_TAIL + lamps * 32;
    let mut out = Vec::new();
    for index in 0..occs {
        let Ok(index) = usize::try_from(index) else {
            break;
        };
        let at = start + index * 64;
        match bytes.get(at..at + 64) {
            Some(rec) => out.push(rec),
            None => break,
        }
    }
    out
}

fn occ_pose_differs(a: &[u8], b: &[u8]) -> bool {
    const POSE: &[usize] = &[0, 1, 2, 3, 4, 5, 6, 7, 14, 15];
    POSE.iter()
        .any(|word| word_u32(a, *word) != word_u32(b, *word))
}

fn occ_aabb(rec: &[u8]) -> Aabb {
    let center = [f32_word(rec, 0), f32_word(rec, 1), f32_word(rec, 2)];
    let shape = f32_word(rec, 3);
    let radius = f32_word(rec, 7).abs();
    let (half_x, half_z) = if shape < 0.5 {
        let cosine = f32_word(rec, 14).abs();
        let sine = f32_word(rec, 15).abs();
        let extent_x = f32_word(rec, 4).abs();
        let extent_z = f32_word(rec, 6).abs();
        (
            cosine * extent_x + sine * extent_z,
            sine * extent_x + cosine * extent_z,
        )
    } else {
        (radius, radius)
    };
    let half_y = f32_word(rec, 5).abs() * 0.5;
    Aabb {
        min: [center[0] - half_x, center[1] - half_y, center[2] - half_z],
        max: [center[0] + half_x, center[1] + half_y, center[2] + half_z],
    }
}

fn word_u32(bytes: &[u8], word: usize) -> u32 {
    let at = word * 4;
    let Some(slice) = bytes.get(at..at + 4) else {
        return 0;
    };
    let Ok(arr) = <[u8; 4]>::try_from(slice) else {
        return 0;
    };
    u32::from_le_bytes(arr)
}

fn f32_word(bytes: &[u8], word: usize) -> f32 {
    f32::from_le_bytes(word_u32(bytes, word).to_le_bytes())
}

/// Floor of `value` as `i32`. Non-finite values become 0. Values past the
/// `i32` range clamp.
fn floor_i32(value: f32) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let bits = value.to_bits();
    let negative = bits & 0x8000_0000 != 0;
    let exp_bits = (bits >> 23) & 0xff;
    if exp_bits == 0 {
        return if negative && bits & 0x007f_ffff != 0 {
            -1
        } else {
            0
        };
    }
    if exp_bits == 0xff {
        return 0;
    }
    let exp = i32::from_le_bytes(exp_bits.to_le_bytes());
    let shift = exp - 127;
    if shift < 0 {
        return if negative { -1 } else { 0 };
    }
    if shift >= 31 {
        return if negative { i32::MIN } else { i32::MAX };
    }
    let Ok(shift_u) = u32::try_from(shift) else {
        return 0;
    };
    let frac = (bits & 0x007f_ffff) | 0x0080_0000;
    let (magnitude, exact) = if shift_u >= 23 {
        (frac << (shift_u - 23), true)
    } else {
        let down = 23 - shift_u;
        let magnitude = frac >> down;
        let exact = frac & ((1_u32 << down) - 1) == 0;
        (magnitude, exact)
    };
    let Ok(limit) = u32::try_from(i32::MAX) else {
        return 0;
    };
    if magnitude > limit {
        return if negative { i32::MIN } else { i32::MAX };
    }
    let mag_i = i32::from_le_bytes(magnitude.to_le_bytes());
    if !negative {
        return mag_i;
    }
    let neg = mag_i.wrapping_neg();
    if exact {
        neg
    } else {
        neg - 1
    }
}

/// `value` as `f32` for integers that fit in the 24-bit mantissa.
fn i32_to_f32(value: i32) -> f32 {
    if value == 0 {
        return 0.0;
    }
    let negative = value < 0;
    let magnitude = value.unsigned_abs();
    let shift = magnitude.ilog2();
    let exp = shift + 127;
    let fraction = if shift <= 23 {
        (magnitude << (23 - shift)) & 0x007f_ffff
    } else {
        (magnitude >> (shift - 23)) & 0x007f_ffff
    };
    let bits = (u32::from(negative) << 31) | (exp << 23) | fraction;
    f32::from_bits(bits)
}

#[cfg(test)]
mod tests {
    use super::{
        bounds_change_records, change_spheres, floor_i32, i32_to_f32, scene_change_records,
        split_spheres, Aabb, ChangeRecord, FinishedLight, WorldCacheRegion, WorldCacheRegions,
    };

    fn region(x: i32, y: i32, z: i32) -> WorldCacheRegion {
        WorldCacheRegion { x, y, z }
    }

    fn light(radiance: [f32; 3]) -> FinishedLight {
        FinishedLight { radiance }
    }

    fn box_record(
        before: [f32; 3],
        before_max: [f32; 3],
        after: [f32; 3],
        after_max: [f32; 3],
    ) -> ChangeRecord {
        ChangeRecord::Mover {
            before: Aabb {
                min: before,
                max: before_max,
            },
            after: Aabb {
                min: after,
                max: after_max,
            },
        }
    }

    #[test]
    fn floor_matches_a_few_known_values() {
        assert_eq!(floor_i32(0.0), 0);
        assert_eq!(floor_i32(1.9), 1);
        assert_eq!(floor_i32(3.0), 3);
        assert_eq!(floor_i32(-0.1), -1);
        assert_eq!(floor_i32(-2.0), -2);
        assert_eq!(floor_i32(-1.5), -2);
        assert_eq!(i32_to_f32(0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(i32_to_f32(1).to_bits(), 1.0_f32.to_bits());
        assert_eq!(i32_to_f32(-4).to_bits(), (-4.0_f32).to_bits());
        assert_eq!(i32_to_f32(1000).to_bits(), 1000.0_f32.to_bits());
    }

    #[test]
    fn a_single_moving_box_dirties_only_nearby_regions() {
        let mut cache = WorldCacheRegions::new(0.25);
        cache.set_cell(1.0);
        let stored = light([0.2, 0.3, 0.4]);
        for x in -4..8 {
            cache.write_finished(region(x, 0, 0), stored);
        }
        let record = box_record(
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.4, 0.0, 0.0],
            [1.4, 1.0, 1.0],
        );
        cache.apply(std::slice::from_ref(&record));
        assert_eq!(cache.change_records(), 1);
        let dirty = cache.dirty_regions();
        assert_eq!(
            dirty,
            [region(0, 0, 0), region(1, 0, 0)].into_iter().collect()
        );
        assert!(!cache.is_dirty(region(2, 0, 0)));
        assert!(!cache.is_dirty(region(-1, 0, 0)));
        assert!(!cache.is_dirty(region(6, 0, 0)));
        assert!(dirty.len() < 8);
        assert_eq!(cache.finished(region(6, 0, 0)), Some(stored));
        assert_eq!(cache.finished(region(0, 0, 0)), Some(stored));
        assert!(!cache.settled(1));
        assert_eq!(cache.still_frames(), 0);
    }

    #[test]
    fn a_changed_lamp_dirties_only_inside_its_reach() {
        let mut cache = WorldCacheRegions::new(1.0);
        let stored = light([1.0, 1.0, 1.0]);
        cache.write_finished(region(0, 0, 0), stored);
        cache.write_finished(region(2, 0, 0), stored);
        cache.write_finished(region(8, 0, 0), stored);
        let record = ChangeRecord::Lamp {
            before_pos: [0.0, 0.0, 0.0],
            after_pos: [0.0, 0.0, 0.0],
            before_intensity: 1.0,
            after_intensity: 2.0,
            before_reach: 1.5,
            after_reach: 1.5,
        };
        if let ChangeRecord::Lamp {
            before_intensity,
            after_intensity,
            ..
        } = record
        {
            assert!(after_intensity > before_intensity);
        }
        cache.apply(std::slice::from_ref(&record));
        assert!(cache.is_dirty(region(0, 0, 0)));
        assert!(cache.is_dirty(region(1, 0, 0)));
        assert!(!cache.is_dirty(region(2, 0, 0)));
        assert!(!cache.is_dirty(region(8, 0, 0)));
        assert_eq!(cache.finished(region(8, 0, 0)), Some(stored));
        assert_eq!(cache.finished(region(0, 0, 0)), Some(stored));
    }

    #[test]
    fn a_frozen_scene_settles_and_keeps_its_light() {
        let mut cache = WorldCacheRegions::new(1.0);
        let stored = light([0.4, 0.5, 0.6]);
        cache.write_finished(region(3, 1, -2), stored);
        for _ in 0..32 {
            cache.apply(&[]);
        }
        assert_eq!(cache.still_frames(), 32);
        assert!(cache.settled(32));
        assert!(cache.dirty_regions().is_empty());
        assert_eq!(cache.finished(region(3, 1, -2)), Some(stored));
        cache.apply(&[]);
        assert!(cache.settled(32));
        cache.apply(&[box_record(
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.2, 0.0, 0.0],
            [1.2, 1.0, 1.0],
        )]);
        assert_eq!(cache.still_frames(), 0);
        assert!(!cache.settled(32));
        assert_eq!(cache.finished(region(3, 1, -2)), Some(stored));
        cache.apply(&[]);
        assert_eq!(cache.still_frames(), 1);
        assert!(!cache.settled(32));
    }

    #[test]
    fn a_camera_move_is_not_a_change_record_and_one_box_stays_local() {
        let still = scene_block([0.0, 1.6, 4.0], &[[0.0, 0.5, 0.0]], &[[0.0, 0.5, 0.0, 0.4]]);
        let mut turned = still.clone();
        put_f32(&mut turned, 4, 0.4);
        assert!(scene_change_records(&still, &turned).is_empty());
        let mut moved = still.clone();
        // Occluder centre x is the first float of the occ tail.
        let occ_word = (crate::pack::SCENE_TAIL / 4) + 8;
        put_f32(&mut moved, occ_word, 0.3);
        let records = scene_change_records(&still, &moved);
        assert_eq!(records.len(), 1);
        let mut cache = WorldCacheRegions::new(1.0);
        cache.write_finished(region(20, 0, 0), light([0.1, 0.1, 0.1]));
        cache.apply(&records);
        assert!(!cache.is_dirty(region(20, 0, 0)));
        assert!(cache.dirty_regions().len() < 16);
        assert_eq!(
            cache.finished(region(20, 0, 0)),
            Some(light([0.1, 0.1, 0.1]))
        );
    }

    #[test]
    fn extra_spheres_are_deferred_not_replaced_by_one_world_sphere() {
        let mut spheres = Vec::new();
        for index in 0..70 {
            let x = i32_to_f32(index);
            spheres.push([x, 0.0, 0.0, 0.5]);
        }
        let (fit, rest) = split_spheres(spheres, 64);
        assert_eq!(fit.len(), 64);
        assert_eq!(rest.len(), 6);
        assert!(fit.iter().chain(&rest).all(|sphere| sphere[3] < 1.0));
    }

    #[test]
    fn mesh_bounds_emit_a_swept_record_only_after_the_first_sample() {
        let mut previous = Vec::new();
        let first = [Aabb {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        }];
        assert!(bounds_change_records(&mut previous, &first).is_empty());
        let next = [Aabb {
            min: [0.5, 0.0, 0.0],
            max: [1.5, 1.0, 1.0],
        }];
        let records = bounds_change_records(&mut previous, &next);
        assert_eq!(records.len(), 1);
        let spheres = change_spheres(&records);
        assert_eq!(spheres.len(), 1);
        assert!(spheres[0][3] < 2.0);
    }

    fn scene_block(eye: [f32; 3], lamps: &[[f32; 3]], occs: &[[f32; 4]]) -> Vec<u8> {
        let mut bytes = vec![0_u8; crate::pack::SCENE_TAIL];
        put_u32(&mut bytes, 0, u32::try_from(lamps.len()).unwrap_or(0));
        put_u32(&mut bytes, 1, u32::try_from(occs.len()).unwrap_or(0));
        put_f32(&mut bytes, 4, eye[0]);
        put_f32(&mut bytes, 5, eye[1]);
        put_f32(&mut bytes, 6, eye[2]);
        for lamp in lamps {
            for value in lamp {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes.extend_from_slice(&0.0_f32.to_le_bytes());
            bytes.extend_from_slice(&1.0_f32.to_le_bytes());
            bytes.extend_from_slice(&1.0_f32.to_le_bytes());
            bytes.extend_from_slice(&1.0_f32.to_le_bytes());
            bytes.extend_from_slice(&3.0_f32.to_le_bytes());
        }
        for occ in occs {
            // centre xyz, shape 0, half extents from the fourth component.
            bytes.extend_from_slice(&occ[0].to_le_bytes());
            bytes.extend_from_slice(&occ[1].to_le_bytes());
            bytes.extend_from_slice(&occ[2].to_le_bytes());
            bytes.extend_from_slice(&0.0_f32.to_le_bytes());
            bytes.extend_from_slice(&occ[3].to_le_bytes());
            bytes.extend_from_slice(&1.0_f32.to_le_bytes());
            bytes.extend_from_slice(&occ[3].to_le_bytes());
            bytes.extend_from_slice(&0.0_f32.to_le_bytes());
            for _ in 0..6 {
                bytes.extend_from_slice(&0.0_f32.to_le_bytes());
            }
            bytes.extend_from_slice(&1.0_f32.to_le_bytes());
            bytes.extend_from_slice(&0.0_f32.to_le_bytes());
        }
        bytes
    }

    fn put_u32(bytes: &mut [u8], word: usize, value: u32) {
        let at = word * 4;
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_f32(bytes: &mut [u8], word: usize, value: f32) {
        let at = word * 4;
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
}
