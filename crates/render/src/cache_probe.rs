//! CPU view of one surface-cache hash cell.
//!
//! The key and the 16-wide probe match `lighting_cache_key_sized` and
//! `lighting_cache_find_sized` in `lighting_cache.glsl`. The slot count matches
//! `LIGHTING_CACHE_SLOTS` in `lighting_cache_slots.glsl`.

/// Slots in the surface-cache hash. `lighting_cache_slots.glsl`.
pub const SLOTS: u32 = 1_048_576;
/// Probe steps before a lookup gives up. `LIGHTING_CACHE_SEARCH`.
pub const SEARCH: u32 = 16;

/// One world point, the normal the picture faces, and the cell size to hash.
#[derive(Clone, Copy, Debug)]
pub struct CacheQuery {
    /// World position on the surface, metres.
    pub position: [f32; 3],
    /// Outward normal.
    pub normal: [f32; 3],
    /// Cell edge in metres. `0.125` is a fine texel, `0.25` is a world cell.
    pub size: f32,
}

/// The cell the hash names, plus the records compose and the relight write.
#[derive(Clone, Copy, Debug)]
pub struct StoredCell {
    /// Hash window for this query.
    pub hit: CacheHit,
    /// Cell size that was hashed.
    pub size: f32,
    /// Stored position. Zero when the window missed.
    pub position: [f32; 3],
    /// Stored normal.
    pub normal: [f32; 3],
    /// Stored cell edge, metres. Zero until the slot is claimed.
    pub stored_size: f32,
    /// Stored irradiance (the running mean).
    pub radiance: [f32; 3],
    /// Sample count in the irradiance record (`w`).
    pub weight: f32,
    /// Frame compose last stamped into the position record.
    pub last_read: u32,
    /// Frame of the last relight (`LIGHTING_SC_OBJ_AT` `.y`).
    pub updated: u32,
    /// Frame a pixel last showed the cell (`LIGHTING_WORLD_AT` `.z`).
    pub shown_frame: u32,
    /// Filtered field compose shows for a fine texel, rgb9e5 in the world record.
    pub field: [f32; 3],
    /// Support of that field. Negative when an edge stopped the rebuild.
    pub field_support: f32,
    /// Frame the filtered field was last pushed.
    pub field_stamp: u32,
    /// Seed irradiance (`LIGHTING_SC_SPARSE_AT` `.y`, rgb9e5). Not the texel's own samples.
    pub seed: [f32; 3],
    /// Seed confidence (`SPARSE_AT` `.z`). Negative marks a detail seed. Zero means none.
    pub seed_weight: f32,
}

/// What the hash window held for one [`CacheQuery`].
#[derive(Clone, Copy, Debug)]
pub struct CacheHit {
    /// Key `lighting_cache_key_sized` would write. Never zero.
    pub key: u32,
    /// First slot in the window that stores `key`. `None` when the window missed.
    pub slot: Option<u32>,
    /// Slots in the window that store `key`. More than one is a duplicate.
    pub copies: u32,
    /// Occupied slots in the window.
    pub live: u32,
    /// Occupied slots whose key is not `key`.
    pub foreign: u32,
}

/// Key of the cell of `size` metres that holds `position` facing `normal`.
///
/// World mode is not zero, so the cell is inset by half a cell along the
/// dominant axis, the same inset the shader uses.
#[must_use]
pub fn cache_key(position: [f32; 3], normal: [f32; 3], size: f32) -> u32 {
    let axis = dominant_axis(normal);
    let inset = axis_component(axis);
    let mut cell = [0i32; 3];
    for i in 0..3 {
        cell[i] = floor_i32(position[i] / size - inset[i]);
    }
    let mut h = hash_cell3(cell);
    h = lighting_hash(h ^ lighting_hash(axis ^ lighting_hash(size.to_bits())));
    h.max(1)
}

/// Slot `step` of the probe window for `key`.
#[must_use]
pub fn probe_slot(key: u32, step: u32) -> u32 {
    let start = lighting_hash(key ^ 0x9e37_79b9);
    start.wrapping_add(step) & (SLOTS - 1)
}

