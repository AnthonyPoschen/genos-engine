//! CPU reference for the SH-L1 world cells (ADR 0039 stage 2).
//!
//! A 25 cm world cell keeps, beside its mean irradiance at the axis normal,
//! the order-1 spherical harmonics of the radiance its relight rays saw.
//! The shader (`lighting_cache.comp`, `lighting_sh_*` in
//! `lighting_cache.glsl`) mirrors these functions. The tests here are the
//! reference the GPU path is graded against.
//!
//! The rays cover the hemisphere about the cell's axis normal only, so the
//! harmonics see no light from behind the cell. Plain L1 irradiance also
//! flattens a narrow source (a sun or a lamp) to `0.25 + 0.5 cos`, which
//! loses a quarter of it at the normal. The cell's read is therefore the
//! exact axis mean plus the harmonics' change from the axis normal to the
//! true normal ([`ShL1::tilt`]). At the axis normal it is the mean exactly,
//! and on a slanted surface it follows the light's direction.
//!
//! An axis-aligned texel seed keeps that mean ([`axis_aligned_floor_seed`]).
//! The picture shows the seed until the texel's own rays replace it, so a
//! blend of the neighbour cell is a different energy in that history.

use std::f32::consts::PI;

/// Real SH basis constants: `Y00` and the `Y1m` factor.
pub const SH_Y0: f32 = 0.282_094_8;
pub const SH_Y1: f32 = 0.488_602_5;
/// Clamped-cosine convolution of bands 0 and 1 (Ramamoorthi and Hanrahan 2001).
pub const SH_A0: f32 = PI;
pub const SH_A1: f32 = 2.0 * PI / 3.0;

/// Order-1 harmonics of radiance, one RGB colour per coefficient:
/// `c[0]` is band 0, `c[1..4]` band 1 along x, y and z.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShL1 {
    pub c: [[f32; 3]; 4],
}

impl ShL1 {
    /// Adds one ray of radiance `l` along unit `dir`, weighted by `w` (the
    /// ray's solid angle, `2 pi / rays` for the uniform hemisphere).
    pub fn add_ray(&mut self, dir: [f32; 3], l: [f32; 3], w: f32) {
        let basis = [SH_Y0, SH_Y1 * dir[0], SH_Y1 * dir[1], SH_Y1 * dir[2]];
        for (c, b) in self.c.iter_mut().zip(basis) {
            for ch in 0..3 {
                c[ch] += w * b * l[ch];
            }
        }
    }

    /// Moves toward `new` by `alpha`, as the cell's mean does.
    pub fn blend(&mut self, new: &ShL1, alpha: f32) {
        for (c, n) in self.c.iter_mut().zip(new.c.iter()) {
            for ch in 0..3 {
                c[ch] += (n[ch] - c[ch]) * alpha;
            }
        }
    }

    /// Plain L1 irradiance at unit normal `n`, unclamped.
    #[must_use]
    pub fn irradiance_raw(&self, n: [f32; 3]) -> [f32; 3] {
        let mut e = [0.0; 3];
        for (ch, out) in e.iter_mut().enumerate() {
            let band1 = self.c[1][ch] * n[0] + self.c[2][ch] * n[1] + self.c[3][ch] * n[2];
            *out = SH_A0 * SH_Y0 * self.c[0][ch] + SH_A1 * SH_Y1 * band1;
        }
        e
    }

    /// Plain L1 irradiance at `n`, clamped at zero (no ringing below it).
    #[must_use]
    pub fn irradiance(&self, n: [f32; 3]) -> [f32; 3] {
        self.irradiance_raw(n).map(|v| v.max(0.0))
    }

