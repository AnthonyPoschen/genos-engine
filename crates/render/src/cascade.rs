//! Level-0 footprint sizing for the sparse cascade.
#![allow(dead_code)]
//!
//! The shader in `lighting_cascade.glsl` is the copy the picture runs. These
//! functions are the same rule, so a test can check a ring without a GPU.
//! GI v1 stays removed.

/// World size over distance. A ring start keeps this projected size.
pub const PROJ: f32 = 0.05;
/// First ring starts at one metre of Chebyshev distance.
pub const RING_NEAR: f32 = 1.0;
/// Last ring index. Farther samples stay on this ring.
pub const RING_MAX: u32 = 6;
/// Outer fraction of a ring that blends into the next ring.
pub const OVERLAP: f32 = 0.1;

/// Chebyshev distance from the eye. The rings are cubes, not spheres.
pub fn chebyshev(eye: [f32; 3], pos: [f32; 3]) -> f32 {
    let x = (pos[0] - eye[0]).abs();
    let y = (pos[1] - eye[1]).abs();
    let z = (pos[2] - eye[2]).abs();
    x.max(y).max(z)
}

fn log_ring(cheb: f32) -> f32 {
    (cheb.max(RING_NEAR) / RING_NEAR).log2()
}

fn ring_max_f() -> f32 {
    let mut edge = 0.0;
    let mut left = RING_MAX;
    while left > 0 {
        edge += 1.0;
        left -= 1;
    }
    edge
}

fn scale_of(index: u32) -> f32 {
    let mut scale = 1.0;
    let mut left = index.min(RING_MAX);
    while left > 0 {
        scale *= 2.0;
        left -= 1;
    }
    scale
}

/// Ring index. Distance doubles from one ring to the next.
pub fn ring(cheb: f32) -> u32 {
    let level = log_ring(cheb).floor();
    if level <= 0.0 {
        return 0;
    }
    let mut index = 0;
    let mut edge = 0.0;
    while index < RING_MAX {
        edge += 1.0;
        if level < edge {
            break;
        }
        index += 1;
    }
    index
}

/// World size of one level-0 cell on `ring`. Doubles with the ring, so the
/// projected size at the ring start stays [`PROJ`].
pub fn cell_size(ring: u32) -> f32 {
    PROJ * RING_NEAR * scale_of(ring)
}

/// Finest bounce cell. Matches `LIGHTING_SC_HIT`. A cell this size keeps the
/// full trace.
pub const FINE: f32 = 0.25;
/// Trace start for the finest cell. Contact, not a 1 m shell.
pub const TRACE_NEAR: f32 = 1.0e-4;
/// Far end of the last shell, and of the finest cell.
pub const TRACE_FAR: f32 = 200.0;
/// Last upper-level index. Its shell runs out to [`TRACE_FAR`].
pub const INTERVAL_MAX: f32 = 6.0;

/// Ray interval for an upper world cell of `size` metres: `[4S, 8S]`,
/// clipped at [`TRACE_FAR`]. Level-0 uses [`l0_interval`].
pub fn interval(size: f32) -> (f32, f32) {
    if size.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return (TRACE_NEAR, TRACE_FAR);
    }
    let near = (4.0 * size).max(TRACE_NEAR);
    let far = (8.0 * size).min(TRACE_FAR);
    if near >= TRACE_FAR {
        return (TRACE_FAR, TRACE_FAR);
    }
    (near, far.max(near))
}

/// Level-0 ray interval: `[ε, 8s]`, clipped at [`TRACE_FAR`].
pub fn l0_interval(size: f32) -> (f32, f32) {
    (TRACE_NEAR, (8.0 * size.max(0.0)).clamp(TRACE_NEAR, TRACE_FAR))
}

/// True when a parent corner survives the plane and facing gates.
pub fn parent_gate(facing: f32, plane: f32, half_cell: f32, stale: bool, weight: f32) -> bool {
    !stale && facing > 0.0 && plane.abs() <= half_cell && weight > 0.5
}

/// Interval merge along one ray. A hit in `[ε, 8s]` is the nearer sample.
/// A miss takes the parent (`β = 1`).
pub fn interval_merge(hit: Option<[f32; 3]>, parent: [f32; 3]) -> [f32; 3] {
    match hit {
        Some(near) => [near[0].max(0.0), near[1].max(0.0), near[2].max(0.0)],
        None => [parent[0].max(0.0), parent[1].max(0.0), parent[2].max(0.0)],
    }
}

