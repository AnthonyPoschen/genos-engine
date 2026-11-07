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
//! and on a slanted surface it follows the light's direction. What a texel
//! shows after that read is `lighting_sc_shown` in `lighting_cache.glsl`.
//! The guards for that function live in `tests/lighting_sh_read.rs`.

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
            if sign == 0 { mag } else { -mag }
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

#[cfg(test)]
mod tests {
    use super::*;

    const UP: [f32; 3] = [0.0, 1.0, 0.0];
    /// `LIGHTING_WRC_CAP` and the quiet-light floor `LIGHTING_WRC_CHANGE_ALPHA`.
    const WRC_CAP: f32 = 128.0;
    const CHANGE_ALPHA: f32 = 1.0 / 32.0;

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

    /// Model of the default-flag world-cell weight in `lighting_cache.comp`.
    ///
    /// Dark fill while the cell is under half the cap, the change floor while
    /// the light is not quiet, and a whole replacement while a lamp change is
    /// settling. The mean and the harmonics take that one weight. A sun/sky
    /// rescale hits both before the blend. A mover's local alpha is not this
    /// path. This is a transcription of that weight. It is not the ADR 0039
    /// picture gate and it does not grade the production cache.
    fn model_wrc_weight(
        cell: &Held,
        sample: [f32; 3],
        sample_sh: &ShL1,
        settling: bool,
        quiet: bool,
        sun_gain: f32,
    ) -> Held {
        let fresh = cell.sets < 0.5;
        let t = if fresh { 0.0 } else { cell.sets };
        let add = 1.0;
        let mut wa = add / (t + add);
        if t < 0.5 * WRC_CAP {
            wa = 1.0 - (t / (t + add)).powi(3);
        }
        if !settling && !quiet {
            wa = wa.max(CHANGE_ALPHA * add);
        }
        if settling {
            wa = 1.0;
        }
        let a = wa.min(1.0);
        let (mut mean, mut sh) = (cell.mean, cell.sh);
        if !fresh && (sun_gain - 1.0).abs() > 1.0e-3 {
            mean = mean.map(|v| v * sun_gain);
            for row in &mut sh.c {
                for v in row {
                    *v *= sun_gain;
                }
            }
        }
        let mean = if fresh {
            sample
        } else {
            std::array::from_fn(|i| mean[i] + (sample[i] - mean[i]) * a)
        };
        let sh = if fresh {
            *sample_sh
        } else {
            let mut blended = sh;
            blended.blend(sample_sh, a);
            blended
        };
        Held {
            mean,
            sh,
            sets: (t + add).min(WRC_CAP),
        }
    }

    #[derive(Clone, Copy)]
    struct Held {
        mean: [f32; 3],
        sh: ShL1,
        sets: f32,
    }

    fn luma(c: [f32; 3]) -> f32 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    #[test]
    fn model_only_wrc_weight_holds_a_constant_and_settling_replaces_it() {
        // Not the stage-2 gate. The picture and the dumped cells are graded
        // in `examples/stress/tests/cache_energy.rs`. This only checks the
        // transcribed weight: the axis read is the stored mean, settling
        // (`wa = 1`) replaces a sample, and the capped mean left alone does not.
        let lit_rad = |_: [f32; 3]| [1.0, 0.85, 0.7];
        let (lit, sh_lit) = relight(UP, 64, 11, lit_rad);
        let (dark, sh_dark) = relight(UP, 64, 12, |_| [0.0; 3]);
        let mut cell = Held {
            mean: [0.0; 3],
            sh: ShL1::default(),
            sets: 0.0,
        };
        for _ in 0..128 {
            cell = model_wrc_weight(&cell, lit, &sh_lit, false, true, 1.0);
        }
        let energy = luma(cell.sh.tilt(cell.mean, UP, UP)) / luma(lit);
        assert!(energy >= 0.9, "axis energy {energy}");
        let face = tilted(35.0);
        let face_energy = luma(cell.sh.tilt(cell.mean, UP, face)) / luma(sh_lit.tilt(lit, UP, face));
        assert!(face_energy >= 0.9, "slanted energy {face_energy}");

        let lit_luma = luma(cell.mean);
        let mut cleared = cell;
        for _ in 0..120 {
            cleared = model_wrc_weight(&cleared, dark, &sh_dark, true, false, 1.0);
        }
        let linger = luma(cleared.sh.tilt(cleared.mean, UP, face)) / lit_luma;
        assert!(linger <= 0.05, "lamp still present at frame 120 ({linger})");

        let mut stuck = cell;
        for _ in 0..120 {
            stuck = model_wrc_weight(&stuck, dark, &sh_dark, false, true, 1.0);
        }
        let remain = luma(stuck.mean) / lit_luma;
        assert!(
            remain > 0.05,
            "the capped mean would already pass the lamp gate ({remain})"
        );

        // Lag: lamps on, the first rounds still dark, then the reference.
        // Settling holds the latest sample, so frame 300 is that reference.
        let mut filling = Held {
            mean: [0.0; 3],
            sh: ShL1::default(),
            sets: 0.0,
        };
        for frame in 1..=300 {
            let (sample, sh) = if frame < 40 { (dark, &sh_dark) } else { (lit, &sh_lit) };
            filling = model_wrc_weight(&filling, sample, sh, frame <= 120, frame > 120, 1.0);
        }
        let lag = luma(filling.sh.tilt(filling.mean, UP, UP)) / luma(lit);
        assert!(lag >= 0.9, "lag {lag}");

        let gained = model_wrc_weight(&cell, lit, &sh_lit, false, true, 2.0);
        assert_eq!(gained.sh.tilt(gained.mean, UP, UP), gained.mean);
        assert!(luma(gained.mean) > 1.5 * lit_luma, "sun rescale {}", luma(gained.mean));
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
}
