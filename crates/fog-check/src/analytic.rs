//! Grades the shared closed forms. The good path calls `genos_math`.
//! The oracle is an independent calculation in this file. The broken path
//! is a deliberate wrong formula, not a second copy of the shared function.

use genos_lighting_ref::{blur_relative, MetricParams, Picture};
use genos_math::{
    height_fog_density, height_optical_depth, homogeneous_inscatter, phase_hg, transmittance,
};

use crate::report::{judge, Metric};
use crate::Fixture;

const BEER_LAMBERT_MAX_ABS: f64 = 1.0e-4;
const HEIGHT_FOG_DENSITY_MAX_ABS: f64 = 1.0e-4;
const PHASE_HG_MAX_ABS: f64 = 1.0e-6;
const PHASE_HG_INTEGRAL_ABS: f64 = 1.0e-4;
const HOMOGENEOUS_INSCATTER_MAX_ABS: f64 = 1.0e-3;
const HEIGHT_RAY_OPTICAL_DEPTH_MAX_ABS: f64 = 1.0e-4;
const IMAGE_GRADE_BLUR_REL: f64 = 0.12;

/// `(g, cos θ, phase)`. Literals are locked from an independent evaluation.
const HG_SAMPLES: [(f64, f64, f64); 7] = [
    (0.0, 1.0, 0.079_577_471_545_947_67),
    (0.0, 0.0, 0.079_577_471_545_947_67),
    (0.0, -1.0, 0.079_577_471_545_947_67),
    (0.4, 1.0, 0.309_467_944_900_907_5),
    (0.4, 0.0, 0.053_503_530_222_411_88),
    (0.4, -1.0, 0.024_360_450_473_249_286),
    (0.7, 0.5, 0.057_798_913_563_976_75),
];

const HG_INTEGRAL_G: [f64; 3] = [0.0, 0.4, 0.7];

const BEER_DISTANCES: [f64; 6] = [0.0, 0.5, 1.0, 2.0, 4.0, 8.0];
const HEIGHTS: [f64; 6] = [0.0, 10.0, 20.0, 40.0, 80.0, 120.0];
const INSCATTER_DISTANCES: [f64; 5] = [0.5, 1.0, 2.0, 4.0, 8.0];

/// `(sigma0, thickness, base_height, y0, dy, cos θ, distance)`.
const HEIGHT_RAYS: [(f64, f64, f64, f64, f64, f64, f64); 5] = [
    (0.08, 40.0, 0.0, 10.0, 20.0, 0.5, 40.0),
    (0.08, 40.0, 0.0, 40.0, -15.0, -0.6, 25.0),
    (0.05, 30.0, 0.0, 0.0, 12.0, 0.95, 12.0 / 0.95),
    (0.1, 20.0, 8.0, 5.0, 2.0, 0.25, 8.0),
    (0.08, 40.0, 0.0, 25.0, 0.0, 0.0, 12.0),
];

pub(crate) fn evaluate_analytic(fixture: Fixture) -> Vec<Metric> {
    vec![
        graded_metric(
            "beer_lambert_max_abs",
            beer_lambert_max_abs(fixture),
            BEER_LAMBERT_MAX_ABS,
        ),
        graded_metric(
            "height_fog_density_max_abs",
            height_fog_density_max_abs(fixture),
            HEIGHT_FOG_DENSITY_MAX_ABS,
        ),
        graded_metric(
            "height_ray_optical_depth_max_abs",
            height_ray_optical_depth_max_abs(fixture),
            HEIGHT_RAY_OPTICAL_DEPTH_MAX_ABS,
        ),
        graded_metric(
            "phase_hg_max_abs",
            phase_hg_max_abs(fixture),
            PHASE_HG_MAX_ABS,
        ),
        graded_metric(
            "phase_hg_integral_abs",
            phase_hg_integral_abs(fixture),
            PHASE_HG_INTEGRAL_ABS,
        ),
        graded_metric(
            "homogeneous_inscatter_max_abs",
            homogeneous_inscatter_max_abs(fixture),
            HOMOGENEOUS_INSCATTER_MAX_ABS,
        ),
        image_grade(fixture),
    ]
}

fn graded_metric(name: &'static str, value: f64, threshold: f64) -> Metric {
    Metric {
        name,
        value: Some(value),
        threshold,
        status: judge(true, Some(value), threshold),
        note: "",
    }
}