/// Walk the same 16 slots the shader walks.
#[must_use]
pub fn classify_window(keys: &[u32], key: u32) -> CacheHit {
    let mut slot = None;
    let mut copies = 0u32;
    let mut live = 0u32;
    let mut foreign = 0u32;
    for step in 0..SEARCH {
        let at = probe_slot(key, step);
        let Some(stored) = keys.get(usize_from_u32(at)).copied() else {
            continue;
        };
        if stored == 0 {
            continue;
        }
        live = live.saturating_add(1);
        if stored == key {
            copies = copies.saturating_add(1);
            if slot.is_none() {
                slot = Some(at);
            }
        } else {
            foreign = foreign.saturating_add(1);
        }
    }
    CacheHit {
        key,
        slot,
        copies,
        live,
        foreign,
    }
}

/// Build the cell record from the seven vec4s the cache stores for one slot.
///
/// Order: position, normal, irradiance, world, object, sparse, field stamp.
/// `None` is a miss: the numeric fields stay zero.
#[must_use]
pub fn stored_cell(query: CacheQuery, hit: CacheHit, records: Option<[[f32; 4]; 7]>) -> StoredCell {
    let Some(words) = records else {
        return StoredCell {
            hit,
            size: query.size,
            position: [0.0; 3],
            normal: [0.0; 3],
            stored_size: 0.0,
            radiance: [0.0; 3],
            weight: 0.0,
            last_read: 0,
            updated: 0,
            shown_frame: 0,
            field: [0.0; 3],
            field_support: 0.0,
            field_stamp: 0,
            seed: [0.0; 3],
            seed_weight: 0.0,
        };
    };
    StoredCell {
        hit,
        size: query.size,
        position: [words[0][0], words[0][1], words[0][2]],
        normal: [words[1][0], words[1][1], words[1][2]],
        stored_size: words[1][3],
        radiance: [words[2][0], words[2][1], words[2][2]],
        weight: words[2][3],
        last_read: words[0][3].to_bits(),
        updated: words[4][1].to_bits(),
        shown_frame: words[3][2].to_bits(),
        field: unpack_rgb9e5(words[3][1].to_bits()),
        field_support: words[5][3],
        field_stamp: words[6][3].to_bits(),
        seed: unpack_rgb9e5(words[5][1].to_bits()),
        seed_weight: words[5][2],
    }
}

/// Shared-exponent RGB, the same layout as `lighting_unpack_rgb9e5`.
#[must_use]
pub fn unpack_rgb9e5(word: u32) -> [f32; 3] {
    let exp = i32::try_from(word >> 27).unwrap_or(0) - 24;
    let scale = exp2_i32(exp);
    [
        f32::from(u16_bits(word & 511)) * scale,
        f32::from(u16_bits((word >> 9) & 511)) * scale,
        f32::from(u16_bits((word >> 18) & 511)) * scale,
    ]
}

fn u16_bits(v: u32) -> u16 {
    u16::try_from(v).unwrap_or(0)
}

fn usize_from_u32(v: u32) -> usize {
    usize::try_from(v).unwrap_or(0)
}

fn exp2_i32(exp: i32) -> f32 {
    if !(-126..=127).contains(&exp) {
        return if exp < 0 { 0.0 } else { f32::MAX };
    }
    let biased = exp + 127;
    f32::from_bits(u32::try_from(biased).unwrap_or(0) << 23)
}

/// `floor` as `i32`, for cell coordinates. Values outside `i32` saturate.
pub(crate) fn floor_i32(v: f32) -> i32 {
    let t = v.floor();
    if !t.is_finite() {
        return 0;
    }
    let bits = t.to_bits();
    if bits.trailing_zeros() >= 31 {
        return 0;
    }
    let sign = bits >> 31 != 0;
    let biased = (bits >> 23) & 0xff;
    if biased == 0 {
        return 0;
    }
    let exp = i32::try_from(biased).unwrap_or(127) - 127;
    if exp < 0 {
        return 0;
    }
    if exp >= 31 {
        return if sign { i32::MIN } else { i32::MAX };
    }
    let mant = (bits & 0x7f_ffff) | 0x80_0000;
    let exp_u = u32::try_from(exp).unwrap_or(0);
    let mag_u = if exp_u >= 23 {
        mant << (exp_u - 23)
    } else {
        mant >> (23 - exp_u)
    };
    let mag = i32::try_from(mag_u).unwrap_or(i32::MAX);
    if sign { mag.saturating_neg() } else { mag }
}

