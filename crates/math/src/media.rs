//! Closed forms for a participating medium.
//!
//! `f64` is the graded expression. The `f32` wrappers are what
//! `crates/render/src/particles.rs` calls. `crates/render/shaders/media.glsl`
//! is the same expressions in GLSL. A horizontal ray is one whose vertical
//! cosine is under [`HEIGHT_FOG_HORIZONTAL_COS`].

/// `|cos θ|` below this is a horizontal ray. `media.glsl` uses the same cut.
pub const HEIGHT_FOG_HORIZONTAL_COS: f64 = 1.0e-4;

/// Beer-Lambert transmittance, `exp(-optical_depth)`.
#[must_use]
pub fn transmittance(optical_depth: f64) -> f64 {
    (-optical_depth).exp()
}

/// `f32` wrapper of [`transmittance`].
#[must_use]
pub fn transmittance_f32(optical_depth: f32) -> f32 {
    f32_from_f64(transmittance(f64::from(optical_depth)))
}

/// Height-fog extinction `σ0 * exp(-(y - base_height) / thickness)`.
///
/// A thickness that is not positive returns `sigma0`.
#[must_use]
pub fn height_fog_density(sigma0: f64, thickness: f64, y: f64, base_height: f64) -> f64 {
    if thickness <= 0.0 {
        return sigma0;
    }
    sigma0 * (-(y - base_height) / thickness).exp()
}

/// `f32` wrapper of [`height_fog_density`].
#[must_use]
pub fn height_fog_density_f32(sigma0: f32, thickness: f32, y: f32, base_height: f32) -> f32 {
    f32_from_f64(height_fog_density(
        f64::from(sigma0),
        f64::from(thickness),
        f64::from(y),
        f64::from(base_height),
    ))
}

/// Henyey-Greenstein phase. The integral over the sphere is 1 for `|g| < 1`.
///
/// `mu` is the cosine between the view direction and the light direction.
/// A non-positive denominator returns 0.
#[must_use]
pub fn phase_hg(g: f64, mu: f64) -> f64 {
    let denom = 1.0 + g * g - 2.0 * g * mu;
    if denom <= 0.0 {
        return 0.0;
    }
    (1.0 - g * g) / (4.0 * std::f64::consts::PI * denom.powf(1.5))
}

/// `f32` wrapper of [`phase_hg`].
#[must_use]
pub fn phase_hg_f32(g: f32, mu: f32) -> f32 {
    f32_from_f64(phase_hg(f64::from(g), f64::from(mu)))
}

/// Homogeneous single-scatter integral `(source / sigma) * (1 - exp(-sigma * distance))`.
///
/// A sigma near 0 uses the limit `source * distance`.
#[must_use]
pub fn homogeneous_inscatter(sigma: f64, source: f64, distance: f64) -> f64 {
    if sigma.abs() <= 1.0e-8 {
        return source * distance;
    }
    (source / sigma) * (1.0 - transmittance(sigma * distance))
}

/// `f32` wrapper of [`homogeneous_inscatter`].
#[must_use]
pub fn homogeneous_inscatter_f32(sigma: f32, source: f32, distance: f32) -> f32 {
    f32_from_f64(homogeneous_inscatter(
        f64::from(sigma),
        f64::from(source),
        f64::from(distance),
    ))
}

/// Optical depth of exponential height fog along one straight ray.
///
/// `cos_theta` is the ray's vertical component, Y up. `dy` is the height
/// change along `distance`. For a slanted ray the value is
/// `σ(y0) * thickness * (1 - exp(-dy / thickness)) / cos_theta`,
/// with `σ(y0) = σ0 * exp(-(y0 - base_height) / thickness)`.
/// That is `σ0 * H * exp(-y0 / H) * (1 - exp(-dy / H)) / cos θ` when
/// `base_height` is 0. A horizontal ray uses `σ(y0) * |distance|`.
#[must_use]
pub fn height_optical_depth(
    sigma0: f64,
    thickness: f64,
    base_height: f64,
    y0: f64,
    dy: f64,
    cos_theta: f64,
    distance: f64,
) -> f64 {
    let sigma_y0 = height_fog_density(sigma0, thickness, y0, base_height);
    if cos_theta.abs() < HEIGHT_FOG_HORIZONTAL_COS || thickness <= 0.0 {
        return sigma_y0 * distance.abs();
    }
    sigma_y0 * thickness * (1.0 - (-dy / thickness).exp()) / cos_theta
}

/// `f32` wrapper of [`height_optical_depth`].
#[must_use]
pub fn height_optical_depth_f32(
    sigma0: f32,
    thickness: f32,
    base_height: f32,
    y0: f32,
    dy: f32,
    cos_theta: f32,
    distance: f32,
) -> f32 {
    f32_from_f64(height_optical_depth(
        f64::from(sigma0),
        f64::from(thickness),
        f64::from(base_height),
        f64::from(y0),
        f64::from(dy),
        f64::from(cos_theta),
        f64::from(distance),
    ))
}

/// Rounds an f64 to the nearest f32. The workspace's one f64 to f32 narrowing.
#[allow(
    clippy::as_conversions,
    reason = "the one shared f64 to f32 narrowing; f32 callers of f64 closed forms use it"
)]
#[allow(
    clippy::cast_possible_truncation,
    reason = "the one shared f64 to f32 narrowing; f32 callers of f64 closed forms use it"
)]
#[must_use]
pub fn f32_from_f64(value: f64) -> f32 {
    value as f32
}