/// New-sample weight for the level-0 EMA. At least 0.5. Grows with the
/// luminance gap against the voxel's own ray spread (`1/sqrt(rays)`).
pub fn ema_weight(current: f32, prior: f32, rays: u32) -> f32 {
    let count = f32::from(u16::try_from(rays.max(1)).unwrap_or(u16::MAX));
    let spread = current.abs().max(1.0e-6) / count.sqrt();
    let gap = (current - prior).abs();
    (gap / (3.0 * spread)).clamp(0.5, 1.0)
}

/// Weight of the next ring. Zero until the outer tenth of this ring.
pub fn overlap_blend(cheb: f32) -> f32 {
    let level = log_ring(cheb);
    if level >= ring_max_f() {
        return 0.0;
    }
    let frac = level - level.floor();
    let start = 1.0 - OVERLAP;
    if frac <= start {
        return 0.0;
    }
    (frac - start) / OVERLAP
}


fn floor_i32(v: f32) -> i32 {
    crate::cache_probe::floor_i32(v)
}

fn i32_f32(v: i32) -> f32 {
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

/// Finest world-grid size. `2^n / 16` m, n from 0 to [`L0_MAX`].
pub const GRID_BASE: f32 = 0.0625;
/// Last level-0 index. Size is 32 m.
pub const L0_MAX: u32 = 9;
/// Angular size of a level-0 box.
pub const FOOTPRINT_K: f32 = 0.03125;
/// Floor on `|n · v|`. Caps the grazing boost at four times, two grid levels.
pub const GRAZE_MIN: f32 = 0.0625;
/// Lower edge of the quarter-level hysteresis band, `2^-0.25`.
pub const HYST_LO: f32 = 0.840_896_4;
/// Upper edge of the quarter-level hysteresis band, `2^1.25`.
pub const HYST_HI: f32 = 2.378_414_2;

/// Dominant axis and sign. Matches `lighting_axis`.
#[must_use]
pub fn axis(n: [f32; 3]) -> u32 {
    let ax = n[0].abs();
    let ay = n[1].abs();
    let az = n[2].abs();
    if ax >= ay && ax >= az {
        u32::from(n[0] < 0.0)
    } else if ay >= az {
        2 + u32::from(n[1] < 0.0)
    } else {
        4 + u32::from(n[2] < 0.0)
    }
}

/// Unit vector for [`axis`].
#[must_use]
pub fn axis_vec(axis: u32) -> [f32; 3] {
    let mut v = [0.0; 3];
    let component = usize::try_from(axis >> 1).unwrap_or(0).min(2);
    v[component] = if axis & 1 == 0 { 1.0 } else { -1.0 };
    v
}

/// World size of a level-0 cell at `level`.
#[must_use]
pub fn l0_size(level: u32) -> f32 {
    let mut size = GRID_BASE;
    let mut left = level.min(L0_MAX);
    while left > 0 {
        size *= 2.0;
        left -= 1;
    }
    size
}

/// Wanted cube size at the cell centre, using the axis normal.
#[must_use]
pub fn want(centre: [f32; 3], n_axis: [f32; 3], eye: [f32; 3]) -> f32 {
    let rel = [centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]];
    let dist = (rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2]).sqrt();
    let inv = if dist > 1.0e-6 { 1.0 / dist } else { 0.0 };
    let view = [rel[0] * inv, rel[1] * inv, rel[2] * inv];
    let cos = (n_axis[0] * view[0] + n_axis[1] * view[1] + n_axis[2] * view[2])
        .abs()
        .max(GRAZE_MIN);
    FOOTPRINT_K * dist / cos.sqrt()
}

/// Cell after a half-cell inset along `n_axis`.
#[must_use]
pub fn l0_cell(pos: [f32; 3], n_axis: [f32; 3], size: f32) -> [i32; 3] {
    [
        floor_i32(pos[0] / size - n_axis[0] * 0.5),
        floor_i32(pos[1] / size - n_axis[1] * 0.5),
        floor_i32(pos[2] / size - n_axis[2] * 0.5),
    ]
}

/// Centre of an axis-aligned cell of `size`.
#[must_use]
pub fn l0_centre(cell: [i32; 3], size: f32) -> [f32; 3] {
    [
        (i32_f32(cell[0]) + 0.5) * size,
        (i32_f32(cell[1]) + 0.5) * size,
        (i32_f32(cell[2]) + 0.5) * size,
    ]
}

