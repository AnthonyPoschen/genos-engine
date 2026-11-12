//! SH-L1 of incoming radiance, irradiance from it, and the stored encodings.

use std::f64::consts::PI;

use crate::geom::V3;

/// Real SH band 0.
pub const Y0: f64 = 0.282_094_791_773_878_14;
/// Real SH band 1 constant.
pub const Y1: f64 = 0.488_602_511_902_919_9;

/// SH-L1 basis at unit `d`, order `(00, 1-1, 10, 11)` = `(1, y, z, x)`.
#[must_use]
pub fn basis(d: V3) -> [f64; 4] {
    [Y0, Y1 * d.y, Y1 * d.z, Y1 * d.x]
}

/// Irradiance at normal `n` from SH-L1 radiance coefficients (cosine lobe
/// convolution, Ramamoorthi and Hanrahan 2001), clamped at zero.
#[must_use]
pub fn irradiance(sh: [f64; 4], n: V3) -> f64 {
    let e = PI * Y0 * sh[0] + (2.0 * PI / 3.0) * Y1 * (sh[1] * n.y + sh[2] * n.z + sh[3] * n.x);
    e.max(0.0)
}

/// [`irradiance`] without the clamp: linear in `n`, so it can be evaluated at
/// a mean (unnormalised) normal to get the mean over the normals.
#[must_use]
pub fn irradiance_lin(sh: [f64; 4], n: V3) -> f64 {
    PI * Y0 * sh[0] + (2.0 * PI / 3.0) * Y1 * (sh[1] * n.y + sh[2] * n.z + sh[3] * n.x)
}

/// Irradiance-normalisation factor as stored: clamped to `[1/4, 4]`; the
/// compact form is a `u8` of `log2` over that range.
#[must_use]
pub fn norm_ratio(k: f64, enc: Encoding) -> f64 {
    let k = k.clamp(0.25, 4.0);
    match enc {
        Encoding::Exact => k,
        Encoding::F16 => f16(k),
        Encoding::Compact => unorm8(k.log2(), -2.0, 2.0).exp2(),
    }
}

/// How a cell is stored between frames. The cell is decoded every frame, so
/// quantisation drift shows in the results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// `f64`, no rounding: the upper bound.
    Exact,
    /// Every float rounded to fp16 (11 significant bits). The plain engine cell.
    F16,
    /// L0 9-bit mantissa (shared-exponent style), L1 as `i8` ratios to L0,
    /// gradient `i8`, centroid `u8` offsets, mean hit distance log `u8`.
    Compact,
}

impl Encoding {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "exact" => Some(Self::Exact),
            "f16" => Some(Self::F16),
            "compact" => Some(Self::Compact),
            _ => None,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::F16 => "f16",
            Self::Compact => "compact",
        }
    }

    /// Estimated bytes per cell for the RGB engine payload (not this crate's
    /// monochrome one). `position` adds the stage 2b fields. Estimates, see
    /// ADR 0039 addendum A.
    #[must_use]
    pub const fn engine_payload_bytes(self, position: bool) -> u32 {
        match (self, position) {
            // 12 floats of 4 bytes, + centroid 3 floats, gradient 3, R 1.
            (Self::Exact, false) => 48,
            (Self::Exact, true) => 48 + 28,
            // 12 halves, + 7 halves.
            (Self::F16, false) => 24,
            (Self::F16, true) => 24 + 14,
            // RGB9E5 L0 4 + L1 9 x i8, + gradient 3 + centroid 3 + R 1.
            (Self::Compact, false) => 13,
            (Self::Compact, true) => 13 + 7,
        }
    }
}

/// Round to `bits` significant bits (including the leading one).
#[must_use]
pub fn round_sig(x: f64, bits: i32) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return 0.0;
    }
    let e = x.abs().log2().floor();
    let step = (e - f64::from(bits - 1)).exp2();
    (x / step).round() * step
}

/// fp16: 11 significant bits, flush below the smallest normal.
#[must_use]
pub fn f16(x: f64) -> f64 {
    if x.abs() < 6.1e-5 {
        return 0.0;
    }
    round_sig(x, 11).clamp(-65504.0, 65504.0)
}

/// Quantise `x` in `[-range, range]` to a signed byte and back.
#[must_use]
pub fn snorm8(x: f64, range: f64) -> f64 {
    if range <= 0.0 {
        return 0.0;
    }
    ((x / range).clamp(-1.0, 1.0) * 127.0).round() / 127.0 * range
}

/// Quantise `x` in `[lo, hi]` to an unsigned byte and back.
#[must_use]
pub fn unorm8(x: f64, lo: f64, hi: f64) -> f64 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    (t * 255.0).round() / 255.0 * (hi - lo) + lo
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::uniform_hemi;
    use crate::rng::Rng;

    #[test]
    fn constant_radiance_gives_pi_l() {
        // Projection of L = 1 over the full sphere: sh0 = 4 pi Y0.
        let sh = [4.0 * PI * Y0, 0.0, 0.0, 0.0];
        assert!((irradiance(sh, V3::new(0.0, 1.0, 0.0)) - PI).abs() < 1e-9);
    }

    #[test]
    fn hemisphere_estimate_matches_cosine_integral() {
        // Radiance 1 from the upper hemisphere only: true irradiance at +y is pi.
        let n = V3::new(0.0, 1.0, 0.0);
        let mut rng = Rng::new(1);
        let mut sh = [0.0; 4];
        let m = 200_000;
        for _ in 0..m {
            let d = uniform_hemi(n, &mut rng);
            for (s, b) in sh.iter_mut().zip(basis(d)) {
                *s += 2.0 * PI * b;
            }
        }
        for s in &mut sh {
            *s /= f64::from(m);
        }
        let e = irradiance(sh, n);
        // SH-L1 ringing: L1 reconstruction of a hemisphere step gives pi exactly
        // at the pole up to sampling noise.
        assert!((e - PI).abs() < 0.02 * PI, "{e}");
    }
}
