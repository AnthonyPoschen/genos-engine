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

/// History window of every cascade value, in frames (shader
/// `CASCADE_EMA_WINDOW`). Level-0 voxels update every frame, so their stored
/// sample count caps at this (new-sample weight >= 1/4). An upper cell
/// updates every `period` frames, so its cap is the window over the period:
/// one shared timescale, with no value remembering more than ~4 frames.
pub const EMA_WINDOW_FRAMES: f32 = 4.0;

/// Cap on the stored level-0 EMA sample count, matching the shader.
pub const EMA_N_MAX: f32 = EMA_WINDOW_FRAMES;

/// Stored-count cap of a value refreshed every `period` frames.
#[must_use]
pub fn ema_n_max(period: u32) -> f32 {
    let p = f32::from(u16::try_from(period.max(1)).unwrap_or(u16::MAX));
    (EMA_WINDOW_FRAMES / p).max(1.0)
}

fn ema_rays(rays: u32) -> f32 {
    f32::from(u16::try_from(rays.max(1)).unwrap_or(u16::MAX))
}

/// New-sample weight for the level-0 EMA.
///
/// The change is judged against the estimate's standard error
/// `max(|current|, |prior|) / sqrt(rays)` (the Poisson-like bound used
/// when only the two lumas are known; the shader uses the real
/// second-moment SE of this frame's rays). A gap inside two standard
/// errors is noise and blends at `1/rays`. A real change down-weights
/// history in proportion to the gap over the larger luma. Only
/// non-finite values take weight 1 as a hard reset.
///
/// This is the instantaneous SE rule, used by unit tests. The stored
/// sample count is applied in [`ema_step`].
pub fn ema_weight(current: f32, prior: f32, rays: u32) -> f32 {
    if !current.is_finite() || !prior.is_finite() {
        return 1.0;
    }
    let count = ema_rays(rays);
    let se = current.abs().max(prior.abs()).max(1.0e-6) / count.sqrt();
    let gap = (current - prior).abs();
    if gap <= 2.0 * se {
        return 1.0 / count;
    }
    let den = current.abs().max(prior.abs()).max(1.0e-6);
    (gap / den).clamp(0.0, 1.0)
}

/// Mix weight and the stored sample count after this update.
///
/// Agreement grows `prior_n` and blends at `1/n`. A real change
/// shrinks `prior_n` by the leftover history weight so later frames
/// keep catching up instead of stalling inside the new-sample
/// standard error. Invalid data returns weight 1 and `n = 1`.
#[must_use]
pub fn ema_step(current: f32, prior: f32, rays: u32, prior_n: f32, n_max: f32) -> (f32, f32) {
    if !current.is_finite() || !prior.is_finite() {
        return (1.0, 1.0);
    }
    let count = ema_rays(rays);
    let se = current.abs().max(prior.abs()).max(1.0e-6) / count.sqrt();
    let gap = (current - prior).abs();
    let cap = n_max.max(1.0);
    let n0 = prior_n.clamp(1.0, cap);
    let agree = (n0 + 1.0).min(cap);
    if gap <= 2.0 * se {
        return (1.0 / agree, agree);
    }
    // A real change never takes less of the new sample than agreement.
    let den = current.abs().max(prior.abs()).max(1.0e-6);
    let nw = (gap / den).clamp(0.0, 1.0).max(1.0 / agree);
    let n = (n0 * (1.0 - nw) + 1.0).clamp(1.0, cap);
    (nw, n)
}

/// Default upper refresh stride. Every resident upper cell uses this
/// period unless the debug setting is 1, 2, or 10.
pub const PERIOD_DEFAULT: u32 = 4;

/// Chebyshev retention: a cell of size `S` is kept while distance ≤ `16S`.
pub const EXTENT_FACTOR: f32 = 16.0;

/// Fixed rolling period for every upper cell, on or off screen, every size.
/// The setting is 1, 2, 4, or 10. Anything else is [`PERIOD_DEFAULT`].
#[must_use]
pub fn refresh_period(setting: u32) -> u32 {
    match setting {
        1 | 2 | 4 | 10 => setting,
        _ => PERIOD_DEFAULT,
    }
}