/// Largest grid level with `S <= want` at that cell's centre.
#[must_use]
pub fn l0_level_plain(pos: [f32; 3], n: [f32; 3], eye: [f32; 3]) -> u32 {
    let n_axis = axis_vec(axis(n));
    let mut level = L0_MAX;
    loop {
        let size = l0_size(level);
        let cell = l0_cell(pos, n_axis, size);
        let centre = l0_centre(cell, size);
        if size <= want(centre, n_axis, eye) {
            return level;
        }
        if level == 0 {
            return 0;
        }
        level -= 1;
    }
}

/// A live leaf from last frame keeps its size while `want / S` stays in
/// [`HYST_LO`], [`HYST_HI`]. `prev` is that size's level, if any.
#[must_use]
pub fn l0_level(pos: [f32; 3], n: [f32; 3], eye: [f32; 3], prev: Option<u32>) -> u32 {
    let plain = l0_level_plain(pos, n, eye);
    let Some(prev) = prev else {
        return plain;
    };
    if prev > L0_MAX {
        return plain;
    }
    let n_axis = axis_vec(axis(n));
    let size = l0_size(prev);
    let cell = l0_cell(pos, n_axis, size);
    let centre = l0_centre(cell, size);
    let ratio = want(centre, n_axis, eye) / size;
    if (HYST_LO..=HYST_HI).contains(&ratio) {
        prev
    } else {
        plain
    }
}

/// One live level-0 record from the current table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level0Voxel {
    pub cell: [i32; 3],
    pub level: u32,
    pub axis: u32,
    pub pos: [f32; 3],
}

impl Level0Voxel {
    /// Key used to compare leaves across frames.
    #[must_use]
    pub fn key(self) -> (i32, i32, i32, u32, u32) {
        (self.cell[0], self.cell[1], self.cell[2], self.level, self.axis)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        axis, axis_vec, cell_size, chebyshev, ema_weight, interval, interval_merge, l0_cell,
        l0_centre, l0_interval, l0_level, l0_level_plain, l0_size, overlap_blend, parent_gate,
        ring, want, FINE, L0_MAX, OVERLAP, PROJ, RING_MAX, RING_NEAR, TRACE_FAR, TRACE_NEAR,
    };

    #[test]
    fn chebyshev_is_the_longest_axis() {
        let eye = [1.0, 2.0, 3.0];
        assert_eq!(chebyshev(eye, [1.0, 2.0, 3.0]), 0.0);
        assert_eq!(chebyshev(eye, [4.0, 2.5, 3.2]), 3.0);
    }

    #[test]
    fn a_ring_start_keeps_the_projected_size() {
        let mut distance = RING_NEAR;
        for index in 0..=RING_MAX {
            assert_eq!(ring(distance), index);
            let ratio = cell_size(index) / distance;
            assert!(
                (ratio - PROJ).abs() < 1.0e-5,
                "ring {index} projects {ratio}, wanted {PROJ}"
            );
            assert_eq!(overlap_blend(distance), 0.0);
            distance *= 2.0;
        }
    }

    #[test]
    fn the_outer_tenth_blends_into_the_next_ring() {
        let start = 1.0 - OVERLAP;
        let mid = start + OVERLAP * 0.5;
        let distance = RING_NEAR * 2.0_f32.powf(2.0 + mid);
        assert_eq!(ring(distance), 2);
        let blend = overlap_blend(distance);
        assert!((blend - 0.5).abs() < 1.0e-4, "overlap blend {blend}");
        assert_eq!(
            overlap_blend(RING_NEAR * 2.0_f32.powf(2.0 + start * 0.5)),
            0.0
        );
        assert_eq!(overlap_blend(RING_NEAR * 2.0_f32.powf(6.0 + 0.99)), 0.0);
        assert_eq!(RING_MAX, 6);
    }