    /// The cell's read at true normal `n`: the axis mean `e_axis` plus the
    /// harmonics' change from `axis` to `n`, clamped at zero. Equal to
    /// `e_axis` when `n` is the axis.
    #[must_use]
    pub fn tilt(&self, e_axis: [f32; 3], axis: [f32; 3], n: [f32; 3]) -> [f32; 3] {
        let d = [n[0] - axis[0], n[1] - axis[1], n[2] - axis[2]];
        let mut out = [0.0; 3];
        for (ch, o) in out.iter_mut().enumerate() {
            let band1 = self.c[1][ch] * d[0] + self.c[2][ch] * d[1] + self.c[3][ch] * d[2];
            *o = (e_axis[ch] + SH_A1 * SH_Y1 * band1).max(0.0);
        }
        out
    }

    /// Twelve half floats, two per `u32` as GLSL `packHalf2x16` (x low).
    #[must_use]
    pub fn pack(&self) -> [u32; 6] {
        let f: Vec<f32> = self.c.iter().flatten().copied().collect();
        std::array::from_fn(|i| pack_half_2x16(f[2 * i], f[2 * i + 1]))
    }

    #[must_use]
    pub fn unpack(words: &[u32; 6]) -> Self {
        let mut s = ShL1::default();
        for (i, w) in words.iter().enumerate() {
            let (a, b) = unpack_half_2x16(*w);
            s.c[(2 * i) / 3][(2 * i) % 3] = a;
            s.c[(2 * i + 1) / 3][(2 * i + 1) % 3] = b;
        }
        s
    }
}

/// Ray `k` of `count` over the hemisphere about `n`, stratified on a
/// `sqrt(count)` grid of (cos theta, turn) with jitter `(j1, j2)`; the CPU
/// twin of `lighting_dir` (`lighting_common.glsl`) with no hash turn.
#[must_use]
pub fn hemi_dir(n: [f32; 3], k: u32, count: u32, j1: f32, j2: f32) -> [f32; 3] {
    let side = isqrt(count).max(1);
    let cos_t = (small_f32(k % side) + j1) / small_f32(side);
    let turn = (small_f32(k / side) + j2) / small_f32(side);
    let r = (1.0 - cos_t * cos_t).max(0.0).sqrt();
    let phi = 2.0 * PI * turn;
    let helper = if n[1].abs() > 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let tx = normalize(cross(helper, n));
    let ty = cross(n, tx);
    let (c, sn) = (phi.cos(), phi.sin());
    std::array::from_fn(|i| tx[i] * r * c + ty[i] * r * sn + n[i] * cos_t)
}

/// One relight of a cell with axis normal `n`: `count` stratified rays of
/// `radiance(dir)`. Returns the mean irradiance at `n` (what the cache stores
/// today, `2 pi / count * sum L cos`) and the harmonics of the same rays.
pub fn relight(
    n: [f32; 3],
    count: u32,
    seed: u32,
    radiance: impl Fn([f32; 3]) -> [f32; 3],
) -> ([f32; 3], ShL1) {
    let weight = 2.0 * PI / small_f32(count);
    let mut irr = [0.0; 3];
    let mut sh = ShL1::default();
    let mut state = seed.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    let mut unit = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        f32::from_bits(0x3f80_0000 | (state >> 9)) - 1.0
    };
    for k in 0..count {
        let (j1, j2) = (unit(), unit());
        let dir = hemi_dir(n, k, count, j1, j2);
        let l = radiance(dir);
        let cos = dot(dir, n).max(0.0);
        for ch in 0..3 {
            irr[ch] += weight * cos * l[ch];
        }
        sh.add_ray(dir, l, weight);
    }
    (irr, sh)
}

/// GLSL `packHalf2x16`: round to nearest even, subnormals kept.
#[must_use]
pub fn pack_half_2x16(a: f32, b: f32) -> u32 {
    f32_to_f16(a) | (f32_to_f16(b) << 16)
}

#[must_use]
pub fn unpack_half_2x16(word: u32) -> (f32, f32) {
    (f16_to_f32(word & 0xffff), f16_to_f32(word >> 16))
}