fn lighting_hash(mut v: u32) -> u32 {
    v ^= v >> 16;
    v = v.wrapping_mul(0x7feb_352d);
    v ^= v >> 15;
    v = v.wrapping_mul(0x846c_a68b);
    v ^= v >> 16;
    v
}

fn i32_bits(v: i32) -> u32 {
    u32::from_le_bytes(v.to_le_bytes())
}

fn hash_cell3(cell: [i32; 3]) -> u32 {
    let x = i32_bits(cell[0]);
    let y = i32_bits(cell[1]);
    let z = i32_bits(cell[2]);
    lighting_hash(x ^ lighting_hash(y ^ lighting_hash(z ^ lighting_hash(0))))
}

fn dominant_axis(normal: [f32; 3]) -> u32 {
    let ax = normal[0].abs();
    let ay = normal[1].abs();
    let az = normal[2].abs();
    if ax >= ay && ax >= az {
        u32::from(normal[0] < 0.0)
    } else if ay >= az {
        2 + u32::from(normal[1] < 0.0)
    } else {
        4 + u32::from(normal[2] < 0.0)
    }
}

/// Axis-aligned centre of the hash cell, `lighting_sc_offer_origin`.
///
/// Along the normal this centre is up to half a cell off the face. A face that
/// sits on a cell boundary (the hall roof at y = 3) lands on the centre. A face
/// between boundaries does not.
#[must_use]
pub fn offer_origin(position: [f32; 3], normal: [f32; 3], size: f32) -> [f32; 3] {
    let inset = axis_component(dominant_axis(normal));
    let mut origin = [0.0f32; 3];
    for i in 0..3 {
        let cell = floor_i32(position[i] / size - inset[i]);
        origin[i] = (f32_i32(cell) + 0.5 + inset[i]) * size;
    }
    origin
}

fn f32_i32(v: i32) -> f32 {
    if v == 0 {
        return 0.0;
    }
    let neg = v < 0;
    let mut n = v.unsigned_abs();
    let mut acc = 0.0f32;
    let mut bit = 1.0f32;
    while n != 0 {
        if n & 1 == 1 {
            acc += bit;
        }
        bit += bit;
        n >>= 1;
    }
    if neg { -acc } else { acc }
}

fn axis_component(axis: u32) -> [f32; 3] {
    let mut v = [0.0f32; 3];
    let sign = if axis & 1 == 0 { 0.5 } else { -0.5 };
    let index = usize_from_u32(axis >> 1);
    if let Some(slot) = v.get_mut(index) {
        *slot = sign;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::{cache_key, floor_i32, offer_origin, unpack_rgb9e5};

    #[test]
    fn floor_matches_the_cell_coordinates() {
        assert_eq!(floor_i32(3.2), 3);
        assert_eq!(floor_i32(12.0), 12);
        assert_eq!(floor_i32(-0.2), -1);
        assert_eq!(floor_i32(-2.0), -2);
        assert_eq!(floor_i32(0.0), 0);
    }

    #[test]
    fn a_roof_and_a_wall_do_not_share_a_key() {
        let roof = cache_key([14.0, 3.0, 20.0], [0.0, -1.0, 0.0], 0.25);
        let wall = cache_key([14.0, 1.7, 20.0], [0.0, 0.0, -1.0], 0.25);
        assert_ne!(roof, 0);
        assert_ne!(wall, 0);
        assert_ne!(roof, wall);
    }

    #[test]
    fn an_off_grid_wall_centre_is_behind_the_face() {
        let origin = offer_origin([0.15, 1.9912, 16.6592], [1.0, 0.0, 0.0], 0.125);
        assert!((origin[0] - 0.125).abs() < 1.0e-4);
        assert!(origin[0] < 0.15);
        let roof = offer_origin([11.81, 3.0, 22.29], [0.0, -1.0, 0.0], 0.125);
        assert!((roof[1] - 3.0).abs() < 1.0e-4);
    }

    #[test]
    fn rgb9e5_round_trips_a_small_colour() {
        let word = 15u32 << 27;
        let rgb = unpack_rgb9e5(word);
        assert!(rgb[0].abs() < 1.0e-6);
        assert!(rgb[1].abs() < 1.0e-6);
        assert!(rgb[2].abs() < 1.0e-6);
    }
}