    #[test]
    fn an_upper_cell_traces_four_to_eight_times_its_size() {
        assert_eq!(interval(FINE), (1.0, 2.0));
        assert_eq!(interval(0.0), (TRACE_NEAR, TRACE_FAR));
        assert_eq!(interval(0.5), (2.0, 4.0));
        assert_eq!(interval(1.0), (4.0, 8.0));
        assert_eq!(interval(2.0), (8.0, 16.0));
        assert_eq!(interval(4.0), (16.0, 32.0));
        assert_eq!(interval(8.0), (32.0, 64.0));
        assert_eq!(interval(16.0), (64.0, 128.0));
        assert_eq!(interval(32.0), (128.0, TRACE_FAR));
        assert_eq!(l0_interval(FINE), (TRACE_NEAR, 2.0));
        assert_eq!(l0_interval(0.0625), (TRACE_NEAR, 0.5));
    }

    #[test]
    fn a_hit_replaces_the_parent_and_a_miss_keeps_it() {
        assert_eq!(
            interval_merge(Some([0.2, 0.0, 0.0]), [9.0, 0.0, 0.0]),
            [0.2, 0.0, 0.0]
        );
        assert_eq!(
            interval_merge(None, [0.4, 0.1, 0.0]),
            [0.4, 0.1, 0.0]
        );
    }

    #[test]
    fn the_level0_ema_weight_is_at_least_a_half() {
        assert!((ema_weight(1.0, 1.0, 32) - 0.5).abs() < 1.0e-5);
        let hot = ema_weight(4.0, 0.1, 32);
        assert!(hot > 0.5, "a large gap must shrink the prior, got {hot}");
        assert!(hot <= 1.0);
    }

    #[test]
    fn a_thin_wall_refuses_the_far_side() {
        let half = 0.125;
        assert!(parent_gate(1.0, 0.0, half, false, 1.0));
        assert!(!parent_gate(0.0, 0.0, half, false, 1.0));
        assert!(!parent_gate(-0.2, 0.0, half, false, 1.0));
        assert!(!parent_gate(1.0, 0.2, half, false, 1.0));
        assert!(!parent_gate(1.0, 0.0, half, true, 1.0));
        assert!(!parent_gate(1.0, 0.0, half, false, 0.0));
    }

    #[test]
    fn the_shader_uses_this_ring_rule() {
        let shader = include_str!("../shaders/lighting_cascade.glsl");
        assert!(
            shader.contains("const float CASCADE_PROJ = 0.05;"),
            "{shader}"
        );
        assert!(shader.contains("const float CASCADE_RING_NEAR = 1.0;"));
        assert!(shader.contains("const uint CASCADE_RING_MAX = 6u;"));
        assert!(shader.contains("const float CASCADE_OVERLAP = 0.1;"));
        assert!(shader.contains("hash_cell3"));
        let gpu = include_str!("lighting_gpu.rs");
        assert!(gpu.contains("CASCADE_LEVEL0_CAP: u64 = 16384"));
        assert!(gpu.contains("CASCADE_LEVEL0_STRIDE: u64 = 12"));
        assert!(gpu.contains("CASCADE_LEVEL0_TABLES: u64 = 2"));
        assert!(gpu.contains("CASCADE_LEVEL0_PIXEL_WORDS: u64 = 4"));
        assert!(gpu.contains("lighting_level0_table_words()"));
        assert!(gpu.contains("CASCADE_L0_RAYS: u64 = 32"));
        assert!(gpu.contains("cascade_l0_trace_pipe"));
        assert!(gpu.contains("cascade_l0_shade_pipe"));
        assert!(shader.contains("const float CASCADE_GRID_BASE = 0.0625;"));
        assert!(shader.contains("const float CASCADE_FOOTPRINT_K = 0.03125;"));
        assert!(shader.contains("lighting_cascade_l0_level_plain"));
        assert!(shader.contains("lighting_cascade_l0_cell"));
        let direct = include_str!("../shaders/lighting_direct.comp");
        assert!(
            direct.contains("lighting_cascade_insert(idx, pos, n)"),
            "the g-buffer pass inserts the surface sample"
        );
        assert!(
            direct.contains("&& idx < pixels;"),
            "a cache ray is not a product pixel, so distant lamps stay in the cell"
        );
        let trace = include_str!("../shaders/lighting_trace.comp");
        assert!(
            trace.contains("lighting_cascade_ray_range"),
            "upper-cell rays use [4S, 8S]"
        );
        assert!(shader.contains("lighting_cascade_interval"));
        assert!(shader.contains("lighting_cascade_l0_interval"));
        assert!(shader.contains("const uint CASCADE_L0_RAYS = 32u;"));
        assert!(shader.contains("lighting_cascade_merge"));
        assert!(shader.contains("lighting_cascade_neighbors"));
        assert!(
            !shader.contains("lighting_sh_compose_shown"),
            "publish does not read the SH lattice"
        );
        assert!(!shader.contains("lighting_cascade_ancestor"));
        assert!(!shader.contains("lighting_cascade_children"));
        assert!(!shader.contains("lighting_cascade_inward"));
        assert!(!shader.contains("lighting_cascade_voxel("));
        assert!(!shader.contains("lighting_cascade_facing"));
        assert!(shader.contains("LIGHTING_DIAG_CASCADE_GAP = 11u"));
        assert!(shader.contains("LIGHTING_CELL_HALF * size"));
        let probe = include_str!("../shaders/lighting_screen_probe.glsl");
        assert!(
            probe.contains("lighting_cascade_published(pixel)"),
            "picture compose loads the published level-0 word"
        );
        let build = include_str!("../build.rs");
        assert!(
            build.contains("GENOS_LEVEL0_PICTURE"),
            "picture compose is the level-0 load"
        );
        assert!(gpu.contains("cascade_pub_pipe"));
        assert!(
            gpu.contains("A camera move does not rebuild the lattice"),
            "a move does not schedule the lattice drain"
        );
        assert!(
            !probe.contains("lighting_cascade_insert"),
            "picture compose does not insert into the level-0 hash"
        );
    }