/// A whole number below 2^23 as an exact float (no `as`).
fn small_f32(v: u32) -> f32 {
    debug_assert!(v < 1 << 23);
    f32::from_bits(0x4b00_0000 | v) - 8_388_608.0
}

fn isqrt(v: u32) -> u32 {
    let mut r = 0u32;
    while (r + 1) * (r + 1) <= v {
        r += 1;
    }
    r
}

/// Half-float bits (low 16) of `x`.
fn f32_to_f16(x: f32) -> u32 {
    let bits = x.to_bits();
    let sign = (bits >> 16) & 0x8000;
    let exp = (bits >> 23) & 0xff;
    let man = bits & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    if exp >= 143 {
        return sign | 0x7c00;
    }
    if exp <= 112 {
        if exp < 102 {
            return sign;
        }
        let full = man | 0x80_0000;
        let shift = 126 - exp;
        let half = 1u32 << (shift - 1);
        let rest = full & ((1u32 << shift) - 1);
        let mut v = full >> shift;
        if rest > half || (rest == half && v & 1 == 1) {
            v += 1;
        }
        return sign | v;
    }
    let mut v = ((exp - 112) << 10) | (man >> 13);
    let rest = man & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && v & 1 == 1) {
        v += 1;
    }
    sign | v
}