fn beer_lambert_max_abs(fixture: Fixture) -> f64 {
    const SIGMA: f64 = 0.5;
    max_abs(BEER_DISTANCES, |distance| {
        let reference = transmittance_stepped(SIGMA, distance);
        let candidate = match fixture {
            Fixture::Good => transmittance(SIGMA * distance),
            Fixture::Broken => (1.0 - SIGMA * distance).max(0.0),
        };
        (candidate - reference).abs()
    })
}

/// Product of per-step transmittance. For constant extinction this is the
/// closed form up to rounding.
fn transmittance_stepped(sigma: f64, distance: f64) -> f64 {
    const STEPS: u32 = 64;
    if distance <= 0.0 {
        return 1.0;
    }
    let step = (-sigma * distance / f64::from(STEPS)).exp();
    let mut product = 1.0;
    let mut index = 0u32;
    while index < STEPS {
        product *= step;
        index += 1;
    }
    product
}

fn height_fog_density_max_abs(fixture: Fixture) -> f64 {
    const SIGMA0: f64 = 0.08;
    const THICKNESS: f64 = 40.0;
    max_abs(HEIGHTS, |height| {
        let reference = height_euler(SIGMA0, THICKNESS, height);
        let candidate = match fixture {
            Fixture::Good => height_fog_density(SIGMA0, THICKNESS, height, 0.0),
            Fixture::Broken => SIGMA0 * (1.0 - height / THICKNESS).max(0.0),
        };
        (candidate - reference).abs()
    })
}

/// Forward Euler on `dσ/dy = -σ/H`. Small steps stay inside the threshold.
fn height_euler(sigma0: f64, thickness: f64, height: f64) -> f64 {
    const DY: f64 = 0.001;
    if height <= 0.0 {
        return sigma0;
    }
    let mut sigma = sigma0;
    let mut y = 0.0;
    while y < height {
        let step = DY.min(height - y);
        sigma *= 1.0 - step / thickness;
        y += step;
    }
    sigma
}

fn height_ray_optical_depth_max_abs(fixture: Fixture) -> f64 {
    max_abs(HEIGHT_RAYS, |ray| {
        let (sigma0, thickness, base, y0, dy, cos_theta, distance) = ray;
        let reference =
            optical_depth_midpoint(sigma0, thickness, base, y0, dy, cos_theta, distance);
        let candidate = match fixture {
            Fixture::Good => {
                height_optical_depth(sigma0, thickness, base, y0, dy, cos_theta, distance)
            }
            Fixture::Broken => slanted_without_cos(sigma0, thickness, base, y0, dy),
        };
        (candidate - reference).abs()
    })
}

/// Midpoint integral of `σ(y) ds`. Independent of [`height_optical_depth`].
fn optical_depth_midpoint(
    sigma0: f64,
    thickness: f64,
    base: f64,
    y0: f64,
    dy: f64,
    cos_theta: f64,
    distance: f64,
) -> f64 {
    const STEPS: u32 = 512;
    let path = if cos_theta.abs() < genos_math::HEIGHT_FOG_HORIZONTAL_COS {
        distance.abs()
    } else {
        (dy / cos_theta).abs()
    };
    if path <= 0.0 || thickness <= 0.0 {
        return 0.0;
    }
    let ds = path / f64::from(STEPS);
    let mut tau = 0.0;
    let mut index = 0u32;
    while index < STEPS {
        let t = (f64::from(index) + 0.5) / f64::from(STEPS);
        let y = y0 + t * dy;
        let sigma = sigma0 * (-(y - base) / thickness).exp();
        tau += sigma * ds;
        index += 1;
    }
    tau
}

/// The slanted closed form with the `/ cos θ` factor left out.
fn slanted_without_cos(sigma0: f64, thickness: f64, base: f64, y0: f64, dy: f64) -> f64 {
    if thickness <= 0.0 {
        return 0.0;
    }
    let sigma_y0 = sigma0 * (-(y0 - base) / thickness).exp();
    sigma_y0 * thickness * (1.0 - (-dy / thickness).exp())
}

fn phase_hg_max_abs(fixture: Fixture) -> f64 {
    max_abs(HG_SAMPLES, |(g, mu, expected)| {
        let candidate = match fixture {
            Fixture::Good => phase_hg(g, mu),
            Fixture::Broken => 1.0,
        };
        (candidate - expected).abs()
    })
}