    #[test]
    fn a_grid_plane_floor_snaps_to_one_cell() {
        let eye = [0.0, 1.8, 5.0];
        let n = [0.0, 1.0, 0.0];
        let n_axis = axis_vec(axis(n));
        let size = l0_size(0);
        let a = l0_cell([0.2, 1.0e-4, 1.3], n_axis, size);
        let b = l0_cell([0.2, -1.0e-4, 1.3], n_axis, size);
        assert_eq!(a, b, "depth noise on y=0 must not flip the cell");
        let raw_a = [
            (0.2 / size).floor(),
            (1.0e-4 / size).floor(),
            (1.3 / size).floor(),
        ];
        let raw_b = [
            (0.2 / size).floor(),
            (-1.0e-4 / size).floor(),
            (1.3 / size).floor(),
        ];
        assert_ne!(raw_a[1], raw_b[1], "the raw snap without an inset does flip");
        let _ = eye;
    }

    #[test]
    fn a_grazing_floor_is_at_most_two_levels_coarser() {
        let n = [0.0, 1.0, 0.0];
        let pos = [0.0, 0.0, 0.0];
        let face = l0_level_plain(pos, n, [0.0, 8.0, 0.0]);
        let graze = l0_level_plain(pos, n, [0.0, 0.5, 8.0]);
        assert!(
            graze <= face + 2,
            "graze {graze} is more than two levels above face {face} at the same distance"
        );
    }

    #[test]
    fn the_plain_rule_picks_the_largest_size_under_want() {
        let eye = [0.0, 1.8, 0.0];
        let n = [0.0, 0.0, 1.0];
        let pos = [0.0, 1.8, -8.0];
        let level = l0_level_plain(pos, n, eye);
        let n_axis = axis_vec(axis(n));
        let size = l0_size(level);
        let cell = l0_cell(pos, n_axis, size);
        let centre = l0_centre(cell, size);
        let needed = want(centre, n_axis, eye);
        assert!(size <= needed, "size {size} want {needed}");
        if level < L0_MAX {
            let coarse = l0_size(level + 1);
            let coarse_cell = l0_cell(pos, n_axis, coarse);
            let coarse_centre = l0_centre(coarse_cell, coarse);
            let coarse_needed = want(coarse_centre, n_axis, eye);
            assert!(
                coarse > coarse_needed,
                "the next size {coarse} should miss want {coarse_needed}"
            );
        }
    }

    #[test]
    fn hysteresis_keeps_a_leaf_inside_the_quarter_level_band() {
        let eye = [0.0, 1.8, 0.0];
        let n = [0.0, 0.0, 1.0];
        let pos = [0.0, 1.8, -8.0];
        let level = l0_level_plain(pos, n, eye);
        assert_eq!(l0_level(pos, n, eye, Some(level)), level);
        let kept = l0_level(pos, n, [0.0, 1.8, 0.2], Some(level));
        assert_eq!(kept, level, "a small eye move must keep the leaf");
    }
}