/// The float of half-float bits `h` (low 16).
fn f16_to_f32(h: u32) -> f32 {
    let sign = (h & 0x8000) << 16;
    let exp = (h >> 10) & 0x1f;
    let man = h & 0x3ff;
    match exp {
        0 => {
            let mag = small_f32(man) * f32::from_bits(0x3380_0000);
            if sign == 0 {
                mag
            } else {
                -mag
            }
        }
        0x1f => f32::from_bits(sign | 0x7f80_0000 | (man << 13)),
        _ => f32::from_bits(sign | ((exp + 112) << 23) | (man << 13)),
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    a.map(|v| v / l)
}

/// Sets in the texel's handover (`LIGHTING_SC_HANDOVER` in `lighting_cache.glsl`).
pub const TEXEL_HANDOVER_SETS: f32 = 16.0;
/// Prior weight in that handover (`LIGHTING_SC_HANDOVER_K`).
pub const TEXEL_HANDOVER_K: f32 = 2.0;
/// Confidence of the hit-cell seed (`lighting_sc_set_seed` of the first fill).
pub const TEXEL_SEED_CONFIDENCE: f32 = 0.25;

/// One 25 cm floor cell. `u` and `v` are the tangent centres the cache key owns.
#[derive(Clone, Copy, Debug)]
pub struct FloorCell {
    pub u: f32,
    pub v: f32,
    pub irradiance: [f32; 3],
}

/// Bounce stored as the first sample of an axis-aligned floor texel at `(u, v)`.
///
/// The cell that owns the point measured this. Compose seeds a new texel from
/// that cell, and [`texel_history_shown`] is the seed until the texel's own
/// rays replace it. A bilinear of the neighbour is a different energy.
#[must_use]
pub fn axis_aligned_floor_seed(u: f32, v: f32, cell: f32, cells: &[FloorCell]) -> [f32; 3] {
    owning_floor_cell(u, v, cell, cells)
}

/// Picture bounce while a texel has `own_sets` of its own samples.
///
/// `own_sets == 0` is the seed alone (`lighting_sc_shown`: the mix weight is
/// 0 until the texel has a sample of its own). Later sets take the shader's
/// handover `n / (n + K * confidence * fade)`.
#[must_use]
pub fn texel_history_shown(
    seed: [f32; 3],
    own: [f32; 3],
    own_sets: f32,
    seed_confidence: f32,
) -> [f32; 3] {
    let fade = (1.0 - own_sets / TEXEL_HANDOVER_SETS).clamp(0.0, 1.0);
    let k = TEXEL_HANDOVER_K * seed_confidence.min(1.0) * fade;
    let a = if own_sets > 0.5 {
        own_sets / (own_sets + k).max(1.0e-4)
    } else {
        0.0
    };
    std::array::from_fn(|ch| seed[ch] + (own[ch] - seed[ch]) * a)
}

fn owning_floor_cell(u: f32, v: f32, cell: f32, cells: &[FloorCell]) -> [f32; 3] {
    let cu = ((u / cell).floor() + 0.5) * cell;
    let cv = ((v / cell).floor() + 0.5) * cell;
    cells
        .iter()
        .find(|c| (c.u - cu).abs() <= 1.0e-4 && (c.v - cv).abs() <= 1.0e-4)
        .map_or([0.0; 3], |c| c.irradiance)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UP: [f32; 3] = [0.0, 1.0, 0.0];

    /// The display bilinear the seed used to store: four lattice cells,
    /// weights renormalised over the lit ones.
    fn display_neighbour_blend(u: f32, v: f32, cell: f32, cells: &[FloorCell]) -> [f32; 3] {
        let mut sum = [0.0; 3];
        let mut ws = 0.0;
        let (g_u, f_u) = lattice_frac(u, cell);
        let (g_v, f_v) = lattice_frac(v, cell);
        for k in 0..4u32 {
            let cu = g_u + if k & 1 == 0 { 0.0 } else { 1.0 };
            let cv = g_v + if k >> 1 == 0 { 0.0 } else { 1.0 };
            let bil = (if k & 1 == 1 { f_u } else { 1.0 - f_u })
                * (if k >> 1 == 1 { f_v } else { 1.0 - f_v });
            if bil <= 0.0 {
                continue;
            }
            let qu = (cu + 0.5) * cell;
            let qv = (cv + 0.5) * cell;
            let Some(hit) = cells
                .iter()
                .find(|c| (c.u - qu).abs() <= 1.0e-4 && (c.v - qv).abs() <= 1.0e-4)
            else {
                continue;
            };
            for (ch, out) in sum.iter_mut().enumerate() {
                *out += bil * hit.irradiance[ch];
            }
            ws += bil;
        }
        if ws > 0.0 {
            sum.map(|v| v / ws)
        } else {
            [0.0; 3]
        }
    }

    fn lattice_frac(p: f32, cell: f32) -> (f32, f32) {
        let x = p / cell - 0.5;
        let g = x.floor();
        (g, x - g)
    }

    fn tilted(deg: f32) -> [f32; 3] {
        let a = deg.to_radians();
        [a.sin(), a.cos(), 0.0]
    }

    /// Reference irradiance at `n` by brute force over the whole sphere.
    fn exact(n: [f32; 3], radiance: impl Fn([f32; 3]) -> [f32; 3]) -> [f32; 3] {
        let (e, _) = relight(n, 256 * 256, 7, radiance);
        e
    }

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol * b.abs().max(1.0e-3)
    }

    #[test]
    fn uniform_sky_reads_pi_l_at_the_axis_and_the_dome_law_when_tilted() {
        // Energy: radiance L over the hemisphere the rays see gives pi L at the
        // axis normal, and at a tilt alpha the dome law pi L (1 + cos alpha) / 2
        // (L1 holds it exactly; the rays only see the upper hemisphere).
        let l = [1.0, 0.5, 0.25];
        let (e, sh) = relight(UP, 64 * 64, 1, |_| l);
        for ch in 0..3 {
            assert!(close(e[ch], PI * l[ch], 1.0e-3), "mean {e:?}");
            assert!(close(sh.irradiance(UP)[ch], PI * l[ch], 1.0e-3));
            assert!(close(sh.tilt(e, UP, UP)[ch], e[ch], 1.0e-6));
        }
        for deg in [15.0f32, 30.0, 45.0, 60.0] {
            let want = PI * l[0] * (1.0 + deg.to_radians().cos()) / 2.0;
            let got = sh.tilt(e, UP, tilted(deg))[0];
            assert!(close(got, want, 2.0e-3), "{deg}: {got} vs {want}");
        }
    }

    #[test]
    fn the_axis_read_is_the_mean_exactly() {
        // The tilt term vanishes at the axis normal, so flag-on reads on an
        // axis-aligned surface equal the per-axis mean whatever the light.
        let sun = |d: [f32; 3]| {
            if dot(d, normalize([0.3, 0.8, 0.2])) > 0.98 {
                [40.0; 3]
            } else {
                [0.2; 3]
            }
        };
        let (e, sh) = relight(UP, 16, 3, sun);
        assert_eq!(sh.tilt(e, UP, UP), e);
    }

    #[test]
    fn a_narrow_source_on_a_slanted_surface_beats_the_per_axis_read() {
        // Known lighting: a sun disc plus a dim sky. A surface tilted toward
        // the sun or away from it reads closer to the exact irradiance than
        // the axis mean (today's read of a slanted surface's cell).
        let dir = normalize([0.6, 0.7, 0.0]);
        let sun = move |d: [f32; 3]| {
            if dot(d, dir) > 0.97 {
                [30.0; 3]
            } else {
                [0.3; 3]
            }
        };
        let (e, sh) = relight(UP, 64 * 64, 5, sun);
        for deg in [-40.0f32, -20.0, 20.0, 40.0] {
            let n = tilted(deg);
            let want = exact(n, sun)[0];
            let axis_err = (e[0] - want).abs();
            let tilt_err = (sh.tilt(e, UP, n)[0] - want).abs();
            assert!(
                tilt_err < axis_err,
                "{deg}: tilt {tilt_err} axis {axis_err} (want {want})"
            );
        }
    }

    #[test]
    fn a_sky_gradient_on_a_slanted_surface_beats_the_per_axis_read() {
        // A gradient a + b . d over the half-sphere the rays see (nothing
        // below). The cut at the horizon is not order 1, so the tilt read is
        // not exact: 8 % off at 30 degrees away from the bright side (the axis
        // mean is 27 % off there), 4 % toward it (axis 6 %).
        let rad = |d: [f32; 3]| {
            if d[1] < 0.0 {
                [0.0; 3]
            } else {
                let v = 1.0 + 0.6 * d[0] + 0.3 * d[1];
                [v, v, v]
            }
        };
        let (e, sh) = relight(UP, 64 * 64, 9, rad);
        for deg in [-30.0f32, 30.0] {
            let n = tilted(deg);
            let want = exact(n, rad)[0];
            let axis_err = (e[0] - want).abs() / want;
            let tilt_err = (sh.tilt(e, UP, n)[0] - want).abs() / want;
            assert!(tilt_err < 0.1, "{deg}: {tilt_err}");
            assert!(
                tilt_err < axis_err,
                "{deg}: tilt {tilt_err} axis {axis_err}"
            );
        }
    }

    #[test]
    fn plain_l1_loses_a_quarter_of_a_narrow_source_so_the_read_keeps_the_mean() {
        // Why the read is the mean plus a tilt, not plain L1: a narrow source
        // straight down the normal reads 0.75 of its light in L1.
        let sun = |d: [f32; 3]| if d[1] > 0.999 { [100.0; 3] } else { [0.0; 3] };
        let (e, sh) = relight(UP, 128 * 128, 2, sun);
        let ratio = sh.irradiance(UP)[0] / e[0];
        assert!((ratio - 0.75).abs() < 0.02, "{ratio}");
        assert_eq!(sh.tilt(e, UP, UP), e);
    }

    #[test]
    fn blending_harmonics_with_the_means_alpha_keeps_the_axis_read() {
        // The cell blends mean and harmonics with one alpha, so the tilt read
        // of the blend is the blend of the tilt reads (it is linear).
        let (e1, s1) = relight(UP, 16, 1, |d| [1.0 + d[0], 1.0, 1.0]);
        let (e2, s2) = relight(UP, 16, 2, |d| [3.0 - d[2], 2.0, 0.5]);
        let a = 0.3;
        let mut s = s1;
        s.blend(&s2, a);
        let e: [f32; 3] = std::array::from_fn(|i| e1[i] + (e2[i] - e1[i]) * a);
        let n = tilted(25.0);
        let got = s.tilt(e, UP, n);
        let (t1, t2) = (s1.tilt(e1, UP, n), s2.tilt(e2, UP, n));
        for ch in 0..3 {
            assert!(close(got[ch], t1[ch] + (t2[ch] - t1[ch]) * a, 1.0e-5));
        }
    }

    #[test]
    fn half_packing_matches_glsl_and_keeps_three_digits() {
        assert_eq!(pack_half_2x16(1.0, -2.0), 0xc000_3c00);
        assert_eq!(unpack_half_2x16(0xc000_3c00), (1.0, -2.0));
        let (e, sh) = relight(tilted(10.0), 16, 4, |d| [5.0 * d[1].max(0.0), 0.1, 2.0]);
        let back = ShL1::unpack(&sh.pack());
        for (a, b) in sh.c.iter().flatten().zip(back.c.iter().flatten()) {
            assert!((a - b).abs() <= a.abs() * 1.0e-3 + 1.0e-6, "{a} {b}");
        }
        let n = tilted(30.0);
        let (x, y) = (sh.tilt(e, UP, n), back.tilt(e, UP, n));
        for ch in 0..3 {
            assert!(close(y[ch], x[ch], 2.0e-3));
        }
    }

    #[test]
    fn an_axis_aligned_floor_seed_keeps_the_measured_energy_in_its_history() {
        // A 12.5 cm texel centre inside a 25 cm floor cell, beside a darker
        // coplanar neighbour (the cell across a contact or a shadow edge).
        // The owning cell measured `bright`. Compose stores the world-cell
        // read as the texel's seed, and with no samples of its own the
        // picture is that seed. One own set still carries the seed
        // (handover confidence 0.25).
        let cell = 0.25;
        let at_u = 0.1875;
        let at_v = 0.125;
        let bright = [2.0, 1.0, 0.4];
        let cells = [
            FloorCell {
                u: 0.125,
                v: 0.125,
                irradiance: bright,
            },
            FloorCell {
                u: 0.375,
                v: 0.125,
                irradiance: [0.2, 0.2, 0.2],
            },
        ];
        let measured = owning_floor_cell(at_u, at_v, cell, &cells);
        for ch in 0..3 {
            assert!(
                (measured[ch] - bright[ch]).abs() <= 1.0e-5,
                "the point must sit in the bright cell, got {measured:?}"
            );
        }
        let seed = axis_aligned_floor_seed(at_u, at_v, cell, &cells);
        let blend = display_neighbour_blend(at_u, at_v, cell, &cells);
        let own = measured;
        for sets in [0.0, 1.0] {
            let shown = texel_history_shown(seed, own, sets, TEXEL_SEED_CONFIDENCE);
            let leaked = texel_history_shown(blend, own, sets, TEXEL_SEED_CONFIDENCE);
            let mut worst_leak: f32 = 0.0;
            for ch in 0..3 {
                let err = (shown[ch] - bright[ch]).abs();
                let leak = (leaked[ch] - bright[ch]).abs();
                worst_leak = worst_leak.max(leak);
                assert!(
                    err <= 1.0e-5,
                    "history at {sets} own sets shows {shown:?}, the cell measured {bright:?}"
                );
                assert!(
                    err < leak,
                    "seed {seed:?} is no closer than the neighbour blend"
                );
            }
            let floor = if sets == 0.0 { 0.2 } else { 0.1 };
            assert!(
                worst_leak > floor,
                "fixture at {sets} own sets must move energy, leak {worst_leak} blend {blend:?}"
            );
        }
    }
}