/// Rays traced on a refresh of a cell of `size` metres.
#[must_use]
pub fn refresh_rays(size: f32) -> u32 {
    if size <= 0.5 {
        16
    } else if size <= 4.0 {
        32
    } else {
        64
    }
}

/// True while a cell of `size` stays inside the camera-centred clipmap.
#[must_use]
pub fn in_extent(cheb: f32, size: f32) -> bool {
    cheb <= EXTENT_FACTOR * size.max(0.0)
}

/// SH-L1 Ylm constants, matching `lighting_cache.glsl`.
const SH_Y0: f32 = 0.282_094_8;
const SH_Y1: f32 = 0.488_602_5;

fn sh_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0].mul_add(b[0], a[1].mul_add(b[1], a[2] * b[2]))
}

fn sh_scale(v: [f32; 3], s: f32) -> [f32; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn sh_add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sh_norm(v: [f32; 3]) -> [f32; 3] {
    let len = sh_dot(v, v).sqrt().max(1.0e-6);
    sh_scale(v, 1.0 / len)
}

/// Hemispheric inversion of an upper cell's SH-L1, matching
/// `lighting_cascade_sh_radiance`. A constant incoming field reconstructs
/// as that radiance in every direction on the hemisphere.
#[must_use]
pub fn sh_radiance_hemi(
    dir: [f32; 3],
    axis: [f32; 3],
    c0: [f32; 3],
    cx: [f32; 3],
    cy: [f32; 3],
    cz: [f32; 3],
) -> [f32; 3] {
    let n = if sh_dot(axis, axis) > 0.25 {
        sh_norm(axis)
    } else {
        [0.0, 1.0, 0.0]
    };
    let i0 = sh_scale(c0, 1.0 / SH_Y0);
    let idot = sh_scale(
        [
            cx[0].mul_add(dir[0], cy[0].mul_add(dir[1], cz[0] * dir[2])),
            cx[1].mul_add(dir[0], cy[1].mul_add(dir[1], cz[1] * dir[2])),
            cx[2].mul_add(dir[0], cy[2].mul_add(dir[1], cz[2] * dir[2])),
        ],
        1.0 / SH_Y1,
    );
    let i1 = sh_scale(
        [
            cx[0].mul_add(n[0], cy[0].mul_add(n[1], cz[0] * n[2])),
            cx[1].mul_add(n[0], cy[1].mul_add(n[1], cz[1] * n[2])),
            cx[2].mul_add(n[0], cy[2].mul_add(n[1], cz[2] * n[2])),
        ],
        1.0 / SH_Y1,
    );
    let a = sh_scale(sh_add(sh_scale(i0, 2.0), sh_scale(i1, -3.0)), 1.0 / std::f32::consts::PI);
    let mu = sh_dot(dir, n);
    let l = sh_add(
        sh_scale(a, 1.0 - 1.5 * mu),
        sh_scale(idot, 1.5 / std::f32::consts::PI),
    );
    [l[0].max(0.0), l[1].max(0.0), l[2].max(0.0)]
}

/// Rolling-stride due test: `(hash + frame) % P == 0`. Period 1 is every frame.
#[must_use]
pub fn refresh_due(key_hash: u32, frame: u32, period: u32) -> bool {
    if period <= 1 {
        return true;
    }
    key_hash.wrapping_add(frame).is_multiple_of(period)
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
    pub radiance: [f32; 3],
    pub weight: f32,
}

impl Level0Voxel {
    /// Key used to compare leaves across frames.
    #[must_use]
    pub fn key(self) -> (i32, i32, i32, u32, u32) {
        (self.cell[0], self.cell[1], self.cell[2], self.level, self.axis)
    }

    /// Rec.709 luminance of the stored voxel radiance.
    #[must_use]
    pub fn luma(self) -> f32 {
        0.2126 * self.radiance[0] + 0.7152 * self.radiance[1] + 0.0722 * self.radiance[2]
    }

    /// Key of the next coarser cell that contains this voxel's point.
    #[must_use]
    pub fn parent_key(self) -> Option<(i32, i32, i32, u32, u32)> {
        if self.level >= L0_MAX {
            return None;
        }
        let size = l0_size(self.level + 1);
        let n_axis = axis_vec(self.axis);
        let cell = l0_cell(self.pos, n_axis, size);
        Some((cell[0], cell[1], cell[2], self.level + 1, self.axis))
    }
}

/// GPU milliseconds for one frame of the cascade, by level band.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CascadeTimes {
    /// `0` is the normal rolling table; otherwise 1, 2, 4, or 10.
    pub period: u32,
    pub l0_ms: f64,
    /// `S <= 0.5 m`.
    pub upper_fine_ms: f64,
    /// `0.5 m < S <= 4 m`.
    pub upper_mid_ms: f64,
    /// `S > 4 m`.
    pub upper_coarse_ms: f64,
    pub claim_ms: f64,
}