fn phase_hg_integral_abs(fixture: Fixture) -> f64 {
    max_abs(HG_INTEGRAL_G, |g| {
        let integral = match fixture {
            Fixture::Good => integrate_phase(g, phase_hg),
            Fixture::Broken => integrate_phase(g, phase_hg_unnormalized),
        };
        (integral - 1.0).abs()
    })
}

/// Henyey-Greenstein with the `4π` normalisation left out.
fn phase_hg_unnormalized(g: f64, mu: f64) -> f64 {
    let denom = 1.0 + g * g - 2.0 * g * mu;
    if denom <= 0.0 {
        return 0.0;
    }
    (1.0 - g * g) / denom.powf(1.5)
}

/// Simpson's rule on `μ`, then `2π` for the azimuth. The sphere integral of a
/// normalised phase is 1.
fn integrate_phase(g: f64, phase: impl Fn(f64, f64) -> f64) -> f64 {
    const STEPS: u32 = 256;
    let h = 2.0 / f64::from(STEPS);
    let mut sum = phase(g, -1.0) + phase(g, 1.0);
    let mut index = 1u32;
    while index < STEPS {
        let mu = -1.0 + f64::from(index) * h;
        let weight = if index.is_multiple_of(2) { 2.0 } else { 4.0 };
        sum += weight * phase(g, mu);
        index += 1;
    }
    2.0 * std::f64::consts::PI * h / 3.0 * sum
}

fn homogeneous_inscatter_max_abs(fixture: Fixture) -> f64 {
    const SIGMA: f64 = 0.2;
    const SOURCE: f64 = 0.15;
    max_abs(INSCATTER_DISTANCES, |distance| {
        let reference = inscatter_exact_steps(SIGMA, SOURCE, distance);
        let candidate = match fixture {
            Fixture::Good => homogeneous_inscatter(SIGMA, SOURCE, distance),
            Fixture::Broken => SOURCE * distance,
        };
        (candidate - reference).abs()
    })
}

/// Piecewise exact integral for constant extinction and a constant source.
fn inscatter_exact_steps(sigma: f64, source: f64, distance: f64) -> f64 {
    const STEPS: u32 = 32;
    if distance <= 0.0 {
        return 0.0;
    }
    let ds = distance / f64::from(STEPS);
    let decay = (-sigma * ds).exp();
    let absorb = 1.0 - decay;
    let mut running = 1.0;
    let mut sum = 0.0;
    let mut index = 0u32;
    while index < STEPS {
        sum += running * (source / sigma) * absorb;
        running *= decay;
        index += 1;
    }
    sum
}

fn image_grade(fixture: Fixture) -> Metric {
    let gain = match fixture {
        Fixture::Good => 1.0,
        Fixture::Broken => 1.4,
    };
    let graded = fog_picture(1.0).and_then(|reference| {
        let candidate = fog_picture(gain)?;
        blur_relative(&reference, &candidate, &MetricParams::default())
    });
    match graded {
        Ok(value) => graded_metric("image_grade_blur_rel", value, IMAGE_GRADE_BLUR_REL),
        Err(_) => Metric {
            name: "image_grade_blur_rel",
            value: None,
            threshold: IMAGE_GRADE_BLUR_REL,
            status: crate::report::Status::Fail,
            note: "picture grade failed to build",
        },
    }
}

fn fog_picture(gain: f32) -> Result<Picture, String> {
    const COL_DISTANCE: [f32; 8] = [0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0];
    let mut linear = Vec::with_capacity(COL_DISTANCE.len().saturating_mul(2).saturating_mul(3));
    for _row in 0..2 {
        for &distance in &COL_DISTANCE {
            let radiance = homogeneous_pixel(distance) * gain;
            linear.push(radiance);
            linear.push(radiance * 0.85);
            linear.push(radiance * 0.55);
        }
    }
    Picture::new(8, 2, linear)
}

fn homogeneous_pixel(distance: f32) -> f32 {
    let sigma = 0.2_f32;
    let source = 0.15_f32;
    let background = 0.25_f32;
    let surviving = genos_math::transmittance_f32(sigma * distance);
    background * surviving + genos_math::homogeneous_inscatter_f32(sigma, source, distance)
}

fn max_abs<T: Copy, const N: usize>(samples: [T; N], error: impl Fn(T) -> f64) -> f64 {
    let mut max_abs = 0.0_f64;
    for sample in samples {
        max_abs = max_abs.max(error(sample));
    }
    max_abs
}