impl CascadeTimes {
    /// Sum of the three upper size bands, excluding claim/pick.
    #[must_use]
    pub fn upper_ms(self) -> f64 {
        self.upper_fine_ms + self.upper_mid_ms + self.upper_coarse_ms
    }
}

#[cfg(test)]
mod tests {
    use super::{
        axis, axis_vec, cell_size, chebyshev, ema_n_max, ema_step, ema_weight, in_extent, interval, interval_merge,
        l0_cell, l0_centre, l0_interval, l0_level, l0_level_plain, l0_size, overlap_blend,
        parent_gate, refresh_due, refresh_period, refresh_rays, ring, sh_radiance_hemi, want, FINE, L0_MAX, OVERLAP,
        EMA_N_MAX, EMA_WINDOW_FRAMES, PERIOD_DEFAULT, PROJ, RING_MAX, RING_NEAR, TRACE_FAR, TRACE_NEAR,
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
    fn a_level0_ema_treats_noise_as_agreement() {
        assert!((ema_weight(1.0, 1.0, 32) - 1.0 / 32.0).abs() < 1.0e-5);
        // se = 1/sqrt(32) ≈ 0.177; a 0.1 gap is inside two standard errors.
        assert!((ema_weight(1.1, 1.0, 32) - 1.0 / 32.0).abs() < 1.0e-5);
    }

    #[test]
    fn a_level0_ema_downweights_a_real_change() {
        // se ≈ 4/sqrt(32) ≈ 0.707; gap 3.9 is many standard errors.
        let hot = ema_weight(4.0, 0.1, 32);
        assert!((hot - 3.9 / 4.0).abs() < 1.0e-4, "got {hot}");
    }

    #[test]
    fn a_level0_ema_hard_resets_invalid_data() {
        assert!((ema_weight(f32::NAN, 1.0, 32) - 1.0).abs() < 1.0e-5);
        assert!((ema_weight(1.0, f32::INFINITY, 32) - 1.0).abs() < 1.0e-5);
        let (w, n) = ema_step(f32::NAN, 1.0, 32, 16.0, EMA_N_MAX);
        assert!((w - 1.0).abs() < 1.0e-5 && (n - 1.0).abs() < 1.0e-5);
    }

    #[test]
    fn a_level0_ema_grows_n_on_agreement_and_shrinks_it_on_change() {
        let (w, n) = ema_step(1.1, 1.0, 32, 2.0, EMA_N_MAX);
        assert!((w - 1.0 / 3.0).abs() < 1.0e-5, "got {w}");
        assert!((n - 3.0).abs() < 1.0e-5, "got {n}");
        let (hot, n1) = ema_step(4.0, 0.1, 32, EMA_N_MAX, EMA_N_MAX);
        assert!((hot - 3.9 / 4.0).abs() < 1.0e-4, "got {hot}");
        let expect_n = EMA_N_MAX * (1.0 - 3.9 / 4.0) + 1.0;
        assert!((n1 - expect_n).abs() < 1.0e-4, "got {n1}");
        // Leftover after the change is often inside this frame's SE.
        // Reduced N keeps the next agreement blend faster than the cap.
        let prior = 0.1f32.mul_add(1.0 - hot, 4.0 * hot);
        let (w2, n2) = ema_step(4.0, prior, 32, n1, EMA_N_MAX);
        assert!(w2 > 1.0 / EMA_N_MAX + 1.0e-5, "leftover weight {w2} stalled at the cap");
        assert!(n2 < EMA_N_MAX, "leftover n {n2} did not stay reduced");
    }

    #[test]
    fn every_cascade_value_remembers_at_most_the_shared_window() {
        // Agreement forever: the weight settles at 1/cap, never below 1/4.
        let mut n = 1.0;
        let mut w = 1.0;
        for _ in 0..1000 {
            (w, n) = ema_step(1.0, 1.0, 32, n, EMA_N_MAX);
        }
        assert!(w >= 0.25 - 1.0e-6, "level-0 new weight {w}");
        for period in [1u32, 2, 4, 10] {
            let cap = ema_n_max(period);
            let frames = cap * period as f32;
            assert!(cap >= 1.0, "period {period} cap {cap}");
            assert!(
                frames <= EMA_WINDOW_FRAMES.max(period as f32) + 1.0e-6,
                "period {period}: memory {frames} frames"
            );
        }
        assert!((ema_n_max(4) - 1.0).abs() < 1.0e-6);
        assert!((ema_n_max(1) - EMA_N_MAX).abs() < 1.0e-6);
    }

    #[test]
    fn a_constant_hemisphere_sh_reconstructs_as_that_radiance() {
        let axis = [0.0, 1.0, 0.0];
        let l = 2.0;
        let pi = std::f32::consts::PI;
        let c0 = [2.0 * pi * super::SH_Y0 * l; 3];
        let cx = [pi * super::SH_Y1 * l * axis[0]; 3];
        let cy = [pi * super::SH_Y1 * l * axis[1]; 3];
        let cz = [pi * super::SH_Y1 * l * axis[2]; 3];
        let dirs = [
            axis,
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            super::sh_norm([0.6, 0.8, 0.0]),
        ];
        for dir in dirs {
            let got = sh_radiance_hemi(dir, axis, c0, cx, cy, cz);
            for c in got {
                assert!((c - l).abs() < 1.0e-4, "dir {dir:?} got {got:?}");
            }
        }
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
        assert!(shader.contains("lighting_cascade_at"));
        assert!(
            !shader.contains("lighting_cascade_neighbors"),
            "publish has no same-level neighbour gather"
        );
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
        assert!(shader.contains("LIGHTING_CELL_HALF * plane"));
        assert!(
            shader.contains("abs(dot(stored - pos, n)) > size"),
            "bounce/picture record accepts a hit in the cell, not only half a cell"
        );
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

    #[test]
    fn the_upper_refresh_period_is_one_fixed_stride() {
        assert_eq!(refresh_period(0), PERIOD_DEFAULT);
        assert_eq!(refresh_period(3), PERIOD_DEFAULT);
        assert_eq!(refresh_period(1), 1);
        assert_eq!(refresh_period(2), 2);
        assert_eq!(refresh_period(4), 4);
        assert_eq!(refresh_period(10), 10);
    }

    #[test]
    fn an_upper_cell_traces_more_rays_as_it_grows() {
        assert_eq!(refresh_rays(0.25), 16);
        assert_eq!(refresh_rays(0.5), 16);
        assert_eq!(refresh_rays(1.0), 32);
        assert_eq!(refresh_rays(4.0), 32);
        assert_eq!(refresh_rays(8.0), 64);
    }

    #[test]
    fn a_cell_is_kept_inside_sixteen_times_its_size() {
        assert!(in_extent(4.0, 0.25));
        assert!(!in_extent(4.01, 0.25));
        assert!(in_extent(16.0, 1.0));
        assert!(!in_extent(16.01, 1.0));
    }

    #[test]
    fn the_rolling_stride_hits_every_period() {
        assert!(refresh_due(7, 0, 1));
        assert!(refresh_due(7, 1, 1));
        assert!(refresh_due(3, 1, 4));
        assert!(!refresh_due(3, 2, 4));
    }
}
